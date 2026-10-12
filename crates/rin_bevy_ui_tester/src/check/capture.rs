//! Values taken while a scenario runs, to compare against later: what a
//! `Capture` step measures, and where the measurements are kept.
//!
//! A [`Measure`] is written like the check that reads the same thing, without
//! its `is`. What it reads is kept as RON text and the name of its type, which
//! is all a comparison needs: it reads the text against the type of what it is
//! compared with, as it does for a literal.

use alloc::collections::BTreeMap;
use core::fmt;

use bevy::{
    ecs::{component::Component, resource::Resource, world::World},
    reflect::{TypePath, TypeRegistry},
};
use serde::{Deserialize, Serialize};

use super::{Problem, Role, field_registration, find_type, join_path, reflect, root_of};
use crate::targets::Target;

/// What a `Capture` step reads from the world, to give a name to.
///
/// In a script it is spelled like the check that reads the same thing:
///
/// ```ron
/// Capture(name: "views", value: Count(with: ["ItemView"])),
/// Capture(name: "score", value: Resource(path: "Score.0")),
/// Capture(name: "x", value: Component(target: Name("popup"), path: "UiGlobalTransform.0.translation.x")),
/// ```
///
/// In Rust the builders take the types:
///
/// ```
/// use rin_bevy_ui_tester::{Measure, Step, Target};
///
/// #[derive(bevy::prelude::Component, bevy::prelude::Reflect)]
/// struct Ghost;
/// let measure = Measure::count::<Ghost>();
/// assert_eq!(measure.to_string(), "count of `Ghost`");
/// let step = Step::capture("ghosts", measure);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Measure {
    /// How many entities of the app have every one of these components, as
    /// [`Check::Count`](super::Check::Count) counts them.
    Count {
        /// The components, by registered type path.
        with: Vec<String>,
    },
    /// A field of a reflected resource, as [`Check::Resource`](super::Check::Resource)
    /// reads it: the resource's type path, then a reflect path into it, or the
    /// type alone for the whole resource.
    Resource {
        /// The type and the field.
        path: String,
    },
    /// A field of a reflected component on the target's node or, failing
    /// that, on the nearest node above it that has the component, as
    /// [`Check::Component`](super::Check::Component) reads it.
    Component {
        /// Where the component is.
        target: Target,
        /// The component's type path, then a reflect path into it.
        path: String,
    },
}

impl Measure {
    /// How many entities have `C`, for example `Measure::count::<Ghost>()`.
    #[must_use]
    pub fn count<C: Component + TypePath>() -> Self {
        Self::Count {
            with: vec![C::short_type_path().to_owned()],
        }
    }

    /// How many entities have every component named in `with`, by type path.
    #[must_use]
    pub fn count_of(with: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Count {
            with: with.into_iter().map(Into::into).collect(),
        }
    }

    /// The field `path` of the resource `R`; an empty path is the whole
    /// resource.
    #[must_use]
    pub fn resource<R: Resource + TypePath>(path: &str) -> Self {
        Self::Resource {
            path: join_path(R::short_type_path(), path),
        }
    }

    /// The field `path` of the component `C` on `target`'s node, or on the
    /// nearest node above it that has one.
    #[must_use]
    pub fn component<C: Component + TypePath>(target: Target, path: &str) -> Self {
        Self::Component {
            target,
            path: join_path(C::short_type_path(), path),
        }
    }

    /// The target this measurement looks at, if it looks at one.
    pub(crate) const fn target_mut(&mut self) -> Option<&mut Target> {
        match self {
            Self::Component { target, .. } => Some(target),
            Self::Count { .. } | Self::Resource { .. } => None,
        }
    }

    /// Checks, from type information alone, that the types and fields this
    /// measurement names are there.
    pub(crate) fn problem(&self, types: &TypeRegistry) -> Result<(), Problem> {
        match self {
            Self::Count { with } => {
                if with.is_empty() {
                    return Err(Problem::Other(
                        "a count needs at least one component".into(),
                    ));
                }
                with.iter()
                    .try_for_each(|name| find_type(types, name, Role::Counted).map(|_| ()))
            }
            Self::Resource { path } => field_registration(types, path, Role::Resource).map(|_| ()),
            Self::Component { path, .. } => {
                field_registration(types, path, Role::Component).map(|_| ())
            }
        }
    }

    /// Reads the measurement now.
    ///
    /// # Errors
    ///
    /// Why it cannot be read: a target that is missing or ambiguous, a
    /// resource or component that is not there, a field of another variant, a
    /// value that cannot be written.
    pub(crate) fn read(&self, world: &World) -> Result<CapturedValue, String> {
        let types = reflect::app_types(world)?;
        let types = types.read();
        match self {
            Self::Count { with } => {
                let mut registrations = Vec::with_capacity(with.len());
                for name in with {
                    registrations.push(
                        find_type(&types, name, Role::Counted)
                            .map_err(|problem| problem.message())?,
                    );
                }
                if registrations.is_empty() {
                    return Err("a count needs at least one component".into());
                }
                Ok(CapturedValue {
                    ron: reflect::count(world, &registrations).to_string(),
                    type_name: "usize".into(),
                })
            }
            Self::Resource { path } => {
                let (root, field) = root_of(world, &types, None, path)??;
                reflect::capture_field(&types, root, path, field)
            }
            Self::Component { target, path } => {
                let (root, field) = root_of(world, &types, Some(target), path)??;
                reflect::capture_field(&types, root, path, field)
            }
        }
    }
}

/// What is measured, in words: ``count of `Ghost` ``, `` `Score.0` ``, or
/// `` `UiGlobalTransform.0.translation.x` of Name("popup") ``.
impl fmt::Display for Measure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Count { with } => {
                let names: Vec<String> = with.iter().map(|name| format!("`{name}`")).collect();
                write!(f, "count of {}", names.join(" and "))
            }
            Self::Resource { path } => write!(f, "`{path}`"),
            Self::Component { target, path } => write!(f, "`{path}` of {target:?}"),
        }
    }
}

/// A value a `Capture` step took: RON text and the name of its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedValue {
    /// The value, written through its reflected form, as a literal in a
    /// script would be.
    ron: String,
    /// The type's short path, for messages.
    type_name: String,
}

impl CapturedValue {
    /// The value as RON.
    pub(crate) fn ron(&self) -> &str {
        &self.ron
    }

    /// The type's short path.
    pub(crate) fn type_name(&self) -> &str {
        &self.type_name
    }

    /// A value of type `type_name` written as `ron`.
    pub(crate) const fn new(ron: String, type_name: String) -> Self {
        Self { ron, type_name }
    }
}

/// Everything captured so far in a run, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CapturedValues {
    /// The values, ordered by name.
    values: BTreeMap<String, CapturedValue>,
}

impl CapturedValues {
    /// What was captured as `name`, if anything.
    pub(crate) fn get(&self, name: &str) -> Option<&CapturedValue> {
        self.values.get(name)
    }

    /// Keeps `value` as `name`, replacing what was there.
    pub(crate) fn insert(&mut self, name: String, value: CapturedValue) {
        self.values.insert(name, value);
    }
}
