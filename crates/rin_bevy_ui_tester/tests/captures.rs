//! `Capture` steps and checks that compare with what they took.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    prelude::*,
    ui::px,
    ui_widgets::{Activate, Button},
};
use rin_bevy_ui_tester::{
    Check, Comparison, HeadlessPlugins, LoadError, Measure, Outcome, Registry, Scenario,
    ScenarioReport, Step, Target, Value,
};

/// A ghost, to be counted.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
struct Ghost;

/// The tool the editor uses.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    /// Lays track.
    Lay,
    /// Draws.
    Draw,
}

/// The editor's settings.
#[derive(Resource, Reflect, Debug)]
#[reflect(Resource)]
struct Editor {
    /// The current tool.
    tool: Tool,
    /// The zoom factor.
    zoom: f32,
    /// How many times it was used.
    uses: u32,
}

/// A gauge with a value, on a node with a "Speed" label inside it.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
struct Gauge {
    /// The value, from 0 to 1.
    value: f32,
}

/// A headless app with the test types registered.
fn app() -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default())
        .register_type::<Ghost>()
        .register_type::<Editor>()
        .register_type::<Gauge>();
    app.finish();
    app.cleanup();
    app
}

/// A registry that knows every type the app registered.
fn registry() -> Registry {
    Registry::default().with_types_of(app().world())
}

/// A button showing `label`.
fn button(label: &str) -> impl Bundle {
    (
        Button,
        Node {
            width: px(120),
            height: px(30),
            ..Node::default()
        },
        children![Text::new(label)],
    )
}

/// Two ghosts, the editor, a gauge, and buttons that add a ghost, raise the
/// gauge and use the editor.
fn spawn(world: &mut World) {
    world.insert_resource(Editor {
        tool: Tool::Draw,
        zoom: 1.5,
        uses: 4,
    });
    world.spawn(Ghost);
    world.spawn(Ghost);
    world.spawn(Camera2d);
    let column = world
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            ..Node::default()
        })
        .id();
    world.spawn((button("Haunt"), ChildOf(column))).observe(
        |_: On<Activate>, mut commands: Commands| {
            commands.spawn(Ghost);
        },
    );
    world.spawn((button("Raise"), ChildOf(column))).observe(
        |_: On<Activate>, mut gauges: Query<&mut Gauge>, mut editor: ResMut<Editor>| {
            for mut gauge in &mut gauges {
                gauge.value += 0.25;
            }
            editor.uses += 3;
        },
    );
    world.spawn((
        Gauge { value: 0.5 },
        Node {
            width: px(120),
            height: px(30),
            ..Node::default()
        },
        children![Text::new("Speed")],
        ChildOf(column),
    ));
}

/// Plays `scenario` in a fresh [`app`].
fn run(scenario: &Scenario) -> ScenarioReport {
    scenario.run_in(&mut app())
}

/// The outcome of every step, in order.
fn outcomes(report: &ScenarioReport) -> Vec<Outcome> {
    report
        .steps
        .iter()
        .map(|record| record.outcome.clone())
        .collect()
}

/// Clicks the button showing `label` and lets one frame pass.
fn click(label: &str) -> [Step; 3] {
    [
        Step::move_to(Target::text(label)),
        Step::click(),
        Step::wait(1),
    ]
}

/// A script of format version 3 with this body.
fn file(body: &str) -> String {
    format!("(format_version: 3, name: \"x\", {body})")
}

/// Loads a script with this body.
fn load(body: &str) -> Result<Scenario, LoadError> {
    Scenario::from_ron_with(&file(body), &registry())
}

/// `ghosts` captured, then compared with the count again.
fn ghosts_after(steps: impl IntoIterator<Item = Step>) -> Scenario {
    let mut all = vec![Step::capture("ghosts", Measure::count::<Ghost>())];
    all.extend(steps);
    all.push(Step::check(
        Check::count::<Ghost>().is(Comparison::Eq(Value::captured("ghosts"))),
    ));
    Scenario::new("ghosts").setup(spawn).steps(all)
}

