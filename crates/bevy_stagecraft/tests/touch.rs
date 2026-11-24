//! Scripted touch steps over a plain Bevy UI: the messages each step writes,
//! and what the widgets and the pointer do with them.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    input::touch::{TouchInput, TouchPhase, Touches},
    picking::{input::PointerInputSettings, pointer::PointerId},
    prelude::*,
    ui::{InteractionDisabled, px},
    ui_widgets::{Activate, Button},
    window::PrimaryWindow,
};
use bevy_stagecraft::{Scenario, ScenarioReport, Step, Target};

/// How many times the button was activated.
#[derive(Resource, Default)]
struct Activations(u32);

/// How many clicks the pad got.
#[derive(Resource, Default)]
struct Clicks(u32);

/// The pad's presses.
#[derive(Resource, Default)]
struct Presses(u32);

/// The horizontal distance the pad was dragged.
#[derive(Resource, Default)]
struct DraggedRight(f32);

/// Every touch message, with the frame it arrived on.
#[derive(Resource, Default)]
struct TouchLog {
    /// The frame the app is on.
    frame: u32,
    /// Frame, finger, phase and position of each message.
    seen: Vec<(u32, u64, TouchPhase, Vec2)>,
}

/// Writes every touch message into the [`TouchLog`].
fn record_touches(mut log: ResMut<TouchLog>, mut touches: MessageReader<TouchInput>) {
    log.frame += 1;
    let frame = log.frame;
    for touch in touches.read() {
        log.seen
            .push((frame, touch.id, touch.phase, touch.position));
    }
}

/// The button's middle.
const BUTTON: Vec2 = Vec2::new(150.0, 50.0);
/// The pad's middle.
const PAD: Vec2 = Vec2::new(400.0, 200.0);

/// A camera, the "Pause" button and a "Pad" to click and drag.
fn spawn_ui(world: &mut World) {
    world.init_resource::<Activations>();
    world.init_resource::<Clicks>();
    world.init_resource::<Presses>();
    world.init_resource::<DraggedRight>();
    world.spawn(Camera2d);
    world
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: px(100),
                top: px(25),
                width: px(100),
                height: px(50),
                ..Node::default()
            },
            children![Text::new("Pause")],
        ))
        .observe(|_: On<Activate>, mut n: ResMut<Activations>| n.0 += 1);
    world
        .spawn((
            Name::new("Pad"),
            Node {
                position_type: PositionType::Absolute,
                left: px(300),
                top: px(100),
                width: px(200),
                height: px(200),
                ..Node::default()
            },
        ))
        .observe(|_: On<PointerClick>, mut n: ResMut<Clicks>| n.0 += 1)
        .observe(|_: On<PointerPress>, mut n: ResMut<Presses>| n.0 += 1)
        .observe(|drag: On<PointerDrag>, mut d: ResMut<DraggedRight>| d.0 += drag.delta.x);
}

/// A scenario over the test UI with these steps.
fn scenario(steps: impl IntoIterator<Item = Step>) -> Scenario {
    Scenario::new("touch").setup(spawn_ui).steps(steps)
}

/// Plays the scenario, returning the app and the report.
fn play(scenario: &Scenario) -> (App, ScenarioReport) {
    let mut app = scenario.headless_app();
    app.init_resource::<TouchLog>();
    app.add_systems(Update, record_touches);
    let report = scenario.run_in(&mut app);
    (app, report)
}

/// The touch messages seen, as (finger, phase) pairs.
fn phases(app: &App) -> Vec<(u64, TouchPhase)> {
    let log = app.world().resource::<TouchLog>();
    log.seen
        .iter()
        .map(|(_, id, phase, _)| (*id, *phase))
        .collect()
}

fn pad() -> Target {
    Target::name("Pad")
}

fn button() -> Target {
    Target::text("Pause")
}

