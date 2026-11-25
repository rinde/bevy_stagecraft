//! Values a `Capture` step measures, to compare against later. Kept as RON
//! text and a type name; a comparison reads the text against the other side.

use core::fmt;

use bevy::{
    ecs::{component::Component, resource::Resource, world::World},
    reflect::{TypePath, TypeRegistry},
};
use serde::{Deserialize, Serialize};

use super::{
    CapturedValue, Problem, Role, Unit, field_registration, find_type, join_path, reflect, root_of,
};
use crate::targets::Target;

/// What a `Capture` step reads from the world, named for later comparison.
///
/// ```ron
/// Capture(name: "views", value: Count(with: ["ItemView"])),
/// Capture(name: "score", value: Resource(path: "Score.0")),
/// Capture(name: "x", value: Component(target: Name("popup"), path: "UiGlobalTransform.0.translation.x")),
/// ```
///
/// ```
/// use bevy_stagecraft::{Measure, Step, Target};
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
    /// Entities having every one of these components.
    Count { with: Vec<String> },
    /// A field of a reflected resource, or all of it when `path` is the type
    /// alone.
    Resource {
        path: String,
        #[serde(default, skip_serializing_if = "Unit::is_physical")]
        unit: Unit,
    },
    /// A field of a reflected component on the target's node, else on the
    /// nearest ancestor that has it.
    Component {
        target: Target,
        path: String,
        #[serde(default, skip_serializing_if = "Unit::is_physical")]
        unit: Unit,
    },
}

impl Measure {
    #[must_use]
    pub fn count<C: Component + TypePath>() -> Self {
        Self::Count {
            with: vec![C::short_type_path().to_owned()],
        }
    }

    #[must_use]
    pub fn count_of(with: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Count {
            with: with.into_iter().map(Into::into).collect(),
        }
    }

    #[must_use]
    pub fn resource<R: Resource + TypePath>(path: &str) -> Self {
        Self::Resource {
            path: join_path(R::short_type_path(), path),
            unit: Unit::Physical,
        }
    }

    #[must_use]
    pub fn component<C: Component + TypePath>(target: Target, path: &str) -> Self {
        Self::Component {
            target,
            path: join_path(C::short_type_path(), path),
            unit: Unit::Physical,
        }
    }

    /// Divides physical values by the primary window's scale factor; a count
    /// is unchanged.
    #[must_use]
    pub const fn logical(mut self) -> Self {
        match &mut self {
            Self::Resource { unit, .. } | Self::Component { unit, .. } => *unit = Unit::Logical,
            Self::Count { .. } => {}
        }
        self
    }

    pub(crate) const fn target_mut(&mut self) -> Option<&mut Target> {
        match self {
            Self::Component { target, .. } => Some(target),
            Self::Count { .. } | Self::Resource { .. } => None,
        }
    }

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
            Self::Resource { path, unit } => {
                let field = field_registration(types, path, Role::Resource)?;
                if unit.is_logical() {
                    reflect::check_logical(field)?;
                }
                Ok(())
            }
            Self::Component { path, unit, .. } => {
                let field = field_registration(types, path, Role::Component)?;
                if unit.is_logical() {
                    reflect::check_logical(field)?;
                }
                Ok(())
            }
        }
    }

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
                Ok(CapturedValue::new(
                    reflect::count(world, &registrations).to_string(),
                    "usize".into(),
                ))
            }
            Self::Resource { path, unit } => {
                let (root, field) = root_of(world, &types, None, path)??;
                reflect::capture_field(world, &types, root, path, field, *unit)
            }
            Self::Component { target, path, unit } => {
                let (root, field) = root_of(world, &types, Some(target), path)??;
                reflect::capture_field(world, &types, root, path, field, *unit)
            }
        }
    }
}

impl fmt::Display for Measure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Count { with } => {
                let names: Vec<String> = with.iter().map(|name| format!("`{name}`")).collect();
                write!(f, "count of {}", names.join(" and "))
            }
            Self::Resource { path, unit } => {
                if unit.is_logical() {
                    write!(f, "logical `{path}`")
                } else {
                    write!(f, "`{path}`")
                }
            }
            Self::Component { target, path, unit } => {
                if unit.is_logical() {
                    write!(f, "logical `{path}` of {target:?}")
                } else {
                    write!(f, "`{path}` of {target:?}")
                }
            }
        }
    }
}