#[test]
fn a_count_that_did_not_change_equals_the_captured_one() {
    let report = run(&ghosts_after([Step::wait(3)]));
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_count_that_changed_fails_showing_the_captured_name_and_value() {
    let report = run(&ghosts_after(click("Haunt")));
    assert_eq!(
        outcomes(&report).last(),
        Some(&Outcome::Fail(
            "count of `Ghost` is 3, expected Captured(\"ghosts\") = 2".into()
        )),
        "{:?}",
        report.failures()
    );
}

#[test]
fn a_capture_takes_no_frame() {
    let report = run(&Scenario::new("frames").setup(spawn).steps([
        Step::capture("ghosts", Measure::count::<Ghost>()),
        Step::capture("zoom", Measure::resource::<Editor>("zoom")),
    ]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(report.steps[0].frame, report.steps[1].frame);
}

#[test]
fn a_field_equals_the_captured_one_plus_an_offset() {
    let gauge = Target::text("Speed");
    let report = run(&Scenario::new("offset").setup(spawn).steps(
        [
            vec![
                Step::capture("value", Measure::component::<Gauge>(gauge.clone(), "value")),
                Step::capture("uses", Measure::resource::<Editor>("uses")),
            ],
            click("Raise").to_vec(),
            vec![
                Step::check(
                    Check::component::<Gauge>(gauge.clone(), "value")
                        .is(Comparison::Eq(Value::captured("value").plus(0.25))),
                ),
                Step::check(
                    Check::resource::<Editor>("uses")
                        .is(Comparison::Eq(Value::captured("uses").plus(3.0))),
                ),
                Step::check(
                    Check::component::<Gauge>(gauge, "value")
                        .is(Comparison::Eq(Value::captured("value").plus(0.5))),
                ),
            ],
        ]
        .concat(),
    ));
    let outcomes = outcomes(&report);
    assert_eq!(&outcomes[5..8], [Outcome::Pass, Outcome::Pass, Outcome::Fail(
        "`Gauge.value` of Text(\"Speed\") is 0.75, expected Captured(\"value\", plus: 0.5) = 1.0".into()
    )]);
}

#[test]
fn a_capture_works_inside_not_all_any_and_within() {
    let more = || Comparison::Gt(Value::captured("ghosts"));
    let report = run(&Scenario::new("nested").setup(spawn).steps(
        [
            vec![Step::capture("ghosts", Measure::count::<Ghost>())],
            click("Haunt").to_vec(),
            vec![
                Step::check(!Check::count::<Ghost>().is(Comparison::Eq(Value::captured("ghosts")))),
                Step::check(Check::all([
                    Check::count::<Ghost>().is(more()),
                    Check::shows("Haunt"),
                ])),
                Step::check(Check::any([
                    Check::shows("Nope"),
                    Check::count::<Ghost>().is(more()),
                ])),
                Step::check(Check::within(5, Check::count::<Ghost>().is(more()))),
            ],
        ]
        .concat(),
    ));
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_capture_that_cannot_be_read_fails_the_step_naming_it_and_skips_the_rest() {
    let report = run(&Scenario::new("missing").setup(spawn).steps([
        Step::capture(
            "value",
            Measure::component::<Gauge>(Target::text("Nowhere"), "value"),
        ),
        Step::wait(1),
    ]));
    let Outcome::Fail(message) = &report.steps[0].outcome else {
        panic!("{:?}", report.steps);
    };
    assert!(message.starts_with("cannot capture \"value\""), "{message}");
    assert!(message.contains("Nowhere"), "{message}");
    assert_eq!(report.steps[1].outcome, Outcome::Skipped);
}

#[test]
fn a_capture_of_a_missing_resource_fails_the_step_naming_it() {
    let report = run(&Scenario::new("missing")
        .steps([Step::capture("zoom", Measure::resource::<Editor>("zoom"))]));
    let Outcome::Fail(message) = &report.steps[0].outcome else {
        panic!("{:?}", report.steps);
    };
    assert!(message.starts_with("cannot capture \"zoom\""), "{message}");
}

#[test]
fn adding_to_something_that_is_not_a_number_fails_naming_both() {
    let report = run(&Scenario::new("plus").setup(spawn).steps([
        Step::capture("tool", Measure::resource::<Editor>("tool")),
        Step::check(
            Check::resource::<Editor>("tool").is(Comparison::Eq(Value::captured("tool").plus(1.0))),
        ),
    ]));
    let Outcome::Fail(message) = &report.steps[1].outcome else {
        panic!("{:?}", report.steps);
    };
    assert!(message.contains("`Editor.tool`"), "{message}");
    assert!(message.contains("Captured(\"tool\""), "{message}");
    assert!(message.contains("not a number"), "{message}");
}

#[test]
fn comparing_with_a_captured_value_of_another_type_fails_naming_both() {
    let report = run(&Scenario::new("types").setup(spawn).steps([
        Step::capture("tool", Measure::resource::<Editor>("tool")),
        Step::check(Check::resource::<Editor>("zoom").is(Comparison::Eq(Value::captured("tool")))),
    ]));
    let Outcome::Fail(message) = &report.steps[1].outcome else {
        panic!("{:?}", report.steps);
    };
    assert!(message.contains("`Editor.zoom`"), "{message}");
    assert!(message.contains("Captured(\"tool\")"), "{message}");
}

#[test]
fn a_whole_number_cannot_take_a_fractional_offset() {
    let report = run(&Scenario::new("fraction").setup(spawn).steps([
        Step::capture("uses", Measure::resource::<Editor>("uses")),
        Step::check(
            Check::resource::<Editor>("uses").is(Comparison::Eq(Value::captured("uses").plus(0.5))),
        ),
    ]));
    let Outcome::Fail(message) = &report.steps[1].outcome else {
        panic!("{:?}", report.steps);
    };
    assert!(message.contains("fractional"), "{message}");
}

#[test]
fn the_expect_list_may_use_a_name_captured_anywhere_in_the_steps() {
    let report = run(&ghosts_after(click("Haunt"))
        .expect_check(Check::count::<Ghost>().is(Comparison::Gt(Value::captured("ghosts")))));
    // The last step compares with the capture and fails; the expectation holds.
    assert!(
        report
            .checks
            .last()
            .is_some_and(|check| check.outcome.is_pass())
    );
}

#[test]
fn a_check_naming_a_value_captured_later_is_a_load_error_naming_the_step() {
    let error = load(
        r#"steps: [Check(Count(with: ["Ghost"], is: Eq(Captured("ghosts")))), Capture(name: "ghosts", value: Count(with: ["Ghost"]))]"#,
    )
    .unwrap_err();
    assert!(
        matches!(&error, LoadError::BadCheck { used_by, .. } if used_by == "step 0"),
        "{error}"
    );
    assert!(error.to_string().contains("\"ghosts\""), "{error}");
}

#[test]
fn an_unknown_captured_name_is_a_load_error_naming_the_step() {
    let error =
        load(r#"steps: [Wait(1), Check(Count(with: ["Ghost"], is: Eq(Captured("ghosts"))))]"#)
            .unwrap_err();
    assert!(
        matches!(&error, LoadError::BadCheck { used_by, .. } if used_by == "step 1"),
        "{error}"
    );
    assert!(error.to_string().contains("nothing is captured"), "{error}");
}

#[test]
fn an_unknown_captured_name_in_the_expect_list_is_a_load_error() {
    let error = load(r#"steps: [], expect: [Count(with: ["Ghost"], is: Eq(Captured("none")))]"#)
        .unwrap_err();
    assert!(
        matches!(&error, LoadError::BadCheck { used_by, .. } if used_by == "the expect list"),
        "{error}"
    );
}

#[test]
fn an_unknown_name_inside_not_and_within_is_found_too() {
    for check in [
        r#"Not(Count(with: ["Ghost"], is: Eq(Captured("zzz"))))"#,
        r#"Within(5, All([Count(with: ["Ghost"], is: Between(1, Captured("zzz")))]))"#,
    ] {
        let error = load(&format!("steps: [Check({check})]")).unwrap_err();
        assert!(error.to_string().contains("zzz"), "{error}");
    }
}

#[test]
fn capturing_a_name_twice_is_a_load_error_naming_the_second_step() {
    let error = load(
        r#"steps: [Capture(name: "n", value: Count(with: ["Ghost"])), Wait(1), Capture(name: "n", value: Resource(path: "Editor.zoom"))]"#,
    )
    .unwrap_err();
    assert!(
        matches!(&error, LoadError::Invalid { used_by, .. } if used_by == "step 2"),
        "{error}"
    );
    assert!(error.to_string().contains("twice"), "{error}");
}

#[test]
fn capturing_a_name_twice_in_different_groups_is_a_load_error_with_the_group_path() {
    let error = load(
        r#"steps: [Capture(name: "n", value: Count(with: ["Ghost"])), Group(name: "later", steps: [Capture(name: "n", value: Count(with: ["Ghost"]))])]"#,
    )
    .unwrap_err();
    assert!(error.to_string().starts_with("later > step 1"), "{error}");
}

#[test]
fn a_capture_of_an_unknown_type_is_a_load_error_suggesting_close_names() {
    let error = load(r#"steps: [Capture(name: "n", value: Count(with: ["Gost"]))]"#).unwrap_err();
    assert!(
        matches!(&error, LoadError::UnknownType { name, used_by, suggestions }
            if name == "Gost" && used_by == "step 0" && suggestions.iter().any(|s| s == "Ghost")),
        "{error}"
    );
}

#[test]
fn a_capture_of_an_unknown_field_is_a_load_error_suggesting_close_names() {
    let error =
        load(r#"steps: [Capture(name: "n", value: Resource(path: "Editor.zom"))]"#).unwrap_err();
    assert!(
        matches!(&error, LoadError::UnknownPath { used_by, suggestions, .. }
            if used_by == "step 0" && suggestions.iter().any(|s| s.contains("zoom"))),
        "{error}"
    );
}

#[test]
fn a_capture_of_an_unknown_component_target_is_a_load_error() {
    let error = load(
        r#"steps: [Capture(name: "n", value: Component(target: Component("NoSuchMarker"), path: "Gauge.value"))]"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("NoSuchMarker"), "{error}");
}

#[test]
fn a_timeout_around_a_capture_is_a_load_error() {
    let error = load(r#"steps: [Timeout(5, Capture(name: "n", value: Count(with: ["Ghost"])))]"#)
        .unwrap_err();
    assert!(matches!(error, LoadError::Invalid { .. }), "{error}");
}

#[test]
fn every_capture_form_and_captured_operand_survives_a_round_trip_through_ron() {
    let scenario = Scenario::new("round trip")
        .steps([
            Step::capture("ghosts", Measure::count::<Ghost>()),
            Step::capture("both", Measure::count_of(["Ghost", "Gauge"])),
            Step::capture("zoom", Measure::resource::<Editor>("zoom")),
            Step::capture(
                "value",
                Measure::component::<Gauge>(Target::text("Speed"), "value"),
            ),
            Step::check(Check::count::<Ghost>().is(Comparison::Eq(Value::captured("ghosts")))),
            Step::check(
                Check::resource::<Editor>("zoom")
                    .is(Comparison::Ge(Value::captured("zoom").plus(0.5))),
            ),
            Step::check(Check::resource::<Editor>("uses").is(Comparison::Between(
                Value::captured("ghosts"),
                Value::captured("ghosts").plus(3.0),
            ))),
        ])
        .expect_check(Check::count::<Ghost>().is(Comparison::Ne(Value::captured("both"))));
    let ron = scenario.to_ron().unwrap();
    assert!(ron.contains("Capture("), "{ron}");
    assert!(ron.contains("Captured(\"ghosts\")"), "{ron}");
    assert!(ron.contains("Captured(name: \"zoom\", plus: 0.5)"), "{ron}");
    let loaded = Scenario::from_ron_with(&ron, &registry()).unwrap();
    assert_eq!(loaded.to_script(), scenario.to_script());
    assert_eq!(loaded.to_ron().unwrap(), ron, "export is stable");
}

#[test]
fn a_captured_value_with_an_offset_reads_in_the_long_form_only() {
    let scenario = load(
        r#"steps: [Capture(name: "n", value: Resource(path: "Editor.uses")), MoveTo(Text("Raise")), Click(Left), Wait(1), Check(Resource(path: "Editor.uses", is: Eq(Captured(name: "n", plus: 3))))]"#,
    )
    .unwrap();
    let report = run(&scenario.setup(spawn));
    assert!(report.passed(), "{:?}", report.failures());
    let error = load(
        r#"steps: [Capture(name: "n", value: Resource(path: "Editor.uses")), Check(Resource(path: "Editor.uses", is: Eq(Captured("n", plus: 3))))]"#,
    )
    .unwrap_err();
    // `Captured("n", plus: 3)` mixes a positional and a named field, which
    // is not RON at all, so the file does not parse.
    assert!(error.to_string().contains("invalid script"), "{error}");
}

#[test]
fn a_script_with_captures_plays_from_a_file_like_the_same_one_in_rust() {
    let scenario = load(
        r#"steps: [
            Capture(name: "ghosts", value: Count(with: ["Ghost"])),
            Check(Count(with: ["Ghost"], is: Eq(Captured("ghosts")))),
            Check(Count(with: ["Ghost"], is: Eq(Captured(name: "ghosts", plus: 1)))),
        ]"#,
    )
    .unwrap();
    let report = run(&scenario.setup(spawn));
    assert_eq!(
        outcomes(&report),
        [
            Outcome::Pass,
            Outcome::Pass,
            Outcome::Fail(
                "count of `Ghost` is 2, expected Captured(\"ghosts\", plus: 1) = 3".into()
            ),
        ]
    );
}

#[test]
fn files_of_the_older_formats_load_unchanged() {
    for version in 1..=3 {
        let scenario = Scenario::from_ron_with(
            &format!(
                "(format_version: {version}, name: \"old\", steps: [Wait(1), Check(Count(with: [\"Ghost\"], is: Eq(2)))])"
            ),
            &registry(),
        )
        .unwrap();
        let report = run(&scenario.setup(spawn));
        assert!(report.passed(), "{version}: {:?}", report.failures());
    }
}

#[test]
fn the_trace_shows_what_a_step_captured() {
    let (report, trace) = ghosts_after([Step::wait(1)]).run_headless_traced();
    assert!(report.passed(), "{:?}", report.failures());
    let captured = &trace.step(0).expect("the capture step").captured;
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].name, "ghosts");
    assert_eq!(captured[0].value, "2");
    assert_eq!(captured[0].type_name, "usize");
    assert!(captured[0].describe().contains("\"ghosts\" = 2"));
    assert!(trace.step(1).is_none_or(|step| step.captured.is_empty()));
}
