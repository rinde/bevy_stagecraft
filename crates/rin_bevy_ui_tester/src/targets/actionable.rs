//! Actionability: whether a target can be acted on right now, as
//! Playwright's auto-waiting decides it.
//!
//! Before a step acts on a target that names a widget, the player waits
//! until, in this order, the target
//!
//! 1. exists: it names exactly one widget;
//! 2. is visible: not hidden, with a size, and with its middle neither
//!    clipped away nor outside the window;
//! 3. is stable: neither its layout nor its transform changed during the
//!    last frame;
//! 4. is enabled, for a step that presses: neither it nor anything above it
//!    carries `InteractionDisabled`;
//! 5. receives the pointer: at the point the step aims at, the topmost UI
//!    node that takes the pointer is the widget or lies below it, not
//!    something covering it.
//!
//! Positions (`Screen`, `World`, and app targets that report no widget) name
//! no widget, so none of this applies to them. Everything here reads the
//! world only, and stability counts frames, never time.

use bevy::{
    camera::visibility::InheritedVisibility,
    color::Alpha,
    ecs::{entity::Entity, hierarchy::ChildOf, query::With, world::World},
    math::Vec2,
    picking::Pickable,
    ui::{
        BackgroundColor, BackgroundGradient, CalculatedClip, ComputedNode, ComputedUiTargetCamera,
        InteractionDisabled, Node, UiGlobalTransform, UiStack,
        widget::{ImageNode, Text},
    },
    window::{PrimaryWindow, Window},
};

use super::{
    Action, Lookup, NotActionable, ResolvedTarget, Target, enclosing_button, is_tooling, locate,
    resolve,
};
use crate::probe::{describe_entity, is_within};

/// How many frames a step waits for its target to become actionable unless
/// the scenario or the step says otherwise: one second at 1/60 s.
pub const DEFAULT_ACTION_TIMEOUT: u16 = 60;

/// Works `target` out to a position and checks every condition `action`
/// needs, the first one that fails saying why.
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

/// Whether a press at `at`, where an earlier step sent the pointer to
/// `widget`, reaches it: the widget is still there, enabled, and takes the
/// pointer at `at`. `label` is how the script named it.
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

/// Condition 1: the target resolves to one widget, or to a position.
fn find(world: &World, target: &Target) -> Result<ResolvedTarget, NotActionable> {
    match target {
        // A position names no widget; a failure to project one is as final
        // as it always was.
        Target::Screen(_) | Target::World(_) => {
            resolve(world, target).map_err(NotActionable::Never)
        }
        // The app's own target says itself whether it is there yet.
        Target::App(value) => value.resolve(world).map_err(NotActionable::Yet),
        Target::Text(_)
        | Target::Component(_)
        | Target::Name(_)
        | Target::Role { .. }
        | Target::Within(..)
        | Target::Nth(..) => match locate(world, target) {
            Lookup::Found(node) => {
                let widget = enclosing_button(world, node).unwrap_or(node);
                if world.get::<Node>(widget).is_none() {
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

/// Condition 2: `widget` is drawn, has a size, and its point `at` is
/// neither clipped away nor outside the window.
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
        .is_some_and(|clip| !clip.clip.contains(physical))
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

/// Condition 3: neither the layout nor the transform of `widget` changed
/// during the last frame, so its position is the same over two frames.
///
/// Bevy's layout only marks these components changed when their values
/// change, so change detection says whether the widget moved. "The last
/// frame" is everything since the player's own system last ran, which the
/// world reports as its last change tick while that system runs.
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

/// Condition 4: neither `widget` nor anything above it carries
/// `InteractionDisabled`. Bevy's widgets read the marker on the widget
/// itself; an app that disables a whole panel means everything in it, as a
/// disabled fieldset does in HTML, so the ancestors count too.
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

/// Condition 5: at `at`, the pointer reaches `widget`.
///
/// This walks the UI stack from the front as Bevy's UI picking does: a node
/// is hit when it is drawn, has a size, holds the point and is not clipped
/// there; the first hit that is the widget or lies below it and is hoverable
/// receives the pointer, unless a hit before it blocks what lies below
/// (`Pickable::should_block_lower`, which nodes without `Pickable` do). Only
/// nodes on the widget's camera count, and never the tooling's.
fn receives_pointer(
    world: &World,
    widget: Entity,
    at: Vec2,
    label: &str,
) -> Result<(), NotActionable> {
    let Some(stack) = world.get_resource::<UiStack>() else {
        return Ok(());
    };
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

/// Whether the middle of `widget` is painted topmost: walking the UI stack
/// from the front, the first node that holds the point and is the widget or
/// lies below it, or paints something, must be the widget's.
///
/// This is condition 5 without `Pickable`: it asks what the person sees,
/// not what the pointer reaches. A popup that lets the pointer through
/// (`should_block_lower: false`) still covers what is below it here. Nodes
/// that paint nothing (no background colour that shows, no background
/// gradient, no image, no text; see `paints`) cover nothing, or an empty
/// full-window container would cover the whole UI; the widget's own nodes
/// count whether they paint or not. Only nodes on the widget's camera count,
/// and never the tooling's.
///
/// # Errors
///
/// A message saying what covers the widget, or why nothing is painted at its
/// middle.
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

/// Whether `node` draws something of its own over its whole box: a
/// background colour that shows, a background gradient, an image or text.
/// A gradient with no stops, or with only transparent ones, still counts as
/// covering: only its presence is looked at, not what it draws. Borders and shadows are left out: they paint the edge or outside of the
/// box, not what lies under its middle.
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

/// The camera `node` is drawn by, if layout has said.
fn camera_of(world: &World, node: Entity) -> Option<Entity> {
    world
        .get::<ComputedUiTargetCamera>(node)
        .and_then(ComputedUiTargetCamera::get)
}

/// Whether `node` is a node of the app that `at` (logical pixels) is on.
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
            .is_none_or(|clip| clip.clip.contains(point))
        && !is_tooling(world, node)
}
