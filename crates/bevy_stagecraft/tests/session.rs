#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

use std::{fs, path::PathBuf};

use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyCode, KeyboardInput},
        mouse::MouseButtonInput,
        touch::{TouchInput, TouchPhase},
    },
    prelude::*,
    ui::{ComputedNode, UiGlobalTransform, px},
    ui_widgets::{Activate, Button},
    window::{CursorMoved, PrimaryWindow, WindowEvent},
};
use bevy_stagecraft::{
    AppSetup, AppTarget, AppView, Check, HeadlessPlugins, ReflectAppSetup, ReflectAppTarget,
    ReflectAppView, Registry, ResolvedTarget, Scenario, Script, SessionPlugin, SessionRecorder,
    Step, Target, WidgetRole,
    script::{Setup, View},
    session::{self, SessionConfig},
};

#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Count(u32);

fn spawn_ui(world: &mut World) {
    world.init_resource::<Count>();
    for (label, left) in [("+1", 100), ("Other", 400)] {
        world
            .spawn((
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(left),
                    top: px(100),
                    width: px(120),
                    height: px(60),
                    ..Node::default()
                },
                BackgroundColor(Color::WHITE),
                children![Text::new(label)],
            ))
            .observe(move |_: On<Activate>, mut count: ResMut<Count>| {
                if label == "+1" {
                    count.0 += 1;
                }
            });
    }
}

#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(AppSetup)]
enum SessionSetup {
    Saved(String),
}

#[derive(Resource)]
struct SaveFails;

impl AppSetup for SessionSetup {
    fn build(&self, world: &mut World) -> Result<(), String> {
        let Self::Saved(path) = self;
        let saved = fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
        let count = saved
            .trim()
            .parse()
            .map_err(|_| format!("{path} holds no count"))?;
        spawn_ui(world);
        world.insert_resource(Count(count));
        world.spawn(Camera2d);
        Ok(())
    }

    fn capture(
        world: &mut World,
        dir: &std::path::Path,
        stem: &str,
    ) -> Result<Option<Self>, String> {
        if world.contains_resource::<SaveFails>() {
            return Err("the save failed".into());
        }
        let count = world.get_resource::<Count>().map_or(0, |count| count.0);
        let file = dir.join(format!("{stem}.setup.txt"));
        fs::write(&file, count.to_string()).map_err(|error| error.to_string())?;
        Ok(Some(Self::Saved(file.display().to_string())))
    }
}

#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(AppView)]
enum SessionView {
    Overview,
}

impl AppView for SessionView {
    fn arrange(&self, _world: &mut World) {}

    fn current(_world: &World) -> Option<Self> {
        Some(Self::Overview)
    }
}

#[derive(Resource)]
struct Lattice;

#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(AppTarget)]
enum SessionTarget {
    Cell(i32, i32),
}

impl AppTarget for SessionTarget {
    fn resolve(&self, _world: &World) -> Result<ResolvedTarget, String> {
        let Self::Cell(column, row) = self;
        let centre =
            |cell: &i32| f32::mul_add(f32::from(i16::try_from(*cell).unwrap_or(0)), 100.0, 50.0);
        Ok(ResolvedTarget::at(Vec2::new(centre(column), centre(row))))
    }

    fn at(world: &World, position: Vec2) -> Option<Self> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a cell of the test window is a small whole number"
        )]
        let cell = |at: f32| (at / 100.0).floor() as i32;
        world
            .contains_resource::<Lattice>()
            .then(|| Self::Cell(cell(position.x), cell(position.y)))
    }
}

fn dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    drop(fs::remove_dir_all(&dir));
    dir
}

fn app(dir: &std::path::Path) -> App {
    let mut app = App::new();
    app.add_plugins((
        HeadlessPlugins::default().with_window_size(640, 480),
        SessionPlugin::new(dir),
    ));
    spawn_ui(app.world_mut());
    app.world_mut().spawn(Camera2d);
    app.finish();
    app.cleanup();
    for _ in 0..3 {
        app.update();
    }
    app
}

