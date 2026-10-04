//! Built-in checks that read plain Bevy data through reflection: counts,
//! resource and component fields, states, widget states and text, combined
//! with `All`, `Any` and `Within`.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    input_focus::InputFocus,
    prelude::*,
    state::app::StatesPlugin,
    ui::{Checked, InteractionDisabled, px},
    ui_widgets::{Activate, Button, Checkbox},
};
use rin_bevy_ui_tester::{
    Check, Comparison, HeadlessPlugins, LoadError, Outcome, Registry, Scenario, ScenarioReport,
    Step, Target, TextMatch, Value,
};

/// A ghost, to be counted.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
struct Ghost;

/// Marks a selected ghost.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
struct Selected;

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
}

/// A gauge with a value, on a node with a "Speed" label inside it.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
struct Gauge {
    /// The value, from 0 to 1.
    value: f32,
}

/// The app's mode.
#[derive(States, Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum Mode {
    /// The start.
    #[default]
    Menu,
    /// Editing.
    Editing,
}

/// Frames left until "Saved" shows.
#[derive(Resource, Debug)]
struct Countdown(u32);

/// How many frames "Later" takes to show "Saved".
const SAVE_DELAY: u32 = 5;

/// Shows "Saved" once the countdown runs out.
fn count_down(mut commands: Commands, countdown: Option<ResMut<Countdown>>) {
    let Some(mut countdown) = countdown else {
        return;
    };
    if countdown.0 == 0 {
        commands.remove_resource::<Countdown>();
        commands.spawn(Text::new("Saved"));
    } else {
        countdown.0 -= 1;
    }
}

/// A headless app with the test types registered, the `Mode` state and the
/// countdown.
fn app() -> App {
    let mut app = App::new();
    app.add_plugins((HeadlessPlugins::default(), StatesPlugin))
        .init_state::<Mode>()
        .register_type_state::<Mode>()
        .register_type::<Ghost>()
        .register_type::<Selected>()
        .register_type::<Editor>()
        .register_type::<Gauge>()
        .add_systems(Update, count_down);
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

/// Spawns two ghosts, one selected, the editor and a column of widgets.
fn spawn(world: &mut World) {
    world.insert_resource(Editor {
        tool: Tool::Draw,
        zoom: 1.5,
    });
    world.spawn((Ghost, Selected));
    world.spawn(Ghost);
    world.spawn(Camera2d);
    let column = world
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            ..Node::default()
        })
        .id();
    let lay = world
        .spawn((button("Lay"), ChildOf(column)))
        .observe(|_: On<Activate>, mut editor: ResMut<Editor>| editor.tool = Tool::Lay)
        .id();
    world.spawn((button("Edit"), ChildOf(column))).observe(
        |_: On<Activate>, mut next: ResMut<NextState<Mode>>| {
            next.set(Mode::Editing);
        },
    );
    world.spawn((button("Later"), ChildOf(column))).observe(
        |_: On<Activate>, mut commands: Commands| {
            commands.insert_resource(Countdown(SAVE_DELAY));
        },
    );
    world.spawn((button("Save"), InteractionDisabled, ChildOf(column)));
    world.spawn((
        Checkbox,
        Checked,
        Node {
            width: px(120),
            height: px(30),
            ..Node::default()
        },
        children![Text::new("Dark")],
        ChildOf(column),
    ));
    world.spawn((
        Gauge { value: 0.75 },
        Node {
            width: px(120),
            height: px(30),
            ..Node::default()
        },
        children![(Text::new("Speed"), children![TextSpan::new(" (m/s)")])],
        ChildOf(column),
    ));
    world.spawn((Text::new("Score: 3"), ChildOf(column)));
    world.spawn((
        Node {
            display: Display::None,
            ..Node::default()
        },
        children![Text::new("Hidden")],
        ChildOf(column),
    ));
    world.insert_resource(InputFocus::from_entity(lay));
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

