//! Finding the screen position to act on.
//!
//! Targets are plain data. They are turned into pixels on the frame a step
//! uses them, so layout changes between steps are followed.

use alloc::{collections::BTreeMap, sync::Arc};
use core::fmt;

use bevy::{
    camera::Camera,
    ecs::{entity::Entity, hierarchy::ChildOf, resource::Resource, world::World},
    math::{Vec2, Vec3},
    transform::components::GlobalTransform,
    ui::{ComputedNode, UiGlobalTransform, widget::Text},
    ui_widgets::Button,
};
use serde::{Deserialize, Serialize};

/// Where a step acts.
///
/// App-defined targets are [`Target::Custom`]: a resolver name (the tag) and a
/// string payload, which survive a round trip through RON unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Target {
    /// A logical pixel position in the window.
    Screen(Vec2),
    /// The UI node showing exactly this text, or a button above it. Works for
    /// any Bevy UI toolkit.
    Text(String),
    /// A world position, seen through the first active camera.
    World(Vec3),
    /// A target an app defines itself: the resolver registered in
    /// [`TargetResolvers`] under `resolver` is asked about `argument`.
    Custom {
        /// Name the resolver is registered under.
        resolver: String,
        /// What to find, in a form the resolver understands.
        argument: String,
    },
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

    /// An app-defined target.
    #[must_use]
    pub fn custom(resolver: impl Into<String>, argument: impl Into<String>) -> Self {
        Self::Custom {
            resolver: resolver.into(),
            argument: argument.into(),
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

/// What an app-defined resolver is: the world and an argument in, a pixel out.
type ResolverFn = dyn Fn(&World, &str) -> Result<ResolvedTarget, String> + Send + Sync;

/// App-defined target resolvers: the extension point for targets this crate
/// knows nothing about, such as a lattice point or a panel action.
#[derive(Resource, Default, Clone)]
pub struct TargetResolvers {
    /// Resolvers by name, ordered so nothing depends on hash order.
    resolvers: BTreeMap<String, Arc<ResolverFn>>,
}

impl fmt::Debug for TargetResolvers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.resolvers.keys()).finish()
    }
}

impl TargetResolvers {
    /// Registers `resolver` under `name`, replacing any earlier one.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        resolver: impl Fn(&World, &str) -> Result<ResolvedTarget, String> + Send + Sync + 'static,
    ) {
        self.resolvers.insert(name.into(), Arc::new(resolver));
    }
}

impl TargetResolvers {
    /// Whether a resolver is registered under `name`.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.resolvers.contains_key(name)
    }

    /// Adds every resolver of `other`, replacing those with the same name.
    pub fn extend_from(&mut self, other: &Self) {
        self.resolvers.extend(
            other
                .resolvers
                .iter()
                .map(|(name, resolver)| (name.clone(), Arc::clone(resolver))),
        );
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
        Target::Custom { resolver, argument } => {
            let resolver = world
                .get_resource::<TargetResolvers>()
                .and_then(|resolvers| resolvers.resolvers.get(resolver))
                .ok_or_else(|| format!("no target resolver named `{resolver}`"))?;
            resolver(world, argument)
        }
    }
}

/// Finds the node showing `text` and the middle of it.
fn resolve_text(world: &World, text: &str) -> Result<ResolvedTarget, String> {
    let mut found: Vec<Entity> = world
        .try_query::<(Entity, &Text)>()
        .map(|query| {
            query
                .iter_manual(world)
                .filter(|(_, shown)| shown.0 == text)
                .map(|(entity, _)| entity)
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    let [entity] = found.as_slice() else {
        return Err(if found.is_empty() {
            format!("no UI node shows \"{text}\"")
        } else {
            format!(
                "{} UI nodes show \"{text}\"; the target is ambiguous",
                found.len()
            )
        });
    };
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
fn enclosing_button(world: &World, entity: Entity) -> Option<Entity> {
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
                .filter(|(_, camera, _)| camera.is_active)
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
