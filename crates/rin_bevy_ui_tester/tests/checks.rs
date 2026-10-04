//! Checks spelled out as data: text, hover, state, negation and checks that
//! take an argument, as steps and as expectations.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests may panic on a broken setup"
)]

mod common;

use bevy::math::Vec2;
use common::{activated, registry, spawn_ui, state_probe};
use rin_bevy_ui_tester::{
    Check, CheckKind, FORMAT_VERSION, LoadError, Outcome, Registry, Scenario, ScenarioReport, Step,
    Target,
};

/// The outcome of every step, in order.
fn step_outcomes(report: &ScenarioReport) -> Vec<Outcome> {
    report
        .steps
        .iter()
        .map(|record| record.outcome.clone())
        .collect()
}

/// The "Pause" button, by its label.
fn pause() -> Target {
    Target::text("Pause")
}

#[test]
fn shows_passes_for_text_on_screen_and_fails_naming_the_text_that_is_missing() {
    let report = Scenario::new("shows")
        .setup(spawn_ui)
        .steps([
            Step::check(Check::shows("Pause")),
            Step::check(Check::shows("Resume")),
        ])
        .run_headless();
    assert_eq!(
        step_outcomes(&report),
        [
            Outcome::Pass,
            Outcome::Fail("no UI node shows \"Resume\"".into())
        ]
    );
    assert_eq!(report.checks[0].name, "\"Pause\" is shown");
}

