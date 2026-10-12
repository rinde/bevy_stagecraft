//! Carrying out requests and pacing the script.

use std::fs;

use bevy::{
    app::AppExit,
    ecs::world::World,
    input::{ButtonInput, keyboard::KeyCode, mouse::MouseButton},
    log::{error, info, warn},
    picking::input::PointerInputSettings,
    window::{PrimaryWindow, Window},
};

use super::{
    GalleryReset,
    catalogue::Gallery,
    pick::spelled,
    state::{
        GalleryAction, GalleryPhase, GalleryState, GalleryTab, PendingStep, Picking, Recording,
        ScenarioOutcome, Suggestions, TraceWorld,
    },
    ui::followed_step,
};
use crate::{
    check::{Check, suggest::NoiseFilter},
    extension::Setup,
    input::SyntheticInput,
    player::{ScenarioPlayer, ScenarioReport, advance_world},
    save,
    scenario::Scenario,
    script::check_scale_factor,
    session::{self, DEFAULT_CHECKPOINT_KEY, SessionConfig, SessionRecorder},
    variant::Variant,
};

/// What a scenario recorded in the gallery is called until the person renames
/// the file.
const RECORDING_NAME: &str = "Recording";

/// Updates between the app being set up and the script starting, so layout
/// and picking have seen the new UI. The headless runner settles for the same
/// number.
const SETTLE_UPDATES: u8 = 2;

/// Releases whatever the script is holding down, so a reset or a hand-over
/// never leaves a button or key stuck.
fn release_held(world: &mut World) {
    let buttons: Vec<MouseButton> = world
        .get_resource::<ButtonInput<MouseButton>>()
        .map(|input| input.get_pressed().copied().collect())
        .unwrap_or_default();
    let keys: Vec<KeyCode> = world
        .get_resource::<ButtonInput<KeyCode>>()
        .map(|input| input.get_pressed().copied().collect())
        .unwrap_or_default();
    if let Some(mut input) = world.get_resource_mut::<SyntheticInput>() {
        for button in buttons {
            input.release_button(button);
        }
        for key in keys {
            input.release_key(key);
        }
    }
}

/// The variant the gallery plays in now; the default one (as written) when
/// the catalogue lists none.
fn chosen_variant(world: &World, state: &GalleryState) -> Variant {
    world
        .get_resource::<Gallery>()
        .and_then(|gallery| gallery.variant(state.variant))
        .cloned()
        .unwrap_or_default()
}

/// Puts the primary window at the size and scale factor the scenario runs in,
/// and back to what the app had where it sets none. The first call remembers
/// what the app had.
fn fit_window(world: &mut World, state: &mut GalleryState, scenario: &Scenario) {
    let scale = scenario
        .scale()
        .filter(|scale| check_scale_factor(*scale).is_ok());
    for mut window in world
        .query_filtered::<&mut Window, bevy::ecs::query::With<PrimaryWindow>>()
        .iter_mut(world)
    {
        let resolution = &mut window.resolution;
        let (original_width, original_height, original_scale) =
            *state.original_window.get_or_insert_with(|| {
                (
                    resolution.width(),
                    resolution.height(),
                    resolution.scale_factor_override(),
                )
            });
        let (width, height) = scenario
            .window()
            .map_or((original_width, original_height), |(width, height)| {
                (pixels(width), pixels(height))
            });
        let scale = scale.or(original_scale);
        if resolution.scale_factor_override() != scale {
            resolution.set_scale_factor_override(scale);
        }
        if (resolution.width(), resolution.height()) != (width, height) {
            resolution.set(width, height);
        }
    }
}