#[test]
fn count_says_how_many_entities_have_the_components_and_what_it_found() {
    let report = run(&Scenario::new("count").setup(spawn).steps([
        Step::check(Check::count::<Ghost>().eq(2)),
        Step::check(Check::count::<Ghost>().and::<Selected>().eq(1)),
        Step::check(Check::count::<Ghost>().between(1, 3)),
        Step::check(Check::count::<Ghost>().eq(1)),
        Step::check(Check::count::<Selected>().gt(1)),
    ]));
    assert_eq!(
        outcomes(&report),
        [
            Outcome::Pass,
            Outcome::Pass,
            Outcome::Pass,
            Outcome::Fail("count of `Ghost` is 2, expected 1".into()),
            Outcome::Fail("count of `Selected` is 1, expected more than 1".into()),
        ]
    );
    assert_eq!(
        report.checks[1].name,
        "count of `Ghost` and `Selected` is 1"
    );
}

#[test]
fn a_resource_field_compares_with_a_typed_value() {
    let report = run(&Scenario::new("resource").setup(spawn).steps(
        [
            vec![
                Step::check(Check::resource::<Editor>("tool").eq(Tool::Draw)),
                Step::check(Check::resource::<Editor>("zoom").gt(1.0_f32)),
                Step::check(Check::resource::<Editor>("zoom").lt(1.0_f32)),
            ],
            click("Lay").to_vec(),
            vec![Step::check(Check::resource::<Editor>("tool").eq(Tool::Lay))],
        ]
        .concat(),
    ));
    assert_eq!(
        outcomes(&report),
        [
            Outcome::Pass,
            Outcome::Pass,
            Outcome::Fail("`Editor.zoom` is 1.5, expected less than 1.0".into()),
            Outcome::Pass,
            Outcome::Pass,
            Outcome::Pass,
            Outcome::Pass,
        ]
    );
}

#[test]
fn a_component_field_is_read_on_the_node_or_the_nearest_one_above_it() {
    let report = run(&Scenario::new("component").setup(spawn).steps([
        Step::check(Check::component::<Gauge>(Target::text("Speed"), "value").gt(0.5_f32)),
        Step::check(Check::component::<Gauge>(Target::text("Speed"), "value").eq(0.5_f32)),
        Step::check(Check::component::<Gauge>(Target::text("Lay"), "value").eq(0.5_f32)),
    ]));
    assert_eq!(
        outcomes(&report),
        [
            Outcome::Pass,
            Outcome::Fail("`Gauge.value` of Text(\"Speed\") is 0.75, expected 0.5".into()),
            Outcome::Fail("neither Text(\"Lay\") nor anything above it has a `Gauge`".into()),
        ]
    );
}

#[test]
fn in_state_reads_the_current_state() {
    let report = run(&Scenario::new("state").setup(spawn).steps(
        [
            vec![
                Step::check(Check::in_state(Mode::Menu)),
                Step::check(Check::in_state(Mode::Editing)),
            ],
            click("Edit").to_vec(),
            vec![Step::check(Check::in_state(Mode::Editing))],
        ]
        .concat(),
    ));
    assert_eq!(
        outcomes(&report),
        [
            Outcome::Pass,
            Outcome::Fail("state `Mode` is Menu, expected Editing".into()),
            Outcome::Pass,
            Outcome::Pass,
            Outcome::Pass,
            Outcome::Pass,
        ]
    );
}

#[test]
fn widget_states_read_bevys_interaction_components() {
    let report = run(&Scenario::new("widgets").setup(spawn).steps([
        Step::check(Check::disabled(Target::text("Save"))),
        Step::check(!Check::disabled(Target::text("Lay"))),
        Step::check(Check::checked(Target::text("Dark"))),
        Step::check(Check::focused(Target::text("Lay"))),
        Step::check(Check::exists(Target::text("Lay"))),
        Step::check(!Check::exists(Target::text("Nope"))),
        Step::check(Check::visible(Target::text("Lay"))),
        Step::check(!Check::visible(Target::text("Hidden"))),
        Step::check(!Check::pressed(Target::text("Edit"))),
        Step::move_to(Target::text("Edit")),
        Step::press(),
        Step::check(Check::pressed(Target::text("Edit"))),
        Step::release(),
        Step::check(Check::checked(Target::text("Lay"))),
    ]));
    let mut expected = vec![Outcome::Pass; 13];
    expected.push(Outcome::Fail("Text(\"Lay\") is not checked".into()));
    assert_eq!(outcomes(&report), expected);
}

