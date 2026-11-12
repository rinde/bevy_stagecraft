//! Checks suggested from a world diff when a recording stops, on a plain
//! Bevy UI: a counter button, a label showing the count, a state and the
//! app's own checks with facts.
//!
//! Real input is imitated by writing the messages `bevy_winit` would.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

use std::{fs, path::PathBuf};

use bevy::{
    input::{ButtonState, mouse::MouseButtonInput},
    prelude::*,
    state::{app::StatesPlugin, reflect::ReflectState},
    ui::{ComputedNode, UiGlobalTransform, px},
    ui_widgets::{Activate, Button},
    window::{CursorMoved, PrimaryWindow, WindowEvent},
};
use bevy_stagecraft::{
    AppCheck, Check, CheckResult, HeadlessPlugins, NoiseFilter, Probe, ReflectAppCheck, Registry,
    Scenario, ScenarioReport, Script, SessionHooks, SessionPlugin, WorldSnapshot, session,
    suggest_checks,
};

/// What the "+1" button counts.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Counter(u32);

/// One per click, so the count of tallies changes.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct Tally;

/// Whether anything has been counted yet.
#[derive(States, Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[reflect(State)]
enum Mode {
    /// Nothing counted.
    #[default]
    Idle,
    /// Counted at least once.
    Counting,
}

/// The label that shows the count.
#[derive(Component)]
struct CountLabel;

/// The app's own checks, with the facts a recording may suggest.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(AppCheck)]
enum CounterCheck {
    /// The counter is at exactly this.
    CounterIs(u32),
    /// Something has been counted.
    Counted,
}

impl AppCheck for CounterCheck {
    fn run(&self, probe: &Probe) -> CheckResult {
        let count = probe.resource::<Counter>().map_or(0, |counter| counter.0);
        match self {
            Self::CounterIs(wanted) if count == *wanted => Ok(()),
            Self::CounterIs(_) => Err(format!("the counter is at {count}")),
            Self::Counted if count > 0 => Ok(()),
            Self::Counted => Err("nothing was counted".into()),
        }
    }

    fn facts(probe: &Probe) -> Vec<Self> {
        let count = probe.resource::<Counter>().map_or(0, |counter| counter.0);
        let mut facts = vec![Self::CounterIs(count)];
        if count > 0 {
            facts.push(Self::Counted);
        }
        facts
    }
}

/// Spawns the counter's UI and its camera.
fn spawn_ui(world: &mut World) {
    world.init_resource::<Counter>();
    world.spawn(Camera2d);
    world
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: px(100),
                top: px(100),
                width: px(120),
                height: px(60),
                ..Node::default()
            },
            BackgroundColor(Color::WHITE),
            children![Text::new("+1")],
        ))
        .observe(
            |_: On<Activate>,
             mut commands: Commands,
             mut counter: ResMut<Counter>,
             mut mode: ResMut<NextState<Mode>>| {
                counter.0 += 1;
                commands.spawn(Tally);
                mode.set(Mode::Counting);
            },
        );
    world.spawn((
        CountLabel,
        Text::new("0"),
        Node {
            position_type: PositionType::Absolute,
            left: px(300),
            top: px(120),
            ..Node::default()
        },
    ));
}

/// Keeps the label in step with the counter.
fn show_count(counter: Res<Counter>, mut labels: Query<&mut Text, With<CountLabel>>) {
    for mut text in &mut labels {
        let wanted = counter.0.to_string();
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
}

/// A finished headless app with the counter, before its UI is spawned.
fn bare_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        HeadlessPlugins::default().with_window_size(640, 480),
        StatesPlugin,
    ))
    .init_state::<Mode>()
    .add_systems(Update, show_count);
    app
}

/// The app with a session plugin writing into `dir`, its UI up and laid out.
fn recording_app(dir: &std::path::Path, hooks: SessionHooks) -> App {
    let mut app = bare_app();
    app.add_plugins(SessionPlugin::new(dir).with_hooks(hooks));
    spawn_ui(app.world_mut());
    app.finish();
    app.cleanup();
    for _ in 0..3 {
        app.update();
    }
    app
}

/// A fresh directory for one test's files.
fn dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    drop(fs::remove_dir_all(&dir));
    dir
}

fn primary(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .expect("a primary window")
}