/// Resets the app, sets the entry up and arranges for it to play.
fn select(world: &mut World, state: &mut GalleryState, index: usize) {
    let Some(entry) = world
        .get_resource::<Gallery>()
        .and_then(|gallery| gallery.get(index))
        .cloned()
    else {
        bevy::log::warn!("the gallery has no entry {index}");
        return;
    };
    let variant = chosen_variant(world, state);
    let refused = variant.problem(&entry.scenario);
    let scenario = variant.varied(&entry.scenario);
    release_held(world);
    if let Some(mut player) = world.get_resource_mut::<ScenarioPlayer>() {
        player.stop();
        // What the run before kept belongs to a run that is over: the next
        // one starts its own, and until then nothing should show this one.
        drop(player.take_trace());
    }
    if let Some(reset) = world.get_resource::<GalleryReset>().cloned() {
        (reset.0)(world);
    }
    if let Some(mut input) = world.get_resource_mut::<SyntheticInput>() {
        input.forget_cursor();
    }
    fit_window(world, state, &scenario);
    // A variant the scenario cannot run in is a failed setup, as in the
    // recorder: the run is over at once and says why.
    let setup = refused.map_or_else(
        || scenario.run_setup(world),
        |problem| Err(problem.to_string()),
    );
    let view = match &setup {
        Ok(_) => scenario.run_view(world),
        Err(_) => Ok(false),
    };
    world.insert_resource(ScenarioReport {
        name: scenario.name().into(),
        ..ScenarioReport::default()
    });
    state.selected = Some(index);
    state.phase = GalleryPhase::Settling(SETTLE_UPDATES);
    state.playing = true;
    state.taken_over = false;
    state.step = None;
    state.jump = None;
    state.tick = 0;
    state.status = match (&setup, view) {
        (Err(error), _) => format!("Setup failed: {error}"),
        (Ok(_), Err(error)) => format!("View failed: {error}"),
        (Ok(_), Ok(_)) => String::new(),
    };
    state.setup = setup;
}

/// Chooses the variant entries are played in and, with an entry selected,
/// plays it again in that one. An index the catalogue has no variant at
/// changes nothing.
fn select_variant(world: &mut World, state: &mut GalleryState, variant: usize) {
    let count = world
        .get_resource::<Gallery>()
        .map_or(1, Gallery::variant_count);
    if variant >= count {
        warn!("the gallery has no variant {variant}");
        return;
    }
    state.variant = variant;
    if let Some(index) = state.selected {
        select(world, state, index);
    }
}

/// Restarts the selected entry and plays it up to the start of step `step`,
/// where it pauses (see [`GalleryAction::JumpTo`]).
fn jump_to(world: &mut World, state: &mut GalleryState, step: usize) {
    let Some(index) = state.selected else {
        "Select a scenario to jump in".clone_into(&mut state.status);
        return;
    };
    select(world, state, index);
    if state.selected != Some(index) || state.phase == GalleryPhase::Idle {
        return;
    }
    state.jump = Some(step);
    state.status = format!("Playing up to step {}", step + 1);
}

/// Gives up a jump on its way, for a request that wants the player to go its
/// own way: the player carries on from wherever it has got to.
fn cancel_jump(world: &mut World, state: &mut GalleryState) {
    if state.jump.take().is_some()
        && let Some(mut player) = world.get_resource_mut::<ScenarioPlayer>()
    {
        player.pause_before(None);
    }
}

/// A window dimension as the `f32` Bevy's window resolution takes. Sizes above
/// 65535 are not real windows.
fn pixels(size: u32) -> f32 {
    f32::from(u16::try_from(size).unwrap_or(u16::MAX))
}

/// A file-name-safe version of a scenario's name.
fn file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if stem.is_empty() {
        "scenario".into()
    } else {
        stem
    }
}

/// Writes the selected entry as RON and says where.
fn export(world: &World, state: &mut GalleryState) {
    let Some(entry) = state
        .selected
        .and_then(|index| world.get_resource::<Gallery>()?.get(index))
    else {
        "Select a scenario to export".clone_into(&mut state.status);
        return;
    };
    let ron = match entry.scenario.to_ron() {
        Ok(ron) => ron,
        Err(error) => {
            state.status = format!("Export failed: {error}");
            return;
        }
    };
    let path = state
        .export_dir
        .join(format!("{}.ron", file_stem(entry.scenario.name())));
    let written = fs::create_dir_all(&state.export_dir).and_then(|()| fs::write(&path, ron));
    state.status = match written {
        Ok(()) => format!("Exported {}", path.display()),
        Err(error) => format!("Export failed: {error}"),
    };
}

