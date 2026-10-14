//! The `Type` step against Bevy's own text field and against a plain observer.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    input::keyboard::KeyboardInput,
    input_focus::{FocusCause, FocusedInput, InputFocus, tab_navigation::TabIndex},
    prelude::*,
    text::EditableText,
    ui::px,
    ui_widgets::TextInput,
};
use rin_bevy_ui_tester::{HeadlessPlugins, Outcome, Scenario, Step, Target, keystroke};

/// Everything typed into the observer field, as the key events delivered it.
#[derive(Resource, Default)]
struct Typed(String);

/// Spawns a camera and two text fields, "First" above "Second".
fn spawn_fields(world: &mut World) {
    world.spawn(Camera2d);
    for (name, top) in [("First", 20), ("Second", 120)] {
        world.spawn((
            Name::new(name),
            // Since Bevy 0.20 `EditableText` only holds the text; `TextInput`
            // is the widget that takes keys, and a click focuses only a node
            // with a `TabIndex`.
            TextInput,
            TabIndex(0),
            EditableText::new(""),
            Node {
                position_type: PositionType::Absolute,
                left: px(20),
                top: px(top),
                width: px(300),
                height: px(40),
                ..Node::default()
            },
        ));
    }
}

/// What a field named `name` holds after the scenario ran.
fn value_of(app: &mut App, name: &str) -> String {
    let mut query = app.world_mut().query::<(&Name, &EditableText)>();
    query
        .iter(app.world())
        .find(|(field, _)| field.as_str() == name)
        .map(|(_, text)| text.value().to_string())
        .expect("the field exists")
}

/// An app with the headless plugins, ready to run a scenario in.
fn headless() -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    app
}

/// Runs `scenario` over two text fields and returns the report and the app.
fn run(scenario: Scenario) -> (rin_bevy_ui_tester::ScenarioReport, App) {
    let mut app = headless();
    let report = scenario.setup(spawn_fields).run_in(&mut app);
    (report, app)
}

#[test]
fn typing_into_a_text_field_ends_with_exactly_that_text() {
    let (report, mut app) = run(Scenario::new("type").steps([
        Step::move_to(Target::name("First")),
        Step::click(),
        Step::type_text("Alpha"),
        Step::wait(2),
    ]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(value_of(&mut app, "First"), "Alpha");
}

#[test]
fn typing_with_a_target_clicks_it_first() {
    let (report, mut app) = run(Scenario::new("type into").steps([
        Step::type_into("one", Target::name("First")),
        Step::type_into("two", Target::name("Second")),
        Step::wait(2),
    ]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(value_of(&mut app, "First"), "one");
    assert_eq!(value_of(&mut app, "Second"), "two");
}

#[test]
fn typing_without_a_target_goes_to_the_focused_field() {
    let (report, mut app) = run(Scenario::new("type focused").steps([
        Step::type_into("ab", Target::name("Second")),
        Step::type_text("cd"),
        Step::wait(2),
    ]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(value_of(&mut app, "Second"), "abcd");
    assert_eq!(value_of(&mut app, "First"), "");
}

#[test]
fn capitals_digits_symbols_and_spaces_are_typed_as_written() {
    let text = "Hello, World! 42 + (x) = \"y\"? #a_b-c/d\\e";
    let (report, mut app) =
        run(Scenario::new("mixed")
            .steps([Step::type_into(text, Target::name("First")), Step::wait(2)]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(value_of(&mut app, "First"), text);
}

#[test]
fn typing_adds_shift_only_where_the_character_needs_it() {
    assert!(keystroke('A').shift);
    assert!(keystroke('!').shift);
    assert!(!keystroke('a').shift);
    assert!(!keystroke('7').shift);
    assert!(!keystroke(' ').shift);
    assert_eq!(keystroke('A').key, keystroke('a').key);
    assert_eq!(keystroke('!').key, keystroke('1').key);
}

#[test]
fn shift_is_released_after_typing() {
    let (report, app) = run(Scenario::new("shift")
        .steps([Step::type_into("ABC", Target::name("First")), Step::wait(2)]));
    assert!(report.passed(), "{:?}", report.failures());
    let keys = app
        .world()
        .resource::<ButtonInput<bevy::input::keyboard::Key>>();
    assert!(!keys.pressed(bevy::input::keyboard::Key::Shift));
}

#[test]
fn typing_with_nothing_focused_does_not_fail_and_types_nowhere() {
    let (report, mut app) =
        run(Scenario::new("nowhere").steps([Step::type_text("lost"), Step::wait(2)]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(value_of(&mut app, "First"), "");
}

#[test]
fn typing_into_a_missing_target_fails_naming_it() {
    let (report, _app) =
        run(Scenario::new("missing")
            .steps([Step::type_into("x", Target::name("Nope")), Step::wait(1)]));
    assert!(!report.passed());
    let failures = report.failures();
    assert!(
        failures.iter().any(|failure| failure.contains("Nope")),
        "{failures:?}"
    );
    assert!(matches!(report.steps[0].outcome, Outcome::Fail(_)));
}

#[test]
fn an_observer_on_keyboard_input_receives_one_event_per_character() {
    let mut app = headless();
    app.init_resource::<Typed>();
    let report = Scenario::new("observer")
        .setup(|world| {
            world.spawn(Camera2d);
            let field = world
                .spawn((
                    Name::new("Plain"),
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(20),
                        top: px(20),
                        width: px(300),
                        height: px(40),
                        ..Node::default()
                    },
                ))
                .observe(
                    |input: On<FocusedInput<KeyboardInput>>, mut typed: ResMut<Typed>| {
                        if input.input.state.is_pressed()
                            && let Some(text) = &input.input.text
                        {
                            typed.0.push_str(text);
                        }
                    },
                )
                .id();
            world
                .resource_mut::<InputFocus>()
                .set(field, FocusCause::Navigated);
        })
        .steps([Step::type_text("Hi there, 5!"), Step::wait(2)])
        .run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(app.world().resource::<Typed>().0, "Hi there, 5!");
}
