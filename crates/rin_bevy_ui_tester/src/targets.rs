//! Finding the screen position to act on.
//!
//! Targets are plain data. They are turned into pixels on the frame a step
//! uses them, so layout changes between steps are followed.

use bevy::{
    camera::Camera,
    ecs::{component::Component, entity::Entity, hierarchy::ChildOf, world::World},
    math::{Vec2, Vec3},
    reflect::{GetTypeRegistration, Reflect, TypePath, TypeRegistry},
    transform::components::GlobalTransform,
    ui::{ComputedNode, UiGlobalTransform, widget::Text},
    ui_widgets::Button,
};
use serde::{Deserialize, Serialize, de::Error as _};

use crate::{
    check::Problem,
    extension::{AppTarget, AppTargetValue, AppValue, NO_CUSTOM_TARGET},
};

/// Where a step acts.
///
/// Targets the crate knows nothing about are the app's own: values of the
/// one enum it marks with `#[reflect(AppTarget)]` (see [`AppTarget`]),
/// written inside `App(..)`.
///
/// It is `Reflect`, so a variant of the app's check type (see
/// [`crate::AppCheck`]) can take one.
#[derive(Debug, Clone, PartialEq, Serialize, Reflect)]
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
}

/// The old `Custom(resolver: .., argument: ..)` is refused saying what to
/// write instead.
impl<'de> Deserialize<'de> for Target {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Written::deserialize(deserializer).map_err(|error| {
            let message = error.to_string();
            if message.contains("variant named `Custom`") {
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

    /// Reads an app target as the app's target type in `types`; any other
    /// target is left as it is.
    pub(crate) fn type_against(&mut self, types: &TypeRegistry) -> Result<(), Problem> {
        match self {
            Self::App(value) => value.type_against(types),
            Self::Screen(_) | Self::Text(_) | Self::World(_) => Ok(()),
        }
    }
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
/// # Errors
///
/// A message saying why the target cannot be found right now.
pub fn resolve(world: &World, target: &Target) -> Result<ResolvedTarget, String> {
    match target {
        Target::Screen(position) => Ok(ResolvedTarget::at(*position)),
        Target::Text(text) => resolve_text(world, text),
        Target::World(position) => resolve_world(world, *position),
        Target::App(value) => value.resolve(world),
    }
}

/// The one node of the app showing exactly `text`.
///
/// # Errors
///
/// Says that no node shows it, or that several do.
pub(crate) fn text_node(world: &World, text: &str) -> Result<Entity, String> {
    let mut found: Vec<Entity> = world
        .try_query::<(Entity, &Text)>()
        .map(|query| {
            query
                .iter_manual(world)
                .filter(|(entity, shown)| shown.0 == text && !is_tooling(world, *entity))
                .map(|(entity, _)| entity)
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    match found.as_slice() {
        [entity] => Ok(*entity),
        [] => Err(format!("no UI node shows \"{text}\"")),
        _ => Err(format!(
            "{} UI nodes show \"{text}\"; the target is ambiguous",
            found.len()
        )),
    }
}

/// Finds the node showing `text` and the middle of it.
fn resolve_text(world: &World, text: &str) -> Result<ResolvedTarget, String> {
    let entity = &text_node(world, text)?;
    // A label is usually a child of the button it names; the button is what
    // the user wants to hit.
    let widget = enclosing_button(world, *entity).unwrap_or(*entity);
    let (Some(node), Some(transform)) = (
        world.get::<ComputedNode>(widget),
        world.get::<UiGlobalTransform>(widget),
    ) else {
        return Err(format!("\"{text}\" has not been laid out yet"));
    };
    // The transform is in physical pixels; pointer positions are logical.
    let position = transform.affine().translation * node.inverse_scale_factor;
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