/// Saves the world as it is as a setup for other scenarios, next to the
/// exported ones, and says where (see [`crate::save`]). Works while a
/// recording is on, a script is paused, taken over or finished, and changes
/// nothing in the world; while a script plays it does nothing but say so, as
/// the world is a different one every frame.
fn save_as_setup(world: &mut World, state: &mut GalleryState) {
    if !state.can_save_as_setup() {
        "Pause the script to save it as a setup".clone_into(&mut state.status);
        return;
    }
    let stem = state
        .selected
        .and_then(|index| world.get_resource::<Gallery>()?.get(index))
        .map_or_else(
            || file_stem(RECORDING_NAME),
            |entry| file_stem(entry.scenario.name()),
        );
    let noise = world
        .get_resource::<SessionConfig>()
        .map_or_else(NoiseFilter::default, |config| config.hooks.noise().clone());
    state.status = match save::save_setup(world, &state.export_dir, &stem, &noise) {
        Ok(saved) => {
            let wrote = saved.path.as_ref().map_or_else(
                || "the app's own format".to_owned(),
                |path| path.display().to_string(),
            );
            let setup = match &saved.setup {
                Setup::Scene(name) => format!("setup: Scene({name:?})"),
                Setup::App(value) => format!("setup: App({value})"),
            };
            format!("Saved setup {wrote}; use it in a scenario as {setup}")
        }
        Err(error) => format!("Save as setup failed: {error}"),
    };
}

/// Starts recording a session from what is on screen now.
fn record(world: &mut World, state: &mut GalleryState) {
    let (taken_over, playing) = (state.taken_over, state.playing);
    // The script lets go of the window first, so the real input reaches the
    // app, and so the state captured is the one the person sees.
    if state.selected.is_some() {
        state.taken_over = true;
        state.playing = false;
        state.step = None;
        release_held(world);
    }
    match session::start(world, RECORDING_NAME) {
        Ok(()) => {
            let key = world
                .get_resource::<SessionConfig>()
                .map_or(DEFAULT_CHECKPOINT_KEY, |config| config.hooks.checkpoint());
            state.status = format!(
                "Recording: use the app ({key:?} takes a checkpoint), then press Stop recording"
            );
        }
        Err(error) => {
            state.taken_over = taken_over;
            state.playing = playing;
            state.status = format!("Cannot record: {error}");
        }
    }
}

/// Drops the selected entry and puts the app's state back, so what follows
/// starts from an empty world with nothing selected.
fn clear(world: &mut World, state: &mut GalleryState) {
    release_held(world);
    if let Some(mut player) = world.get_resource_mut::<ScenarioPlayer>() {
        player.stop();
        // The run before is over, so its trace should not show on the new
        // recording.
        drop(player.take_trace());
    }
    if let Some(reset) = world.get_resource::<GalleryReset>().cloned() {
        (reset.0)(world);
    }
    if let Some(mut input) = world.get_resource_mut::<SyntheticInput>() {
        input.forget_cursor();
    }
    world.insert_resource(ScenarioReport::default());
    state.trace = None;
    state.trace_step = None;
    state.selected = None;
    state.phase = GalleryPhase::Idle;
    state.playing = false;
    state.taken_over = false;
    state.step = None;
    state.jump = None;
    state.tick = 0;
    state.setup = Ok(false);
}

/// Starts recording a new scenario, from an empty world or from the app as it
/// is.
fn new_recording(world: &mut World, state: &mut GalleryState, empty: bool) {
    if empty {
        clear(world, state);
    }
    record(world, state);
}

/// Stops recording, says where the file went and offers the checks it
/// suggests.
fn stop_recording(world: &mut World, state: &mut GalleryState) {
    match session::stop(world) {
        Ok(saved) => {
            let suggestions = Suggestions::new(saved);
            let (ticked, all) = suggestions.tally();
            state.status = format!(
                "{}, {}",
                suggestions.session().describe(),
                suggested(ticked, all)
            );
            state.suggestions = Some(suggestions);
        }
        Err(error) => state.status = format!("Cannot save the recording: {error}"),
    }
}

/// How many of the suggested checks the file holds, in words.
fn suggested(ticked: usize, all: usize) -> String {
    match all {
        0 => "no checks suggested".to_owned(),
        1 => format!("{ticked} of 1 suggested check kept"),
        _ => format!("{ticked} of {all} suggested checks kept"),
    }
}