fn primary(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .expect("a primary window")
}

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

fn centre_of(app: &mut App, text: &str) -> Vec2 {
    let world = app.world_mut();
    let label = world
        .query::<(Entity, &Text)>()
        .iter(world)
        .find(|(_, shown)| shown.0 == text)
        .expect("a node shows the text")
        .0;
    let button = world.get::<ChildOf>(label).expect("a button").parent();
    let node = world.get::<ComputedNode>(button).expect("laid out");
    let transform = world.get::<UiGlobalTransform>(button).expect("positioned");
    transform.affine().translation * node.inverse_scale_factor
}

fn real_glide(app: &mut App, from: Vec2, to: Vec2) {
    for frame in 1..=12_u8 {
        let t = f32::from(frame) / 12.0;
        let bend = Vec2::new(0.0, (t * core::f32::consts::PI).sin() * 30.0);
        real_move(app, from.lerp(to, t) + bend);
    }
}

fn use_the_app(app: &mut App) {
    let button = centre_of(app, "+1");
    real_glide(app, Vec2::new(10.0, 10.0), button);
    for _ in 0..3 {
        real_click(app);
        for _ in 0..25 {
            app.update();
        }
    }
}

fn play(path: &std::path::Path) -> Result<(bevy_stagecraft::ScenarioReport, u32), String> {
    let text = fs::read_to_string(path).expect("the file was written");
    let script = Script::from_ron(&text).expect("the file reads back");
    if script.setup.is_none() {
        return Err(format!("the script has no setup: {text}"));
    }
    let registry = Registry::default();
    let scenario = Scenario::from_script(script, &registry).map_err(|error| error.to_string())?;
    let mut app = scenario.headless_app();
    let report = scenario.run_in(&mut app);
    let count = app.world().resource::<Count>().0;
    Ok((report, count))
}

#[test]
fn a_recorded_session_plays_back_to_the_same_end_state() {
    let dir = dir("session-roundtrip");
    let mut app = app(&dir);
    session::start(app.world_mut(), "Three clicks").expect("recording starts");
    assert!(app.world().resource::<SessionRecorder>().is_recording());
    use_the_app(&mut app);
    let saved = session::stop(app.world_mut()).expect("the session is saved");
    assert_eq!(
        app.world().resource::<Count>().0,
        3,
        "the app saw the clicks"
    );

    let (report, count) = play(&saved.path).expect("the file plays");
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(count, 3, "playback reaches the same end state");
}

#[test]
fn the_recorded_file_reads_as_a_few_labelled_steps() {
    let dir = dir("session-readable");
    let mut app = app(&dir);
    session::start(app.world_mut(), "Three clicks").expect("recording starts");
    use_the_app(&mut app);
    let saved = session::stop(app.world_mut()).expect("the session is saved");
    let script = saved.script;

    assert_eq!(script.name, "Three clicks");
    assert_eq!(script.window_size, Some((640, 480)));
    assert_eq!(
        script.setup,
        Some(Setup::app(SessionSetup::Saved(
            dir.join("three_clicks.setup.txt").display().to_string()
        )))
    );
    assert_eq!(script.view, Some(View::app(SessionView::Overview)));
    assert!(
        script
            .expect
            .contains(&Check::resource::<Count>(".0").eq(3_u32)),
        "the change to the count is suggested: {:?}",
        script.expect
    );
    let clicks = script.steps.iter().filter(|s| **s == Step::click()).count();
    assert_eq!(clicks, 3, "{:?}", script.steps);
    assert!(
        script.steps.iter().any(|step| matches!(
            step,
            Step::Glide { target, .. } if *target == Target::role(WidgetRole::Button, "+1")
        )),
        "the path ends on the button by its role and label: {:?}",
        script.steps
    );
    let glides = script
        .steps
        .iter()
        .filter(|step| matches!(step, Step::Glide { .. }))
        .count();
    assert!((1..=6).contains(&glides), "{glides} glides");

    let text = fs::read_to_string(&saved.path).expect("written");
    assert!(text.starts_with("//"), "the file explains itself");
    assert!(
        text.contains("role: Button") && text.contains("label: \"+1\""),
        "{text}"
    );
    assert!(
        text.contains("path: \"Count.0\",\n            is: Eq(3),"),
        "{text}"
    );
}

