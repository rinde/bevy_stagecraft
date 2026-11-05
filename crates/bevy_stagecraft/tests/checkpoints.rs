//! Checkpoints while recording a session, on a plain Bevy UI: a "+1" button
//! and a label showing the count.
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
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        mouse::MouseButtonInput,
    },
    prelude::*,
    ui::{ComputedNode, UiGlobalTransform, px},
    ui_widgets::{Activate, Button},
    window::{CursorMoved, PrimaryWindow, WindowEvent},
};
use bevy_stagecraft::{
    Check, CheckKind, CursorOverlayPlugin, HeadlessPlugins, Registry, SavedSession, Scenario,
    ScenarioReport, Script, SessionHooks, SessionPlugin, Step, flatten,
    overlay::{
        CHECKPOINT_CAPTION_FRAMES, CHECKPOINT_FLASH_FRAMES, OverlayCaptionStrip,
        OverlayCheckpointFlash, OverlayCheckpointText, naming_text,
    },
    session,
};

/// What the "+1" button counts.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Counter(u32);

/// The label that shows the count.
#[derive(Component)]
struct CountLabel;

/// Every key the app saw pressed, through each way an app can read keys.
#[derive(Resource, Default, Debug)]
struct SeenKeys {
    /// From `KeyboardInput` messages.
    messages: Vec<KeyCode>,
    /// From `WindowEvent` messages.
    window_events: Vec<KeyCode>,
    /// From `ButtonInput<KeyCode>`.
    just_pressed: Vec<KeyCode>,
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
        .observe(|_: On<Activate>, mut counter: ResMut<Counter>| counter.0 += 1);
    world.spawn((
        CountLabel,
        Text::new("Count: 0"),
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
        let wanted = format!("Count: {}", counter.0);
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
}

/// Notes every key press the app can see.
fn watch_keys(
    mut seen: ResMut<SeenKeys>,
    mut keys: MessageReader<KeyboardInput>,
    mut window_events: MessageReader<WindowEvent>,
    input: Res<ButtonInput<KeyCode>>,
) {
    for key in keys.read() {
        if key.state == ButtonState::Pressed {
            seen.messages.push(key.key_code);
        }
    }
    for event in window_events.read() {
        if let WindowEvent::KeyboardInput(key) = event
            && key.state == ButtonState::Pressed
        {
            seen.window_events.push(key.key_code);
        }
    }
    seen.just_pressed.extend(input.get_just_pressed().copied());
}

/// A headless app with the counter, before its UI is spawned.
fn bare_app() -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default().with_window_size(640, 480))
        .add_systems(Update, show_count);
    app
}

