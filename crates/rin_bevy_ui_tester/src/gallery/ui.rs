//! The Scenarios window: its camera, the catalogue, the controls and the live
//! list of checks.

use bevy::{
    camera::{Camera2d, RenderTarget},
    color::Color,
    ecs::{
        component::Component,
        entity::Entity,
        hierarchy::ChildSpawnerCommands,
        query::{With, Without},
        system::{Commands, Query, Res, ResMut},
    },
    math::Vec2,
    picking::{
        events::{Click, Pointer},
        pointer::PointerButton,
    },
    prelude::On,
    text::{FontSize, TextColor, TextFont},
    ui::{
        AlignItems, BackgroundColor, BorderRadius, FlexDirection, FlexWrap, Node, Overflow,
        ScrollPosition, UiRect, UiTargetCamera, Val, px, widget::Text,
    },
    window::{Window, WindowRef, WindowResolution},
};

use super::{
    Gallery, GalleryAction, GalleryState, Speed, input::ScenariosScroll, state::GalleryPhase,
};
use crate::{
    overlay::OverlayTheme,
    player::{CheckKind, Outcome, ScenarioReport},
    scenario::Step,
    targets::NotUnderTest,
};

/// Default size of the Scenarios window, in logical pixels.
pub const SCENARIOS_WINDOW_SIZE: (u32, u32) = (460, 760);
/// Font size of the Scenarios window's text, in logical pixels.
const FONT_SIZE: f32 = 15.0;
/// Padding inside buttons and around panels, in logical pixels.
const PADDING: f32 = 6.0;
/// Gap between neighbouring controls, in logical pixels.
const GAP: f32 = 6.0;
/// Height of the panel with the selected scenario's checks, in logical pixels.
const DETAILS_HEIGHT: f32 = 220.0;

/// Background of the window.
const BACKGROUND: Color = Color::srgb(0.1, 0.11, 0.14);
/// A button at rest.
const BUTTON: Color = Color::srgb(0.2, 0.22, 0.28);
/// A button whose state is on: the speed in use, looping, the selected entry.
const BUTTON_ON: Color = Color::srgb(0.2, 0.4, 0.65);
/// Ordinary text.
const TEXT: Color = Color::srgb(0.9, 0.9, 0.92);
/// Secondary text.
const DIM: Color = Color::srgb(0.6, 0.62, 0.68);

/// The second OS window, titled "Scenarios".
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct ScenariosWindow;

/// The UI camera of the Scenarios window. It renders to that window only, so
/// it is never the default UI camera the app and the overlay use.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct ScenariosCamera;

/// The row of control buttons, for apps that add one of their own.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct GalleryToolbar;

/// What a button or label in the window stands for.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Control {
    /// Play or pause.
    Play,
    /// One frame.
    StepFrame,
    /// One step.
    StepStep,
    /// Restart.
    Restart,
    /// A speed.
    Speed(Speed),
    /// Looping.
    Loop,
    /// Take over, or resume.
    TakeOver,
    /// Export.
    Export,
    /// Start or stop recording a session.
    Record,
    /// A catalogue entry.
    Entry(usize),
}

/// Text that shows a control's current wording.
#[derive(Component, Debug, Clone, Copy)]
pub(super) struct ControlLabel(Control);

/// The result beside a catalogue entry.
#[derive(Component, Debug, Clone, Copy)]
pub(super) struct EntryResult(usize);

/// The one-line status under the controls.
#[derive(Component, Debug, Clone, Copy)]
pub(super) struct StatusLine;

/// The scrolling list of entries.
#[derive(Component, Debug, Clone, Copy)]
pub(super) struct Catalogue;

/// The panel that describes the selected entry and lists its checks.
#[derive(Component, Debug, Clone, Copy)]
pub(super) struct Details;

/// The text style of the window.
fn font() -> TextFont {
    TextFont {
        font_size: FontSize::Px(FONT_SIZE),
        ..TextFont::default()
    }
}

/// A line of text.
fn label(parent: &mut ChildSpawnerCommands<'_>, text: impl Into<String>, colour: Color) -> Entity {
    parent
        .spawn((Text::new(text), font(), TextColor(colour), NotUnderTest))
        .id()
}

