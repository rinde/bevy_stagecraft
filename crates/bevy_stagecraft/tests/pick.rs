//! Picking one node of a plain Bevy UI, headless: the node under a position,
//! the targets that name it, the checks that hold for it and the app's check
//! forms. The gallery's pick mode is tested in `gallery.rs`.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

mod common;

use bevy::{
    input::{ButtonState, mouse::MouseButton},
    prelude::*,
    ui::{ComputedNode, InteractionDisabled, UiGlobalTransform, px},
    ui_widgets::Button,
};
use common::{Activations, UiTarget, spawn_ui};
use bevy_stagecraft::{
    AppCheckForm, Check, FieldValue, HeadlessPlugins, NoiseFilter, NotUnderTest, Picked,
    RecordedEvent, RecordedKind, Step, StepOptions, Target, TextMatch, WidgetRole, WorldSnapshot,
    app_check_forms, checks_that_hold, node_at, read_log, subject_of, target_alternatives,
};

/// A value the "Save" button carries, so a component check has a field.
#[derive(Component, Reflect, Debug, Clone, Copy)]
#[reflect(Component)]
struct Level(u32);

/// Spawns the common "Pause" button, a disabled "Save" button with a
/// [`Level`], a label showing a number, two labels showing the same text,
/// and a tooling node lying over "Pause".
fn spawn_form(world: &mut World) {
    spawn_ui(world);
    world.spawn((
        Button,
        InteractionDisabled,
        Level(3),
        Node {
            position_type: PositionType::Absolute,
            left: px(100),
            top: px(120),
            width: px(100),
            height: px(50),
            ..Node::default()
        },
        BackgroundColor(Color::WHITE),
        children![Text::new("Save")],
    ));
    world.spawn((
        Text::new("Score: 3"),
        Node {
            position_type: PositionType::Absolute,
            left: px(300),
            top: px(25),
            ..Node::default()
        },
    ));
    for top in [200, 240] {
        world.spawn((
            Text::new("Twice"),
            Node {
                position_type: PositionType::Absolute,
                left: px(300),
                top: px(top),
                ..Node::default()
            },
        ));
    }
    world.spawn((
        NotUnderTest,
        Node {
            position_type: PositionType::Absolute,
            left: px(90),
            top: px(15),
            width: px(120),
            height: px(70),
            ..Node::default()
        },
        GlobalZIndex(10),
    ));
}

/// A laid-out headless app showing [`spawn_form`].
fn form_app() -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    spawn_form(app.world_mut());
    for _ in 0..3 {
        app.update();
    }
    app
}

/// The node showing exactly `text`.
fn text_node(app: &mut App, text: &str) -> Entity {
    let world = app.world_mut();
    world
        .query::<(Entity, &Text)>()
        .iter(world)
        .find(|(_, shown)| shown.0 == text)
        .expect("a node shows the text")
        .0
}

/// The button labelled `text`.
fn button_with(app: &mut App, text: &str) -> Entity {
    let label = text_node(app, text);
    app.world()
        .get::<ChildOf>(label)
        .expect("a label sits in a button")
        .parent()
}

/// The middle of `entity`, in logical pixels.
fn centre(app: &App, entity: Entity) -> Vec2 {
    let world = app.world();
    let node = world.get::<ComputedNode>(entity).expect("laid out");
    let transform = world.get::<UiGlobalTransform>(entity).expect("positioned");
    transform.affine().translation * node.inverse_scale_factor
}

#[test]
fn the_node_under_a_position_is_the_apps_topmost_and_never_the_toolings() {
    let mut app = form_app();
    let save = button_with(&mut app, "Save");
    let hit = node_at(app.world(), centre(&app, save)).expect("something is there");
    assert!(
        hit == save || hit == text_node(&mut app, "Save"),
        "the label or its button"
    );
    assert_eq!(
        subject_of(app.world(), hit),
        save,
        "the widget it belongs to"
    );

    let pause = button_with(&mut app, "Pause");
    let under_tooling = node_at(app.world(), centre(&app, pause)).expect("the app's node");
    assert_eq!(
        subject_of(app.world(), under_tooling),
        pause,
        "the tooling on top is passed over"
    );
    assert_eq!(node_at(app.world(), Vec2::new(5.0, 470.0)), None, "nothing");
}