/// The app with a session plugin writing into `dir`, its UI up and laid out.
fn recording_app(dir: &std::path::Path, hooks: SessionHooks, overlay: bool) -> App {
    let mut app = bare_app();
    app.add_plugins(SessionPlugin::new(dir).with_hooks(hooks))
        .init_resource::<SeenKeys>()
        .add_systems(Update, watch_keys);
    if overlay {
        app.add_plugins(CursorOverlayPlugin);
    }
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

/// What `bevy_winit` writes for a real key changing state, without updating.
fn write_key(app: &mut App, key_code: KeyCode, logical_key: Key, state: ButtonState) {
    let window = primary(app);
    let input = KeyboardInput {
        key_code,
        logical_key,
        state,
        text: None,
        repeat: false,
        window,
    };
    app.world_mut()
        .write_message(WindowEvent::from(input.clone()));
    app.world_mut().write_message(input);
}

/// A real key pressed and released, one update apart.
fn real_tap(app: &mut App, key_code: KeyCode, logical_key: &Key) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        write_key(app, key_code, logical_key.clone(), state);
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

/// Clicks "+1" and lets the app settle.
fn click_and_settle(app: &mut App) {
    real_click(app);
    for _ in 0..10 {
        app.update();
    }
}

/// Records three clicks on "+1" with a checkpoint (F8) after each of the
/// first two, and returns what was saved and the app.
fn record_with_two_checkpoints(name: &str) -> (SavedSession, App) {
    let dir = dir(name);
    let mut app = recording_app(&dir, SessionHooks::new(), false);
    session::start(app.world_mut(), "two checkpoints").expect("recording starts");
    let centre = button_centre(&mut app);
    real_move(&mut app, centre);
    real_move(&mut app, centre);
    for _ in 0..2 {
        click_and_settle(&mut app);
        real_tap(&mut app, KeyCode::F8, &Key::F8);
        app.update();
    }
    click_and_settle(&mut app);
    assert_eq!(app.world().resource::<Counter>().0, 3, "the clicks counted");
    let saved = session::stop(app.world_mut()).expect("the session is saved");
    (saved, app)
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

/// Where `wanted` is among `steps`.
fn position_of(steps: &[Step], wanted: &Step) -> usize {
    steps
        .iter()
        .position(|step| step == wanted)
        .unwrap_or_else(|| unreachable!("{wanted:?} is among {steps:#?}"))
}

#[test]
fn a_session_with_two_checkpoints_yields_check_steps_at_both_points_then_expect() {
    let (saved, _) = record_with_two_checkpoints("checkpoints-two");
    let steps = &saved.script.steps;
    assert_eq!(saved.checkpoints.len(), 2, "{steps:#?}");

    let first = Step::check(Check::resource::<Counter>(".0").eq(1_u32));
    let second = Step::check(Check::resource::<Counter>(".0").eq(2_u32));
    let clicks: Vec<usize> = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| **step == Step::click())
        .map(|(index, _)| index)
        .collect();
    assert_eq!(clicks.len(), 3, "{steps:#?}");
    let (first, second) = (position_of(steps, &first), position_of(steps, &second));
    assert!(
        clicks[0] < first && first < clicks[1],
        "the first checkpoint's checks follow the first click: {steps:#?}"
    );
    assert!(
        clicks[1] < second && second < clicks[2],
        "the second checkpoint's checks follow the second click: {steps:#?}"
    );
    assert!(
        steps.contains(&Step::check(Check::shows("Count: 1")))
            && steps.contains(&Step::check(!Check::shows("Count: 0"))),
        "{steps:#?}"
    );

    // The end is compared with the last checkpoint, not with the start.
    let expect = &saved.script.expect;
    assert!(
        expect.contains(&Check::resource::<Counter>(".0").eq(3_u32)),
        "{expect:#?}"
    );
    assert!(expect.contains(&!Check::shows("Count: 2")), "{expect:#?}");
    assert!(!expect.contains(&!Check::shows("Count: 0")), "{expect:#?}");

    // Every candidate starts ticked: the file has them all where they go.
    let check_steps: Vec<&Check> = steps
        .iter()
        .filter_map(|step| match step {
            Step::Check(check) => Some(check),
            _ => None,
        })
        .collect();
    let candidates: Vec<&Check> = saved
        .checkpoints
        .iter()
        .flat_map(|checkpoint| &checkpoint.candidates)
        .collect();
    assert_eq!(check_steps, candidates);
    assert_eq!(saved.script.expect, saved.candidates);
}

#[test]
fn the_checkpoint_key_is_not_seen_by_the_app_and_is_no_step() {
    let (saved, app) = record_with_two_checkpoints("checkpoints-key");
    let seen = app.world().resource::<SeenKeys>();
    assert!(!seen.messages.contains(&KeyCode::F8), "{seen:?}");
    assert!(!seen.window_events.contains(&KeyCode::F8), "{seen:?}");
    assert!(!seen.just_pressed.contains(&KeyCode::F8), "{seen:?}");
    assert!(
        !saved.script.steps.iter().any(|step| matches!(
            step,
            Step::Tap {
                key: KeyCode::F8,
                ..
            } | Step::Key {
                key: KeyCode::F8,
                ..
            } | Step::KeyUp {
                key: KeyCode::F8,
                ..
            }
        )),
        "{:#?}",
        saved.script.steps
    );
}

#[test]
fn other_keys_pressed_with_the_checkpoint_key_reach_the_app_once() {
    let dir = dir("checkpoints-other-keys");
    let mut app = recording_app(&dir, SessionHooks::new(), false);
    session::start(app.world_mut(), "keys").expect("recording starts");
    // A key the update before, then another key with the checkpoint key.
    write_key(
        &mut app,
        KeyCode::KeyA,
        Key::Character("a".into()),
        ButtonState::Pressed,
    );
    app.update();
    write_key(
        &mut app,
        KeyCode::KeyB,
        Key::Character("b".into()),
        ButtonState::Pressed,
    );
    write_key(&mut app, KeyCode::F8, Key::F8, ButtonState::Pressed);
    app.update();
    app.update();
    let seen = app.world().resource::<SeenKeys>();
    assert_eq!(seen.messages, [KeyCode::KeyA, KeyCode::KeyB], "{seen:?}");
    assert_eq!(
        seen.window_events,
        [KeyCode::KeyA, KeyCode::KeyB],
        "{seen:?}"
    );
    assert_eq!(
        seen.just_pressed,
        [KeyCode::KeyA, KeyCode::KeyB],
        "{seen:?}"
    );
    let saved = session::stop(app.world_mut()).expect("saved");
    assert_eq!(saved.checkpoints.len(), 1);
}

#[test]
fn the_exported_scenario_with_checkpoints_passes_when_played_back() {
    let (saved, _) = record_with_two_checkpoints("checkpoints-playback");
    let report = play(&saved.path);
    assert!(report.passed(), "{:#?}", report.failures());
    let at_checkpoints: usize = saved
        .checkpoints
        .iter()
        .map(|checkpoint| checkpoint.candidates.len())
        .sum();
    let ran = |kind: CheckKind| {
        report
            .checks
            .iter()
            .filter(|check| check.kind == kind)
            .count()
    };
    assert_eq!(ran(CheckKind::Step), at_checkpoints, "every step check ran");
    assert_eq!(ran(CheckKind::Expect), saved.candidates.len());
}

#[test]
fn unticked_checkpoint_candidates_leave_the_file_and_the_rest_still_pass() {
    let (mut saved, _) = record_with_two_checkpoints("checkpoints-ticked");
    let kept = vec![vec![Check::resource::<Counter>(".0").eq(1_u32)], Vec::new()];
    let expect = vec![Check::shows("Count: 3")];
    saved
        .save_checks(&kept, expect.clone())
        .expect("written again");
    let text = fs::read_to_string(&saved.path).expect("the file");
    let script = Script::from_ron(&text).expect("it reads back");
    let checks: Vec<&Step> = script
        .steps
        .iter()
        .filter(|step| matches!(step, Step::Check(_)))
        .collect();
    assert_eq!(checks, [&Step::check(kept[0][0].clone())], "{text}");
    assert_eq!(script.expect, expect);
    assert_eq!(
        script
            .steps
            .iter()
            .filter(|step| **step == Step::click())
            .count(),
        3,
        "the recorded steps stay: {text}"
    );
    let report = play(&saved.path);
    assert!(report.passed(), "{:#?}", report.failures());
    assert_eq!(report.checks.len(), 2);

    // Writing the expect alone keeps the checkpoints' choice.
    saved.save_expect(Vec::new()).expect("written again");
    let script = Script::from_ron(&fs::read_to_string(&saved.path).expect("the file"))
        .expect("it reads back");
    assert!(script.steps.contains(&Step::check(kept[0][0].clone())));
}

#[test]
fn the_checkpoint_key_can_be_changed_and_f8_is_then_left_to_the_app() {
    let dir = dir("checkpoints-other-key");
    let hooks = SessionHooks::new().checkpoint_key(KeyCode::F2);
    assert_eq!(hooks.checkpoint(), KeyCode::F2);
    let mut app = recording_app(&dir, hooks, false);
    session::start(app.world_mut(), "f2").expect("recording starts");
    real_tap(&mut app, KeyCode::F8, &Key::F8);
    real_tap(&mut app, KeyCode::F2, &Key::F2);
    app.update();
    let seen = app.world().resource::<SeenKeys>();
    assert_eq!(seen.messages, [KeyCode::F8], "{seen:?}");
    let saved = session::stop(app.world_mut()).expect("saved");
    assert_eq!(saved.checkpoints.len(), 1);
    assert!(
        saved.script.steps.contains(&Step::tap(KeyCode::F8)),
        "F8 is an ordinary key now: {:#?}",
        saved.script.steps
    );
}

#[test]
fn the_checkpoint_key_reaches_the_app_when_nothing_is_recorded() {
    let dir = dir("checkpoints-not-recording");
    let mut app = recording_app(&dir, SessionHooks::new(), false);
    real_tap(&mut app, KeyCode::F8, &Key::F8);
    app.update();
    assert_eq!(app.world().resource::<SeenKeys>().messages, [KeyCode::F8]);
}

#[test]
fn a_checkpoint_taken_from_code_lands_where_the_key_would() {
    let dir = dir("checkpoints-code");
    let mut app = recording_app(&dir, SessionHooks::new(), false);
    assert!(
        session::checkpoint(app.world_mut()).is_err(),
        "not recording"
    );
    session::start(app.world_mut(), "code").expect("recording starts");
    let centre = button_centre(&mut app);
    real_move(&mut app, centre);
    click_and_settle(&mut app);
    let taken = session::checkpoint(app.world_mut()).expect("taken");
    assert_eq!(taken.number, 1);
    assert!(taken.candidates > 0);
    click_and_settle(&mut app);
    let saved = session::stop(app.world_mut()).expect("saved");
    assert_eq!(saved.checkpoints.len(), 1);
    assert_eq!(saved.checkpoints[0].candidates.len(), taken.candidates);
    let report = play(&saved.path);
    assert!(report.passed(), "{:#?}", report.failures());
}

#[test]
fn the_overlay_flashes_and_names_a_checkpoint_then_fades() {
    let dir = dir("checkpoints-overlay");
    let mut app = recording_app(&dir, SessionHooks::new().checkpoint_names(false), true);
    session::start(app.world_mut(), "overlay").expect("recording starts");
    let centre = button_centre(&mut app);
    real_move(&mut app, centre);
    click_and_settle(&mut app);

    let flash_shown = |app: &mut App| {
        let world = app.world_mut();
        let mut flash = world.query_filtered::<&Visibility, With<OverlayCheckpointFlash>>();
        let visibility = flash.single(world).expect("one flash");
        *visibility != Visibility::Hidden
    };
    let caption = |app: &mut App| {
        let world = app.world_mut();
        let mut line = world.query_filtered::<&Text, With<OverlayCheckpointText>>();
        line.single(world).expect("one line").0.clone()
    };
    let strip_shown = |app: &mut App| {
        let world = app.world_mut();
        let mut strip = world.query_filtered::<&Node, With<OverlayCaptionStrip>>();
        strip.single(world).expect("one strip").display != Display::None
    };
    assert!(!flash_shown(&mut app));
    assert!(caption(&mut app).is_empty());
    assert!(!strip_shown(&mut app), "nothing to say yet");

    write_key(&mut app, KeyCode::F8, Key::F8, ButtonState::Pressed);
    app.update();
    assert!(flash_shown(&mut app), "the window flashes");
    let text = caption(&mut app);
    assert!(text.starts_with("Checkpoint 1: "), "{text}");
    assert!(text.ends_with("checks suggested"), "{text}");
    assert!(strip_shown(&mut app));

    for _ in 0..CHECKPOINT_FLASH_FRAMES {
        app.update();
    }
    assert!(!flash_shown(&mut app), "the flash is short");
    assert!(!caption(&mut app).is_empty(), "the caption stays longer");
    for _ in CHECKPOINT_FLASH_FRAMES..CHECKPOINT_CAPTION_FRAMES {
        app.update();
    }
    assert!(caption(&mut app).is_empty(), "and then goes");
    assert!(!strip_shown(&mut app));
}

/// What `bevy_winit` writes for a key that types `text`, without updating.
fn write_typed(app: &mut App, key_code: KeyCode, text: &str, state: ButtonState) {
    let window = primary(app);
    let input = KeyboardInput {
        key_code,
        logical_key: Key::Character(text.into()),
        state,
        text: (state == ButtonState::Pressed).then(|| text.into()),
        repeat: false,
        window,
    };
    app.world_mut()
        .write_message(WindowEvent::from(input.clone()));
    app.world_mut().write_message(input);
}

/// Types `text` letter by letter, one update for each press and release.
fn type_text(app: &mut App, text: &str) {
    for letter in text.chars() {
        let letter = letter.to_string();
        for state in [ButtonState::Pressed, ButtonState::Released] {
            write_typed(app, KeyCode::KeyN, &letter, state);
            app.update();
        }
    }
}

/// The steps of the first top-level group called `name`.
fn group_steps<'a>(steps: &'a [Step], name: &str) -> Option<&'a [Step]> {
    steps.iter().find_map(|step| match step {
        Step::Group { name: found, steps } if found == name => Some(steps.as_slice()),
        _ => None,
    })
}

