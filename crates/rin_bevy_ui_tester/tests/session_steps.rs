//! Turning a log of real input into steps, without a window.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests may panic on a broken expectation"
)]

use bevy::{
    input::{ButtonState, keyboard::KeyCode, mouse::MouseButton},
    math::Vec2,
};
use rin_bevy_ui_tester::{
    Modifiers, RecordedEvent, RecordedKind, Step, StepOptions, Target, read_log, steps_from_events,
};

const fn at(frame: u32, kind: RecordedKind) -> RecordedEvent {
    RecordedEvent { frame, kind }
}

const fn moved(frame: u32, x: f32, y: f32) -> RecordedEvent {
    at(frame, RecordedKind::Moved(Vec2::new(x, y)))
}

const fn button(frame: u32, state: ButtonState, x: f32, y: f32, target: Target) -> RecordedEvent {
    at(
        frame,
        RecordedKind::Button {
            button: MouseButton::Left,
            state,
            position: Vec2::new(x, y),
            target,
        },
    )
}

const fn key(frame: u32, key: KeyCode, state: ButtonState) -> RecordedEvent {
    at(frame, RecordedKind::Key { key, state })
}

fn convert(events: &[RecordedEvent]) -> Vec<Step> {
    let last = events.last().map_or(0, |event| event.frame);
    steps_from_events(events, last, &StepOptions::default())
}

#[test]
fn a_straight_path_of_many_samples_becomes_one_glide() {
    let mut events = vec![moved(0, 0.0, 0.0)];
    events
        .extend((1_u16..=40).map(|i| moved(u32::from(i), 5.0 * f32::from(i), 2.0 * f32::from(i))));
    let steps = convert(&events);
    let glides: Vec<_> = steps
        .iter()
        .filter(|step| matches!(step, Step::Glide { .. }))
        .collect();
    assert_eq!(glides.len(), 1, "{steps:?}");
    assert_eq!(
        glides[0],
        &Step::Glide {
            target: Target::screen(Vec2::new(200.0, 80.0)),
            frames: 40
        }
    );
}

#[test]
fn a_path_with_a_corner_keeps_the_corner() {
    let mut events = vec![moved(0, 0.0, 0.0)];
    events.extend((1_u16..=20).map(|i| moved(u32::from(i), 10.0 * f32::from(i), 0.0)));
    events.extend((1_u16..=20).map(|i| moved(20 + u32::from(i), 200.0, 10.0 * f32::from(i))));
    let glides = convert(&events)
        .into_iter()
        .filter(|step| matches!(step, Step::Glide { .. }))
        .count();
    assert_eq!(glides, 2);
}

#[test]
fn a_press_and_release_in_place_is_a_click_on_the_label() {
    let label = Target::text("+1");
    let events = [
        moved(5, 100.0, 50.0),
        button(6, ButtonState::Pressed, 100.0, 50.0, label.clone()),
        button(8, ButtonState::Released, 100.0, 50.0, label),
    ];
    let steps = convert(&events);
    assert!(steps.contains(&Step::click()), "{steps:?}");
    assert!(
        steps.iter().any(|step| matches!(
            step,
            Step::MoveTo(Target::Text(text)) if text == "+1"
        )),
        "the move to the button is by label: {steps:?}"
    );
}

#[test]
fn a_path_that_ends_on_a_button_glides_to_its_label() {
    let mut events = vec![moved(0, 0.0, 0.0), moved(1, 5.0, 0.0)];
    events
        .extend((2_u16..=30).map(|i| moved(u32::from(i), 5.0 * f32::from(i), 3.0 * f32::from(i))));
    events.push(button(
        31,
        ButtonState::Pressed,
        150.0,
        90.0,
        Target::text("Go"),
    ));
    events.push(button(
        32,
        ButtonState::Released,
        150.0,
        90.0,
        Target::text("Go"),
    ));
    let steps = convert(&events);
    assert!(
        steps.iter().any(|step| matches!(
            step,
            Step::Glide { target: Target::Text(text), .. } if text == "Go"
        )),
        "{steps:?}"
    );
}

#[test]
fn a_straight_press_move_release_is_a_drag() {
    let events = [
        moved(0, 10.0, 10.0),
        button(
            5,
            ButtonState::Pressed,
            10.0,
            10.0,
            Target::screen(Vec2::new(10.0, 10.0)),
        ),
        moved(10, 60.0, 10.0),
        moved(15, 110.0, 10.0),
        button(
            16,
            ButtonState::Released,
            110.0,
            10.0,
            Target::screen(Vec2::new(110.0, 10.0)),
        ),
    ];
    let steps = convert(&events);
    assert!(
        steps.iter().any(|step| matches!(
            step,
            Step::Drag { from: Target::Screen(from), to: Target::Screen(to), frames: 9 }
                if *from == Vec2::new(10.0, 10.0) && *to == Vec2::new(110.0, 10.0)
        )),
        "{steps:?}"
    );
}