#[test]
fn a_widget_state_of_a_bare_position_cannot_run_even_inside_not() {
    let report = run(&Scenario::new("position")
        .setup(spawn)
        .step(Step::check(!Check::disabled(Target::screen(Vec2::ZERO)))));
    assert_eq!(
        outcomes(&report),
        [Outcome::Fail(
            "Screen(Vec2(0.0, 0.0)) is a position, not a widget".into()
        )]
    );
}

#[test]
fn text_of_matches_the_text_the_widget_shows_spans_included() {
    let report = run(&Scenario::new("text").setup(spawn).steps([
        Step::check(Check::text_of(
            Target::text("Score: 3"),
            TextMatch::exact("Score: 3"),
        )),
        Step::check(Check::text_of(
            Target::text("Speed"),
            TextMatch::exact("Speed (m/s)"),
        )),
        Step::check(Check::text_of(
            Target::text("Score: 3"),
            TextMatch::starts_with("Score: "),
        )),
        Step::check(Check::text_of(
            Target::text("Score: 3"),
            TextMatch::contains("4"),
        )),
    ]));
    assert_eq!(
        outcomes(&report),
        [
            Outcome::Pass,
            Outcome::Pass,
            Outcome::Pass,
            Outcome::Fail("Text(\"Score: 3\") shows \"Score: 3\"".into()),
        ]
    );
    assert_eq!(
        report.checks[3].name,
        "text of Text(\"Score: 3\") contains \"4\""
    );
}

#[test]
fn all_holds_when_every_check_does_and_any_when_one_does() {
    let report = run(&Scenario::new("all and any").setup(spawn).steps([
        Step::check(Check::all([
            Check::shows("Lay"),
            Check::count::<Ghost>().eq(2),
        ])),
        Step::check(Check::all([Check::shows("Lay"), Check::shows("Nope")])),
        Step::check(Check::any([Check::shows("Nope"), Check::shows("Lay")])),
        Step::check(Check::any([Check::shows("Nope"), Check::shows("Gone")])),
        Step::check(Check::all(Vec::<Check>::new())),
        Step::check(Check::any(Vec::<Check>::new())),
    ]));
    assert_eq!(
        outcomes(&report),
        [
            Outcome::Pass,
            Outcome::Fail("no UI node shows \"Nope\"".into()),
            Outcome::Pass,
            Outcome::Fail(
                "none holds: no UI node shows \"Nope\"; no UI node shows \"Gone\"".into()
            ),
            Outcome::Pass,
            Outcome::Fail("there is no check to hold".into()),
        ]
    );
}

#[test]
fn within_waits_for_a_check_to_hold_and_takes_only_the_frames_it_waits() {
    let report = run(&Scenario::new("within").setup(spawn).steps([
        Step::move_to(Target::text("Later")),
        Step::click(),
        Step::check(Check::within(30, Check::shows("Saved"))),
        Step::wait(1),
    ]));
    assert!(report.passed(), "{:?}", report.failures());
    let waited = report.steps[3].frame - report.steps[2].frame;
    assert!(
        waited > 1 && waited < 30,
        "waited {waited} frames for \"Saved\""
    );
}

#[test]
fn within_fails_once_its_frames_are_up_saying_what_it_last_saw() {
    let report = run(&Scenario::new("too soon").setup(spawn).steps([
        Step::move_to(Target::text("Later")),
        Step::click(),
        Step::check(Check::within(2, Check::shows("Saved"))),
        Step::wait(1),
    ]));
    assert_eq!(
        report.steps[2].outcome,
        Outcome::Fail("no UI node shows \"Saved\" (still after 2 frames)".into())
    );
    assert_eq!(report.steps[3].frame - report.steps[2].frame, 2);
}

