//! Locators on a plain Bevy UI: finding a widget by marker component, by
//! `Name`, by role and label, inside another target and as the nth of
//! several; refusing ambiguity, suggesting close matches for a miss, and
//! reading and writing them as RON.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::panic,
    unreachable_pub,
    reason = "tests may panic on a broken setup; the twin components are `pub` in private modules"
)]

mod common;

use bevy::{
    prelude::*,
    ui::{ComputedNode, UiGlobalTransform, px},
    ui_widgets::{Activate, Button, Checkbox},
};
use bevy_stagecraft::{
    Check, HeadlessPlugins, LoadError, Outcome, Registry, Scenario, ScenarioReport, Step, Target,
    WidgetRole, target_alternatives,
};
use common::UiCheck;

/// The test id of the "Pause" button.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
struct PauseButton;

/// Marks each row of the list; every row has it, so it is no test id.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
struct Row;

/// A reflected type that is not a component.
#[derive(Reflect, Debug, Default)]
struct NotAComponent;

/// One of two reflected components with the same short name.
mod first {
    use bevy::prelude::*;

    /// A component whose short name [`super::second::Twin`] shares.
    #[derive(Component, Reflect, Debug, Default)]
    #[reflect(Component)]
    pub struct Twin;
}

/// The other of two reflected components with the same short name.
mod second {
    use bevy::prelude::*;

    /// A component whose short name [`super::first::Twin`] shares.
    #[derive(Component, Reflect, Debug, Default)]
    #[reflect(Component)]
    pub struct Twin;
}

/// What was clicked: how often "Pause", and which rows were deleted, in
/// order.
#[derive(Resource, Default, Debug)]
struct Clicks {
    /// Activations of "Pause".
    pauses: u32,
    /// The fruit of each row whose "Delete" was activated.
    deleted: Vec<String>,
}

/// A button showing `label`.
fn button(label: &str) -> impl Bundle {
    (
        Button,
        Node {
            width: px(100),
            height: px(30),
            ..Node::default()
        },
        children![Text::new(label)],
    )
}

/// A column with a "Pause" button (test id, `Name` and label), two rows of
/// a fruit and a "Delete" button, and an unchecked "Dark mode" checkbox.
fn spawn(world: &mut World) {
    world.init_resource::<Clicks>();
    world.spawn(Camera2d);
    let column = world
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            ..Node::default()
        })
        .id();
    world
        .spawn((
            button("Pause"),
            PauseButton,
            Name::new("pause"),
            ChildOf(column),
        ))
        .observe(|_: On<Activate>, mut clicks: ResMut<Clicks>| clicks.pauses += 1);
    for (row, fruit) in [("first row", "Apples"), ("second row", "Pears")] {
        let row = world
            .spawn((
                Row,
                Name::new(row),
                Node {
                    flex_direction: FlexDirection::Row,
                    ..Node::default()
                },
                ChildOf(column),
            ))
            .id();
        world.spawn((Text::new(fruit), ChildOf(row)));
        world.spawn((button("Delete"), ChildOf(row))).observe(
            move |_: On<Activate>, mut clicks: ResMut<Clicks>| {
                clicks.deleted.push(fruit.to_owned());
            },
        );
    }
    world.spawn((
        Checkbox,
        Node {
            width: px(100),
            height: px(30),
            ..Node::default()
        },
        children![Text::new("Dark mode")],
        ChildOf(column),
    ));
}

/// A headless app.
fn app() -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    app
}

/// Plays `scenario` in a fresh app, returning the report and the clicks.
fn run(scenario: &Scenario) -> (ScenarioReport, Clicks) {
    let mut app = app();
    let report = scenario.run_in(&mut app);
    let clicks = app
        .world_mut()
        .remove_resource::<Clicks>()
        .expect("the setup ran");
    (report, clicks)
}

/// The outcome of every step, in order.
fn outcomes(report: &ScenarioReport) -> Vec<Outcome> {
    report
        .steps
        .iter()
        .map(|record| record.outcome.clone())
        .collect()
}

/// Moves to `target`, clicks, and lets a frame pass.
const fn click(target: Target) -> [Step; 3] {
    [Step::move_to(target), Step::click(), Step::wait(1)]
}

/// The "Delete" button of the row with this name.
fn delete_in(row: &str) -> Target {
    Target::within(
        Target::name(row),
        Target::role(WidgetRole::Button, "Delete"),
    )
}

