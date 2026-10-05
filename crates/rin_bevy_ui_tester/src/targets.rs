//! Finding the screen position, or the widget, to act on.
//!
//! Targets are plain data. They are turned into pixels on the frame a step
//! uses them, so layout changes between steps are followed. Submodules:
//! `locate` (finding the one node a target names, strictly, with close
//! matches for a miss and a list for an ambiguity) and `role` (widget roles
//! read from `bevy_ui_widgets`).

mod locate;
mod role;

use core::fmt;

use bevy::{
    camera::Camera,
    ecs::{component::Component, entity::Entity, hierarchy::ChildOf, world::World},
    math::{Vec2, Vec3},
    reflect::{
        GetTypeRegistration, Reflect, ReflectDeserialize, ReflectSerialize, TypePath, TypeRegistry,
    },
    transform::components::GlobalTransform,
    ui::{ComputedNode, Node, UiGlobalTransform},
    ui_widgets::Button,
};
use serde::{Deserialize, Serialize, de::Error as _};

pub use self::role::WidgetRole;
use crate::{
    check::{Problem, check_component_type},
    extension::{AppTarget, AppTargetValue, AppValue, NO_CUSTOM_TARGET},
    probe::describe_entity,
};

/// Where a step acts.
///
/// Besides positions, a target can name a widget the way Playwright's
/// locators do: by a reflected marker component (a test id), by Bevy's
/// `Name`, by role and label, by its text, inside another target, or as the
/// nth of several. A target that names a widget is strict: if it matches
/// several, the step or check fails and lists them, unless it is wrapped in
/// [`Target::Nth`]; if it matches none, the failure suggests close matches.
/// An ambiguous target fails a check even inside `Not(..)` or `Any(..)`,
/// where a plain miss would not.
///
/// ```ron
/// MoveTo(Component("PauseButton")),
/// MoveTo(Name("pause")),
/// MoveTo(Role(role: Button, label: "Pause")),
/// MoveTo(Within(Component("Dialog"), Text("OK"))),
/// MoveTo(Nth(1, Role(role: Button, label: "Delete"))),
/// ```
///
/// Targets the crate knows nothing about are the app's own: values of the
/// one enum it marks with `#[reflect(AppTarget)]` (see [`AppTarget`]),
/// written inside `App(..)`.
///
/// It is `Reflect` (as an opaque value, read and written as RON), so a
/// variant of the app's check type (see [`crate::AppCheck`]) can take one.
#[derive(Clone, PartialEq, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Target {
    /// A logical pixel position in the window.
    Screen(Vec2),
    /// The UI node showing exactly this text, or a button above it. Works for
    /// any Bevy UI toolkit.
    Text(String),
    /// A world position, seen through the first active camera.
    World(Vec3),
    /// One of the app's own targets: a value of the type the app marks with
    /// `#[reflect(AppTarget)]` (see [`AppTarget`]). Written as
    /// `App(Station("Alpha"))`.
    App(AppTargetValue),
    /// The entity with this reflected component, by its short type path (or
    /// its full one where the short one names several types): Playwright's
    /// `getByTestId`. A unit struct such as `struct PauseButton;` with
    /// `#[derive(Component, Reflect)]` and `#[reflect(Component)]` makes a
    /// good test id. The type is checked when the script loads.
    Component(String),
    /// The entity whose Bevy `Name` is exactly this.
    Name(String),
    /// A widget of this role (see [`WidgetRole`]) whose text, every text
    /// node at or below it joined by spaces, is exactly `label`; any label
    /// when there is none. Playwright's `getByRole(.., { name })`.
    Role {
        /// The kind of widget.
        role: WidgetRole,
        /// The text it shows, exactly.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The match of the second target that lies below a match of the first.
    Within(Box<Self>, Box<Self>),
    /// The match at this index, counting from 0 in the order the UI
    /// hierarchy reads: the explicit way out of a target that matches
    /// several. That order is depth first, roots sorted by `Entity` and
    /// children by their index among their parent's children: the same on
    /// every run, but not the order of layout on screen nor of `UiStack`.
    Nth(usize, Box<Self>),
}

/// The derived form for the variants that have always been there; a role
/// reads as the file writes it, `Role(role: Button, label: "Pause")`.
impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Screen(position) => f.debug_tuple("Screen").field(position).finish(),
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            Self::World(position) => f.debug_tuple("World").field(position).finish(),
            Self::App(value) => f.debug_tuple("App").field(value).finish(),
            Self::Component(name) => f.debug_tuple("Component").field(name).finish(),
            Self::Name(name) => f.debug_tuple("Name").field(name).finish(),
            Self::Role { role, label: None } => write!(f, "Role(role: {role:?})"),
            Self::Role {
                role,
                label: Some(label),
            } => write!(f, "Role(role: {role:?}, label: {label:?})"),
            Self::Within(outer, inner) => {
                f.debug_tuple("Within").field(outer).field(inner).finish()
            }
            Self::Nth(index, inner) => f.debug_tuple("Nth").field(index).field(inner).finish(),
        }
    }
}

