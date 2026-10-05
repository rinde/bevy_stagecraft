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
        GalleryAction, GalleryPhase, GalleryState, PendingStep, Picking, Recording,
        ScenarioOutcome, Suggestions,
    },
};
use crate::{
    check::Check,
    input::SyntheticInput,
    player::{ScenarioPlayer, ScenarioReport, advance_world},
    session::{self, DEFAULT_CHECKPOINT_KEY, SessionConfig, SessionRecorder},
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
    release_held(world);
    if let Some(mut player) = world.get_resource_mut::<ScenarioPlayer>() {
        player.stop();
    }
    if let Some(reset) = world.get_resource::<GalleryReset>().cloned() {
        (reset.0)(world);
    }
    if let Some(mut input) = world.get_resource_mut::<SyntheticInput>() {
        input.forget_cursor();
    }
    if let Some((width, height)) = entry.scenario.window() {
        for mut window in world
            .query_filtered::<&mut Window, bevy::ecs::query::With<PrimaryWindow>>()
            .iter_mut(world)
        {
            window.resolution.set(pixels(width), pixels(height));
        }
    }
    let setup = entry.scenario.run_setup(world);
    let view = match &setup {
        Ok(_) => entry.scenario.run_view(world),
        Err(_) => Ok(false),
    };
    world.insert_resource(ScenarioReport {
        name: entry.scenario.name().into(),
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
    if matches!(
        action,
        GalleryAction::Select(_)
            | GalleryAction::JumpTo(_)
            | GalleryAction::Restart
            | GalleryAction::Record
    ) {
        state.suggestions = None;
    }
    if matches!(
        action,
        GalleryAction::Select(_)
            | GalleryAction::JumpTo(_)
            | GalleryAction::Restart
            | GalleryAction::TogglePlay
            | GalleryAction::StepFrame
            | GalleryAction::StepStep
            | GalleryAction::TakeOver
            | GalleryAction::Resume
            | GalleryAction::Record
            | GalleryAction::StopRecording
    ) {
        state.picking = Picking::Off;
        state.picked = None;
    }
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
    // These take the primary window back from the person or reset the app,
    // which would spoil what is being recorded.
    if recording
        && matches!(
            action,
            GalleryAction::Select(_)
                | GalleryAction::JumpTo(_)
                | GalleryAction::Restart
                | GalleryAction::TogglePlay
                | GalleryAction::StepFrame
                | GalleryAction::StepStep
                | GalleryAction::Resume
                | GalleryAction::TakeOver
                | GalleryAction::Record
        )
    {
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
        GalleryAction::StopRecording => stop_recording(world, state),
        GalleryAction::Select(index) => select(world, state, index),
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
            let scenario = state
                .selected
                .and_then(|index| world.get_resource::<Gallery>()?.get(index))
                .map(|entry| entry.scenario.clone());
            if let (Some(scenario), Some(mut player)) =
                (scenario, world.get_resource_mut::<ScenarioPlayer>())
            {
                player.pause_before(state.jump);
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
            state.results.insert(index, passed);
        }
        state.step = None;
        state.jump = None;
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
    if world
        .get_resource::<Gallery>()
        .is_some_and(|gallery| gallery.get(next).is_some())
    {
        select(world, state, next);
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
