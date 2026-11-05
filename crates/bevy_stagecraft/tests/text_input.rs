//! A text input has a role and a value that a scenario can check.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    input_focus::tab_navigation::TabIndex,
    prelude::*,
    text::{EditableText, TextEdit},
    ui::px,
    ui_widgets::TextInput,
};
use bevy_stagecraft::{
    Check, HeadlessPlugins, NoiseFilter, Outcome, Scenario, Step, Target, TextMatch, WidgetRole,
    WorldSnapshot, checks_that_hold, suggest_checks, target_alternatives, tree_snapshot,
};

/// Spawns a camera and one field labelled "Name".
fn spawn_field(world: &mut World) {
    world.spawn(Camera2d);
    world.spawn((
        TextInput,
        TabIndex(0),
        EditableText::new(""),
        Node {
            position_type: PositionType::Absolute,
            left: px(20),
            top: px(20),
            width: px(300),
            height: px(40),
            ..Node::default()
        },
        children![Text::new("Name")],
    ));
}

/// The field, named by its role and label.
fn field() -> Target {
    Target::role(WidgetRole::TextInput, "Name")
}

/// A headless app with the field laid out, and the field.
fn app() -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    spawn_field(app.world_mut());
    for _ in 0..3 {
        app.update();
    }
    let root = app
        .world_mut()
        .query_filtered::<Entity, With<TextInput>>()
        .iter(app.world())
        .next()
        .expect("the field");
    (app, root)
}

/// Types `text` into the field of `app` as the player would not: through
/// an edit queued on it.
fn insert(app: &mut App, root: Entity, text: &str) {
    app.world_mut()
        .get_mut::<EditableText>(root)
        .expect("text")
        .queue_edit(TextEdit::Insert(text.into()));
    app.update();
}

/// The outcome of a check after "Alice" was typed into the field.
fn outcome_after_typing(check: Check) -> Outcome {
    let report = Scenario::new("typed")
        .setup(spawn_field)
        .steps([
            Step::type_into("Alice", field()),
            Step::wait(2),
            Step::check(check),
        ])
        .run_headless();
    assert!(
        report.steps[0].outcome == Outcome::Pass,
        "{:?}",
        report.failures()
    );
    report.steps[2].outcome.clone()
}

#[test]
fn a_text_input_is_found_by_its_role_and_label_and_typed_into() {
    let report = Scenario::new("typed")
        .setup(spawn_field)
        .steps([Step::type_into("Alice", field()), Step::wait(2)])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn the_value_of_a_text_input_is_checked_as_text() {
    assert_eq!(
        outcome_after_typing(Check::text_of(field(), TextMatch::exact("Alice"))),
        Outcome::Pass
    );
    assert_eq!(outcome_after_typing(Check::shows("Alice")), Outcome::Pass);
    assert!(matches!(
        outcome_after_typing(Check::text_of(field(), TextMatch::exact("Bob"))),
        Outcome::Fail(_)
    ));
}

#[test]
fn a_tree_snapshot_shows_a_text_inputs_role_and_value() {
    let (mut app, root) = app();
    insert(&mut app, root, "a \"b\"");
    let snapshot = tree_snapshot(app.world(), &field()).expect("snapshot");
    assert_eq!(snapshot, "- text-input \"Name\" = \"a \\\"b\\\"\"");
    let report = Scenario::new("tree")
        .setup(spawn_field)
        .steps([
            Step::type_into("hi", field()),
            Step::wait(2),
            Step::check(Check::tree(
                field(),
                "- text-input \"Name\" [focused] = \"hi\"",
            )),
            Step::check(Check::tree(field(), "- text-input")),
        ])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn picking_a_text_input_offers_its_role_and_value() {
    let (mut app, root) = app();
    insert(&mut app, root, "Alice");
    let targets = target_alternatives(app.world(), Some(root), None);
    assert!(targets.contains(&field()), "{targets:?}");
    let checks = checks_that_hold(app.world(), root, &field(), &NoiseFilter::default());
    assert!(
        checks.contains(&Check::text_of(field(), TextMatch::exact("Alice"))),
        "{checks:?}"
    );
}

#[test]
fn a_changed_value_is_suggested_as_shown_text() {
    let (mut app, root) = app();
    let before = WorldSnapshot::take(app.world(), &NoiseFilter::default());
    insert(&mut app, root, "Alice");
    let after = WorldSnapshot::take(app.world(), &NoiseFilter::default());
    let checks = suggest_checks(&before, &after, app.world());
    assert!(checks.contains(&Check::shows("Alice")), "{checks:?}");
}