#[test]
fn an_expectation_can_wait_too() {
    let report = run(&Scenario::new("expect within")
        .setup(spawn)
        .steps(click("Later"))
        .expect_check(Check::within(30, Check::shows("Saved")))
        .expect_check(Check::count::<Ghost>().eq(2)));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(report.checks.len(), 2);
}

#[test]
fn not_within_holds_only_if_the_check_never_holds_in_time() {
    let report = run(&Scenario::new("never").setup(spawn).steps([
        Step::check(!Check::within(3, Check::shows("Saved"))),
        Step::move_to(Target::text("Later")),
        Step::click(),
        Step::check(!Check::within(30, Check::shows("Saved"))),
    ]));
    assert_eq!(report.steps[0].outcome, Outcome::Pass);
    assert_eq!(
        report.steps[3].outcome,
        Outcome::Fail("within 30 frames: \"Saved\" is shown holds, but should not".into())
    );
}

/// A scenario with every reflected check.
fn every_kind() -> Scenario {
    Scenario::new("every kind").steps([
        Step::check(Check::count::<Ghost>().and::<Selected>().eq(1)),
        Step::check(Check::resource::<Editor>("tool").eq(Tool::Lay)),
        Step::check(Check::resource::<Editor>("zoom").between(0.5_f32, 2.0_f32)),
        Step::check(Check::component::<Gauge>(Target::text("Speed"), "value").ge(0.5_f32)),
        Step::check(Check::in_state(Mode::Editing)),
        Step::check(Check::disabled(Target::text("Save"))),
        Step::check(Check::checked(Target::text("Dark"))),
        Step::check(Check::pressed(Target::text("Edit"))),
        Step::check(Check::focused(Target::text("Lay"))),
        Step::check(Check::visible(Target::text("Lay"))),
        Step::check(Check::exists(Target::text("Lay"))),
        Step::check(Check::text_of(
            Target::text("Score: 3"),
            TextMatch::starts_with("Score"),
        )),
        Step::check(Check::all([Check::shows("Lay"), !Check::shows("Error")])),
        Step::check(Check::any([Check::shows("saved"), Check::shows("done")])),
        Step::check(Check::within(30, Check::shows("Saved"))),
        Step::check(Check::count::<Ghost>().is(Comparison::Ne(Value::from_ron("3").unwrap()))),
    ])
}

#[test]
fn every_reflected_check_round_trips_through_ron_and_exports_stably() {
    let ron = every_kind().to_ron().unwrap();
    for written in [
        "Count(\n",
        "with: [\n                \"Ghost\",\n                \"Selected\",\n            ]",
        "is: Eq(1)",
        "path: \"Editor.tool\"",
        "is: Eq(Lay)",
        "is: Between(0.5, 2.0)",
        "path: \"Gauge.value\"",
        "state: \"Mode\"",
        "value: Editing",
        "Check(Disabled(Text(\"Save\")))",
        "Check(TextOf(Text(\"Score: 3\"), StartsWith(\"Score\")))",
        "Check(Within(30, Shows(\"Saved\")))",
        "Check(Any([\n            Shows(\"saved\"),\n            Shows(\"done\"),\n        ]))",
        "is: Ne(3)",
    ] {
        assert!(ron.contains(written), "{written} in {ron}");
    }
    let registry = registry();
    let loaded = Scenario::from_ron_with(&ron, &registry).unwrap();
    assert_eq!(loaded.to_script(), every_kind().to_script());
    assert_eq!(loaded.to_ron().unwrap(), ron, "export is stable");
}

/// Loads a script with these steps against the test types.
fn load(steps: &str) -> Result<Scenario, LoadError> {
    Scenario::from_ron_with(
        &format!("(format_version: 3, name: \"x\", steps: [Wait(1), {steps}])"),
        &registry(),
    )
}

