//! Carrying out requests and pacing the script.

use std::fs;

use bevy::{
    ecs::world::World,
    input::{ButtonInput, keyboard::KeyCode, mouse::MouseButton},
    window::{PrimaryWindow, Window},
};

use super::{
    GalleryReset,
    catalogue::Gallery,
    state::{GalleryAction, GalleryPhase, GalleryState, PendingStep},
};
use crate::{
    input::SyntheticInput,
    player::{ScenarioPlayer, ScenarioReport, advance_world},
};

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
    entry.scenario.run_setup(world);
    entry.scenario.run_view(world);
    world.insert_resource(ScenarioReport {
        name: entry.scenario.name().into(),
        ..ScenarioReport::default()
    });
    state.selected = Some(index);
    state.phase = GalleryPhase::Settling(SETTLE_UPDATES);
    state.playing = true;
    state.taken_over = false;
    state.step = None;
    state.tick = 0;
    state.status.clear();
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

/// Carries out one request.
fn apply(world: &mut World, state: &mut GalleryState, action: GalleryAction) {
    match action {
        GalleryAction::Select(index) => select(world, state, index),
        GalleryAction::Restart => {
            if let Some(index) = state.selected {
                select(world, state, index);
            }
        }
        GalleryAction::TogglePlay => {
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
        GalleryPhase::Idle => {}
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
                player.start(scenario);
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
    let frames = if state.step.is_some() {
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
        state.phase = GalleryPhase::Finished(state.loop_pause);
    }
}