#[test]
fn a_session_is_the_same_whatever_frame_rate_it_ran_at() {
    let steps_for = |extra_updates: usize| {
        let dir = dir(&format!("session-rate-{extra_updates}"));
        let mut app = app(&dir);
        session::start(app.world_mut(), "rate").expect("starts");
        let button = centre_of(&mut app, "+1");
        real_glide(&mut app, Vec2::new(10.0, 10.0), button);
        real_click(&mut app);
        for _ in 0..(20 + extra_updates) {
            app.update();
        }
        real_click(&mut app);
        session::stop(app.world_mut()).expect("saved").script.steps
    };
    let quick = steps_for(0);
    let slow = steps_for(20);
    let wait = |steps: &[Step]| {
        steps
            .iter()
            .filter_map(|step| match step {
                Step::Wait(frames) => Some(u32::from(*frames)),
                _ => None,
            })
            .sum::<u32>()
    };
    assert!(
        wait(&slow) >= wait(&quick) + 15,
        "pauses are counted in frames of 1/60 s: {quick:?} vs {slow:?}"
    );
}

#[test]
fn recording_nothing_in_particular_writes_a_script_that_plays() {
    let dir = dir("session-empty");
    let mut app = app(&dir);
    session::start(app.world_mut(), "empty").expect("starts");
    for _ in 0..5 {
        app.update();
    }
    let saved = session::stop(app.world_mut()).expect("saved");
    assert!(saved.script.expect.is_empty(), "nothing changed");
    assert!(saved.candidates.is_empty(), "nothing changed");
    let (report, _) = play(&saved.path).expect("the file plays");
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_second_recording_does_not_overwrite_the_first() {
    let dir = dir("session-twice");
    let mut app = app(&dir);
    let mut paths = Vec::new();
    for _ in 0..2 {
        session::start(app.world_mut(), "again").expect("starts");
        paths.push(session::stop(app.world_mut()).expect("saved").path);
    }
    assert_ne!(paths[0], paths[1]);
    assert!(paths.iter().all(|path| path.exists()));
}

#[test]
fn a_session_does_not_reuse_a_stem_an_orphaned_sidecar_holds() {
    let dir = dir("session-orphan-sidecar");
    fs::create_dir_all(&dir).expect("directory");
    let sidecar = dir.join("again.scene.ron");
    fs::write(&sidecar, "kept").expect("sidecar");
    let mut app = app(&dir);
    session::start(app.world_mut(), "again").expect("starts");
    let saved = session::stop(app.world_mut()).expect("saved");
    assert_eq!(saved.path, dir.join("again-2.ron"));
    assert_eq!(fs::read_to_string(&sidecar).expect("read"), "kept");
}

#[test]
fn starting_twice_or_stopping_nothing_is_an_error_not_a_panic() {
    let dir = dir("session-errors");
    let mut app = app(&dir);
    assert!(session::stop(app.world_mut()).is_err(), "nothing to stop");
    session::start(app.world_mut(), "one").expect("starts");
    assert!(session::start(app.world_mut(), "two").is_err());
}

#[test]
fn a_failing_setup_capture_stops_the_recording_from_starting() {
    let dir = dir("session-bad-capture");
    let mut app = App::new();
    app.add_plugins((HeadlessPlugins::default(), SessionPlugin::new(&dir)))
        .insert_resource(SaveFails);
    app.finish();
    app.cleanup();
    let error = session::start(app.world_mut(), "x").expect_err("the capture failed");
    assert!(error.to_string().contains("the save failed"), "{error}");
    assert!(!app.world().resource::<SessionRecorder>().is_recording());
}

#[test]
fn requests_start_and_stop_at_the_next_update_for_a_command_line_flag() {
    let dir = dir("session-requests");
    let mut app = app(&dir);
    app.world_mut()
        .resource_mut::<SessionRecorder>()
        .request_start("flag");
    app.update();
    assert!(app.world().resource::<SessionRecorder>().is_recording());
    app.world_mut()
        .resource_mut::<SessionRecorder>()
        .request_stop();
    app.update();
    let recorder = app.world().resource::<SessionRecorder>();
    assert!(!recorder.is_recording());
    let saved = recorder
        .last()
        .and_then(|last| last.as_ref().ok())
        .expect("the session was saved");
    assert!(saved.path.exists());
    assert!(app.world().contains_resource::<SessionConfig>());
}

#[test]
fn a_click_that_landed_on_nothing_labelled_stays_a_screen_position() {
    let dir = dir("session-screen");
    let mut app = app(&dir);
    session::start(app.world_mut(), "ground").expect("starts");
    real_move(&mut app, Vec2::new(500.0, 400.0));
    real_move(&mut app, Vec2::new(500.0, 400.0));
    real_click(&mut app);
    let saved = session::stop(app.world_mut()).expect("saved");
    assert!(
        saved
            .script
            .steps
            .iter()
            .any(|step| matches!(step, Step::MoveTo(Target::Screen(at)) if *at == Vec2::new(500.0, 400.0))),
        "{:?}",
        saved.script.steps
    );
}

#[test]
fn the_apps_own_target_is_offered_clicks_that_hit_no_label() {
    let dir = dir("session-app-target");
    let mut app = App::new();
    app.add_plugins((HeadlessPlugins::default(), SessionPlugin::new(&dir)))
        .insert_resource(Lattice);
    app.world_mut().spawn(Camera2d);
    app.finish();
    app.cleanup();
    app.update();
    session::start(app.world_mut(), "lattice").expect("starts");
    real_move(&mut app, Vec2::new(250.0, 130.0));
    real_move(&mut app, Vec2::new(250.0, 130.0));
    real_click(&mut app);
    let saved = session::stop(app.world_mut()).expect("saved");
    assert!(
        saved
            .script
            .steps
            .contains(&Step::MoveTo(Target::app(SessionTarget::Cell(2, 1)))),
        "{:?}",
        saved.script.steps
    );
}

#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
struct Tagged;

fn recorded_target(app: &mut App, name: &str, position: Vec2) -> Target {
    session::start(app.world_mut(), name).expect("starts");
    real_move(app, position);
    real_move(app, position);
    real_click(app);
    let saved = session::stop(app.world_mut()).expect("saved");
    saved
        .script
        .steps
        .iter()
        .find_map(|step| match step {
            Step::MoveTo(target) | Step::Glide { target, .. } => Some(target.clone()),
            _ => None,
        })
        .expect("the pointer was moved to the click")
}

#[test]
fn a_recorded_click_names_a_test_id_then_role_and_label_then_text_then_the_position() {
    let dir = dir("session-sturdiest");
    let mut app = app(&dir);
    app.world_mut().spawn((
        Button,
        Tagged,
        Node {
            position_type: PositionType::Absolute,
            left: px(100),
            top: px(250),
            width: px(120),
            height: px(60),
            ..Node::default()
        },
        children![Text::new("Tagged")],
    ));
    app.world_mut().spawn((
        Text::new("Note"),
        Node {
            position_type: PositionType::Absolute,
            left: px(400),
            top: px(250),
            ..Node::default()
        },
    ));
    for _ in 0..2 {
        app.update();
    }
    let tagged = centre_of(&mut app, "Tagged");
    assert_eq!(
        recorded_target(&mut app, "tagged", tagged),
        Target::component::<Tagged>(),
        "a test id first"
    );
    let plus = centre_of(&mut app, "+1");
    assert_eq!(
        recorded_target(&mut app, "role", plus),
        Target::role(WidgetRole::Button, "+1"),
        "then role and label"
    );
    let note = {
        let world = app.world_mut();
        let (node, transform) = world
            .query::<(&Text, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(text, _, _)| text.0 == "Note")
            .map(|(_, node, transform)| (*node, *transform))
            .expect("the note is laid out");
        transform.affine().translation * node.inverse_scale_factor
    };
    assert_eq!(
        recorded_target(&mut app, "text", note),
        Target::text("Note"),
        "then text"
    );
    let ground = Vec2::new(500.0, 420.0);
    assert_eq!(
        recorded_target(&mut app, "position", ground),
        Target::screen(ground),
        "then the position"
    );
}

fn real_key(app: &mut App, key_code: KeyCode, logical_key: Key, state: ButtonState) {
    let window = primary(app);
    let text = match (&logical_key, state) {
        (Key::Character(text), ButtonState::Pressed) => Some(text.clone()),
        _ => None,
    };
    let input = KeyboardInput {
        key_code,
        logical_key,
        state,
        text,
        repeat: false,
        window,
    };
    app.world_mut()
        .write_message(WindowEvent::from(input.clone()));
    app.world_mut().write_message(input);
    app.update();
}

fn type_hi(app: &mut App) {
    let character = |text: &str| Key::Character(text.into());
    real_key(app, KeyCode::ShiftLeft, Key::Shift, ButtonState::Pressed);
    real_key(app, KeyCode::KeyH, character("H"), ButtonState::Pressed);
    real_key(app, KeyCode::KeyH, character("H"), ButtonState::Released);
    real_key(app, KeyCode::ShiftLeft, Key::Shift, ButtonState::Released);
    for (code, text) in [(KeyCode::KeyI, "i"), (KeyCode::Digit5, "5")] {
        for _ in 0..3 {
            app.update();
        }
        real_key(app, code, character(text), ButtonState::Pressed);
        real_key(app, code, character(text), ButtonState::Released);
    }
}

#[test]
fn a_recorded_typing_session_exports_a_single_type_step() {
    let dir = dir("session-typing");
    let mut app = app(&dir);
    session::start(app.world_mut(), "Typing").expect("recording starts");
    type_hi(&mut app);
    let saved = session::stop(app.world_mut()).expect("the session is saved");
    let typed: Vec<_> = saved
        .script
        .steps
        .iter()
        .filter(|step| matches!(step, Step::Type { .. }))
        .collect();
    assert_eq!(typed, [&Step::type_text("Hi5")], "{:?}", saved.script.steps);
    assert!(
        saved.script.steps.iter().all(|step| !matches!(
            step,
            Step::Key { .. } | Step::KeyUp { .. } | Step::Tap { .. }
        )),
        "{:?}",
        saved.script.steps
    );
    let text = fs::read_to_string(&saved.path).expect("written");
    assert!(text.contains("text: \"Hi5\","), "{text}");
}

fn real_touch(app: &mut App, phase: TouchPhase, position: Vec2) {
    let window = primary(app);
    let input = TouchInput {
        phase,
        position,
        window,
        force: None,
        id: 7,
    };
    app.world_mut().write_message(WindowEvent::from(input));
    app.world_mut().write_message(input);
    app.update();
}

#[test]
fn a_recorded_real_tap_is_a_touch_tap_that_plays_back() {
    let dir = dir("session-touch");
    let mut app = app(&dir);
    session::start(app.world_mut(), "Touch").expect("recording starts");
    let button = centre_of(&mut app, "+1");
    real_touch(&mut app, TouchPhase::Started, button);
    real_touch(&mut app, TouchPhase::Ended, button);
    for _ in 0..5 {
        app.update();
    }
    let saved = session::stop(app.world_mut()).expect("the session is saved");
    assert!(
        saved
            .script
            .steps
            .iter()
            .any(|step| matches!(step, Step::TouchTap(Target::Role { .. }))),
        "{:?}",
        saved.script.steps
    );
    let (report, count) = play(&saved.path).expect("the file plays");
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(count, 1, "playback taps the button once");
}