#[test]
fn a_bent_drag_is_spelt_out_so_the_bend_is_kept() {
    let mut events = vec![
        moved(0, 0.0, 0.0),
        button(
            1,
            ButtonState::Pressed,
            0.0,
            0.0,
            Target::screen(Vec2::ZERO),
        ),
    ];
    events.extend((1_u16..=10).map(|i| moved(1 + u32::from(i), 10.0 * f32::from(i), 0.0)));
    events.extend((1_u16..=10).map(|i| moved(11 + u32::from(i), 100.0, 10.0 * f32::from(i))));
    events.push(button(
        22,
        ButtonState::Released,
        100.0,
        100.0,
        Target::screen(Vec2::new(100.0, 100.0)),
    ));
    let steps = convert(&events);
    assert!(steps.contains(&Step::press()), "{steps:?}");
    assert!(steps.contains(&Step::release()), "{steps:?}");
    assert!(!steps.iter().any(|step| matches!(step, Step::Drag { .. })));
}

#[test]
fn long_idle_time_is_shortened_to_a_short_wait() {
    let events = [
        moved(0, 10.0, 10.0),
        button(
            1,
            ButtonState::Pressed,
            10.0,
            10.0,
            Target::screen(Vec2::new(10.0, 10.0)),
        ),
        button(
            2,
            ButtonState::Released,
            10.0,
            10.0,
            Target::screen(Vec2::new(10.0, 10.0)),
        ),
        button(
            1002,
            ButtonState::Pressed,
            10.0,
            10.0,
            Target::screen(Vec2::new(10.0, 10.0)),
        ),
        button(
            1003,
            ButtonState::Released,
            10.0,
            10.0,
            Target::screen(Vec2::new(10.0, 10.0)),
        ),
    ];
    let steps = convert(&events);
    let longest = steps
        .iter()
        .filter_map(|step| match step {
            Step::Wait(frames) => Some(*frames),
            _ => None,
        })
        .max()
        .unwrap();
    assert_eq!(longest, 30, "{steps:?}");
}

#[test]
fn short_gaps_keep_their_length() {
    let events = [
        moved(0, 10.0, 10.0),
        button(
            1,
            ButtonState::Pressed,
            10.0,
            10.0,
            Target::screen(Vec2::new(10.0, 10.0)),
        ),
        button(
            2,
            ButtonState::Released,
            10.0,
            10.0,
            Target::screen(Vec2::new(10.0, 10.0)),
        ),
        button(
            32,
            ButtonState::Pressed,
            10.0,
            10.0,
            Target::screen(Vec2::new(10.0, 10.0)),
        ),
        button(
            33,
            ButtonState::Released,
            10.0,
            10.0,
            Target::screen(Vec2::new(10.0, 10.0)),
        ),
    ];
    assert!(convert(&events).contains(&Step::Wait(30)));
}

#[test]
fn a_key_press_and_release_is_a_tap() {
    let events = [
        key(3, KeyCode::KeyA, ButtonState::Pressed),
        key(5, KeyCode::KeyA, ButtonState::Released),
    ];
    assert!(convert(&events).contains(&Step::tap(KeyCode::KeyA)));
}

#[test]
fn a_modifier_held_around_a_key_folds_into_the_tap() {
    let events = [
        key(3, KeyCode::ControlLeft, ButtonState::Pressed),
        key(4, KeyCode::KeyS, ButtonState::Pressed),
        key(5, KeyCode::KeyS, ButtonState::Released),
        key(6, KeyCode::ControlLeft, ButtonState::Released),
    ];
    assert!(convert(&events).contains(&Step::tap_with(KeyCode::KeyS, Modifiers::CTRL)),);
}

#[test]
fn a_modifier_held_while_clicking_stays_a_held_key() {
    let at_ten = Target::screen(Vec2::new(10.0, 10.0));
    let events = [
        moved(0, 10.0, 10.0),
        key(2, KeyCode::ShiftLeft, ButtonState::Pressed),
        button(3, ButtonState::Pressed, 10.0, 10.0, at_ten.clone()),
        button(4, ButtonState::Released, 10.0, 10.0, at_ten),
        key(6, KeyCode::ShiftLeft, ButtonState::Released),
    ];
    let steps = convert(&events);
    let order: Vec<_> = steps
        .iter()
        .filter(|step| !matches!(step, Step::Wait(_) | Step::MoveTo(_)))
        .collect();
    assert!(matches!(
        order[0],
        Step::Key {
            key: KeyCode::ShiftLeft,
            ..
        }
    ));
    assert_eq!(order[1], &Step::click());
    assert!(matches!(
        order[2],
        Step::KeyUp {
            key: KeyCode::ShiftLeft,
            ..
        }
    ));
}