/// Ticks or unticks a suggested check, of the checkpoint given or else of
/// the end, and says what the file now holds.
fn toggle_suggestion(state: &mut GalleryState, checkpoint: Option<usize>, index: usize) {
    let Some(suggestions) = state.suggestions.as_mut() else {
        return;
    };
    let saved = match checkpoint {
        Some(checkpoint) => suggestions.toggle_at_checkpoint(checkpoint, index),
        None => suggestions.toggle(index),
    };
    state.status = match saved {
        Ok(()) => {
            let (ticked, all) = suggestions.tally();
            format!(
                "Saved {}, {}",
                suggestions.session().path.display(),
                suggested(ticked, all)
            )
        }
        Err(error) => format!("Cannot save the checks: {error}"),
    };
}

/// Starts or stops pick mode. Without a recording it needs a selected
/// scenario, which it pauses: a check picked then is about the state on
/// screen.
fn toggle_pick(state: &mut GalleryState, recording: bool) {
    if state.is_picking() {
        state.picking = Picking::Off;
        state.picked = None;
        state.status.clear();
        return;
    }
    if !recording && state.selected.is_none() {
        "Select a scenario or start recording to pick".clone_into(&mut state.status);
        return;
    }
    if !recording {
        state.playing = false;
        state.step = None;
    }
    state.picking = Picking::On;
    state.picked = None;
    "Pick: point at a node in the app window and click it (Escape ends)"
        .clone_into(&mut state.status);
}

/// Puts a picked check where it belongs: a `Check` step at this point of the
/// recording, or else an expectation of the selected scenario, in the
/// catalogue and in the run being played.
fn place_check(world: &mut World, state: &mut GalleryState, check: Check, recording: bool) {
    let written = spelled(&check);
    if recording {
        if let Err(error) = session::add_check(world, check.clone()) {
            state.status = format!("Cannot add the check: {error}");
            return;
        }
        state.status = format!("Added Check({written}) to the recording here");
    } else {
        let Some(index) = state.selected else {
            "Select a scenario to add the check to".clone_into(&mut state.status);
            return;
        };
        let name = world
            .get_resource_mut::<Gallery>()
            .and_then(|mut gallery| {
                let entry = gallery.get_mut(index)?;
                entry.scenario.push_expect(check.clone());
                Some(entry.scenario.name().to_owned())
            })
            .unwrap_or_default();
        if let Some(mut player) = world.get_resource_mut::<ScenarioPlayer>() {
            player.add_expect(check.clone());
        }
        state.revision = state.revision.wrapping_add(1);
        state.status = format!("Added {written} to the expect of {name}; Export RON writes it out");
    }
    if let Some(picked) = state.picked.as_mut() {
        picked.mark_added(check);
    }
}

/// Carries out a request about the picked node.
fn apply_pick(world: &mut World, state: &mut GalleryState, action: GalleryAction, recording: bool) {
    let chosen = match action {
        GalleryAction::PickTarget(index) => {
            if let Some(picked) = state.picked.as_mut() {
                picked.choose_target(world, index);
            }
            None
        }
        GalleryAction::StepFormField { form, field, up } => {
            if let Some(picked) = state.picked.as_mut() {
                picked.step_field(world, form, field, up);
            }
            None
        }
        GalleryAction::AddPickedCheck(index) => state
            .picked
            .as_ref()
            .and_then(|picked| picked.checks().get(index).cloned()),
        GalleryAction::AddFormCheck(index) => state
            .picked
            .as_ref()
            .and_then(|picked| picked.forms().get(index))
            .and_then(|form| form.check.clone().ok()),
        _ => None,
    };
    if let Some(check) = chosen {
        place_check(world, state, check, recording);
    }
}

/// Lets go of what `action` makes stale: the checklist belongs to the
/// recording it came from, and what is picked to the state on screen, which
/// playing on, resetting or starting and stopping a recording moves on from.
fn forget_for(state: &mut GalleryState, action: GalleryAction) {
    // A jump replays the same run, so the trace stays; a new selection or a
    // restart starts over.
    if matches!(
        action,
        GalleryAction::Select(_) | GalleryAction::SelectVariant(_) | GalleryAction::Restart
    ) {
        state.trace = None;
        state.trace_step = None;
    }
    if matches!(
        action,
        GalleryAction::Select(_)
            | GalleryAction::SelectVariant(_)
            | GalleryAction::JumpTo(_)
            | GalleryAction::Restart
            | GalleryAction::Record
            | GalleryAction::NewEmpty
            | GalleryAction::NewFromCurrent
    ) {
        state.suggestions = None;
    }
    if matches!(
        action,
        GalleryAction::Select(_)
            | GalleryAction::SelectVariant(_)
            | GalleryAction::JumpTo(_)
            | GalleryAction::Restart
            | GalleryAction::TogglePlay
            | GalleryAction::StepFrame
            | GalleryAction::StepStep
            | GalleryAction::TakeOver
            | GalleryAction::Resume
            | GalleryAction::Record
            | GalleryAction::NewEmpty
            | GalleryAction::NewFromCurrent
            | GalleryAction::StopRecording
    ) {
        state.picking = Picking::Off;
        state.picked = None;
    }
}