/// The words on a control, which change with the gallery's state.
fn control_text(control: Control, state: &GalleryState) -> String {
    match control {
        Control::Play => if state.is_playing() { "Pause" } else { "Play" }.into(),
        Control::StepFrame => "Step frame".into(),
        Control::StepStep => "Step to next step".into(),
        Control::Restart => "Restart".into(),
        Control::Speed(speed) => speed.label().into(),
        Control::Loop => format!("Loop: {}", if state.is_looping() { "on" } else { "off" }),
        Control::TakeOver => if state.is_taken_over() {
            "Resume script"
        } else {
            "Take over"
        }
        .into(),
        Control::Export => "Export RON".into(),
        Control::Record => if state.is_recording() {
            "Stop recording"
        } else {
            "Record"
        }
        .into(),
        Control::Entry(_) => String::new(),
    }
}

/// The action a control asks for, given the state it is clicked in.
const fn control_action(control: Control, state: &GalleryState) -> GalleryAction {
    match control {
        Control::Play => GalleryAction::TogglePlay,
        Control::StepFrame => GalleryAction::StepFrame,
        Control::StepStep => GalleryAction::StepStep,
        Control::Restart => GalleryAction::Restart,
        Control::Speed(speed) => GalleryAction::SetSpeed(speed),
        Control::Loop => GalleryAction::ToggleLoop,
        Control::TakeOver => {
            if state.is_taken_over() {
                GalleryAction::Resume
            } else {
                GalleryAction::TakeOver
            }
        }
        Control::Export => GalleryAction::Export,
        Control::Record => {
            if state.is_recording() {
                GalleryAction::StopRecording
            } else {
                GalleryAction::Record
            }
        }
        Control::Entry(index) => GalleryAction::Select(index),
    }
}

/// Spawns a button for `control`.
fn button(parent: &mut ChildSpawnerCommands<'_>, control: Control, state: &GalleryState) {
    parent
        .spawn((
            control,
            NotUnderTest,
            Node {
                padding: UiRect::axes(px(PADDING * 2.0), px(PADDING)),
                border_radius: BorderRadius::all(px(4)),
                ..Node::default()
            },
            BackgroundColor(BUTTON),
        ))
        .observe(
            move |click: On<Pointer<Click>>, mut state: ResMut<GalleryState>| {
                if click.button == PointerButton::Primary {
                    let action = control_action(control, &state);
                    state.request(action);
                }
            },
        )
        .with_children(|button| {
            let text = control_text(control, state);
            let entity = label(button, text, TEXT);
            button
                .commands()
                .entity(entity)
                .insert(ControlLabel(control));
        });
}

/// A row of controls.
fn row(parent: &mut ChildSpawnerCommands<'_>, fill: impl FnOnce(&mut ChildSpawnerCommands<'_>)) {
    parent
        .spawn((
            NotUnderTest,
            Node {
                flex_wrap: FlexWrap::Wrap,
                column_gap: px(GAP),
                row_gap: px(GAP),
                ..Node::default()
            },
        ))
        .with_children(fill);
}

/// Spawns the Scenarios window, its camera and everything in it.
pub(super) fn spawn_window(
    mut commands: Commands,
    gallery: Res<Gallery>,
    state: Res<GalleryState>,
) {
    let (width, height) = SCENARIOS_WINDOW_SIZE;
    let window = commands
        .spawn((
            ScenariosWindow,
            Window {
                title: "Scenarios".into(),
                resolution: WindowResolution::new(width, height),
                ..Window::default()
            },
        ))
        .id();
    let camera = commands
        .spawn((
            ScenariosCamera,
            NotUnderTest,
            Camera2d,
            RenderTarget::Window(WindowRef::Entity(window)),
        ))
        .id();
    commands
        .spawn((
            NotUnderTest,
            UiTargetCamera(camera),
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: px(GAP),
                padding: UiRect::all(px(PADDING * 2.0)),
                ..Node::default()
            },
            BackgroundColor(BACKGROUND),
        ))
        .with_children(|root| {
            label(root, "Scenarios", TEXT);
            row(root, |row| {
                button(row, Control::Play, &state);
                button(row, Control::StepFrame, &state);
                button(row, Control::StepStep, &state);
                button(row, Control::Restart, &state);
            });
            row(root, |row| {
                for speed in Speed::ALL {
                    button(row, Control::Speed(speed), &state);
                }
                button(row, Control::Loop, &state);
            });
            root.spawn((
                GalleryToolbar,
                NotUnderTest,
                Node {
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: px(GAP),
                    row_gap: px(GAP),
                    ..Node::default()
                },
            ))
            .with_children(|row| {
                button(row, Control::TakeOver, &state);
                button(row, Control::Export, &state);
                button(row, Control::Record, &state);
            });
            root.spawn((
                StatusLine,
                NotUnderTest,
                Text::new(""),
                font(),
                TextColor(DIM),
            ));
            root.spawn((
                Catalogue,
                NotUnderTest,
                ScrollPosition::default(),
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    min_height: px(0),
                    row_gap: px(GAP),
                    overflow: Overflow::scroll_y(),
                    ..Node::default()
                },
            ))
            .with_children(|list| spawn_catalogue(list, &gallery));
            root.spawn((
                Details,
                NotUnderTest,
                Node {
                    flex_direction: FlexDirection::Column,
                    height: px(DETAILS_HEIGHT),
                    overflow: Overflow::clip(),
                    padding: UiRect::all(px(PADDING)),
                    row_gap: px(2),
                    ..Node::default()
                },
                BackgroundColor(Color::srgb(0.07, 0.08, 0.1)),
            ));
        });
}