#[test]
fn the_session_ends_with_a_settling_wait() {
    let events = [
        key(3, KeyCode::KeyA, ButtonState::Pressed),
        key(4, KeyCode::KeyA, ButtonState::Released),
    ];
    assert!(matches!(convert(&events).last(), Some(Step::Wait(wait)) if *wait >= 10));
}

#[test]
fn wheel_messages_of_one_frame_add_up() {
    let at_ten = Vec2::new(10.0, 10.0);
    let events = [
        moved(0, 10.0, 10.0),
        at(
            4,
            RecordedKind::Wheel {
                x: 0.0,
                y: 24.0,
                position: at_ten,
            },
        ),
        at(
            4,
            RecordedKind::Wheel {
                x: 0.0,
                y: 24.0,
                position: at_ten,
            },
        ),
    ];
    assert!(convert(&events).contains(&Step::wheel(0.0, 48.0)));
}

#[test]
fn a_checkpoint_falls_between_the_steps_around_it_and_is_no_step_itself() {
    let label = Target::text("+1");
    let events = [
        moved(0, 50.0, 50.0),
        button(10, ButtonState::Pressed, 50.0, 50.0, label.clone()),
        button(11, ButtonState::Released, 50.0, 50.0, label.clone()),
        at(30, RecordedKind::Checkpoint),
        button(40, ButtonState::Pressed, 50.0, 50.0, label.clone()),
        button(41, ButtonState::Released, 50.0, 50.0, label),
        at(60, RecordedKind::Checkpoint),
    ];
    let logged = read_log(&events, 70, &StepOptions::default());
    assert_eq!(
        logged.steps,
        steps_from_events(&events, 70, &StepOptions::default())
    );
    assert_eq!(logged.checkpoints.len(), 2);
    let clicks: Vec<usize> = logged
        .steps
        .iter()
        .enumerate()
        .filter(|(_, step)| **step == Step::click())
        .map(|(index, _)| index)
        .collect();
    assert_eq!(clicks.len(), 2, "{:?}", logged.steps);
    assert!(
        clicks[0] < logged.checkpoints[0] && logged.checkpoints[0] <= clicks[1],
        "the first checkpoint is between the clicks: {logged:?}"
    );
    assert!(clicks[1] < logged.checkpoints[1], "{logged:?}");
    // The script's clock reaches the checkpoint's frame before its checks.
    let frames_before = |count: usize| -> u32 {
        logged.steps[..count]
            .iter()
            .map(|step| match step {
                Step::Wait(frames) | Step::Glide { frames, .. } => u32::from(*frames),
                _ => 1,
            })
            .sum()
    };
    assert_eq!(frames_before(logged.checkpoints[0]), 30, "{logged:?}");
}

#[test]
fn a_checkpoint_while_a_button_is_held_keeps_the_press_and_release_apart() {
    let label = Target::text("+1");
    let events = [
        moved(0, 50.0, 50.0),
        button(10, ButtonState::Pressed, 50.0, 50.0, label.clone()),
        at(12, RecordedKind::Checkpoint),
        button(14, ButtonState::Released, 50.0, 50.0, label),
    ];
    let logged = read_log(&events, 20, &StepOptions::default());
    let press = logged
        .steps
        .iter()
        .position(|step| *step == Step::press())
        .unwrap();
    let release = logged
        .steps
        .iter()
        .position(|step| *step == Step::release())
        .unwrap();
    assert!(press < logged.checkpoints[0] && logged.checkpoints[0] <= release);
}

#[test]
fn a_checkpoint_inside_a_folded_chord_moves_after_it() {
    let events = [
        key(0, KeyCode::ControlLeft, ButtonState::Pressed),
        at(1, RecordedKind::Checkpoint),
        key(2, KeyCode::KeyS, ButtonState::Pressed),
        key(3, KeyCode::KeyS, ButtonState::Released),
        key(4, KeyCode::ControlLeft, ButtonState::Released),
    ];
    let logged = read_log(&events, 20, &StepOptions::default());
    let tap = logged
        .steps
        .iter()
        .position(|step| *step == Step::tap_with(KeyCode::KeyS, Modifiers::CTRL))
        .unwrap();
    assert_eq!(logged.checkpoints, vec![tap + 1], "{logged:?}");
}
