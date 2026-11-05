//! A scenario plays the same way on every run.
//!
//! Since Bevy 0.20 several of Bevy's own system sets (the UI's in `PostUpdate`
//! among them) are ordered only where systems' tracked data access conflicts.
//! These tests play a scenario whose every click moves and relabels what it
//! clicked, so layout, text and the actionability wait all take part, and
//! compare whole runs.

#![allow(
    clippy::expect_used,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

mod common;

use bevy::{
    prelude::*,
    ui::px,
    ui_widgets::{Activate, Button},
};
use bevy_stagecraft::{Check, NoiseFilter, Scenario, Step, Target};
use common::{SHUFFLE_SEEDS, shuffle, shuffled_schedules};

/// How far the button moves to the right each time it is activated.
const HOP: f32 = 60.0;

/// How many times the hopping button was activated.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Hops(u32);

/// Marks the text inside the hopping button.
#[derive(Component)]
struct HopLabel;

/// A camera and a "Hop" button that, when activated, moves right and says how
/// often it was activated, so the next step must wait for the new layout.
fn spawn_hopping_button(world: &mut World) {
    world.init_resource::<Hops>();
    world.spawn(Camera2d);
    world
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: px(20),
                top: px(40),
                padding: UiRect::all(px(8)),
                ..Node::default()
            },
            BackgroundColor(Color::WHITE),
            children![(HopLabel, Text::new("Hop"), TextColor(Color::BLACK))],
        ))
        .observe(
            |activate: On<Activate>,
             mut hops: ResMut<Hops>,
             mut nodes: Query<&mut Node>,
             mut labels: Query<&mut Text, With<HopLabel>>| {
                hops.0 += 1;
                if let Ok(mut node) = nodes.get_mut(activate.entity)
                    && let Val::Px(left) = node.left
                {
                    node.left = px(left + HOP);
                }
                for mut text in &mut labels {
                    text.0 = format!("Hop {}", hops.0);
                }
            },
        );
}

/// Clicks the button three times, each time where it has moved to, by its new
/// label, then checks the count.
fn hop_three_times(scale_factor: f32) -> Scenario {
    Scenario::new("hop three times")
        .window_size(320, 160)
        .scale_factor(scale_factor)
        .setup(spawn_hopping_button)
        .steps([
            Step::glide(Target::text("Hop"), 5),
            Step::click(),
            Step::move_to(Target::text("Hop 1")),
            Step::click(),
            Step::glide(Target::text("Hop 2"), 3),
            Step::click(),
            Step::wait(2),
            Step::check(Check::resource::<Hops>(".0").eq(3_u32)),
        ])
        .expect_check(Check::shows("Hop 3"))
}

#[test]
fn the_same_scenario_played_twice_headless_gives_identical_reports_and_traces() {
    for scale_factor in [1.0, 1.5] {
        let scenario = hop_three_times(scale_factor);
        let (first_report, first_trace) = scenario.run_headless_traced();
        let (second_report, second_trace) = scenario.run_headless_traced();

        assert!(
            first_report.passed(),
            "at scale {scale_factor}: {:?}",
            first_report.failures()
        );
        assert_eq!(
            first_report, second_report,
            "at scale {scale_factor}, the reports are identical"
        );
        assert_eq!(
            first_trace, second_trace,
            "at scale {scale_factor}, the traces are identical"
        );
    }
}

#[test]
fn shuffling_the_order_of_unordered_systems_changes_no_report_or_trace() {
    for scale_factor in [1.0, 1.5] {
        let scenario = hop_three_times(scale_factor);
        let (expected_report, expected_trace) = scenario.run_headless_traced();
        assert!(
            expected_report.passed(),
            "at scale {scale_factor}: {:?}",
            expected_report.failures()
        );
        for (name, schedule) in shuffled_schedules() {
            for seed in SHUFFLE_SEEDS {
                let mut app = scenario.headless_app();
                shuffle(&mut app, schedule, seed);
                let (report, trace) = scenario.run_in_traced(&mut app, NoiseFilter::default());
                assert_eq!(
                    report, expected_report,
                    "at scale {scale_factor}, the report with {name} shuffled by seed {seed}"
                );
                assert_eq!(
                    trace, expected_trace,
                    "at scale {scale_factor}, the trace with {name} shuffled by seed {seed}"
                );
            }
        }
    }
}

#[test]
fn shuffling_every_schedule_at_once_changes_no_report_or_trace() {
    let scenario = hop_three_times(1.0);
    let (expected_report, expected_trace) = scenario.run_headless_traced();
    for seed in SHUFFLE_SEEDS {
        let mut app = scenario.headless_app();
        for (_, schedule) in shuffled_schedules() {
            shuffle(&mut app, schedule, seed);
        }
        let (report, trace) = scenario.run_in_traced(&mut app, NoiseFilter::default());
        assert_eq!(
            report, expected_report,
            "the report with every schedule shuffled by seed {seed}"
        );
        assert_eq!(
            trace, expected_trace,
            "the trace with every schedule shuffled by seed {seed}"
        );
    }
}