#[test]
fn role_and_label_come_before_a_unique_label_and_the_screen_position_is_last() {
    let mut app = form_app();
    let save = button_with(&mut app, "Save");
    let targets = target_alternatives(app.world(), Some(save), None);
    assert_eq!(
        targets,
        [
            Target::role(WidgetRole::Button, "Save"),
            Target::text("Save"),
            Target::screen(Vec2::new(150.0, 145.0))
        ]
    );
    let twice = text_node(&mut app, "Twice");
    let targets = target_alternatives(app.world(), Some(twice), None);
    assert!(
        matches!(targets.as_slice(), [Target::Screen(_)]),
        "a text two nodes show finds neither: {targets:?}"
    );
}

#[test]
fn picking_a_disabled_button_offers_disabled_through_its_label() {
    let mut app = form_app();
    let save = button_with(&mut app, "Save");
    let checks = checks_that_hold(
        app.world(),
        save,
        &Target::text("Save"),
        &NoiseFilter::default(),
    );
    let save_target = || Target::text("Save");
    for wanted in [
        Check::shows("Save"),
        Check::visible(save_target()),
        Check::disabled(save_target()),
        Check::component::<Level>(save_target(), ".0").eq(3_u32),
    ] {
        assert!(checks.contains(&wanted), "{wanted} in {checks:?}");
    }
    assert!(
        !checks.contains(&!Check::disabled(save_target())),
        "only what holds: {checks:?}"
    );
    let pause = button_with(&mut app, "Pause");
    let checks = checks_that_hold(
        app.world(),
        pause,
        &Target::text("Pause"),
        &NoiseFilter::default(),
    );
    assert!(
        checks.contains(&!Check::disabled(Target::text("Pause"))),
        "an enabled button offers that it is not disabled: {checks:?}"
    );
}

#[test]
fn through_a_screen_target_only_the_texts_shown_are_offered() {
    let mut app = form_app();
    let save = button_with(&mut app, "Save");
    let checks = checks_that_hold(
        app.world(),
        save,
        &Target::screen(Vec2::new(150.0, 145.0)),
        &NoiseFilter::default(),
    );
    assert_eq!(checks, [Check::shows("Save")]);
}

#[test]
fn a_label_that_is_not_its_own_target_offers_its_text_through_text_of() {
    let mut app = form_app();
    let label = text_node(&mut app, "Score: 3");
    // The app names the label with a target of its own.
    let app_target = Target::app(UiTarget::Showing("Score: 3".into()));
    let checks = checks_that_hold(app.world(), label, &app_target, &NoiseFilter::default());
    assert!(
        checks.contains(&Check::text_of(app_target, TextMatch::exact("Score: 3"))),
        "{checks:?}"
    );
}

#[test]
fn app_check_forms_fill_the_target_and_a_number_from_the_picked_node() {
    let mut app = form_app();
    let label = text_node(&mut app, "Score: 3");
    let target = Target::text("Score: 3");
    let forms = app_check_forms(app.world(), label, Some(&target));
    let written: Vec<Option<String>> = forms.iter().map(AppCheckForm::ron).collect();
    assert_eq!(
        written,
        [
            Some("ActivatedOnce".to_owned()),
            Some("Activated(3)".to_owned()),
            Some("Found(Text(\"Score: 3\"))".to_owned()),
            Some("ActivatedBetween(at_least: 3, at_most: 3)".to_owned()),
        ]
    );
}