#[test]
fn a_click_through_each_locator_lands_on_the_widget_it_names() {
    let scenario = Scenario::new("locators")
        .setup(spawn)
        .steps(click(Target::component::<PauseButton>()))
        .steps(click(Target::name("pause")))
        .steps(click(Target::role(WidgetRole::Button, "Pause")))
        .steps(click(delete_in("second row")))
        .steps(click(Target::nth(0, Target::text("Delete"))))
        .steps([
            Step::check(Check::exists(Target::of_role(WidgetRole::Checkbox))),
            Step::check(!Check::checked(Target::role(
                WidgetRole::Checkbox,
                "Dark mode",
            ))),
            Step::check(Check::hovered(Target::nth(
                0,
                Target::role(WidgetRole::Button, "Delete"),
            ))),
            Step::check(Check::text_of(
                Target::within(Target::component::<PauseButton>(), Target::text("Pause")),
                bevy_stagecraft::TextMatch::exact("Pause"),
            )),
        ]);
    let (report, clicks) = run(&scenario);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(clicks.pauses, 3, "three ways to the same button");
    assert_eq!(
        clicks.deleted,
        ["Pears", "Apples"],
        "the second row's by nesting, then the first in document order"
    );
}

#[test]
fn a_target_that_matches_several_fails_the_step_and_lists_them() {
    let (report, clicks) = run(&Scenario::new("ambiguous").setup(spawn).steps([
        Step::check(Check::exists(Target::component::<Row>())),
        Step::move_to(Target::text("Delete")),
    ]));
    let outcomes = outcomes(&report);
    let Outcome::Fail(rows) = &outcomes[0] else {
        panic!("{outcomes:?}");
    };
    assert!(
        rows.starts_with("Component(\"Row\") is ambiguous: it matches 2 widgets: `first row` (")
            && rows.contains("`second row` (")
            && rows.ends_with("; pick one with Nth(..) or narrow it with Within(..)"),
        "{rows}"
    );
    let Outcome::Fail(deletes) = &outcomes[1] else {
        panic!("{outcomes:?}");
    };
    assert!(
        deletes.starts_with("Text(\"Delete\") is ambiguous: it matches 2 widgets: ")
            && deletes.matches("a node showing \"Delete\" (").count() == 2
            && deletes.contains(") at ("),
        "each match is listed with where it is: {deletes}"
    );
    assert!(clicks.deleted.is_empty(), "nothing was clicked");
}

#[test]
fn an_ambiguous_target_fails_a_check_even_inside_not_or_any() {
    let deletes = Target::role(WidgetRole::Button, "Delete");
    let checks = [
        !Check::exists(Target::text("Delete")),
        !Check::disabled(deletes.clone()),
        !Check::hovered(Target::component::<Row>()),
        Check::any([Check::exists(deletes), Check::exists(Target::text("Pause"))]),
    ];
    // A failed step skips the rest, so each check plays on its own.
    for check in checks {
        let (report, _) = run(&Scenario::new("ambiguous inverted")
            .setup(spawn)
            .steps([Step::check(check.clone())]));
        let outcomes = outcomes(&report);
        let [Outcome::Fail(message)] = outcomes.as_slice() else {
            panic!("{check} should fail, but came to {outcomes:?}");
        };
        assert!(
            message.contains(" is ambiguous: it matches 2 widgets: ")
                && message.ends_with("; pick one with Nth(..) or narrow it with Within(..)"),
            "{check} lists the matches: {message}"
        );
    }
}

#[test]
fn a_target_that_names_an_entity_outside_the_ui_says_it_is_not_a_ui_node() {
    let (report, _) = run(&Scenario::new("camera")
        .setup(|world: &mut World| {
            spawn(world);
            world.spawn((
                Camera2d,
                Name::new("spare camera"),
                Camera {
                    is_active: false,
                    ..Camera::default()
                },
            ));
        })
        .steps([Step::move_to(Target::name("spare camera"))]));
    let outcomes = outcomes(&report);
    let [Outcome::Fail(message)] = outcomes.as_slice() else {
        panic!("{outcomes:?}");
    };
    assert!(
        message.starts_with("Name(\"spare camera\") names `spare camera` (")
            && message.ends_with("), which is not a UI node"),
        "{message}"
    );
}

#[test]
fn not_exists_over_a_target_that_matches_nothing_still_holds() {
    let (report, _) = run(&Scenario::new("absent").setup(spawn).steps([
        Step::check(!Check::exists(Target::text("Quit"))),
        Step::check(!Check::exists(Target::within(
            Target::name("first row"),
            Target::text("Pears"),
        ))),
    ]));
    assert_eq!(outcomes(&report), [Outcome::Pass, Outcome::Pass]);
}

