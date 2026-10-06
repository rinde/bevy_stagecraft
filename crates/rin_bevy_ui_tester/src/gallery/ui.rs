//! The Scenarios window: its camera, the catalogue, the controls and the live
//! list of checks.

use bevy::{
    camera::{Camera2d, RenderTarget},
    color::Color,
    ecs::{
        component::Component,
        entity::Entity,
        hierarchy::ChildOf,
        hierarchy::ChildSpawnerCommands,
        query::{Has, Or, With, Without},
        system::{Commands, Local, Query, Res, ResMut},
    },
    math::{DVec2, IVec2, Vec2},
    picking::{
        events::{Click, Pointer},
        hover::HoverMap,
        pointer::{PointerButton, PointerId},
    },
    prelude::On,
    text::{FontSize, TextColor, TextFont},
    ui::{
        AlignItems, BackgroundColor, BorderRadius, FlexDirection, FlexWrap, Node, Overflow,
        ScrollPosition, UiRect, UiTargetCamera, Val, px, widget::Text,
    },
    window::{
        Monitor, PrimaryMonitor, PrimaryWindow, Window, WindowPosition, WindowRef, WindowResolution,
    },
};

use super::{
    Gallery, GalleryAction, GalleryState, Speed,
    input::{ScenariosScroll, scenarios_pointer},
    pick::spelled,
    state::{GalleryPhase, Suggestions},
};
use crate::{
    check::{Check, pick::Picked},
    overlay::{OverlayTheme, TargetDescriptions, describe_step},
    player::{CheckKind, Outcome, ScenarioReport},
    scenario::{Scenario, Step},
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
    /// Start recording a new scenario from an empty world.
    NewEmpty,
    /// Start recording a new scenario from the app as it is.
    NewFromCurrent,
    /// A catalogue entry.
    Entry(usize),
    /// A step of the selected entry, to jump to the start of.
    JumpTo(usize),
    /// A check the last recording suggested at its end, in the checklist.
    Suggestion(usize),
    /// A check the last recording suggested at a checkpoint: the checkpoint
    /// and the check's index in its list.
    CheckpointSuggestion(usize, usize),
    /// Start or stop pick mode.
    Pick,
    /// One of the picked node's targets.
    PickTarget(usize),
    /// One of the checks that hold for the picked node.
    PickCheck(usize),
    /// The check an app check form makes.
    FormCheck(usize),
    /// Steps a form's field: the form, the field, and up rather than down.
    FormField(usize, usize, bool),
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
        Control::NewEmpty => "New: empty world".into(),
        Control::NewFromCurrent => "New: current state".into(),
        Control::Pick => if state.is_picking() {
            "Stop picking"
        } else {
            "Pick"
        }
        .into(),
        Control::Entry(_)
        | Control::JumpTo(_)
        | Control::Suggestion(_)
        | Control::CheckpointSuggestion(..)
        | Control::PickTarget(_)
        | Control::PickCheck(_)
        | Control::FormCheck(_)
        | Control::FormField(..) => String::new(),
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
        Control::NewEmpty => GalleryAction::NewEmpty,
        Control::NewFromCurrent => GalleryAction::NewFromCurrent,
        Control::Entry(index) => GalleryAction::Select(index),
        Control::JumpTo(step) => GalleryAction::JumpTo(step),
        Control::Suggestion(index) => GalleryAction::ToggleSuggestion(index),
        Control::CheckpointSuggestion(checkpoint, index) => {
            GalleryAction::ToggleCheckpointSuggestion { checkpoint, index }
        }
        Control::Pick => GalleryAction::TogglePick,
        Control::PickTarget(index) => GalleryAction::PickTarget(index),
        Control::PickCheck(index) => GalleryAction::AddPickedCheck(index),
        Control::FormCheck(index) => GalleryAction::AddFormCheck(index),
        Control::FormField(form, field, up) => GalleryAction::StepFormField { form, field, up },
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

/// Puts the Scenarios window directly to the left of the app window, the pair
/// centred horizontally at the top of the primary monitor, once.
///
/// The top rather than the middle: the menu bar and the Dock are not part of
/// what Bevy knows of a monitor, and macOS pushes a window that overlaps them.
/// At the top both windows are pushed below the menu bar alike and stay
/// aligned. Bevy reports a window's position only after it has been moved, so the
/// positions are worked out from the monitor rather than from the app window.
/// It waits until the monitors are known; on Wayland, which does not let
/// clients place windows, the window manager decides.
pub(super) fn place_scenarios_window(
    mut done: Local<bool>,
    monitors: Query<&Monitor, With<PrimaryMonitor>>,
    mut primary: Query<&mut Window, (With<PrimaryWindow>, Without<ScenariosWindow>)>,
    mut scenarios: Query<&mut Window, With<ScenariosWindow>>,
) {
    if *done {
        return;
    }
    let (Some(monitor), Some(mut primary), Some(mut scenarios)) = (
        monitors.iter().next(),
        primary.iter_mut().next(),
        scenarios.iter_mut().next(),
    ) else {
        return;
    };
    let physical = |window: &Window| {
        (DVec2::new(f64::from(window.width()), f64::from(window.height())) * monitor.scale_factor)
            .round()
            .as_ivec2()
    };
    let (left, right) = (physical(&scenarios), physical(&primary));
    let pair = left.x.saturating_add(right.x);
    let screen = i32::try_from(monitor.physical_width).unwrap_or(i32::MAX);
    let corner = monitor
        .physical_position
        .saturating_add(IVec2::new(screen.saturating_sub(pair).max(0) / 2, 0));
    scenarios.position = WindowPosition::At(corner);
    primary.position = WindowPosition::At(corner.saturating_add(IVec2::new(left.x, 0)));
    *done = true;
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
            toolbar(root, &state);
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
                ScrollPosition::default(),
                Node {
                    flex_direction: FlexDirection::Column,
                    height: px(DETAILS_HEIGHT),
                    overflow: Overflow::scroll_y(),
                    padding: UiRect::all(px(PADDING)),
                    row_gap: px(2),
                    ..Node::default()
                },
                BackgroundColor(Color::srgb(0.07, 0.08, 0.1)),
            ));
        });
}

