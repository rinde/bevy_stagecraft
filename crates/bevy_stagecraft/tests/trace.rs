//! The per-step trace of a run, and stopping a run before a step.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

mod common;

use bevy::prelude::*;
use common::{Activations, spawn_ui};
use bevy_stagecraft::{
    Check, Outcome, Scenario, ScenarioPlayer, ScenarioPlayerPlugin, ScenarioReport, Step, Target,
};

/// Frames the app runs before the scenario begins, as the player's own runs do.
const SETTLE_FRAMES: usize = 2;

/// Clicks "Pause" once, waits, and checks it was activated; the check at the
/// end wants it twice, so the scenario fails there.
fn click_pause(wanted: u32) -> Scenario {
    Scenario::new("click pause")
        .window_size(320, 240)
        .setup(spawn_ui)
        .steps([
            Step::move_to(Target::text("Pause")),
            Step::click(),
            Step::wait(2),
            Step::check(Check::resource::<Activations>(".0").eq(wanted)),
        ])
}

#[test]
fn a_traced_run_has_a_point_before_the_first_step_and_one_at_the_end() {
    let (report, trace) = click_pause(1).run_headless_traced();
    assert!(report.passed(), "{report:?}");
    let first = trace.points().first().expect("a first point");
    assert_eq!((first.frame, first.started, first.finished), (0, 0, false));
    let last = trace.last().expect("a last point");
    assert!(last.finished, "the last point is the end of the run");
    assert_eq!(last.frame, report.frames);
}

#[test]
fn the_snapshots_around_a_click_differ_by_what_the_click_did() {
    let (_, trace) = click_pause(1).run_headless_traced();
    let before = trace.before(1).expect("the world before the click");
    let after = trace.after(1).expect("the world after the click");
    let changes = after.snapshot.changes_from(&before.snapshot);
    assert!(
        changes
            .iter()
            .any(|change| change.key.contains("Activations")),
        "the activation count changed: {changes:?}"
    );
    assert!(
        before
            .ui
            .iter()
            .any(|line| line.text.as_deref() == Some("Pause")),
        "the UI tree has the button's text"
    );
}

#[test]
fn a_step_records_where_its_target_was_found_and_the_checks_it_ran() {
    let (_, trace) = click_pause(1).run_headless_traced();
    let moved = trace.step(0).expect("what step 0 recorded");
    assert!(
        moved.targets.iter().any(|target| target.found.is_ok()),
        "the move found its target: {moved:?}"
    );
    let checked = trace.step(3).expect("what step 3 recorded");
    assert_eq!(checked.checks.len(), 1);
    assert_eq!(
        checked.checks.first().map(|c| &c.outcome),
        Some(&Outcome::Pass)
    );
}

#[test]
fn a_failing_check_is_in_the_trace_with_its_message() {
    let (report, trace) = click_pause(2).run_headless_traced();
    assert!(!report.passed());
    let checked = trace.step(3).expect("what step 3 recorded");
    let outcome = checked.checks.first().map(|c| c.outcome.clone());
    assert!(
        matches!(outcome, Some(Outcome::Fail(ref why)) if why.contains("expected 2")),
        "{outcome:?}"
    );
}

#[test]
fn a_run_not_traced_leaves_no_trace_behind() {
    let scenario = click_pause(1);
    let mut app = scenario.headless_app();
    scenario.run_in(&mut app);
    let trace = app
        .world_mut()
        .get_resource_mut::<ScenarioPlayer>()
        .and_then(|mut player| player.take_trace());
    assert!(trace.is_none());
}

/// Runs `scenario` in a headless app until the player halts before step
/// `index`, and returns the app.
fn halted_before(scenario: &Scenario, index: usize) -> App {
    let mut app = scenario.headless_app();
    Plugin::build(&ScenarioPlayerPlugin, &mut app);
    let setup = scenario.run_setup(app.world_mut());
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    let mut player = app
        .world_mut()
        .get_resource_mut::<ScenarioPlayer>()
        .expect("the player");
    player.pause_before(Some(index));
    player.start_after_setup(scenario.clone(), setup);
    for _ in 0..1000 {
        if app
            .world()
            .get_resource::<ScenarioPlayer>()
            .is_some_and(ScenarioPlayer::halted)
        {
            return app;
        }
        app.update();
    }
    panic!("the player never halted");
}

#[test]
fn stopping_before_a_step_lands_on_the_frame_and_world_the_trace_shows_for_it() {
    let scenario = click_pause(1);
    let (_, trace) = scenario.run_headless_traced();
    for index in 0..4 {
        let app = halted_before(&scenario, index);
        let report = app
            .world()
            .get_resource::<ScenarioReport>()
            .expect("a report");
        let point = trace.before(index).expect("a point before the step");
        assert_eq!(report.frames, point.frame, "step {index}");
        let count = app.world().get_resource::<Activations>().map(|a| a.0);
        let snapshot = bevy_stagecraft::WorldSnapshot::take(app.world(), trace.filter());
        assert!(
            snapshot.changes_from(&point.snapshot).is_empty(),
            "step {index}: the world is the traced one ({count:?} activations)"
        );
    }
}

#[test]
fn a_checks_step_without_frames_shares_its_world_with_the_step_before() {
    let (_, trace) = click_pause(1).run_headless_traced();
    let before_check = trace.before(3).expect("before the check");
    let after_wait = trace.after(2).expect("after the wait");
    assert_eq!(before_check.frame, after_wait.frame);
}