#[test]
fn a_target_that_matches_nothing_suggests_close_matches() {
    let misses = [
        (
            Target::text("Pasue"),
            "no widget shows \"Pasue\"; did you mean \"Pause\"?",
        ),
        (
            Target::name("paus"),
            "no entity is named \"paus\"; did you mean \"pause\"?",
        ),
        (
            Target::role(WidgetRole::Button, "Delte"),
            "no Button is labelled \"Delte\"; did you mean \"Delete\"?",
        ),
        (Target::of_role(WidgetRole::Slider), "there is no Slider"),
        (
            Target::nth(2, Target::text("Delete")),
            "Nth(2, Text(\"Delete\")) counts from 0, but only 2 widgets match Text(\"Delete\")",
        ),
        (
            Target::within(Target::name("first row"), Target::text("Pears")),
            "Text(\"Pears\") matches 1 widget, but none below Name(\"first row\")",
        ),
        (
            Target::within(Target::name("frist row"), Target::text("Pears")),
            "no entity is named \"frist row\"; did you mean \"first row\"?",
        ),
    ];
    let scenario = Scenario::new("misses").setup(spawn).steps(
        misses
            .iter()
            .map(|(target, _)| Step::check(Check::exists(target.clone()))),
    );
    let (report, _) = run(&scenario);
    let wanted: Vec<Outcome> = misses
        .iter()
        .map(|(_, message)| Outcome::Fail((*message).to_owned()))
        .collect();
    assert_eq!(outcomes(&report), wanted);
}

/// A scenario using every locator in steps, in checks and in an app check.
fn every_locator() -> Scenario {
    Scenario::new("every locator")
        .steps(click(Target::component::<PauseButton>()))
        .steps(click(Target::name("pause")))
        .steps(click(Target::role(WidgetRole::Button, "Pause")))
        .steps(click(delete_in("second row")))
        .steps(click(Target::nth(1, Target::text("Delete"))))
        .steps([
            Step::check(Check::exists(Target::of_role(WidgetRole::Checkbox))),
            Step::check(Check::app(UiCheck::Found(Target::nth(
                0,
                Target::component::<Row>(),
            )))),
        ])
}

#[test]
fn every_locator_round_trips_through_ron_and_plays_from_a_file() {
    let ron = every_locator().to_ron().unwrap();
    for written in [
        "MoveTo(Component(\"PauseButton\"))",
        "MoveTo(Name(\"pause\"))",
        "MoveTo(Within(Name(\"second row\"), Role(",
        "MoveTo(Nth(1, Text(\"Delete\")))",
        "role: Checkbox,",
        "label: \"Pause\",",
        // An app check's value is written compact.
        "Check(App(Found(Nth(0,Component(\"Row\")))))",
    ] {
        assert!(ron.contains(written), "{written} in {ron}");
    }
    // A role reads as written by hand on one line, the label left out for
    // any.
    let by_hand = "(format_version: 3, name: \"x\", steps: [\
        MoveTo(Role(role: Button, label: \"Pause\")), Check(Exists(Role(role: Checkbox)))])";
    let read = Scenario::from_ron_with(by_hand, &Registry::default()).unwrap();
    assert_eq!(
        read.to_script().steps,
        [
            Step::move_to(Target::role(WidgetRole::Button, "Pause")),
            Step::check(Check::exists(Target::of_role(WidgetRole::Checkbox))),
        ]
    );
    let loaded = Scenario::from_ron_with(&ron, &Registry::default()).unwrap();
    assert_eq!(loaded.to_script(), every_locator().to_script());
    assert_eq!(loaded.to_ron().unwrap(), ron, "export is stable");

    let (report, clicks) = run(&loaded.setup(spawn));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(clicks.pauses, 3);
    assert_eq!(clicks.deleted, ["Pears", "Pears"]);
}

/// Loads a script with these steps against the default registry.
fn load(steps: &str) -> Result<Scenario, LoadError> {
    Scenario::from_ron_with(
        &format!("(format_version: 3, name: \"x\", steps: [Wait(1), {steps}])"),
        &Registry::default(),
    )
}