/// What `bevy_winit` does for a moved real mouse.
fn real_move(app: &mut App, position: Vec2) {
    let window = primary(app);
    if let Some(mut window) = app.world_mut().get_mut::<Window>(window) {
        window.set_cursor_position(Some(position));
    }
    let moved = CursorMoved {
        window,
        position,
        delta: None,
    };
    app.world_mut()
        .write_message(WindowEvent::from(moved.clone()));
    app.world_mut().write_message(moved);
    app.update();
}

/// A real press and release, one update apart.
fn real_click(app: &mut App) {
    let window = primary(app);
    for state in [ButtonState::Pressed, ButtonState::Released] {
        let input = MouseButtonInput {
            button: MouseButton::Left,
            state,
            window,
        };
        app.world_mut().write_message(WindowEvent::from(input));
        app.world_mut().write_message(input);
        app.update();
    }
}

/// The middle of the button labelled "+1", in logical pixels.
fn button_centre(app: &mut App) -> Vec2 {
    let world = app.world_mut();
    let label = world
        .query::<(Entity, &Text)>()
        .iter(world)
        .find(|(_, shown)| shown.0 == "+1")
        .expect("the button's label")
        .0;
    let button = world.get::<ChildOf>(label).expect("a button").parent();
    let node = world.get::<ComputedNode>(button).expect("laid out");
    let transform = world.get::<UiGlobalTransform>(button).expect("positioned");
    transform.affine().translation * node.inverse_scale_factor
}

/// Records one click on "+1" and returns what was saved.
fn record_one_click(name: &str, hooks: SessionHooks) -> session::SavedSession {
    let dir = dir(name);
    let mut app = recording_app(&dir, hooks);
    session::start(app.world_mut(), "one click").expect("recording starts");
    let centre = button_centre(&mut app);
    real_move(&mut app, centre);
    real_move(&mut app, centre);
    real_click(&mut app);
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(app.world().resource::<Counter>().0, 1, "the click counted");
    session::stop(app.world_mut()).expect("the session is saved")
}

/// Plays the recording's file in a fresh app with the same start.
fn play(path: &std::path::Path) -> ScenarioReport {
    let text = fs::read_to_string(path).expect("the file was written");
    let script = Script::from_ron(&text).expect("the file reads back");
    let scenario = Scenario::from_script(script, &Registry::default())
        .expect("every suggested check loads")
        .setup(spawn_ui);
    let mut app = bare_app();
    app.finish();
    app.cleanup();
    scenario.run_in(&mut app)
}

#[test]
fn a_click_on_a_counter_suggests_its_new_value_and_text_and_no_layout_noise() {
    let saved = record_one_click("suggest-counter", SessionHooks::new());
    let candidates = &saved.candidates;
    for wanted in [
        Check::resource::<Counter>(".0").eq(1_u32),
        Check::shows("1"),
        !Check::shows("0"),
        Check::count::<Tally>().eq(1),
        Check::in_state(Mode::Counting),
    ] {
        assert!(
            candidates.contains(&wanted),
            "{wanted} is suggested: {candidates:#?}"
        );
    }
    let ron = saved.script.to_ron().expect("the suggestions write out");
    assert!(
        ron.contains("path: \"Counter.0\",\n            is: Eq(1),")
            && ron.contains("Shows(\"1\")"),
        "{ron}"
    );
    for noise in [
        "Transform",
        "GlobalTransform",
        "ComputedNode",
        "Node",
        "Interaction",
        "HoverMap",
        "Time",
        "ButtonInput",
        "Window",
    ] {
        assert!(
            !ron.contains(&format!("\"{noise}")),
            "{noise} is layout or engine noise: {ron}"
        );
    }
}

#[test]
fn an_app_check_that_implements_facts_is_suggested() {
    let saved = record_one_click("suggest-facts", SessionHooks::new());
    for wanted in [
        Check::app(CounterCheck::CounterIs(1)),
        Check::app(CounterCheck::Counted),
        !Check::app(CounterCheck::CounterIs(0)),
    ] {
        assert!(
            saved.candidates.contains(&wanted),
            "{wanted} is suggested: {:#?}",
            saved.candidates
        );
    }
}

