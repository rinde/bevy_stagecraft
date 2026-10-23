//! What a target's widget is doing: disabled, checked, selected, pressed,
//! focused, visible, and the text it shows.
//!
//! These read the components Bevy's own widgets keep (`bevy_ui_widgets`) and
//! `bevy_ui`'s layout and interaction components, so they work for any toolkit that
//! uses them.

use crate::{
    probe::{describe_entity, is_within},
    targets::{Lookup, Target, locate, painted_topmost},
};
use bevy::{
    camera::visibility::InheritedVisibility,
    ecs::{entity::Entity, hierarchy::ChildOf, hierarchy::Children, world::World},
    input_focus::InputFocus,
    text::{EditableText, InlineBox, TextSpan},
    ui::{
        Checkable, Checked, ComputedNode, InteractionDisabled, Pressed, Selectable, Selected,
        widget::Text,
    },
    ui_widgets::{Button, MenuButton, MenuItem, SelectedTab, Slider, Tab, TextInput},
};

/// A state a widget can be in, as Bevy's widgets record it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WidgetState {
    /// Carries `InteractionDisabled`.
    Disabled,
    /// Carries `Checked`.
    Checked,
    /// Carries `Selected`.
    Selected,
    /// Carries `Pressed`.
    Pressed,
    /// Has input focus, or something below it has.
    Focused,
    /// Visible in the hierarchy and taking up room on screen.
    Visible,
    /// Painted topmost at its middle, whatever covers it lets through.
    OnTop,
    /// Can be found at all.
    Exists,
}

impl WidgetState {
    /// Whether the state is about the named node (its own visibility), not the
    /// widget it belongs to.
    pub(super) const fn is_about_the_node(self) -> bool {
        matches!(self, Self::Visible)
    }

    /// The state in words, after the target: `is disabled`.
    pub(super) const fn words(self) -> &'static str {
        match self {
            Self::Disabled => "is disabled",
            Self::Checked => "is checked",
            Self::Selected => "is selected",
            Self::Pressed => "is pressed",
            Self::Focused => "has focus",
            Self::Visible => "is visible",
            Self::OnTop => "is on top",
            Self::Exists => "exists",
        }
    }
}

/// The node `target` names, found strictly: for a text target, the node
/// showing the text; for one of the app's own, the widget its
/// [`AppTarget`](crate::AppTarget) reports.
pub(super) fn node(world: &World, target: &Target) -> Lookup {
    locate(world, target)
}

/// The widget `node` belongs to: the nearest widget that is `node` or above
/// it, as a label belongs to its button; otherwise `node` itself.
pub(super) fn widget(world: &World, node: Entity) -> Entity {
    let mut current = node;
    loop {
        if is_widget(world, current) {
            return current;
        }
        match world.get::<ChildOf>(current) {
            Some(parent) => current = parent.parent(),
            None => return node,
        }
    }
}

/// Whether `entity` is a widget of `bevy_ui_widgets`, or a `bevy_ui` node
/// that tracks interaction.
pub(super) fn is_widget(world: &World, entity: Entity) -> bool {
    let Ok(entity) = world.get_entity(entity) else {
        return false;
    };
    entity.contains::<Button>()
        || entity.contains::<Checkable>()
        || entity.contains::<Selectable>()
        || entity.contains::<Slider>()
        || entity.contains::<MenuItem>()
        || entity.contains::<Tab>()
        || entity.contains::<MenuButton>()
        || entity.contains::<TextInput>()
}

/// Whether `widget` is in `state`; the error says what it is instead.
pub(super) fn in_state(
    world: &World,
    widget: Entity,
    state: WidgetState,
    label: &str,
) -> Result<(), String> {
    let holds = |held: bool, otherwise: &str| {
        if held {
            Ok(())
        } else {
            Err(format!("{label} {otherwise}"))
        }
    };
    match state {
        WidgetState::Disabled => holds(
            world.get::<InteractionDisabled>(widget).is_some(),
            "is not disabled",
        ),
        WidgetState::Checked => holds(world.get::<Checked>(widget).is_some(), "is not checked"),
        WidgetState::Selected => holds(is_selected(world, widget), "is not selected"),
        WidgetState::Pressed => holds(world.get::<Pressed>(widget).is_some(), "is not pressed"),
        WidgetState::Focused => {
            match world.get_resource::<InputFocus>().and_then(InputFocus::get) {
                Some(focus) if is_within(world, focus, widget) => Ok(()),
                Some(focus) => Err(format!(
                    "{label} does not have focus: {} has",
                    describe_entity(world, focus)
                )),
                None => Err(format!("{label} does not have focus: nothing has")),
            }
        }
        WidgetState::Visible => visible(world, widget, label),
        WidgetState::OnTop => on_top(world, widget, label),
        WidgetState::Exists => Ok(()),
    }
}