/// Records a click on "+1" and a checkpoint after it, then types `typed`
/// (keeping it with Enter when `finish` is that key), another click, and the
/// stop. Returns what was saved and the app.
fn record_named_checkpoint(
    name: &str,
    typed: &str,
    finish: Option<KeyCode>,
) -> (SavedSession, App) {
    let dir = dir(name);
    let mut app = recording_app(&dir, SessionHooks::new(), false);
    session::start(app.world_mut(), "named").expect("recording starts");
    let centre = button_centre(&mut app);
    real_move(&mut app, centre);
    real_move(&mut app, centre);
    click_and_settle(&mut app);
    real_tap(&mut app, KeyCode::F8, &Key::F8);
    type_text(&mut app, typed);
    if let Some(key) = finish {
        real_tap(&mut app, key, &Key::Enter);
    }
    app.update();
    click_and_settle(&mut app);
    let saved = session::stop(app.world_mut()).expect("the session is saved");
    (saved, app)
}

#[test]
fn a_named_checkpoint_groups_the_steps_since_the_one_before_with_its_checks() {
    let (saved, _) = record_named_checkpoint("checkpoints-named", "first", Some(KeyCode::Enter));
    let steps = &saved.script.steps;
    let group = group_steps(steps, "first").unwrap_or_else(|| unreachable!("{steps:#?}"));
    assert!(group.contains(&Step::click()), "{steps:#?}");
    assert!(
        group.contains(&Step::check(Check::resource::<Counter>(".0").eq(1_u32))),
        "the checkpoint's own checks are in it: {steps:#?}"
    );
    assert!(
        matches!(steps.first(), Some(Step::Group { .. })),
        "everything before the checkpoint is in it: {steps:#?}"
    );
    let after: Vec<&Step> = steps.iter().skip(1).collect();
    assert!(
        after.contains(&&Step::click()),
        "the click after it is no part of it: {steps:#?}"
    );
    assert!(
        after.iter().all(|step| !matches!(step, Step::Group { .. })),
        "{steps:#?}"
    );
}