/// Whether the request takes the primary window back from the person or resets
/// the app, which would spoil what is being recorded.
const fn spoils_a_recording(action: GalleryAction) -> bool {
    matches!(
        action,
        GalleryAction::Select(_)
            | GalleryAction::SelectVariant(_)
            | GalleryAction::JumpTo(_)
            | GalleryAction::Restart
            | GalleryAction::TogglePlay
            | GalleryAction::StepFrame
            | GalleryAction::StepStep
            | GalleryAction::Resume
            | GalleryAction::TakeOver
            | GalleryAction::Record
            | GalleryAction::NewEmpty
            | GalleryAction::NewFromCurrent
    )
}

/// Shows a panel of the Scenarios window.
fn show_tab(state: &mut GalleryState, tab: GalleryTab) {
    state.tab = tab;
    if state.is_picking() {
        "Stop picking to see the panel".clone_into(&mut state.status);
    }
}

/// Pins the Trace tab to the step it shows now, or lets it follow again.
fn toggle_trace_follow(world: &World, state: &mut GalleryState) {
    state.trace_step = if state.trace_step.is_some() {
        None
    } else {
        world
            .get_resource::<ScenarioReport>()
            .map(|report| followed_step(report, world.get_resource::<ScenarioPlayer>()))
    };
}

/// Carries out one request.
fn apply(world: &mut World, state: &mut GalleryState, action: GalleryAction) {
    if state.is_running_all() {
        "Running every scenario: the controls are off until it is done"
            .clone_into(&mut state.status);
        return;
    }
    let recording = world
        .get_resource::<SessionRecorder>()
        .is_some_and(SessionRecorder::is_recording);
    if recording && spoils_a_recording(action) {
        "Stop recording first".clone_into(&mut state.status);
        return;
    }
    forget_for(state, action);
    match action {
        GalleryAction::TogglePick => toggle_pick(state, recording),
        GalleryAction::PickTarget(_)
        | GalleryAction::StepFormField { .. }
        | GalleryAction::AddPickedCheck(_)
        | GalleryAction::AddFormCheck(_) => apply_pick(world, state, action, recording),
        GalleryAction::ToggleSuggestion(index) => toggle_suggestion(state, None, index),
        GalleryAction::ToggleCheckpointSuggestion { checkpoint, index } => {
            toggle_suggestion(state, Some(checkpoint), index);
        }
        GalleryAction::Record => record(world, state),
        GalleryAction::NewEmpty => new_recording(world, state, true),
        GalleryAction::NewFromCurrent => new_recording(world, state, false),
        GalleryAction::StopRecording => {
            // The checklist is on the Scenario tab.
            state.tab = GalleryTab::Scenario;
            stop_recording(world, state);
        }
        GalleryAction::ShowTab(tab) => show_tab(state, tab),
        GalleryAction::ToggleTraceFollow => toggle_trace_follow(world, state),
        GalleryAction::InspectStep(step) => state.trace_step = Some(step),
        GalleryAction::ShowTraceWorld(shown) => state.trace_world = shown,
        GalleryAction::Select(index) => select(world, state, index),
        GalleryAction::SelectVariant(variant) => select_variant(world, state, variant),
        GalleryAction::JumpTo(step) => jump_to(world, state, step),
        GalleryAction::Restart => {
            if let Some(index) = state.selected {
                select(world, state, index);
            }
        }
        GalleryAction::TogglePlay => {
            cancel_jump(world, state);
            if state.taken_over {
                state.taken_over = false;
                state.playing = true;
            } else if matches!(state.phase, GalleryPhase::Finished(_)) && !state.looping {
                if let Some(index) = state.selected {
                    select(world, state, index);
                }
            } else if state.selected.is_some() {
                state.playing = !state.playing;
                state.step = None;
            }
        }
        GalleryAction::StepFrame | GalleryAction::StepStep => {
            if matches!(state.phase, GalleryPhase::Idle | GalleryPhase::Finished(_)) {
                return;
            }
            cancel_jump(world, state);
            state.taken_over = false;
            state.playing = false;
            state.step = Some(if action == GalleryAction::StepFrame {
                PendingStep::Frame
            } else {
                let player = world.get_resource::<ScenarioPlayer>();
                PendingStep::Step {
                    from_started: player.map_or(0, ScenarioPlayer::steps_started),
                    from_boundary: player.is_none_or(ScenarioPlayer::at_step_boundary),
                }
            });
        }
        GalleryAction::SetSpeed(speed) => state.speed = speed,
        GalleryAction::ToggleLoop => state.looping = !state.looping,
        GalleryAction::TakeOver => {
            cancel_jump(world, state);
            if state.selected.is_some() && !state.taken_over {
                state.taken_over = true;
                state.playing = false;
                state.step = None;
                release_held(world);
            }
        }
        GalleryAction::Resume => {
            if state.taken_over {
                state.taken_over = false;
                state.playing = true;
            }
        }
        GalleryAction::Export => export(world, state),
        GalleryAction::SaveAsSetup => save_as_setup(world, state),
    }
}