/// [`Target`] as serde's derive reads it, which [`Target`]'s own impl adds
/// the hint for the old custom target to.
#[derive(Deserialize)]
#[serde(remote = "Target", rename = "Target")]
enum Written {
    /// See [`Target::Screen`].
    Screen(Vec2),
    /// See [`Target::Text`].
    Text(String),
    /// See [`Target::World`].
    World(Vec3),
    /// See [`Target::App`].
    App(AppTargetValue),
    /// See [`Target::Component`].
    Component(String),
    /// See [`Target::Name`].
    Name(String),
    /// See [`Target::Role`].
    Role {
        /// See [`Target::Role`].
        role: WidgetRole,
        /// See [`Target::Role`].
        #[serde(default)]
        label: Option<String>,
    },
    /// See [`Target::Within`].
    Within(Box<Target>, Box<Target>),
    /// See [`Target::Nth`].
    Nth(usize, Box<Target>),
}

/// The old `Custom(resolver: .., argument: ..)` is refused saying what to
/// write instead.
impl<'de> Deserialize<'de> for Target {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Written::deserialize(deserializer).map_err(|error| {
            let message = error.to_string();
            if message.contains("variant named `Custom`") && !message.contains(NO_CUSTOM_TARGET) {
                D::Error::custom(format!("{message}: {NO_CUSTOM_TARGET}"))
            } else {
                error
            }
        })
    }
}

impl Target {
    /// A logical pixel position in the window.
    #[must_use]
    pub const fn screen(position: Vec2) -> Self {
        Self::Screen(position)
    }

    /// The UI node showing exactly `text`.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    /// A world position.
    #[must_use]
    pub const fn world(position: Vec3) -> Self {
        Self::World(position)
    }

    /// One of the app's own targets, for example
    /// `Target::app(RailTarget::Station("Alpha".into()))`.
    #[must_use]
    pub fn app<T: AppTarget + Reflect + GetTypeRegistration + TypePath>(target: T) -> Self {
        Self::App(AppValue::of(target))
    }

    /// The entity with the reflected component `C`, a test id, for example
    /// `Target::component::<PauseButton>()`.
    #[must_use]
    pub fn component<C: Component + TypePath>() -> Self {
        Self::Component(C::short_type_path().to_owned())
    }

    /// The entity whose Bevy `Name` is exactly `name`.
    #[must_use]
    pub fn name(name: impl Into<String>) -> Self {
        Self::Name(name.into())
    }

    /// The widget of `role` labelled exactly `label`, for example
    /// `Target::role(WidgetRole::Button, "Pause")`.
    #[must_use]
    pub fn role(role: WidgetRole, label: impl Into<String>) -> Self {
        Self::Role {
            role,
            label: Some(label.into()),
        }
    }

    /// The widget of `role`, whatever it shows, for example the one slider.
    #[must_use]
    pub const fn of_role(role: WidgetRole) -> Self {
        Self::Role { role, label: None }
    }

    /// The match of `inner` that lies below a match of `outer`.
    #[must_use]
    pub fn within(outer: Self, inner: Self) -> Self {
        Self::Within(Box::new(outer), Box::new(inner))
    }

    /// The match of `target` at `index`, counting from 0 in the order the UI
    /// hierarchy reads.
    #[must_use]
    pub fn nth(index: usize, target: Self) -> Self {
        Self::Nth(index, Box::new(target))
    }

    /// Reads an app target as the app's target type in `types` and checks
    /// that a component target names a reflected component there, in this
    /// target and every target inside it. A position inside `Within(..)` or
    /// `Nth(..)` is refused: it names no widget to narrow down.
    pub(crate) fn type_against(&mut self, types: &TypeRegistry) -> Result<(), Problem> {
        match self {
            Self::App(value) => value.type_against(types),
            Self::Component(name) => check_component_type(types, name),
            Self::Within(outer, inner) => {
                outer.type_against_nested(types, "Within")?;
                inner.type_against_nested(types, "Within")
            }
            Self::Nth(_, inner) => inner.type_against_nested(types, "Nth"),
            Self::Screen(_)
            | Self::Text(_)
            | Self::World(_)
            | Self::Name(_)
            | Self::Role { .. } => Ok(()),
        }
    }

    /// [`Target::type_against`] for a target inside `Within(..)` or `Nth(..)`
    /// (`wrapper`), which must name a widget.
    fn type_against_nested(&mut self, types: &TypeRegistry, wrapper: &str) -> Result<(), Problem> {
        if matches!(self, Self::Screen(_) | Self::World(_)) {
            return Err(Problem::Other(format!(
                "`{wrapper}(..)` narrows down targets that name widgets, and {self:?} is a position"
            )));
        }
        self.type_against(types)
    }
}

/// What looking a target up for a node came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// The one node the target names.
    Found(Entity),
    /// No node matches right now; that may change.
    Missing(String),
    /// More than one node matches, listed in the message. Unlike a miss, a
    /// check cannot hold or fail on it, so not even inside `Not(..)` or
    /// `Any(..)` does it pass: the target is the mistake.
    Ambiguous(String),
    /// The target can never name a node: it is a position, or it names a
    /// type that is not a reflected component.
    NotANode(String),
}