#[test]
fn a_named_checkpoint_plays_back_and_its_failures_show_the_group() {
    let (saved, _) =
        record_named_checkpoint("checkpoints-named-play", "first", Some(KeyCode::Enter));
    let report = play(&saved.path);
    assert!(report.passed(), "{:#?}", report.failures());
    assert!(
        report
            .steps
            .iter()
            .any(|step| step.group == ["first"] && step.step == Step::click()),
        "{:#?}",
        report.steps
    );
    assert!(
        report.steps.iter().any(|step| step.group.is_empty()),
        "the steps after it are in no group"
    );

    // A checkpoint's check that does not hold names the group it is in.
    let mut script = saved.script;
    if let Some(Step::Group { steps, .. }) = script.steps.first_mut() {
        steps.push(Step::check(Check::shows("Count: 99")));
    }
    let scenario = Scenario::from_script(script, &Registry::default())
        .expect("it loads")
        .setup(spawn_ui);
    let mut app = bare_app();
    app.finish();
    app.cleanup();
    let report = scenario.run_in(&mut app);
    assert!(
        report
            .failures()
            .iter()
            .any(|line| line.starts_with("first > step ")),
        "{:#?}",
        report.failures()
    );
}

#[test]
fn escape_skips_the_name_and_the_steps_stay_ungrouped() {
    let (saved, _) =
        record_named_checkpoint("checkpoints-skipped", "ignored", Some(KeyCode::Escape));
    assert!(
        !saved
            .script
            .steps
            .iter()
            .any(|step| matches!(step, Step::Group { .. })),
        "{:#?}",
        saved.script.steps
    );
    assert!(
        saved
            .script
            .steps
            .contains(&Step::check(Check::resource::<Counter>(".0").eq(1_u32))),
        "the checkpoint's checks are still there"
    );
}