/// Carries out the requests made since the last update.
pub(super) fn apply_requests(world: &mut World) {
    let Some(mut state) = world.remove_resource::<GalleryState>() else {
        return;
    };
    for action in core::mem::take(&mut state.pending) {
        apply(world, &mut state, action);
    }
    // Bevy's mouse picking may read this update's input before the gallery
    // sorts it, so it is switched off as soon as the script or pick mode
    // takes the app window, not an update later.
    let owned = state.input_is_scripted() || state.is_picking();
    if let Some(mut settings) = world.get_resource_mut::<PointerInputSettings>()
        && settings.is_mouse_enabled == owned
    {
        settings.is_mouse_enabled = !owned;
        settings.is_touch_enabled = !owned;
    }
    state.recording = if world
        .get_resource::<SessionRecorder>()
        .is_some_and(SessionRecorder::is_recording)
    {
        Recording::On
    } else {
        Recording::Off
    };
    world.insert_resource(state);
}

/// Advances the script by as many frames as the mode and speed allow.
pub(super) fn drive(world: &mut World) {
    let Some(mut state) = world.remove_resource::<GalleryState>() else {
        return;
    };
    drive_state(world, &mut state);
    world.insert_resource(state);
}

/// The body of [`drive`], with the state taken out of the world.
fn drive_state(world: &mut World, state: &mut GalleryState) {
    if state.taken_over {
        return;
    }
    match state.phase {
        GalleryPhase::Idle => {
            if state.is_running_all() {
                run_next(world, state);
            }
        }
        GalleryPhase::Settling(left) => {
            if left > 1 {
                state.phase = GalleryPhase::Settling(left - 1);
                return;
            }
            let variant = chosen_variant(world, state);
            let scenario = state
                .selected
                .and_then(|index| world.get_resource::<Gallery>()?.get(index))
                .map(|entry| variant.varied(&entry.scenario));
            if let (Some(scenario), Some(mut player)) =
                (scenario, world.get_resource_mut::<ScenarioPlayer>())
            {
                player.pause_before(state.jump);
                // Not for a run of everything, which keeps nothing but its
                // outcomes.
                if !state.is_running_all() {
                    player.enable_trace(NoiseFilter::default());
                }
                player.start_after_setup(scenario, state.setup.clone());
            }
            state.phase = GalleryPhase::Running;
        }
        GalleryPhase::Running => run_frames(world, state),
        GalleryPhase::Finished(wait) => {
            if !(state.playing && state.looping) {
                return;
            }
            if wait == 0 {
                if let Some(index) = state.selected {
                    select(world, state, index);
                }
            } else {
                state.phase = GalleryPhase::Finished(wait - 1);
            }
        }
    }
}