#[test]
fn a_picked_node_works_out_which_app_checks_hold_and_a_stepped_field_changes_them() {
    let mut app = form_app();
    let pause = button_with(&mut app, "Pause");
    let mut picked = Picked::new(app.world(), pause, &NoiseFilter::default(), None);
    assert_eq!(picked.subject(), pause);
    assert_eq!(
        picked.target(),
        Some(&Target::role(WidgetRole::Button, "Pause"))
    );
    let activated = |picked: &Picked| {
        picked
            .forms()
            .iter()
            .position(|form| form.form.variant.name == "Activated")
            .expect("a form per variant")
    };
    let at = activated(&picked);
    let form = &picked.forms()[at];
    assert_eq!(
        form.form.values,
        [FieldValue::Integer {
            value: 0,
            signed: false
        }]
    );
    assert!(form.holds, "nothing was activated yet");
    picked.step_field(app.world(), at, 0, true);
    let form = &picked.forms()[at];
    assert_eq!(form.form.ron().as_deref(), Some("Activated(1)"));
    assert!(!form.holds, "it was not activated once");
    let found = picked
        .forms()
        .iter()
        .find(|form| form.form.variant.name == "Found")
        .expect("a form with a target");
    assert!(found.holds, "{:?}", found.check);
}

#[test]
fn choosing_another_target_works_the_checks_out_through_it() {
    let mut app = form_app();
    let save = button_with(&mut app, "Save");
    let mut picked = Picked::new(app.world(), save, &NoiseFilter::default(), None);
    assert!(
        picked
            .checks()
            .contains(&Check::disabled(Target::role(WidgetRole::Button, "Save")))
    );
    picked.choose_target(app.world(), 1);
    assert_eq!(picked.target(), Some(&Target::text("Save")));
    assert!(
        picked
            .checks()
            .contains(&Check::disabled(Target::text("Save")))
    );
    picked.choose_target(app.world(), 2);
    assert_eq!(
        picked.target(),
        Some(&Target::screen(Vec2::new(150.0, 145.0)))
    );
    assert_eq!(picked.checks(), [Check::shows("Save")]);
}

#[test]
fn picking_reads_the_world_and_changes_nothing_in_it() {
    let mut app = form_app();
    let save = button_with(&mut app, "Save");
    let before = WorldSnapshot::take(app.world(), &NoiseFilter::default());
    let picked = Picked::new(app.world(), save, &NoiseFilter::default(), None);
    assert!(!picked.checks().is_empty());
    let after = WorldSnapshot::take(app.world(), &NoiseFilter::default());
    assert_eq!(before, after);
    assert_eq!(app.world().resource::<Activations>().0, 0);
}

#[test]
fn a_check_chosen_while_recording_becomes_a_check_step_where_it_was_chosen() {
    let check = Check::disabled(Target::text("Save"));
    let events = [
        RecordedEvent {
            frame: 1,
            kind: RecordedKind::Moved(Vec2::new(10.0, 10.0)),
        },
        RecordedEvent {
            frame: 5,
            kind: RecordedKind::Check(check.clone()),
        },
        RecordedEvent {
            frame: 8,
            kind: RecordedKind::Button {
                button: MouseButton::Left,
                state: ButtonState::Pressed,
                position: Vec2::new(10.0, 10.0),
                target: Target::screen(Vec2::new(10.0, 10.0)),
            },
        },
        RecordedEvent {
            frame: 9,
            kind: RecordedKind::Button {
                button: MouseButton::Left,
                state: ButtonState::Released,
                position: Vec2::new(10.0, 10.0),
                target: Target::screen(Vec2::new(10.0, 10.0)),
            },
        },
    ];
    let logged = read_log(&events, 20, &StepOptions::default());
    let at = logged
        .steps
        .iter()
        .position(|step| *step == Step::check(check.clone()))
        .expect("the check is a step");
    let click = logged
        .steps
        .iter()
        .position(|step| *step == Step::click())
        .expect("the click is a step");
    assert!(at < click, "{:?}", logged.steps);
    assert!(
        logged.checkpoints.is_empty(),
        "a chosen check is no checkpoint"
    );
}