#[test]
fn a_click_keeps_the_name_typed_so_far() {
    let (saved, _) = record_named_checkpoint("checkpoints-click-names", "typed", None);
    let steps = &saved.script.steps;
    assert!(group_steps(steps, "typed").is_some(), "{steps:#?}");
}

#[test]
fn stopping_keeps_the_name_typed_so_far() {
    let dir = dir("checkpoints-stop-names");
    let mut app = recording_app(&dir, SessionHooks::new(), false);
    session::start(app.world_mut(), "stop").expect("recording starts");
    let centre = button_centre(&mut app);
    real_move(&mut app, centre);
    click_and_settle(&mut app);
    real_tap(&mut app, KeyCode::F8, &Key::F8);
    type_text(&mut app, "the end");
    let saved = session::stop(app.world_mut()).expect("saved");
    assert!(
        group_steps(&saved.script.steps, "the end").is_some(),
        "{:#?}",
        saved.script.steps
    );
}

#[test]
fn backspace_edits_the_name_and_blanks_are_no_name() {
    let dir = dir("checkpoints-backspace");
    let mut app = recording_app(&dir, SessionHooks::new(), false);
    session::start(app.world_mut(), "backspace").expect("recording starts");
    let centre = button_centre(&mut app);
    real_move(&mut app, centre);
    click_and_settle(&mut app);
    real_tap(&mut app, KeyCode::F8, &Key::F8);
    type_text(&mut app, "abx");
    real_tap(&mut app, KeyCode::Backspace, &Key::Backspace);
    type_text(&mut app, "c");
    real_tap(&mut app, KeyCode::Enter, &Key::Enter);
    click_and_settle(&mut app);
    real_tap(&mut app, KeyCode::F8, &Key::F8);
    type_text(&mut app, "  ");
    real_tap(&mut app, KeyCode::Enter, &Key::Enter);
    let saved = session::stop(app.world_mut()).expect("saved");
    let steps = &saved.script.steps;
    assert!(group_steps(steps, "abc").is_some(), "{steps:#?}");
    assert_eq!(
        steps
            .iter()
            .filter(|step| matches!(step, Step::Group { .. }))
            .count(),
        1,
        "a blank name is no group: {steps:#?}"
    );
}

