//! Targets: where a step acts, turned into pixels on the frame a step uses them.
//!
//! A locator is strict: several matches are `Lookup::Ambiguous`, none is `Lookup::Missing`.

mod actionable;
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

pub use self::actionable::DEFAULT_ACTION_TIMEOUT;
pub use self::role::WidgetRole;
use crate::{
    check::{Problem, check_component_type},
    extension::{AppTarget, AppTargetValue, AppValue, NO_CUSTOM_TARGET},
    probe::describe_entity,
};

/// Where a step acts: a position, or a widget named the way Playwright's locators do.
///
/// A target that matches several widgets fails and lists them, even inside `Not(..)` or
/// `Any(..)`, unless wrapped in [`Target::Nth`].
///
/// ```ron
/// MoveTo(Component("PauseButton")),
/// MoveTo(Name("pause")),
/// MoveTo(Role(role: Button, label: "Pause")),
/// MoveTo(Within(Component("Dialog"), Text("OK"))),
/// MoveTo(Nth(1, Role(role: Button, label: "Delete"))),
/// ```
#[derive(Clone, PartialEq, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Target {
    Screen(Vec2),
    Text(String),
    World(Vec3),
    App(AppTargetValue),
    /// By short type path, or the full one where the short one is shared. Checked when the
    /// script loads.
    Component(String),
    Name(String),
    /// `label` is every text node at or below the widget joined by spaces; `None` matches any.
    Role {
        role: WidgetRole,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Within(Box<Self>, Box<Self>),
    /// Counts from 0 in document order (depth first, roots by `Entity`), not layout or
    /// `UiStack` order.
    Nth(usize, Box<Self>),
}

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

#[derive(Deserialize)]
#[serde(remote = "Target", rename = "Target")]
enum Written {
    Screen(Vec2),
    Text(String),
    World(Vec3),
    App(AppTargetValue),
    Component(String),
    Name(String),
    Role {
        role: WidgetRole,
        #[serde(default)]
        label: Option<String>,
    },
    Within(Box<Target>, Box<Target>),
    Nth(usize, Box<Target>),
}