/// Spawns the groups and their entries.
fn spawn_catalogue(list: &mut ChildSpawnerCommands<'_>, gallery: &Gallery) {
    let mut index = 0;
    for group in gallery.groups() {
        label(list, group.name.clone(), DIM);
        for entry in &group.entries {
            let at = index;
            index += 1;
            list.spawn((
                Control::Entry(at),
                NotUnderTest,
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(px(PADDING)),
                    border_radius: BorderRadius::all(px(4)),
                    ..Node::default()
                },
                BackgroundColor(BUTTON),
            ))
            .observe(
                move |click: On<Pointer<Click>>, mut state: ResMut<GalleryState>| {
                    if click.button == PointerButton::Primary {
                        state.request(GalleryAction::Select(at));
                    }
                },
            )
            .with_children(|item| {
                item.spawn((
                    NotUnderTest,
                    Node {
                        align_items: AlignItems::Center,
                        column_gap: px(GAP),
                        ..Node::default()
                    },
                ))
                .with_children(|head| {
                    let result = label(head, "-", DIM);
                    head.commands().entity(result).insert(EntryResult(at));
                    label(head, entry.scenario.name(), TEXT);
                });
                if !entry.description.is_empty() {
                    label(item, entry.description.clone(), DIM);
                }
            });
        }
    }
}

/// Shades the buttons whose state is on.
pub(super) fn style_controls(
    state: Res<GalleryState>,
    mut buttons: Query<(&Control, &mut BackgroundColor)>,
) {
    for (control, mut colour) in &mut buttons {
        let on = match control {
            Control::Play => state.is_playing(),
            Control::Speed(speed) => state.speed() == *speed,
            Control::Loop => state.is_looping(),
            Control::TakeOver => state.is_taken_over(),
            Control::Record => state.is_recording(),
            Control::Entry(index) => state.selected() == Some(*index),
            Control::StepFrame | Control::StepStep | Control::Restart | Control::Export => false,
        };
        let wanted = if on { BUTTON_ON } else { BUTTON };
        if colour.0 != wanted {
            colour.0 = wanted;
        }
    }
}