/// Runs this update's frames of script and notices when it ends.
fn run_frames(world: &mut World, state: &mut GalleryState) {
    state.tick = state.tick.wrapping_add(1);
    // A jump plays one script frame per update at any speed, as the headless
    // runner does, so it lands on the very frame that playing up to the step
    // would have.
    let frames = if state.step.is_some() || state.jump.is_some() {
        1
    } else if state.playing {
        state.speed.frames_at(state.tick)
    } else {
        0
    };
    for index in 0..frames {
        // Only the first frame of an update sees fresh layout and picking.
        advance_world(world, index == 0);
    }
    let halted = world
        .get_resource::<ScenarioPlayer>()
        .is_some_and(ScenarioPlayer::halted);
    if halted {
        arrive(world, state);
    }
    let (active, boundary, started) =
        world
            .get_resource::<ScenarioPlayer>()
            .map_or((false, true, 0), |player| {
                (
                    player.is_active(),
                    player.at_step_boundary(),
                    player.steps_started(),
                )
            });
    match state.step {
        Some(PendingStep::Frame) if frames > 0 => state.step = None,
        Some(PendingStep::Step {
            from_started,
            from_boundary,
        }) if !active || (boundary && (started != from_started || !from_boundary)) => {
            state.step = None;
        }
        _ => {}
    }
    if !active {
        let passed = world
            .get_resource::<ScenarioReport>()
            .is_some_and(ScenarioReport::passed);
        if let Some(index) = state.selected {
            state.results.insert((index, state.variant), passed);
        }
        state.step = None;
        state.jump = None;
        if let Some(trace) = world
            .get_resource_mut::<ScenarioPlayer>()
            .and_then(|mut player| player.take_trace())
        {
            state.trace = Some(trace);
            state.revision = state.revision.wrapping_add(1);
        }
        if state.is_running_all() {
            record_outcome(world, state);
            run_next(world, state);
            return;
        }
        state.phase = GalleryPhase::Finished(state.loop_pause);
    }
}

/// The player has halted before the step a jump was after: pause there, with
/// the live app as it is.
fn arrive(world: &mut World, state: &mut GalleryState) {
    let Some(step) = state.jump.take() else {
        return;
    };
    if let Some(mut player) = world.get_resource_mut::<ScenarioPlayer>() {
        player.pause_before(None);
    }
    state.playing = false;
    state.step = None;
    // The step it stopped before is what the person came for; the world as
    // it found it is the one on screen.
    state.trace_step = Some(step);
    state.trace_world = TraceWorld::Before;
    state.status = format!("Paused before step {}", step + 1);
}

/// Adds the scenario that just finished to the run of the whole catalogue.
fn record_outcome(world: &World, state: &mut GalleryState) {
    let Some(report) = world.get_resource::<ScenarioReport>() else {
        return;
    };
    let mut failures = report.failures();
    if failures.is_empty() && !report.passed() {
        failures.push("the scenario did not finish".into());
    }
    if let Some(run) = state.run_all.as_mut() {
        run.outcomes.push(ScenarioOutcome {
            name: report.name.clone(),
            failures,
        });
    }
}

/// Plays the entry after the last one that finished in a run of the whole
/// catalogue, or ends the run when there is none.
fn run_next(world: &mut World, state: &mut GalleryState) {
    let Some(next) = state.run_all.as_ref().map(|run| run.outcomes.len()) else {
        return;
    };
    // Every entry in every variant, entry by entry.
    let variants = world
        .get_resource::<Gallery>()
        .map_or(1, Gallery::variant_count);
    let (entry, variant) = (next / variants, next % variants);
    if world
        .get_resource::<Gallery>()
        .is_some_and(|gallery| gallery.get(entry).is_some())
    {
        state.variant = variant;
        select(world, state, entry);
        return;
    }
    let report = state.run_all.take().unwrap_or_default();
    if report.outcomes.is_empty() {
        warn!("the gallery has no scenarios to run");
    } else if report.passed() {
        info!("{}", report.summary());
    } else {
        error!("{}", report.summary());
    }
    state.status = format!(
        "Done: {}",
        report.summary().lines().next().unwrap_or_default()
    );
    let exit = if report.passed() {
        AppExit::Success
    } else {
        AppExit::error()
    };
    world.insert_resource(report);
    if world.write_message(exit).is_none() {
        warn!("could not end the app after running every scenario");
    }
}