#[test]
fn keys_typed_into_the_name_never_reach_the_app_but_later_ones_do() {
    let dir = dir("checkpoints-typed-keys");
    let mut app = recording_app(&dir, SessionHooks::new(), false);
    session::start(app.world_mut(), "keys").expect("recording starts");
    real_tap(&mut app, KeyCode::F8, &Key::F8);
    type_text(&mut app, "name");
    real_tap(&mut app, KeyCode::Enter, &Key::Enter);
    app.update();
    {
        let seen = app.world().resource::<SeenKeys>();
        assert!(seen.messages.is_empty(), "{seen:?}");
        assert!(seen.window_events.is_empty(), "{seen:?}");
        assert!(seen.just_pressed.is_empty(), "{seen:?}");
    }
    real_tap(&mut app, KeyCode::KeyA, &Key::Character("a".into()));
    app.update();
    let seen = app.world().resource::<SeenKeys>();
    assert_eq!(seen.messages, [KeyCode::KeyA], "{seen:?}");
    assert_eq!(seen.just_pressed, [KeyCode::KeyA], "{seen:?}");
    let saved = session::stop(app.world_mut()).expect("saved");
    let keys: Vec<&Step> = flatten(&saved.script.steps)
        .into_iter()
        .map(|flat| flat.step)
        .filter(|step| !matches!(step, Step::Wait(_)))
        .collect();
    assert_eq!(
        keys,
        [&Step::tap(KeyCode::KeyA)],
        "only the key the app saw is recorded: {:#?}",
        saved.script.steps
    );
}