/// The one node `target` names, found strictly: a target that matches
/// nothing says what is close to it, one that matches several lists them.
/// For one of the app's own targets, the widget its
/// [`AppTarget`] reports.
pub(crate) fn locate(world: &World, target: &Target) -> Lookup {
    locate::locate(world, target)
}

/// A target worked out to a pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedTarget {
    /// Logical pixel position in the window.
    pub position: Vec2,
    /// The widget at that position, if the target names one. When set, the
    /// player checks after moving that the widget, or a descendant, is under
    /// the pointer.
    pub widget: Option<Entity>,
}

impl ResolvedTarget {
    /// A bare position with no widget to confirm.
    #[must_use]
    pub const fn at(position: Vec2) -> Self {
        Self {
            position,
            widget: None,
        }
    }

    /// A position that belongs to `widget`.
    #[must_use]
    pub const fn on_widget(position: Vec2, widget: Entity) -> Self {
        Self {
            position,
            widget: Some(widget),
        }
    }
}

/// Marks UI and cameras that belong to the tooling (the cursor overlay, the
/// gallery's window), not to the app under test.
///
/// Target lookups, [`crate::Probe::has_text`] and the world-target camera skip
/// anything that carries it or sits below something that does, so a script
/// that clicks "Reset" never lands on a gallery button of the same name.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct NotUnderTest;

/// Whether `entity` is, or lies below, something marked [`NotUnderTest`].
pub(crate) fn is_tooling(world: &World, entity: Entity) -> bool {
    let mut current = entity;
    loop {
        if world.get::<NotUnderTest>(current).is_some() {
            return true;
        }
        match world.get::<ChildOf>(current) {
            Some(parent) => current = parent.parent(),
            None => return false,
        }
    }
}

/// Works `target` out to a pixel using the current state of `world`.
///
/// A target that names a widget is found strictly (see [`Target`]); the
/// pixel is the middle of its node or of the button above it, which is
/// what the user wants to hit when a target names a button's label.
///
/// # Errors
///
/// A message saying why the target cannot be found right now: nothing
/// matches it (with close matches), several do (listing them), or it is not
/// laid out yet, or it is not a UI node at all.
pub fn resolve(world: &World, target: &Target) -> Result<ResolvedTarget, String> {
    match target {
        Target::Screen(position) => Ok(ResolvedTarget::at(*position)),
        Target::World(position) => resolve_world(world, *position),
        Target::App(value) => value.resolve(world),
        Target::Text(_)
        | Target::Component(_)
        | Target::Name(_)
        | Target::Role { .. }
        | Target::Within(..)
        | Target::Nth(..) => match locate(world, target) {
            Lookup::Found(node) => resolve_node(world, target, node),
            Lookup::Missing(message) | Lookup::Ambiguous(message) | Lookup::NotANode(message) => {
                Err(message)
            }
        },
    }
}

/// The middle of `node`, or of the button above it.
fn resolve_node(world: &World, target: &Target, node: Entity) -> Result<ResolvedTarget, String> {
    // A label is usually a child of the button it names; the button is what
    // the user wants to hit.
    let widget = enclosing_button(world, node).unwrap_or(node);
    // `Name`, `Component` and `Role` can match any entity, a camera say,
    // which no layout will ever place.
    if world.get::<Node>(widget).is_none() {
        return Err(format!(
            "{target:?} names {} ({widget}), which is not a UI node",
            describe_entity(world, widget)
        ));
    }
    let (Some(computed), Some(transform)) = (
        world.get::<ComputedNode>(widget),
        world.get::<UiGlobalTransform>(widget),
    ) else {
        return Err(format!("{target:?} has not been laid out yet"));
    };
    // The transform is in physical pixels; pointer positions are logical.
    let position = transform.affine().translation * computed.inverse_scale_factor;
    Ok(ResolvedTarget::on_widget(position, widget))
}

/// The nearest `Button` that is `entity` or above it.
pub(crate) fn enclosing_button(world: &World, entity: Entity) -> Option<Entity> {
    let mut current = entity;
    loop {
        if world.get::<Button>(current).is_some() {
            return Some(current);
        }
        current = world.get::<ChildOf>(current)?.parent();
    }
}

/// Projects `position` through the first active camera.
fn resolve_world(world: &World, position: Vec3) -> Result<ResolvedTarget, String> {
    let mut cameras: Vec<_> = world
        .try_query::<(Entity, &Camera, &GlobalTransform)>()
        .map(|query| {
            query
                .iter_manual(world)
                .filter(|(entity, camera, _)| camera.is_active && !is_tooling(world, *entity))
                .map(|(entity, camera, transform)| (camera.order, entity, camera, *transform))
                .collect()
        })
        .unwrap_or_default();
    cameras.sort_by_key(|(order, entity, _, _)| (*order, *entity));
    let Some((_, _, camera, transform)) = cameras.first() else {
        return Err("there is no active camera to project a world target through".into());
    };
    camera
        .world_to_viewport(transform, position)
        .map(ResolvedTarget::at)
        .map_err(|error| format!("cannot project {position} to the screen: {error}"))
}