/// Rewrites the words that change: control labels, results and the status.
pub(super) fn update_labels(
    state: Res<GalleryState>,
    report: Res<ScenarioReport>,
    theme: Res<OverlayTheme>,
    gallery: Res<Gallery>,
    mut labels: Query<(&ControlLabel, &mut Text), (Without<EntryResult>, Without<StatusLine>)>,
    mut results: Query<
        (&EntryResult, &mut Text, &mut TextColor),
        (Without<ControlLabel>, Without<StatusLine>),
    >,
    mut status: Query<
        &mut Text,
        (
            With<StatusLine>,
            Without<ControlLabel>,
            Without<EntryResult>,
        ),
    >,
) {
    for (ControlLabel(control), mut text) in &mut labels {
        let wanted = control_text(*control, &state);
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
    for (EntryResult(index), mut text, mut colour) in &mut results {
        let (mark, tint) = match state.result(*index) {
            Some(true) => (theme.pass_mark.clone(), theme.pass_text),
            Some(false) => (theme.fail_mark.clone(), theme.fail_text),
            None => (theme.skipped_mark.clone(), DIM),
        };
        if text.0 != mark {
            text.0 = mark;
        }
        colour.0 = tint;
    }
    let line = status_text(&state, &report, &gallery);
    for mut text in &mut status {
        if text.0 != line {
            text.0.clone_from(&line);
        }
    }
}

/// What the gallery is doing, in a line.
fn status_text(state: &GalleryState, report: &ScenarioReport, gallery: &Gallery) -> String {
    if !state.status().is_empty() {
        return state.status().to_owned();
    }
    let Some(entry) = state.selected().and_then(|index| gallery.get(index)) else {
        return "Pick a scenario to play it".into();
    };
    let reached = report
        .steps
        .iter()
        .filter(|record| !matches!(record.outcome, Outcome::Skipped))
        .count();
    let mode = if state.is_taken_over() {
        "taken over: the real mouse and keyboard have the app"
    } else {
        match state.phase {
            GalleryPhase::Idle | GalleryPhase::Settling(_) => "starting",
            GalleryPhase::Running if state.is_playing() => "playing",
            GalleryPhase::Running => "paused",
            GalleryPhase::Finished(_) if report.passed() => "finished, passed",
            GalleryPhase::Finished(_) => "finished, failed",
        }
    };
    format!(
        "{}: {mode} (step {reached}/{})",
        entry.scenario.name(),
        report.steps.len()
    )
}

/// Applies the wheel to the catalogue.
pub(super) fn scroll_catalogue(
    mut scroll: ResMut<ScenariosScroll>,
    mut lists: Query<&mut ScrollPosition, With<Catalogue>>,
) {
    if scroll.0 == 0.0 {
        return;
    }
    for mut position in &mut lists {
        position.0 = Vec2::new(0.0, (position.0.y - scroll.0).max(0.0));
    }
    scroll.0 = 0.0;
}

/// Rewrites the selected entry's description and its checks, with a mark for
/// each: the checks the scenario names, in the order they run, with those not
/// reached yet shown as such.
pub(super) fn update_details(
    mut commands: Commands,
    state: Res<GalleryState>,
    report: Res<ScenarioReport>,
    gallery: Res<Gallery>,
    theme: Res<OverlayTheme>,
    details: Query<Entity, With<Details>>,
    mut shown: bevy::ecs::system::Local<Option<(Option<usize>, ScenarioReport)>>,
) {
    let key = (state.selected(), report.clone());
    if shown.as_ref() == Some(&key) {
        return;
    }
    *shown = Some(key);
    let Some(entry) = state.selected().and_then(|index| gallery.get(index)) else {
        return;
    };
    let scenario = &entry.scenario;
    // Slots in the order the player records them: step checks as the script
    // reaches them, then the expectations.
    let step_checks = scenario.script().iter().filter_map(|step| match step {
        Step::Check(name) => Some((CheckKind::Step, name.as_str())),
        _ => None,
    });
    let expects = scenario
        .expects()
        .iter()
        .map(|check| (CheckKind::Expect, check.name.as_str()));
    let mut seen_step = 0;
    let mut seen_expect = 0;
    let mut lines = Vec::new();
    for (kind, name) in step_checks.chain(expects) {
        let nth = match kind {
            CheckKind::Step => &mut seen_step,
            CheckKind::Expect => &mut seen_expect,
        };
        let record = report
            .checks
            .iter()
            .filter(|record| record.kind == kind)
            .nth(*nth);
        *nth += 1;
        let (mark, tint, why) = match record.map(|record| &record.outcome) {
            Some(Outcome::Pass) => (theme.pass_mark.clone(), theme.pass_text, String::new()),
            Some(Outcome::Fail(message)) => (
                theme.fail_mark.clone(),
                theme.fail_text,
                format!(": {message}"),
            ),
            Some(Outcome::Skipped) | None => (theme.skipped_mark.clone(), DIM, String::new()),
        };
        lines.push((format!("{mark} {name}{why}"), tint));
    }
    for entity in &details {
        commands
            .entity(entity)
            .despawn_children()
            .with_children(|panel| {
                label(panel, scenario.name(), TEXT);
                if !entry.description.is_empty() {
                    label(panel, entry.description.clone(), DIM);
                }
                if lines.is_empty() {
                    label(panel, "No checks", DIM);
                }
                for (line, tint) in &lines {
                    label(panel, line.clone(), *tint);
                }
            });
    }
}
