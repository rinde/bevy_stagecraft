//! Actionability, as Playwright's auto-waiting decides it: a target must exist, be visible,
//! be stable, be enabled (for a press) and receive the pointer.
//!
//! Positions name no widget, so none of this applies. Stability counts frames, never time.

use bevy::{
    camera::visibility::InheritedVisibility,
    color::Alpha,
    ecs::{entity::Entity, hierarchy::ChildOf, query::With, world::World},
    math::Vec2,
    picking::Pickable,
    ui::{
        BackgroundColor, BackgroundGradient, CalculatedClip, ComputedNode, ComputedUiTargetCamera,
        InteractionDisabled, UiGlobalTransform, UiStack,
        widget::{ImageNode, Text},
    },
    window::{PrimaryWindow, Window},
};

use super::{
    Action, Lookup, NotActionable, ResolvedTarget, Target, enclosing_button, is_tooling,
    is_ui_node, locate, resolve,
};
use crate::probe::{describe_entity, is_within};

/// One second at 60 frames per second.
pub const DEFAULT_ACTION_TIMEOUT: u16 = 60;

pub(super) fn actionable(
    world: &World,
    target: &Target,
    action: Action,
) -> Result<ResolvedTarget, NotActionable> {
    let resolved = find(world, target)?;
    let Some(widget) = resolved.widget else {
        return Ok(resolved);
    };
    let label = format!("{target:?}");
    visible(world, widget, resolved.position, &label)?;
    stable(world, widget, &label)?;
    if action == Action::Press {
        enabled(world, widget, &label)?;
    }
    receives_pointer(world, widget, resolved.position, &label)?;
    Ok(resolved)
}

pub(super) fn pressable(
    world: &World,
    widget: Entity,
    at: Vec2,
    label: &str,
) -> Result<(), NotActionable> {
    if world.get_entity(widget).is_err() {
        return Err(NotActionable::Never(format!(
            "{label} is gone: it was despawned after the pointer moved to it"
        )));
    }
    enabled(world, widget, label)?;
    receives_pointer(world, widget, at, label)
}

fn find(world: &World, target: &Target) -> Result<ResolvedTarget, NotActionable> {
    match target {
        Target::Screen(_) | Target::World(_) => {
            resolve(world, target).map_err(NotActionable::Never)
        }
        Target::App(value) => value.resolve(world).map_err(NotActionable::Yet),
        Target::Text(_)
        | Target::Component(_)
        | Target::Name(_)
        | Target::Role { .. }
        | Target::Within(..)
        | Target::Nth(..) => match locate(world, target) {
            Lookup::Found(node) => {
                let widget = enclosing_button(world, node).unwrap_or(node);
                if !is_ui_node(world, widget) {
                    return Err(NotActionable::Never(format!(
                        "{target:?} names {} ({widget}), which is not a UI node",
                        describe_entity(world, widget)
                    )));
                }
                super::resolve_node(world, target, node).map_err(NotActionable::Yet)
            }
            Lookup::Missing(message) => Err(NotActionable::Yet(message)),
            Lookup::Ambiguous(message) | Lookup::NotANode(message) => {
                Err(NotActionable::Never(message))
            }
        },
    }
}

fn visible(world: &World, widget: Entity, at: Vec2, label: &str) -> Result<(), NotActionable> {
    let not_visible = |why: &str| NotActionable::Yet(format!("{label} is not visible: {why}"));
    if world
        .get::<InheritedVisibility>(widget)
        .is_some_and(|visibility| !visibility.get())
    {
        return Err(not_visible("it is hidden"));
    }
    let Some(node) = world.get::<ComputedNode>(widget) else {
        return Err(not_visible("it has not been laid out yet"));
    };
    if node.size().cmple(Vec2::ZERO).any() {
        return Err(not_visible("it takes up no room on screen"));
    }
    // Clip rectangles are in physical pixels; positions are logical.
    let physical = at / node.inverse_scale_factor;
    if world
        .get::<CalculatedClip>(widget)
        .is_some_and(|clip| !clip.contains_point(physical))
    {
        return Err(not_visible("its middle is clipped away"));
    }
    let window = world
        .try_query_filtered::<&Window, With<PrimaryWindow>>()
        .and_then(|query| query.iter_manual(world).next().map(Window::size));
    if let Some(size) = window
        && (at.cmplt(Vec2::ZERO).any() || at.cmpge(size).any())
    {
        return Err(not_visible("its middle lies outside the window"));
    }
    Ok(())
}

/// Change detection says whether the widget moved. The last frame is everything since the
/// player's own system last ran.
fn stable(world: &World, widget: Entity, label: &str) -> Result<(), NotActionable> {
    let Ok(entity) = world.get_entity(widget) else {
        return Err(NotActionable::Yet(format!("{label} is gone")));
    };
    let (last_run, this_run) = (world.last_change_tick(), world.read_change_tick());
    let moved = entity
        .get_change_ticks::<ComputedNode>()
        .is_some_and(|ticks| ticks.is_changed(last_run, this_run))
        || entity
            .get_change_ticks::<UiGlobalTransform>()
            .is_some_and(|ticks| ticks.is_changed(last_run, this_run));
    if moved {
        Err(NotActionable::Yet(format!(
            "{label} is not stable: it moved or changed size during the last frame"
        )))
    } else {
        Ok(())
    }
}