/// Spawns the row of toolbar buttons: take over, export, the recording
/// starters, and pick.
fn toolbar(root: &mut ChildSpawnerCommands<'_>, state: &GalleryState) {
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
        button(row, Control::TakeOver, state);
        button(row, Control::Export, state);
        button(row, Control::Record, state);
        button(row, Control::NewEmpty, state);
        button(row, Control::NewFromCurrent, state);
        button(row, Control::Pick, state);
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
            Control::Suggestion(index) => state
                .suggestions()
                .is_some_and(|suggestions| suggestions.is_ticked(*index)),
            Control::CheckpointSuggestion(checkpoint, index) => state
                .suggestions()
                .is_some_and(|suggestions| suggestions.is_checkpoint_ticked(*checkpoint, *index)),
            Control::Pick => state.is_picking(),
            Control::PickTarget(index) => state
                .picked()
                .is_some_and(|picked| picked.chosen() == *index),
            Control::PickCheck(index) => state.picked().is_some_and(|picked| {
                picked
                    .checks()
                    .get(*index)
                    .is_some_and(|check| picked.is_added(check))
            }),
            Control::FormCheck(index) => state.picked().is_some_and(|picked| {
                picked
                    .forms()
                    .get(*index)
                    .and_then(|form| form.check.as_ref().ok())
                    .is_some_and(|check| picked.is_added(check))
            }),
            Control::JumpTo(_)
            | Control::StepFrame
            | Control::StepStep
            | Control::Restart
            | Control::Export
            | Control::NewEmpty
            | Control::NewFromCurrent
            | Control::FormField(..) => false,
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

/// Applies the wheel to the catalogue, or to the details panel while a
/// pointer of the Scenarios window is over it: a recording's checklists can
/// be longer than the panel.
pub(super) fn scroll_catalogue(
    mut scroll: ResMut<ScenariosScroll>,
    hovers: Option<Res<HoverMap>>,
    parents: Query<&ChildOf>,
    details: Query<Entity, With<Details>>,
    mut lists: Query<(&mut ScrollPosition, Has<Details>), Or<(With<Catalogue>, With<Details>)>>,
) {
    if scroll.0 == 0.0 {
        return;
    }
    let over_details = hovers.is_some_and(|hovers| {
        [scenarios_pointer(), PointerId::Mouse]
            .iter()
            .filter_map(|pointer| hovers.get(pointer))
            .flat_map(|hits| hits.keys())
            .any(|hit| {
                parents
                    .iter_ancestors(*hit)
                    .chain(core::iter::once(*hit))
                    .any(|entity| details.contains(entity))
            })
    });
    for (mut position, is_details) in &mut lists {
        if is_details == over_details {
            position.0 = Vec2::new(0.0, (position.0.y - scroll.0).max(0.0));
        }
    }
    scroll.0 = 0.0;
}

/// What the details panel shows: the selection, its report, the checklist
/// of a recording, pick mode and what it picked, and the scenarios'
/// revision.
type DetailsKey = (
    Option<usize>,
    ScenarioReport,
    Option<Suggestions>,
    bool,
    Option<Picked>,
    u32,
);

/// Rewrites the selected entry's description and its checks, with a mark for
/// each: the checks the scenario names, in the order they run, with those not
/// reached yet shown as such. After a recording it shows the checks the
/// recording suggests instead, as a checklist.
pub(super) fn update_details(
    mut commands: Commands,
    state: Res<GalleryState>,
    report: Res<ScenarioReport>,
    gallery: Res<Gallery>,
    theme: Res<OverlayTheme>,
    descriptions: Option<Res<TargetDescriptions>>,
    details: Query<Entity, With<Details>>,
    mut shown: Local<Option<DetailsKey>>,
) {
    let key = (
        state.selected(),
        report.clone(),
        state.suggestions().cloned(),
        state.is_picking(),
        state.picked().cloned(),
        state.revision,
    );
    if shown.as_ref() == Some(&key) {
        return;
    }
    *shown = Some(key);
    if state.is_picking() {
        let destination = if state.is_recording() {
            "the recording, here"
        } else {
            "the expect"
        };
        for entity in &details {
            commands
                .entity(entity)
                .despawn_children()
                .with_children(|panel| {
                    spawn_pick_panel(panel, state.picked(), destination, &theme);
                });
        }
        return;
    }
    if let Some(suggestions) = state.suggestions() {
        for entity in &details {
            commands
                .entity(entity)
                .despawn_children()
                .with_children(|panel| spawn_checklist(panel, suggestions));
        }
        return;
    }
    let Some(entry) = state.selected().and_then(|index| gallery.get(index)) else {
        return;
    };
    let scenario = &entry.scenario;
    let lines = check_lines(scenario, &report, &theme);
    let descriptions = descriptions.as_deref().cloned().unwrap_or_default();
    let steps = step_lines(scenario, &report, &theme, &descriptions);
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
                if !steps.is_empty() {
                    label(panel, "Steps (click to play up to one and pause)", DIM);
                }
                for (index, (mark, words)) in steps.iter().enumerate() {
                    panel_button(
                        panel,
                        Control::JumpTo(index),
                        format!("{mark} {words}"),
                        false,
                    );
                }
            });
    }
}