#[test]
fn touch_down_up_writes_a_started_and_an_ended_message_at_the_target() {
    let (app, report) = play(&scenario([Step::touch_down(button()), Step::touch_up()]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(
        phases(&app),
        vec![(0, TouchPhase::Started), (0, TouchPhase::Ended)]
    );
    let log = app.world().resource::<TouchLog>();
    assert_eq!(log.seen[0].3, BUTTON);
    assert_eq!(log.seen[1].3, BUTTON, "the finger lifts where it is");
    assert_eq!(log.seen[1].0, log.seen[0].0 + 1, "one frame each");
}

#[test]
fn touch_move_and_cancel_write_moved_and_canceled() {
    let (app, report) = play(&scenario([
        Step::touch_down(button()),
        Step::touch_move(pad()),
        Step::touch_cancel(),
    ]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(
        phases(&app),
        vec![
            (0, TouchPhase::Started),
            (0, TouchPhase::Moved),
            (0, TouchPhase::Canceled)
        ]
    );
    assert_eq!(app.world().resource::<TouchLog>().seen[1].3, PAD);
}

#[test]
fn a_tap_is_a_down_and_an_up_on_consecutive_frames() {
    let (app, report) = play(&scenario([Step::touch_tap(button())]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(
        phases(&app),
        vec![(0, TouchPhase::Started), (0, TouchPhase::Ended)]
    );
    let log = app.world().resource::<TouchLog>();
    assert_eq!(log.seen[1].0, log.seen[0].0 + 1);
}

#[test]
fn a_drag_takes_frames_plus_two_frames() {
    let (app, report) = play(&scenario([Step::touch_drag(
        Target::screen(Vec2::new(350.0, 150.0)),
        Target::screen(Vec2::new(450.0, 150.0)),
        10,
    )]));
    assert!(report.passed(), "{:?}", report.failures());
    let log = app.world().resource::<TouchLog>();
    assert_eq!(log.seen.len(), 12, "down, ten moves, up");
    assert_eq!(log.seen[11].0 - log.seen[0].0, 11);
    assert_eq!(log.seen[11].3, Vec2::new(450.0, 150.0));
}

#[test]
fn another_finger_has_its_own_id() {
    let (app, report) = play(&scenario([
        Step::touch_down(button()).with_finger(3),
        Step::touch_up().with_finger(3),
    ]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(
        phases(&app),
        vec![(3, TouchPhase::Started), (3, TouchPhase::Ended)]
    );
}

#[test]
fn touch_does_not_move_the_mouse() {
    let (mut app, report) = play(&scenario([Step::touch_tap(button())]));
    assert!(report.passed(), "{:?}", report.failures());
    let world = app.world_mut();
    let cursor = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .expect("one primary window")
        .cursor_position();
    assert_eq!(cursor, None);
}

#[test]
fn touches_and_the_touch_pointer_see_a_finger_while_it_is_down() {
    let scenario = scenario([Step::touch_down(button()), Step::wait(2)]);
    let mut app = scenario.headless_app();
    let report = scenario.run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());
    let world = app.world_mut();
    assert_eq!(world.resource::<Touches>().iter().count(), 1);
    let pointers = world
        .query::<&PointerId>()
        .iter(world)
        .filter(|id| matches!(id, PointerId::Touch(0)))
        .count();
    assert_eq!(pointers, 1);
}

#[test]
fn the_touch_pointer_is_gone_after_the_finger_lifts() {
    let (mut app, report) = play(&scenario([Step::touch_tap(button()), Step::wait(2)]));
    assert!(report.passed(), "{:?}", report.failures());
    let world = app.world_mut();
    let pointers = world
        .query::<&PointerId>()
        .iter(world)
        .filter(|id| matches!(id, PointerId::Touch(_)))
        .count();
    assert_eq!(pointers, 0);
}

#[test]
fn tapping_a_button_activates_it() {
    let (app, report) = play(&scenario([Step::touch_tap(button())]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(app.world().resource::<Activations>().0, 1);
}

#[test]
fn tapping_a_pad_clicks_it() {
    let (app, report) = play(&scenario([Step::touch_tap(pad())]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(app.world().resource::<Clicks>().0, 1);
    assert_eq!(app.world().resource::<Presses>().0, 1);
}

#[test]
fn dragging_a_pad_reports_the_distance() {
    let (app, report) = play(&scenario([Step::touch_drag(
        Target::screen(Vec2::new(350.0, 150.0)),
        Target::screen(Vec2::new(550.0, 150.0)),
        10,
    )]));
    assert!(report.passed(), "{:?}", report.failures());
    let moved = app.world().resource::<DraggedRight>().0;
    assert!((199.0..=201.0).contains(&moved), "dragged {moved}");
}

#[test]
fn two_fingers_can_be_down_at_once() {
    let (app, report) = play(&scenario([
        Step::touch_down(button()).with_finger(1),
        Step::touch_down(pad()).with_finger(2),
        Step::touch_up().with_finger(1),
        Step::touch_up().with_finger(2),
    ]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(
        phases(&app),
        vec![
            (1, TouchPhase::Started),
            (2, TouchPhase::Started),
            (1, TouchPhase::Ended),
            (2, TouchPhase::Ended)
        ]
    );
}

#[test]
fn a_cancelled_finger_gives_no_click() {
    let (app, report) = play(&scenario([
        Step::touch_down(pad()),
        Step::wait(1),
        Step::touch_cancel(),
        Step::wait(2),
    ]));
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(app.world().resource::<Presses>().0, 1);
    assert_eq!(app.world().resource::<Clicks>().0, 0);
}

#[test]
fn lifting_a_finger_that_is_not_down_fails_the_step() {
    let (_, report) = play(&scenario([Step::touch_up()]));
    assert!(!report.passed());
    assert!(
        report
            .failures()
            .join("\n")
            .contains("finger 0 is not down"),
        "{:?}",
        report.failures()
    );
}

#[test]
fn putting_down_a_finger_that_is_down_fails_the_step() {
    let (_, report) = play(&scenario([
        Step::touch_down(button()),
        Step::touch_down(pad()),
    ]));
    assert!(
        report
            .failures()
            .join("\n")
            .contains("finger 0 is already down"),
        "{:?}",
        report.failures()
    );
}

#[test]
fn a_tap_on_a_disabled_button_waits_and_times_out_but_forced_activates_nothing() {
    let disable = |world: &mut World| {
        spawn_ui(world);
        let mut buttons = world.query_filtered::<Entity, With<Button>>();
        let entities: Vec<Entity> = buttons.iter(world).collect();
        for entity in entities {
            world.entity_mut(entity).insert(InteractionDisabled);
        }
    };
    let waiting = Scenario::new("t")
        .setup(disable)
        .action_timeout(3)
        .step(Step::touch_tap(button()));
    let (_, report) = play(&waiting);
    assert!(
        report.failures().join("\n").contains("disabled"),
        "{:?}",
        report.failures()
    );
    let forced = Scenario::new("f")
        .setup(disable)
        .step(Step::touch_tap(button()).forced());
    let (app, report) = play(&forced);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(app.world().resource::<Activations>().0, 0);
}

#[test]
fn a_timeout_may_wrap_a_touch_step_but_not_a_lift() {
    let ok = Scenario::new("a").step(Step::touch_tap(button()).with_timeout(5));
    assert!(ok.to_ron().is_ok());
    let text = "(format_version: 3, name: \"x\", steps: [Timeout(5, TouchUp())])";
    assert!(Scenario::from_ron(text).is_err());
}

#[test]
fn touch_steps_round_trip_through_ron() {
    let original = Scenario::new("rt").steps([
        Step::touch_down(button()),
        Step::touch_move(pad()).with_finger(2),
        Step::touch_up(),
        Step::touch_cancel().with_finger(2),
        Step::touch_tap(button()),
        Step::touch_drag(button(), pad(), 7),
    ]);
    let ron = original.to_ron().expect("exports");
    assert!(ron.contains("TouchUp()"), "{ron}");
    assert!(ron.contains("finger: 2"), "{ron}");
    assert!(!ron.contains("finger: 0"), "{ron}");
    let loaded = Scenario::from_ron(&ron).expect("loads");
    assert_eq!(loaded.to_ron().expect("exports"), ron);
}

#[test]
fn a_file_may_leave_the_finger_out() {
    let text = "(format_version: 3, name: \"x\", steps: [TouchDown(at: Text(\"Pause\")), TouchUp(finger: 1)])";
    let scenario = Scenario::from_ron(text).expect("loads");
    assert_eq!(scenario.script()[0], Step::touch_down(button()));
}

#[test]
fn a_failing_touch_step_is_named_in_the_report() {
    let (_, report) = play(&scenario([Step::touch_tap(Target::text("Nope"))]));
    assert!(
        report.failures().join("\n").contains("TouchTap"),
        "{:?}",
        report.failures()
    );
}

#[test]
fn touch_picking_is_on_in_a_headless_app() {
    let app = scenario([]).headless_app();
    assert!(
        app.world()
            .resource::<PointerInputSettings>()
            .is_touch_enabled
    );
}