/// Refuses the old `Custom(..)` form with a hint to write `App(..)`.
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
    #[must_use]
    pub const fn screen(position: Vec2) -> Self {
        Self::Screen(position)
    }

    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    #[must_use]
    pub const fn world(position: Vec3) -> Self {
        Self::World(position)
    }

    #[must_use]
    pub fn app<T: AppTarget + Reflect + GetTypeRegistration + TypePath>(target: T) -> Self {
        Self::App(AppValue::of(target))
    }

    /// Writes the short type path; use `Target::Component` with the full one where it is shared.
    #[must_use]
    pub fn component<C: Component + TypePath>() -> Self {
        Self::Component(C::short_type_path().to_owned())
    }

    #[must_use]
    pub fn name(name: impl Into<String>) -> Self {
        Self::Name(name.into())
    }

    #[must_use]
    pub fn role(role: WidgetRole, label: impl Into<String>) -> Self {
        Self::Role {
            role,
            label: Some(label.into()),
        }
    }

    #[must_use]
    pub const fn of_role(role: WidgetRole) -> Self {
        Self::Role { role, label: None }
    }

    #[must_use]
    pub fn within(outer: Self, inner: Self) -> Self {
        Self::Within(Box::new(outer), Box::new(inner))
    }

    #[must_use]
    pub fn nth(index: usize, target: Self) -> Self {
        Self::Nth(index, Box::new(target))
    }

    /// Refuses a position inside `Within(..)` or `Nth(..)`: it names no widget to narrow down.
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

    fn type_against_nested(&mut self, types: &TypeRegistry, wrapper: &str) -> Result<(), Problem> {
        if matches!(self, Self::Screen(_) | Self::World(_)) {
            return Err(Problem::Other(format!(
                "`{wrapper}(..)` narrows down targets that name widgets, and {self:?} is a position"
            )));
        }
        self.type_against(types)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Lookup {
    Found(Entity),
    Missing(String),
    /// A check cannot hold or fail on it, not even inside `Not(..)` or `Any(..)`: the target
    /// is the mistake.
    Ambiguous(String),
    NotANode(String),
}

pub(crate) fn locate(world: &World, target: &Target) -> Lookup {
    locate::locate(world, target)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Point,
    Press,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotActionable {
    Yet(String),
    /// The target itself is the mistake, so the step fails at once.
    Never(String),
}

pub(crate) fn actionable(
    world: &World,
    target: &Target,
    action: Action,
) -> Result<ResolvedTarget, NotActionable> {
    actionable::actionable(world, target, action)
}

pub(crate) fn pressable(
    world: &World,
    widget: Entity,
    at: Vec2,
    label: &str,
) -> Result<(), NotActionable> {
    actionable::pressable(world, widget, at, label)
}

pub(crate) fn painted_topmost(world: &World, widget: Entity) -> Result<(), String> {
    actionable::painted_topmost(world, widget)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedTarget {
    pub position: Vec2,
    /// When set, the player checks after moving that the widget or a descendant is under the
    /// pointer.
    pub widget: Option<Entity>,
}

impl ResolvedTarget {
    #[must_use]
    pub const fn at(position: Vec2) -> Self {
        Self {
            position,
            widget: None,
        }
    }

    #[must_use]
    pub const fn on_widget(position: Vec2, widget: Entity) -> Self {
        Self {
            position,
            widget: Some(widget),
        }
    }
}

/// Marks UI and cameras that belong to the tooling, not the app under test; lookups skip it
/// and anything below it.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct NotUnderTest;

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

/// # Errors
///
/// Why the target cannot be found now: no match, several matches, or not a laid-out UI node.
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

/// The entity `target` names, which need not be a UI node: a bare entity holding a component
/// or `Name` is found too.
///
/// Parentless roots are counted in `Entity` order, so `Nth(..)` over bare entities is
/// predictable only when they share a parent.
///
/// # Errors
///
/// Why the target is not found: no match, or several.
pub fn entity_of(world: &World, target: &Target) -> Result<Entity, String> {
    match locate(world, target) {
        Lookup::Found(entity) => Ok(entity),
        Lookup::Missing(message) | Lookup::Ambiguous(message) | Lookup::NotANode(message) => {
            Err(message)
        }
    }
}

fn resolve_node(world: &World, target: &Target, node: Entity) -> Result<ResolvedTarget, String> {
    // Hit the button around a label, not the label.
    let widget = enclosing_button(world, node).unwrap_or(node);
    // `Name`, `Component` and `Role` can match entities no layout places, such as a camera.
    if !is_ui_node(world, widget) {
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

/// A `ScrollbarThumb` has a `ComputedNode` and transform but no `Node`: adding one would make it
/// and its `Scrollbar` overwrite each other every frame.
pub(crate) fn is_ui_node(world: &World, entity: Entity) -> bool {
    world.get::<Node>(entity).is_some()
        || (world.get::<ComputedNode>(entity).is_some()
            && world.get::<UiGlobalTransform>(entity).is_some())
}

pub(crate) fn enclosing_button(world: &World, entity: Entity) -> Option<Entity> {
    let mut current = entity;
    loop {
        if world.get::<Button>(current).is_some() {
            return Some(current);
        }
        current = world.get::<ChildOf>(current)?.parent();
    }
}

fn resolve_world(world: &World, position: Vec3) -> Result<ResolvedTarget, String> {
    let mut cameras: Vec<_> = world
        .try_query::<(Entity, &Camera, &GlobalTransform)>()
        .map_or_default(|query| {
            query
                .iter_manual(world)
                .filter(|(entity, camera, _)| camera.is_active && !is_tooling(world, *entity))
                .map(|(entity, camera, transform)| (camera.order, entity, camera, *transform))
                .collect()
        });
    cameras.sort_by_key(|(order, entity, _, _)| (*order, *entity));
    let Some((_, _, camera, transform)) = cameras.first() else {
        return Err("there is no active camera to project a world target through".into());
    };
    camera
        .world_to_viewport(transform, position)
        .map(ResolvedTarget::at)
        .map_err(|error| format!("cannot project {position} to the screen: {error}"))
}