#[test]
fn hovered_holds_only_while_the_pointer_is_over_the_widget() {
    let report = Scenario::new("hovered")
        .setup(spawn_ui)
        .steps([
            Step::check(!Check::hovered(pause())),
            Step::move_to(pause()),
            Step::check(Check::hovered(pause())),
            Step::move_to(Target::screen(Vec2::new(5.0, 5.0))),
            Step::check(!Check::hovered(pause())),
        ])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn hovered_fails_for_a_bare_position_even_inside_not() {
    let report = Scenario::new("position")
        .setup(spawn_ui)
        .step(Step::check(!Check::hovered(Target::screen(Vec2::ZERO))))
        .run_headless();
    let failures = report.failures();
    assert!(
        failures.iter().any(|line| line.contains("is a position")),
        "{failures:?}"
    );
}

#[test]
fn not_fails_on_a_check_nobody_defined_rather_than_passing() {
    let report = Scenario::new("undefined")
        .setup(spawn_ui)
        .step(Step::check(!Check::named("nope")))
        .run_headless();
    assert_eq!(
        step_outcomes(&report),
        [Outcome::Fail(
            "the scenario defines no check named `nope`".into()
        )]
    );
}

#[test]
fn not_fails_when_the_check_inside_holds_and_says_which() {
    let report = Scenario::new("negated")
        .setup(spawn_ui)
        .step(Step::check(!Check::shows("Pause")))
        .run_headless();
    assert_eq!(
        step_outcomes(&report),
        [Outcome::Fail(
            "\"Pause\" is shown holds, but should not".into()
        )]
    );
    assert_eq!(report.checks[0].name, "not: \"Pause\" is shown");
}

#[test]
fn a_custom_check_gets_its_argument() {
    let report = Scenario::new("custom")
        .setup(spawn_ui)
        .custom_check("activated", activated)
        .steps([
            Step::check(Check::custom("activated", "0")),
            Step::move_to(pause()),
            Step::click(),
            Step::wait(1),
            Step::check(Check::custom("activated", "1")),
            Step::check(Check::custom("activated", "2")),
        ])
        .expect_check(!Check::custom("activated", "2"))
        .run_headless();
    assert_eq!(
        step_outcomes(&report)[3..],
        [
            Outcome::Pass,
            Outcome::Pass,
            Outcome::Fail("activated 1 times, not 2".into())
        ]
    );
    assert_eq!(report.checks[1].name, "activated \"1\"");
    assert_eq!(report.checks.last().unwrap().outcome, Outcome::Pass);
}

#[test]
fn a_state_check_reads_the_probe_part_way_through() {
    let report = Scenario::new("state")
        .setup(spawn_ui)
        .with_state(state_probe())
        .steps([
            Step::check(Check::state("activations", "0")),
            Step::move_to(pause()),
            Step::click(),
            Step::wait(1),
            Step::check(Check::state("activations", "1")),
            Step::check(Check::state("activations", "2")),
            Step::check(!Check::state("score", "1")),
        ])
        .run_headless();
    assert_eq!(
        step_outcomes(&report)[4..],
        [
            Outcome::Pass,
            Outcome::Fail("state `activations` is \"1\", not \"2\"".into()),
            Outcome::Fail("the app's state has no `score`".into()),
        ]
    );
}

#[test]
fn expectations_run_at_the_end_in_order_and_are_named_in_words() {
    let report = Scenario::new("expectations")
        .setup(spawn_ui)
        .with_state(state_probe())
        .expect_check(Check::shows("Pause"))
        .expect("never", |_probe| false)
        .expect_check(Check::state("activations", "0"))
        .run_headless();
    let checks: Vec<_> = report
        .checks
        .iter()
        .map(|record| (record.kind, record.name.as_str(), record.outcome.is_pass()))
        .collect();
    assert_eq!(
        checks,
        [
            (CheckKind::Expect, "\"Pause\" is shown", true),
            (CheckKind::Expect, "never", false),
            (CheckKind::Expect, "state `activations` is \"0\"", true),
        ]
    );
}

/// A scenario using every kind of check, with names the registry knows.
fn every_kind() -> Scenario {
    Scenario::new("every kind")
        .setup_named("plain_ui", spawn_ui)
        .steps([
            Step::check("activated once"),
            Step::check(Check::shows("Pause")),
            Step::check(!Check::hovered(Target::text("Pause"))),
            Step::check(Check::state("activations", "0")),
            Step::check(Check::custom("activated", "0")),
            Step::check(!Check::named("activated once")),
        ])
        .expect_check(Check::hovered(Target::custom("pause_button", "")))
}

#[test]
fn checks_round_trip_through_ron_and_a_named_check_stays_a_bare_name() {
    let ron = every_kind().to_ron().unwrap();
    for written in [
        "Check(\"activated once\")",
        "Check(Shows(\"Pause\"))",
        "Check(Not(Hovered(Text(\"Pause\"))))",
        "Check(Not(\"activated once\"))",
        "Custom(\n",
        "State(\n",
    ] {
        assert!(ron.contains(written), "{written} in {ron}");
    }
    let loaded = Scenario::from_ron_with(&ron, &registry()).unwrap();
    assert_eq!(loaded.to_script(), every_kind().to_script());
    assert_eq!(loaded.to_ron().unwrap(), ron, "export is stable");
}

#[test]
fn a_version_1_file_loads_and_is_written_back_as_the_current_version() {
    let scenario =
        Scenario::from_ron_with(include_str!("ron/click_pause.ron"), &registry()).unwrap();
    assert_eq!(scenario.expects(), [Check::named("activated once")]);
    assert_eq!(scenario.to_script().format_version, FORMAT_VERSION);
}

#[test]
fn a_misspelt_check_is_a_syntax_error_at_its_position() {
    let error = Scenario::from_ron(
        "(format_version: 2, name: \"x\", steps: [Check(Shows(\"a\")), Check(Showz(\"b\"))])",
    )
    .unwrap_err();
    assert!(
        matches!(
            &error,
            LoadError::Syntax(message) if message.starts_with("1:") && message.contains("a check:")
        ),
        "{error}"
    );
}

#[test]
fn checking_state_without_a_state_probe_is_a_load_error() {
    let error = Scenario::from_ron(
        "(format_version: 2, name: \"x\", steps: [Check(Not(State(key: \"a\", value: \"b\")))])",
    )
    .unwrap_err();
    assert_eq!(error, LoadError::NoStateProbe);
}

#[test]
fn names_inside_checks_are_resolved_when_the_file_loads() {
    let registry: Registry = registry();
    let load = |body: &str| {
        Scenario::from_ron_with(
            &format!("(format_version: 2, name: \"x\", {body})"),
            &registry,
        )
        .unwrap_err()
    };
    assert!(matches!(
        load("steps: [Wait(1), Check(Not(\"nope\"))]"),
        LoadError::UnknownCheck { ref name, ref used_by } if name == "nope" && used_by == "step 1"
    ));
    assert!(matches!(
        load("steps: [], expect: [Custom(check: \"nope\", argument: \"\")]"),
        LoadError::UnknownCheck { ref used_by, .. } if used_by == "the expect list"
    ));
    assert!(matches!(
        load("steps: [Check(Hovered(Custom(resolver: \"nope\", argument: \"\")))]"),
        LoadError::UnknownTarget { ref resolver, .. } if resolver == "nope"
    ));
}

#[test]
fn a_misspelt_check_inside_another_is_reported_on_its_line() {
    let error = Scenario::from_ron(
        "(format_version: 3, name: \"x\", steps: [\n    Wait(1),\n    Check(All([Shows(\"a\"), Not(Showz(\"b\"))])),\n])",
    )
    .unwrap_err();
    assert!(matches!(&error, LoadError::Syntax(_)), "{error:?}");
    let message = error.to_string();
    assert!(message.starts_with("invalid script: 3:"), "{message}");
    assert!(message.contains("Showz"), "{message}");
    assert_eq!(message.matches("a check:").count(), 1, "{message}");
}