/// Ancestors count: an app that disables a whole panel means everything in it.
fn enabled(world: &World, widget: Entity, label: &str) -> Result<(), NotActionable> {
    let mut current = widget;
    loop {
        if world.get::<InteractionDisabled>(current).is_some() {
            return Err(NotActionable::Yet(if current == widget {
                format!("{label} is disabled")
            } else {
                format!(
                    "{label} is disabled: {} above it is",
                    describe_entity(world, current)
                )
            }));
        }
        match world.get::<ChildOf>(current) {
            Some(parent) => current = parent.parent(),
            None => return Ok(()),
        }
    }
}

/// Walks the UI stack front to back as Bevy's UI picking does, honoring
/// `Pickable::should_block_lower`.
fn receives_pointer(
    world: &World,
    widget: Entity,
    at: Vec2,
    label: &str,
) -> Result<(), NotActionable> {
    let Some(stack) = world.get_resource::<UiStack>() else {
        return Ok(());
    };
    let widget = stacked(world, stack, widget);
    let camera = camera_of(world, widget);
    for node in stack.uinodes.iter().rev().copied() {
        if !is_hit(world, node, at) || camera_of(world, node) != camera {
            continue;
        }
        let pickable = world.get::<Pickable>(node).copied().unwrap_or_default();
        let ours = is_within(world, node, widget);
        if ours && pickable.is_hoverable {
            return Ok(());
        }
        if pickable.should_block_lower {
            return Err(NotActionable::Yet(if ours {
                format!(
                    "{label} does not receive the pointer: a part of it that ignores the pointer blocks it"
                )
            } else {
                format!(
                    "{label} does not receive the pointer: it is covered by {}",
                    describe_entity(world, node)
                )
            }));
        }
    }
    Err(NotActionable::Yet(format!(
        "{label} does not receive the pointer: nothing at its middle takes it"
    )))
}

/// Asks what the person sees, not what the pointer reaches: a popup that lets the pointer
/// through still covers. Nodes that paint nothing cover nothing, or an empty full-window
/// container would cover the whole UI.
pub(super) fn painted_topmost(world: &World, widget: Entity) -> Result<(), String> {
    let (Some(computed), Some(transform)) = (
        world.get::<ComputedNode>(widget),
        world.get::<UiGlobalTransform>(widget),
    ) else {
        return Err("it has not been laid out yet".into());
    };
    let Some(stack) = world.get_resource::<UiStack>() else {
        return Err("the UI has not been stacked yet".into());
    };
    // The transform is in physical pixels; the hit tests take logical ones.
    let at = transform.affine().translation * computed.inverse_scale_factor;
    let widget = stacked(world, stack, widget);
    let camera = camera_of(world, widget);
    for node in stack.uinodes.iter().rev().copied() {
        if !is_hit(world, node, at) || camera_of(world, node) != camera {
            continue;
        }
        if is_within(world, node, widget) {
            return Ok(());
        }
        if paints(world, node) {
            return Err(format!("it is covered by {}", describe_entity(world, node)));
        }
    }
    Err("nothing is drawn at its middle (it is hidden, clipped or outside the window)".into())
}

/// Without Bevy's `ghost_nodes` feature the UI stack holds nodes without a `Node` (a
/// `ScrollbarThumb`); with it, ask about the nearest entity above that the stack holds.
fn stacked(world: &World, stack: &UiStack, widget: Entity) -> Entity {
    let mut current = widget;
    loop {
        if stack.uinodes.contains(&current) {
            return current;
        }
        match world.get::<ChildOf>(current) {
            Some(parent) => current = parent.parent(),
            None => return widget,
        }
    }
}

/// A gradient counts however transparent: only its presence is looked at. Borders and
/// shadows are left out.
fn paints(world: &World, node: Entity) -> bool {
    world
        .get::<BackgroundColor>(node)
        .is_some_and(|background| background.0.alpha() > 0.0)
        || world
            .get::<BackgroundGradient>(node)
            .is_some_and(|gradient| !gradient.0.is_empty())
        || world.get::<ImageNode>(node).is_some()
        || world.get::<Text>(node).is_some()
}

fn camera_of(world: &World, node: Entity) -> Option<Entity> {
    world
        .get::<ComputedUiTargetCamera>(node)
        .and_then(ComputedUiTargetCamera::get)
}

fn is_hit(world: &World, node: Entity, at: Vec2) -> bool {
    let (Some(computed), Some(transform)) = (
        world.get::<ComputedNode>(node),
        world.get::<UiGlobalTransform>(node),
    ) else {
        return false;
    };
    if computed.size() == Vec2::ZERO
        || world
            .get::<InheritedVisibility>(node)
            .is_some_and(|visibility| !visibility.get())
    {
        return false;
    }
    let point = at / computed.inverse_scale_factor;
    computed.contains_point(*transform, point)
        && world
            .get::<CalculatedClip>(node)
            .is_none_or(|clip| clip.contains_point(point))
        && !is_tooling(world, node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_missing_from_the_ui_stack_stands_in_as_its_nearest_stacked_ancestor() {
        // Bevy's `ghost_nodes` feature leaves a node without a `Node` out of
        // the stack; building that stack by hand reaches the same case.
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let child = world.spawn(ChildOf(root)).id();
        let grandchild = world.spawn(ChildOf(child)).id();
        let orphan = world.spawn_empty().id();
        let mut stack = UiStack::default();
        stack.uinodes.push(root);
        assert_eq!(stacked(&world, &stack, root), root);
        assert_eq!(stacked(&world, &stack, grandchild), root);
        assert_eq!(stacked(&world, &stack, orphan), orphan);
    }
}