#[test]
fn a_misspelt_component_is_a_load_error_wherever_the_target_is() {
    for steps in [
        "MoveTo(Component(\"PauseButon\"))",
        "Check(Exists(Within(Name(\"pause\"), Component(\"PauseButon\"))))",
        "Check(Not(Hovered(Nth(0, Component(\"PauseButon\")))))",
        "Check(App(Found(Component(\"PauseButon\"))))",
    ] {
        let error = load(steps).unwrap_err();
        assert!(
            matches!(
                &error,
                LoadError::UnknownType { name, used_by, suggestions }
                    if name == "PauseButon" && used_by == "step 1"
                        && suggestions.first().is_some_and(|first| first == "PauseButton")
            ),
            "{steps}: {error:?}"
        );
        assert!(
            error.to_string().starts_with(
                "step 1 names type `PauseButon`, which is not registered; did you mean `PauseButton`"
            ),
            "{error}"
        );
    }
}

#[test]
fn a_component_target_must_name_one_reflected_component_and_nesting_must_name_widgets() {
    let error = load("MoveTo(Component(\"NotAComponent\"))").unwrap_err();
    assert_eq!(
        error.to_string(),
        "step 1: `NotAComponent` is not a reflected component; add `#[reflect(Component)]`"
    );
    let error = load("MoveTo(Component(\"Twin\"))").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("`Twin` names more than one registered type"),
        "{error}"
    );
    let error = load("MoveTo(Nth(0, Screen((1.0, 2.0))))").unwrap_err();
    assert_eq!(
        error.to_string(),
        "step 1: `Nth(..)` narrows down targets that name widgets, and Screen(Vec2(1.0, 2.0)) is a position"
    );
    assert!(load("MoveTo(Component(\"bevy_ui_widgets::button::Button\"))").is_ok());
}

/// A laid-out app showing [`spawn`].
fn laid_out() -> App {
    let mut app = app();
    spawn(app.world_mut());
    for _ in 0..3 {
        app.update();
    }
    app
}

/// The middle of `entity`, in logical pixels.
fn centre(app: &App, entity: Entity) -> Vec2 {
    let world = app.world();
    let node = world.get::<ComputedNode>(entity).expect("laid out");
    let transform = world.get::<UiGlobalTransform>(entity).expect("positioned");
    transform.affine().translation * node.inverse_scale_factor
}

/// The entity named `name`.
fn named(app: &mut App, name: &str) -> Entity {
    let world = app.world_mut();
    world
        .query::<(Entity, &Name)>()
        .iter(world)
        .find(|(_, named)| named.as_str() == name)
        .expect("an entity has the name")
        .0
}

#[test]
fn targets_rank_a_test_id_then_role_and_label_then_text_then_name_then_the_position() {
    let mut app = laid_out();
    let pause = named(&mut app, "pause");
    let at = centre(&app, pause);
    assert_eq!(
        target_alternatives(app.world(), Some(pause), None),
        [
            Target::component::<PauseButton>(),
            Target::role(WidgetRole::Button, "Pause"),
            Target::text("Pause"),
            Target::name("pause"),
            Target::screen(at),
        ]
    );
    // Every row has the marker and nothing in it is unique to it but its
    // name: a target that is ambiguous is never offered.
    let row = named(&mut app, "first row");
    let at = centre(&app, row);
    assert_eq!(
        target_alternatives(app.world(), Some(row), Some(at)),
        [Target::name("first row"), Target::screen(at)]
    );
}

#[test]
fn a_bevy_ui_button_is_no_button_role_and_a_widgets_button_is() {
    let mut world = World::new();
    // Since Bevy 0.20 the deprecated `bevy_ui` `Button` is an alias of a
    // crate-private struct that cannot be named here, so it is built and
    // inserted through reflection, as a scene would.
    let registry = AppTypeRegistry::new_with_derived_types();
    let registration = registry
        .read()
        .get_with_type_path("bevy_ui::widget::button::DeprecatedButton")
        .cloned()
        .expect("bevy_ui's deprecated Button is reflected");
    let value = registration
        .data::<ReflectDefault>()
        .expect("it has a default")
        .default();
    let ui_button = world.spawn_empty().id();
    registration
        .data::<ReflectComponent>()
        .expect("it is a component")
        .insert(
            &mut world.entity_mut(ui_button),
            value.as_partial_reflect(),
            &registry.read(),
        );
    let widget_button = world.spawn(Button).id();
    let fits = |entity: Entity| {
        world
            .get_entity(entity)
            .is_ok_and(|entity| WidgetRole::Button.fits(&entity))
    };
    assert!(!fits(ui_button), "bevy_ui's Button is deprecated");
    assert!(fits(widget_button));
}
