//! Working out what a click landed on while recording.

use bevy::{
    ecs::{entity::Entity, world::World},
    math::Vec2,
    ui::widget::Text,
};

use crate::{
    check::{
        pick::{node_at, target_alternatives},
        widget_of,
    },
    targets::{Target, WidgetRole},
};

pub(super) fn classify(world: &World, position: Vec2) -> Target {
    let subject = node_at(world, position).and_then(|hit| clicked(world, hit));
    target_alternatives(world, subject, Some(position))
        .into_iter()
        .next()
        .unwrap_or_else(|| Target::screen(position))
}

/// Positional roles (a slider) and non-text nodes return `None` so the click keeps its position.
fn clicked(world: &World, hit: Entity) -> Option<Entity> {
    let owner = widget_of(world, hit);
    let role = world
        .get_entity(owner)
        .ok()
        .and_then(|entity| WidgetRole::of(&entity));
    match role {
        Some(role) if !role.is_positional() => Some(owner),
        _ => world.get::<Text>(hit).map(|_| hit),
    }
}