#[test]
fn an_unknown_type_is_a_load_error_naming_the_step_and_close_names() {
    let error = load("Check(Count(with: [\"Gost\"], is: Eq(1)))").unwrap_err();
    // The app registers every type that derives `Reflect`, Bevy's own
    // among them, so other close names may follow; the closest comes first.
    assert!(
        matches!(
            &error,
            LoadError::UnknownType { name, used_by, suggestions }
                if name == "Gost" && used_by == "step 1" && suggestions.first().is_some_and(|first| first == "Ghost")
        ),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .starts_with("step 1 names type `Gost`, which is not registered; did you mean `Ghost`"),
        "{error}"
    );
}

#[test]
fn an_unknown_field_is_a_load_error_suggesting_close_fields() {
    let error = load("Check(Not(Resource(path: \"Editor.tol\", is: Eq(Lay))))").unwrap_err();
    assert_eq!(
        error.to_string(),
        "step 1: `Editor` has no field `tol`; did you mean `tool`?"
    );
}

#[test]
fn a_value_that_does_not_fit_the_field_is_a_load_error() {
    let error = load("Check(Resource(path: \"Editor.tool\", is: Eq(Lya)))").unwrap_err();
    assert!(
        matches!(&error, LoadError::BadValue { value, used_by, .. } if value == "Lya" && used_by == "step 1"),
        "{error:?}"
    );
    let error = load("Check(Within(5, InState(state: \"Mode\", value: \"Editing\")))").unwrap_err();
    assert!(matches!(&error, LoadError::BadValue { .. }), "{error:?}");
}

#[test]
fn ordering_values_that_have_no_order_is_a_load_error() {
    let error = load("Check(Resource(path: \"Editor.tool\", is: Lt(Lay)))").unwrap_err();
    assert_eq!(
        error.to_string(),
        "step 1: `Tool` values have no order; compare them with `Eq` or `Ne`"
    );
}

#[test]
fn a_type_of_the_wrong_kind_is_a_load_error() {
    let error = load("Check(Resource(path: \"Ghost\", is: Eq(())))").unwrap_err();
    assert_eq!(
        error.to_string(),
        "step 1: `Ghost` is not a reflected resource; add `#[reflect(Resource)]`"
    );
}

#[test]
fn a_type_the_app_never_registered_fails_when_the_check_runs() {
    /// Never registered anywhere, not even by itself.
    #[derive(Component, Reflect, Debug)]
    #[reflect(no_auto_register)]
    struct Unregistered;
    let report = run(&Scenario::new("unregistered")
        .setup(spawn)
        .step(Step::check(!Check::count::<Unregistered>().eq(0))));
    assert_eq!(
        outcomes(&report),
        [Outcome::Fail("no type `Unregistered` is registered".into())]
    );
}

#[test]
fn the_same_scenario_gives_the_same_report_every_time() {
    let scenario = every_kind().setup(spawn);
    assert_eq!(run(&scenario), run(&scenario));
}

#[test]
fn a_misspelt_field_in_rust_cannot_run_even_inside_not() {
    let report = run(&Scenario::new("misspelt")
        .setup(spawn)
        .step(Step::check(!Check::resource::<Editor>("tol").eq(Tool::Lay))));
    assert_eq!(
        outcomes(&report),
        [Outcome::Fail(
            "`Editor` has no field `tol`; did you mean `tool`?".into()
        )]
    );
}

#[test]
fn installing_adds_state_type_data_to_a_state_the_app_registered_without_it() {
    // The app knows `Mode` as a plain reflected type, as Bevy's automatic
    // registration would leave it, but not as a state.
    let mut app = App::new();
    app.add_plugins((HeadlessPlugins::default(), StatesPlugin))
        .init_state::<Mode>()
        .register_type::<Mode>();
    app.finish();
    app.cleanup();
    let mut registry = Registry::default();
    registry.register_state::<Mode>();
    let scenario = Scenario::from_ron_with(
        "(format_version: 3, name: \"x\", steps: [Check(InState(state: \"Mode\", value: Menu))])",
        &registry,
    )
    .unwrap();
    registry.install(app.world_mut());
    let report = scenario.run_in(&mut app);
    assert_eq!(outcomes(&report), [Outcome::Pass]);
}