#[test]
fn a_session_that_does_not_ask_for_names_leaves_every_key_to_the_app() {
    let dir = dir("checkpoints-no-names");
    let hooks = SessionHooks::new().checkpoint_names(false);
    assert!(!hooks.asks_names());
    let mut app = recording_app(&dir, hooks, false);
    session::start(app.world_mut(), "no names").expect("recording starts");
    real_tap(&mut app, KeyCode::F8, &Key::F8);
    type_text(&mut app, "n");
    real_tap(&mut app, KeyCode::Enter, &Key::Enter);
    let seen = app.world().resource::<SeenKeys>();
    assert_eq!(seen.messages, [KeyCode::KeyN, KeyCode::Enter], "{seen:?}");
    let saved = session::stop(app.world_mut()).expect("saved");
    assert_eq!(saved.checkpoints.len(), 1);
}

#[test]
fn a_checkpoint_named_from_code_opens_no_prompt_and_groups_the_steps() {
    let dir = dir("checkpoints-named-code");
    let mut app = recording_app(&dir, SessionHooks::new(), false);
    assert!(
        session::checkpoint_named(app.world_mut(), "x").is_err(),
        "not recording"
    );
    session::start(app.world_mut(), "named code").expect("recording starts");
    let centre = button_centre(&mut app);
    real_move(&mut app, centre);
    click_and_settle(&mut app);
    let taken = session::checkpoint_named(app.world_mut(), "one click").expect("taken");
    assert_eq!(taken.number, 1);
    real_tap(&mut app, KeyCode::KeyA, &Key::Character("a".into()));
    app.update();
    let seen = app.world().resource::<SeenKeys>();
    assert_eq!(seen.messages, [KeyCode::KeyA], "{seen:?}");
    let saved = session::stop(app.world_mut()).expect("saved");
    let steps = &saved.script.steps;
    let group = group_steps(steps, "one click").unwrap_or_else(|| unreachable!("{steps:#?}"));
    assert!(group.contains(&Step::click()), "{steps:#?}");
    assert!(steps.contains(&Step::tap(KeyCode::KeyA)), "{steps:#?}");
    let report = play(&saved.path);
    assert!(report.passed(), "{:#?}", report.failures());
}

#[test]
fn saving_the_checks_again_keeps_the_groups() {
    let (mut saved, _) =
        record_named_checkpoint("checkpoints-named-save", "first", Some(KeyCode::Enter));
    saved
        .save_checks(&[Vec::new()], Vec::new())
        .expect("written again");
    let text = fs::read_to_string(&saved.path).expect("the file");
    let script = Script::from_ron(&text).expect("it reads back");
    let group = group_steps(&script.steps, "first").unwrap_or_else(|| unreachable!("{text}"));
    assert!(group.contains(&Step::click()), "{text}");
    assert!(
        group.iter().all(|step| !matches!(step, Step::Check(_))),
        "the unticked checks are gone from inside it: {text}"
    );
    let report = play(&saved.path);
    assert!(report.passed(), "{:#?}", report.failures());
}

/// How many of `steps` are clicks.
fn clicks_in(steps: &[Step]) -> usize {
    steps.iter().filter(|step| **step == Step::click()).count()
}