#[test]
fn every_suggested_check_is_written_as_expect_and_passes_when_played_back() {
    let saved = record_one_click("suggest-playback", SessionHooks::new());
    assert_eq!(saved.script.expect, saved.candidates, "all start ticked");
    let report = play(&saved.path);
    assert!(report.passed(), "{:#?}", report.failures());
    assert_eq!(
        report.checks.len(),
        saved.candidates.len(),
        "every suggestion ran"
    );
}

#[test]
fn only_the_ticked_candidates_are_kept_in_the_file_and_they_pass_when_played_back() {
    let mut saved = record_one_click("suggest-ticked", SessionHooks::new());
    let ticked = vec![
        Check::resource::<Counter>(".0").eq(1_u32),
        Check::shows("1"),
        Check::app(CounterCheck::Counted),
    ];
    for check in &ticked {
        assert!(saved.candidates.contains(check), "{check}");
    }
    saved.save_expect(ticked.clone()).expect("written again");
    let text = fs::read_to_string(&saved.path).expect("the file");
    let script = Script::from_ron(&text).expect("it reads back");
    assert_eq!(script.expect, ticked, "{text}");
    assert!(!text.contains("Tally"), "unticked: {text}");
    let report = play(&saved.path);
    assert!(report.passed(), "{:#?}", report.failures());
    assert_eq!(report.checks.len(), ticked.len());
}

#[test]
fn the_candidates_come_out_in_the_same_order_on_every_run() {
    let first = record_one_click("suggest-order-1", SessionHooks::new()).candidates;
    for run in 2..=4 {
        let again = record_one_click(&format!("suggest-order-{run}"), SessionHooks::new());
        assert_eq!(again.candidates, first, "run {run}");
    }
    // Resources, counts, states, texts, widgets, then the app's facts.
    let position = |check: &Check| {
        first
            .iter()
            .position(|candidate| candidate == check)
            .expect("suggested")
    };
    assert!(
        position(&Check::resource::<Counter>(".0").eq(1_u32))
            < position(&Check::count::<Tally>().eq(1))
    );
    assert!(position(&Check::count::<Tally>().eq(1)) < position(&Check::in_state(Mode::Counting)));
    assert!(position(&Check::in_state(Mode::Counting)) < position(&Check::shows("1")));
    assert!(position(&Check::shows("1")) < position(&Check::app(CounterCheck::CounterIs(1))));
}

#[test]
fn the_noise_filter_can_be_changed_to_leave_out_an_apps_type() {
    let hooks =
        SessionHooks::new().noise_filter(NoiseFilter::default().ignore("Counter").ignore("Tally"));
    let saved = record_one_click("suggest-filtered", hooks);
    let ron = saved.script.to_ron().expect("written");
    assert!(!ron.contains("Counter.0"), "{ron}");
    assert!(!ron.contains("Tally"), "{ron}");
    assert!(saved.candidates.contains(&Check::shows("1")), "{ron}");
}

#[test]
fn keeping_a_bevy_type_brings_its_changes_back() {
    let mut app = bare_app();
    spawn_ui(app.world_mut());
    app.finish();
    app.cleanup();
    app.update();
    let mut changes = |filter: &NoiseFilter| {
        let before = WorldSnapshot::take(app.world(), filter);
        app.world_mut().spawn(Node::default());
        app.update();
        let after = WorldSnapshot::take(app.world(), filter);
        suggest_checks(&before, &after, app.world())
    };
    let counts_nodes = |candidates: &[Check]| {
        candidates
            .iter()
            .any(|check| matches!(check, Check::Count { with, .. } if with == &["Node".to_owned()]))
    };
    let default = changes(&NoiseFilter::default());
    assert!(!counts_nodes(&default), "{default:#?}");
    let kept = changes(&NoiseFilter::default().keep("Node"));
    assert!(counts_nodes(&kept), "{kept:#?}");
}

#[test]
fn nothing_suggests_nothing() {
    let mut app = bare_app();
    spawn_ui(app.world_mut());
    app.finish();
    app.cleanup();
    app.update();
    let snapshot = WorldSnapshot::take(app.world(), &NoiseFilter::default());
    app.update();
    let again = WorldSnapshot::take(app.world(), &NoiseFilter::default());
    assert_eq!(snapshot, again);
    assert_eq!(suggest_checks(&snapshot, &again, app.world()).len(), 0);
}
