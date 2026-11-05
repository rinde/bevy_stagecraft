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

/// What a click at `position` should be called in the script: the first of
/// [`target_alternatives`] for the widget under it, the one ranking pick
/// mode uses too (a test id, then role and label, then text, then name),
/// else the app's own target there
/// ([`AppTarget::at`](crate::AppTarget::at)), else the position.
pub(super) fn classify(world: &World, position: Vec2) -> Target {
    let subject = node_at(world, position).and_then(|hit| clicked(world, hit));
    target_alternatives(world, subject, Some(position))
        .into_iter()
        .next()
        .unwrap_or_else(|| Target::screen(position))
}

/// What a click on the node `hit` is for, when that can be named rather
/// than placed: the widget it belongs to when that has a role (see
/// [`WidgetRole`]) and where on it the click lands does not matter, else
/// `hit` itself when it is a text. A click anywhere else (on a slider's
/// track, on a bare panel) keeps its position.
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