#[test]
fn two_named_checkpoints_each_group_only_the_steps_since_the_one_before() {
    let dir = dir("checkpoints-named-two");
    let mut app = recording_app(&dir, SessionHooks::new(), false);
    session::start(app.world_mut(), "two names").expect("recording starts");
    let centre = button_centre(&mut app);
    real_move(&mut app, centre);
    real_move(&mut app, centre);
    click_and_settle(&mut app);
    session::checkpoint_named(app.world_mut(), "first click").expect("taken");
    click_and_settle(&mut app);
    session::checkpoint_named(app.world_mut(), "second click").expect("taken");
    click_and_settle(&mut app);
    let saved = session::stop(app.world_mut()).expect("saved");
    let steps = &saved.script.steps;

    let first = group_steps(steps, "first click").unwrap_or_else(|| unreachable!("{steps:#?}"));
    let second = group_steps(steps, "second click").unwrap_or_else(|| unreachable!("{steps:#?}"));
    assert_eq!(clicks_in(first), 1, "{steps:#?}");
    assert_eq!(clicks_in(second), 1, "only its own click: {steps:#?}");
    let one = Step::check(Check::resource::<Counter>(".0").eq(1_u32));
    let two = Step::check(Check::resource::<Counter>(".0").eq(2_u32));
    assert!(first.contains(&one), "{steps:#?}");
    assert!(!first.contains(&two), "{steps:#?}");
    assert!(second.contains(&two), "{steps:#?}");
    assert!(!second.contains(&one), "{steps:#?}");

    let groups = steps
        .iter()
        .filter(|step| matches!(step, Step::Group { .. }))
        .count();
    assert_eq!(groups, 2, "{steps:#?}");
    let loose: Vec<&Step> = steps
        .iter()
        .filter(|step| !matches!(step, Step::Group { .. }))
        .collect();
    assert_eq!(
        loose.iter().filter(|step| ***step == Step::click()).count(),
        1,
        "the click after the last checkpoint is outside both: {steps:#?}"
    );
    let report = play(&saved.path);
    assert!(report.passed(), "{:#?}", report.failures());
}

#[test]
fn a_named_checkpoint_straight_after_another_is_a_group_of_only_its_checks() {
    let dir = dir("checkpoints-named-empty");
    let mut app = recording_app(&dir, SessionHooks::new(), false);
    session::start(app.world_mut(), "empty segment").expect("recording starts");
    let centre = button_centre(&mut app);
    real_move(&mut app, centre);
    real_move(&mut app, centre);
    click_and_settle(&mut app);
    session::checkpoint_named(app.world_mut(), "before").expect("taken");
    session::checkpoint_named(app.world_mut(), "again").expect("taken");
    let mut saved = session::stop(app.world_mut()).expect("saved");
    let kept = Check::resource::<Counter>(".0").eq(1_u32);
    saved
        .save_checks(&[Vec::new(), vec![kept.clone()]], Vec::new())
        .expect("written again");
    let text = fs::read_to_string(&saved.path).expect("the file");
    let script = Script::from_ron(&text).expect("it reads back");

    let again = group_steps(&script.steps, "again").unwrap_or_else(|| unreachable!("{text}"));
    assert_eq!(again, [Step::check(kept)], "only its checks: {text}");
    let before = group_steps(&script.steps, "before").unwrap_or_else(|| unreachable!("{text}"));
    assert_eq!(clicks_in(before), 1, "{text}");
    let report = play(&saved.path);
    assert!(report.passed(), "{:#?}", report.failures());
}

#[test]
fn the_overlay_shows_the_name_as_it_is_typed_then_the_checkpoint_again() {
    let dir = dir("checkpoints-overlay-naming");
    let mut app = recording_app(&dir, SessionHooks::new(), true);
    session::start(app.world_mut(), "overlay").expect("recording starts");
    let caption = |app: &mut App| {
        let world = app.world_mut();
        let mut line = world.query_filtered::<&Text, With<OverlayCheckpointText>>();
        line.single(world).expect("one line").0.clone()
    };
    real_tap(&mut app, KeyCode::F8, &Key::F8);
    assert_eq!(caption(&mut app), naming_text(1, ""));
    type_text(&mut app, "ab");
    assert_eq!(caption(&mut app), naming_text(1, "ab"));
    assert!(
        naming_text(1, "ab").is_ascii(),
        "the bundled font has no more"
    );
    real_tap(&mut app, KeyCode::Enter, &Key::Enter);
    let text = caption(&mut app);
    assert!(text.starts_with("Checkpoint 1: "), "{text}");
}