/// Each check of `scenario`, with its mark and failure message once it has
/// run, and the colour to show it in.
fn check_lines(
    scenario: &Scenario,
    report: &ScenarioReport,
    theme: &OverlayTheme,
) -> Vec<(String, Color)> {
    // Slots in the order the player records them: step checks as the script
    // reaches them, then the expectations.
    let step_checks = scenario.script().iter().filter_map(|step| match step {
        Step::Check(check) => Some((CheckKind::Step, check.to_string())),
        _ => None,
    });
    let expects = scenario
        .expects()
        .into_iter()
        .map(|check| (CheckKind::Expect, check.to_string()));
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
    lines
}

/// Each step of `scenario` as a mark for how it went and its number and words.
fn step_lines(
    scenario: &Scenario,
    report: &ScenarioReport,
    theme: &OverlayTheme,
    descriptions: &TargetDescriptions,
) -> Vec<(String, String)> {
    scenario
        .script()
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let mark = match report.steps.get(index).map(|record| &record.outcome) {
                Some(Outcome::Pass) => theme.pass_mark.clone(),
                Some(Outcome::Fail(_)) => theme.fail_mark.clone(),
                Some(Outcome::Skipped) | None => theme.skipped_mark.clone(),
            };
            let words = describe_step(
                step,
                scenario.script().get(..index).unwrap_or(&[]),
                descriptions,
            );
            (mark, format!("{}. {words}", index + 1))
        })
        .collect()
}

/// What a ticked candidate starts with. ASCII, as the bundled font has no
/// tick.
const TICKED: &str = "[x]";

/// What an unticked candidate starts with.
const UNTICKED: &str = "[ ]";

/// The checklists of the checks a recording suggests: one per checkpoint,
/// then the end's, with one row per candidate, which a click ticks or
/// unticks.
fn spawn_checklist(panel: &mut ChildSpawnerCommands<'_>, suggestions: &Suggestions) {
    if suggestions.checkpoints() == 0 {
        if suggestions.is_empty() {
            label(panel, "Nothing changed that a check could say", DIM);
            return;
        }
        label(panel, "Suggested checks (click to tick or untick)", TEXT);
        spawn_rows(panel, suggestions.entries(), Control::Suggestion);
        return;
    }
    label(panel, "Suggested checks (click to tick or untick)", TEXT);
    for checkpoint in 0..suggestions.checkpoints() {
        let number = checkpoint + 1;
        if suggestions.checkpoint_entries(checkpoint).next().is_none() {
            label(panel, format!("Checkpoint {number}: nothing changed"), DIM);
            continue;
        }
        label(panel, format!("Checkpoint {number}"), DIM);
        spawn_rows(panel, suggestions.checkpoint_entries(checkpoint), |index| {
            Control::CheckpointSuggestion(checkpoint, index)
        });
    }
    if suggestions.is_empty() {
        label(panel, "At the end: nothing changed", DIM);
    } else {
        label(panel, "At the end", DIM);
        spawn_rows(panel, suggestions.entries(), Control::Suggestion);
    }
}