/// Whether `widget` carries `Selected`, or is the tab its list's
/// `SelectedTab` names (Bevy derives `Selected` from it only in `PostUpdate`).
fn is_selected(world: &World, widget: Entity) -> bool {
    world.get::<Selected>(widget).is_some()
        || (world.get::<Tab>(widget).is_some()
            && world.get::<ChildOf>(widget).is_some_and(|parent| {
                world
                    .get::<SelectedTab>(parent.parent())
                    .is_some_and(|selected| selected.0 == Some(widget))
            }))
}

/// Whether `widget` is visible in the hierarchy and has a size.
fn visible(world: &World, widget: Entity, label: &str) -> Result<(), String> {
    if world
        .get::<InheritedVisibility>(widget)
        .is_some_and(|visibility| !visibility.get())
    {
        return Err(format!("{label} is hidden"));
    }
    if world
        .get::<ComputedNode>(widget)
        .is_some_and(|node| node.size().cmple(bevy::math::Vec2::ZERO).any())
    {
        return Err(format!("{label} takes up no room on screen"));
    }
    Ok(())
}

/// Whether nothing painted later covers the middle of `widget`.
fn on_top(world: &World, widget: Entity, label: &str) -> Result<(), String> {
    painted_topmost(world, widget).map_err(|why| format!("{label} is not on top: {why}"))
}

/// The text `widget` shows: each text node at or below it, in hierarchy
/// order, with its spans, joined by single spaces.
///
/// The text of a text input is the value it holds, what was typed, and
/// nothing else: its label is [`label_of`].
pub(super) fn text_of(world: &World, widget: Entity) -> String {
    if let Some(field) = world.get::<EditableText>(widget) {
        return field.value().to_string();
    }
    let mut blocks: Vec<String> = Vec::new();
    collect_text(world, widget, &mut blocks);
    blocks.join(" ")
}

/// What names `widget`: its text as [`text_of`] reads it, except that a text
/// input's own value is left out, as it changes with every key and is what a
/// check reads, not what a target finds the field by.
pub(super) fn label_of(world: &World, widget: Entity) -> String {
    let mut blocks: Vec<String> = Vec::new();
    if world.get::<EditableText>(widget).is_some() {
        collect_below(world, widget, &mut blocks);
    } else {
        collect_text(world, widget, &mut blocks);
    }
    blocks.join(" ")
}

/// Adds the text of everything below `entity`, not of `entity` itself.
fn collect_below(world: &World, entity: Entity, blocks: &mut Vec<String>) {
    if let Some(children) = world.get::<Children>(entity) {
        for child in children {
            collect_text(world, *child, blocks);
        }
    }
}

/// Adds the text of `entity` and of everything below it to `blocks`.
fn collect_text(world: &World, entity: Entity, blocks: &mut Vec<String>) {
    if world.get::<InlineBox>(entity).is_some() {
        // An inline box is room in the text, not text, and Bevy lays out
        // nothing below it.
        return;
    }
    if let Some(text) = world.get::<Text>(entity) {
        blocks.push(text.0.clone());
    } else if let Some(editable) = world.get::<EditableText>(entity) {
        let value = editable.value().to_string();
        if !value.is_empty() {
            blocks.push(value);
        }
    } else if let Some(span) = world.get::<TextSpan>(entity) {
        // A span continues the text it sits below.
        match blocks.last_mut() {
            Some(last) => last.push_str(&span.0),
            None => blocks.push(span.0.clone()),
        }
    }
    if let Some(children) = world.get::<Children>(entity) {
        for child in children {
            collect_text(world, *child, blocks);
        }
    }
}
