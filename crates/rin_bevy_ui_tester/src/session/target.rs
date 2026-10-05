//! Working out what a click landed on while recording.

use bevy::{
    ecs::{entity::Entity, hierarchy::Children, world::World},
    math::Vec2,
    ui::{ComputedNode, UiGlobalTransform, widget::Text},
};

use crate::{
    extension::app_target_at,
    probe::hovered_entities,
    targets::{Target, enclosing_button, is_tooling, resolve},
};

/// What a click at `position` should be called in the script: the label of
/// the UI node under it, else the app's own target there
/// ([`AppTarget::at`](crate::AppTarget::at)), else the position.
pub(super) fn classify(world: &World, position: Vec2) -> Target {
    label_target(world, position)
        .or_else(|| app_target_at(world, position))
        .unwrap_or_else(|| Target::screen(position))
}

/// The first text at or below `entity`, depth first.
fn label_within(world: &World, entity: Entity) -> Option<String> {
    if let Some(text) = world.get::<Text>(entity) {
        return Some(text.0.clone());
    }
    let children = world.get::<Children>(entity)?;
    children
        .iter()
        .find_map(|child| label_within(world, *child))
}

/// `Target::text(..)` for the labelled node under `position`, if there is one
/// and the label really leads back to it.
///
/// A label is only used when looking it up gives the same widget again, which
/// also means it is unique and not the gallery's own.
fn label_target(world: &World, position: Vec2) -> Option<Target> {
    let hit = hovered_entities(world)
        .into_iter()
        .find(|entity| !is_tooling(world, *entity))?;
    // The widget a click on `hit` is meant for: its button, or the text
    // itself.
    let widget = enclosing_button(world, hit).or_else(|| world.get::<Text>(hit).map(|_| hit))?;
    let label = label_within(world, widget)?;
    // The hover map is from before this update's cursor moves; make sure the
    // click is really on the node.
    let node = world.get::<ComputedNode>(widget)?;
    let transform = world.get::<UiGlobalTransform>(widget)?;
    if !node.contains_point(*transform, position / node.inverse_scale_factor) {
        return None;
    }
    let target = Target::text(label);
    (resolve(world, &target).ok()?.widget == Some(widget)).then_some(target)
}