/// One checklist row per entry, each clicking as `control` of its index.
fn spawn_rows<'a>(
    panel: &mut ChildSpawnerCommands<'_>,
    entries: impl Iterator<Item = (&'a Check, bool)>,
    control: impl Fn(usize) -> Control,
) {
    for (index, (check, ticked)) in entries.enumerate() {
        let control = control(index);
        panel
            .spawn((
                control,
                NotUnderTest,
                Node {
                    padding: UiRect::axes(px(PADDING), px(1)),
                    border_radius: BorderRadius::all(px(3)),
                    ..Node::default()
                },
                BackgroundColor(if ticked { BUTTON_ON } else { BUTTON }),
            ))
            .observe(
                move |click: On<Pointer<Click>>, mut state: ResMut<GalleryState>| {
                    if click.button == PointerButton::Primary {
                        let action = control_action(control, &state);
                        state.request(action);
                    }
                },
            )
            .with_children(|row| {
                let mark = if ticked { TICKED } else { UNTICKED };
                label(row, format!("{mark} {check}"), TEXT);
            });
    }
}

/// What a check row starts with before it is taken. ASCII, as the bundled
/// font has no other marks.
const TO_ADD: &str = "[+]";

/// What a check row starts with once it is taken.
const ADDED: &str = "[added]";

/// A button in the details panel that asks for `control`'s action, with
/// `text` on it.
fn panel_button(
    parent: &mut ChildSpawnerCommands<'_>,
    control: Control,
    text: impl Into<String>,
    on: bool,
) {
    parent
        .spawn((
            control,
            NotUnderTest,
            Node {
                padding: UiRect::axes(px(PADDING), px(1)),
                border_radius: BorderRadius::all(px(3)),
                ..Node::default()
            },
            BackgroundColor(if on { BUTTON_ON } else { BUTTON }),
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
            label(button, text, TEXT);
        });
}

/// The pick panel: how to pick, or the picked node's targets, the checks that
/// hold for it and the app's check forms, each a button that takes it.
fn spawn_pick_panel(
    panel: &mut ChildSpawnerCommands<'_>,
    picked: Option<&Picked>,
    destination: &str,
    theme: &OverlayTheme,
) {
    let Some(picked) = picked else {
        label(
            panel,
            "Pick mode: point at a node in the app window and click it",
            TEXT,
        );
        label(
            panel,
            "The app gets no input meanwhile; Escape ends it",
            DIM,
        );
        return;
    };
    label(panel, "Target (click another to use it)", DIM);
    row(panel, |row| {
        for (index, target) in picked.targets().iter().enumerate() {
            panel_button(
                row,
                Control::PickTarget(index),
                format!("{target:?}"),
                index == picked.chosen(),
            );
        }
    });
    if picked.checks().is_empty() {
        label(panel, "No built-in check holds for it", DIM);
    } else {
        label(
            panel,
            format!("Checks that hold now (click to add to {destination})"),
            DIM,
        );
    }
    for (index, check) in picked.checks().iter().enumerate() {
        let added = picked.is_added(check);
        let mark = if added { ADDED } else { TO_ADD };
        panel_button(
            panel,
            Control::PickCheck(index),
            format!("{mark} {}", spelled(check)),
            added,
        );
    }
    if picked.forms().is_empty() {
        return;
    }
    label(panel, "App checks (- and + change a value)", DIM);
    for (index, form) in picked.forms().iter().enumerate() {
        match &form.check {
            Ok(check) => {
                let added = picked.is_added(check);
                let mark = if added {
                    ADDED.to_owned()
                } else if form.holds {
                    theme.pass_mark.clone()
                } else {
                    theme.fail_mark.clone()
                };
                panel_button(
                    panel,
                    Control::FormCheck(index),
                    format!("{mark} {}", spelled(check)),
                    added,
                );
            }
            Err(why) => {
                label(panel, format!("{}: {why}", form.form.variant.form()), DIM);
            }
        }
        for (field, value) in form.form.values.iter().enumerate() {
            if !value.can_step() {
                continue;
            }
            let name = form
                .form
                .variant
                .fields
                .get(field)
                .and_then(|field| field.name.clone())
                .unwrap_or_else(|| field.to_string());
            row(panel, |row| {
                label(
                    row,
                    format!("  {name}: {}", value.ron().unwrap_or_default()),
                    DIM,
                );
                panel_button(row, Control::FormField(index, field, false), "-", false);
                panel_button(row, Control::FormField(index, field, true), "+", false);
            });
        }
    }
}
