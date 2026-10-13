//! The gallery over a plain Bevy UI, with two headless windows.
//!
//! Real input is imitated by writing the messages `bevy_winit` would: the
//! typed ones (`CursorMoved`, `MouseButtonInput`, `KeyboardInput`) and the
//! `WindowEvent` wrapper Bevy's own mouse picking reads. Nothing here opens a
//! real window; see the example for that.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    app::AppExit,
    input::{
        ButtonInput, ButtonState,
        keyboard::{Key, KeyCode, KeyboardInput, NativeKey},
        mouse::{MouseButton, MouseButtonInput, MouseScrollUnit, MouseWheel},
        touch::TouchPhase,
    },
    picking::events::{Over, Pointer},
    picking::input::PointerInputSettings,
    prelude::*,
    ui::InteractionDisabled,
    ui::{
        ComputedNode, ComputedUiTargetCamera, OverflowAxis, ScrollPosition, UiGlobalTransform, px,
    },
    ui_widgets::{Activate, Button},
    window::{CursorMoved, PrimaryWindow, WindowEvent},
};
use rin_bevy_ui_tester::gallery::GalleryPhase;
use rin_bevy_ui_tester::{
    AppSetup, Check, Gallery, GalleryAction, GalleryEntry, GalleryPlugin, GalleryRunReport,
    GalleryState, GalleryTab, HeadlessPlugins, Measure, NoiseFilter, NoisePreset, Outcome, Probe,
    ReflectAppSetup, Registry, Scenario, ScenarioPlayer, ScenarioResult, ScenariosCamera,
    ScenariosWindow, Script, SessionHooks, Speed, Step, Target, TraceWorld, Variant, WidgetRole,
    overlay::{OverlayCheckpointFlash, OverlayPickOutline, OverlayRoot, PickHighlight},
    player::ScenarioReport,
    script::Setup,
};

/// How often the "Add" button was activated.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Count(u32);

/// How often the reset hook ran.
#[derive(Resource, Default)]
struct Resets(u32);

/// Marks what the app under test spawned, so its reset hook can remove it.
#[derive(Component)]
struct AppRoot;

/// How many real mouse button messages the app's own systems have read.
#[derive(Resource, Default)]
struct SeenPresses(u32);

/// The cursor position the app saw during `Update`.
#[derive(Resource, Default)]
struct SeenCursor(Option<Vec2>);

/// Spawns the app: a camera, an "Add" button and a button named like one of
/// the gallery's own.
fn spawn_app(world: &mut World) {
    for (label, left) in [("Add", 100), ("Restart", 300)] {
        world
            .spawn((
                AppRoot,
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
            .observe(|_: On<Activate>, mut count: ResMut<Count>| count.0 += 1);
    }
}

/// Puts the app's state back.
fn reset_app(world: &mut World) {
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, With<AppRoot>>()
        .iter(world)
        .collect();
    for root in roots {
        world.despawn(root);
    }
    world.insert_resource(Count(0));
    world.resource_mut::<Resets>().0 += 1;
}

/// Clicks "Add" the given number of times.
fn adds(times: u32) -> Scenario {
    let mut steps = vec![Step::glide(Target::text("Add"), 10)];
    for _ in 0..times {
        steps.push(Step::click());
        steps.push(Step::wait(2));
    }
    steps.push(Step::check(Check::resource::<Count>(".0").eq(times)));
    Scenario::new(format!("add {times}"))
        .setup(spawn_app)
        .window_size(640, 480)
        .steps(steps)
        .expect_check(Check::resource::<Count>(".0").eq(times))
}

/// Spawns the app with a disabled "Save" button besides.
fn spawn_form(world: &mut World) {
    spawn_app(world);
    world.spawn((
        AppRoot,
        Button,
        InteractionDisabled,
        Node {
            position_type: PositionType::Absolute,
            left: px(100),
            top: px(250),
            width: px(120),
            height: px(60),
            ..Node::default()
        },
        BackgroundColor(Color::WHITE),
        children![Text::new("Save")],
    ));
}

/// Where a recording of this app starts from, found by Bevy with no
/// registration call.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(AppSetup)]
enum GallerySetup {
    /// The app with the count at `count`, and the form if `form`.
    Counted {
        /// The count.
        count: u32,
        /// Whether the disabled "Save" button is there too.
        form: bool,
    },
    /// A save that is gone, so the setup fails.
    Lost,
}

/// Present when this app cannot capture its state.
#[derive(Resource)]
struct CaptureFails;

/// Makes the app's setup capture nothing, so the gallery saves a world in the
/// crate's own format.
#[derive(Resource)]
struct CaptureNothing;

impl AppSetup for GallerySetup {
    fn build(&self, world: &mut World) -> Result<(), String> {
        let Self::Counted { count, form } = self else {
            return Err("the save is gone".into());
        };
        world.spawn(Camera2d);
        world.insert_resource(Count(*count));
        if *form {
            spawn_form(world);
        } else {
            spawn_app(world);
        }
        Ok(())
    }

    fn capture(
        world: &mut World,
        _dir: &std::path::Path,
        _stem: &str,
    ) -> Result<Option<Self>, String> {
        if world.contains_resource::<CaptureFails>() {
            return Err("no way to save this app".into());
        }
        if world.contains_resource::<CaptureNothing>() {
            return Ok(None);
        }
        Ok(Some(Self::Counted {
            count: world.get_resource::<Count>().map_or(0, |count| count.0),
            form: Probe::new(world).has_text("Save"),
        }))
    }
}

/// Shows the form and waits.
fn form() -> Scenario {
    Scenario::new("form")
        .setup(spawn_form)
        .window_size(640, 480)
        .steps([Step::wait(5)])
}

/// Clicks the app's button named "Restart", which the gallery also has one of.
fn restart_the_app_button() -> Scenario {
    Scenario::new("restart button")
        .setup(spawn_app)
        .steps([Step::glide(Target::text("Restart"), 5), Step::click()])
        .expect_check(Check::resource::<Count>(".0").eq(1_u32))
}

/// A catalogue of three scenarios.
fn gallery() -> Gallery {
    Gallery::new()
        .group(
            "Counting",
            [
                GalleryEntry::new(adds(2)).described("Two clicks"),
                GalleryEntry::new(adds(3)),
            ],
        )
        .group("Names", [restart_the_app_button()])
        .group("Forms", [form()])
}

/// The index of the form in [`gallery`].
const FORM: usize = 3;

/// A finished app with the gallery on top of headless windows.
fn app_with(plugin: GalleryPlugin) -> App {
    let mut app = App::new();
    app.add_plugins((HeadlessPlugins::default(), plugin))
        .init_resource::<Count>()
        .init_resource::<Resets>()
        .init_resource::<SeenCursor>()
        // The app's camera lives as long as the app: headless apps have no
        // render world to sync a despawned camera to, so the reset hook must
        // not despawn cameras (a real renderer copes).
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Camera2d);
        })
        .add_systems(
            Update,
            |windows: Query<&Window, With<PrimaryWindow>>, mut seen: ResMut<SeenCursor>| {
                seen.0 = windows.iter().next().and_then(Window::cursor_position);
            },
        );
    app.finish();
    app.cleanup();
    app.update();
    app
}

/// The usual app: loops with a pause of two updates.
fn gallery_app() -> App {
    app_with(
        GalleryPlugin::new(gallery())
            .with_reset(reset_app)
            .with_loop_pause(2),
    )
}

/// Asks the gallery for something and lets one update carry it out.
fn ask(app: &mut App, action: GalleryAction) {
    app.world_mut()
        .resource_mut::<GalleryState>()
        .request(action);
    app.update();
}

/// Updates until `done`, failing after `limit`.
fn run_until(app: &mut App, limit: usize, done: impl Fn(&World) -> bool) {
    for _ in 0..limit {
        if done(app.world()) {
            return;
        }
        app.update();
    }
    assert!(done(app.world()), "gave up after {limit} updates");
}

fn finished(world: &World) -> bool {
    world.resource::<ScenarioReport>().finished
}

fn count(world: &World) -> u32 {
    world.resource::<Count>().0
}

fn frames(world: &World) -> u32 {
    world.resource::<ScenarioReport>().frames
}

fn primary(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .expect("a primary window")
}

fn scenarios_window(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<ScenariosWindow>>()
        .single(app.world())
        .expect("a Scenarios window")
}

/// What `bevy_winit` does for a moved real mouse.
fn real_move(app: &mut App, window: Entity, position: Vec2) {
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
}

/// What `bevy_winit` does for a real click: a press and a release.
fn real_click(app: &mut App, window: Entity) {
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

fn real_key(app: &mut App, window: Entity, key_code: KeyCode) {
    let input = KeyboardInput {
        key_code,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state: ButtonState::Pressed,
        text: None,
        repeat: false,
        window,
    };
    app.world_mut()
        .write_message(WindowEvent::from(input.clone()));
    app.world_mut().write_message(input);
}

/// The middle of the button showing `text` (logical pixels), found by hand
/// because the gallery's own nodes are hidden from target lookups.
fn centre_of_button_with(app: &mut App, text: &str) -> Vec2 {
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

#[test]
fn picking_a_scenario_resets_the_app_and_plays_it_to_a_passing_result() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 400, finished);
    assert_eq!(count(app.world()), 2);
    assert!(app.world().resource::<ScenarioReport>().passed());
    let state = app.world().resource::<GalleryState>();
    assert_eq!(state.result(0), Some(ScenarioResult::Passed));
    assert_eq!(state.result(1), None, "the other entry has not run");
    assert_eq!(app.world().resource::<Resets>().0, 1);
}

#[test]
fn a_failing_scenario_records_a_failing_result() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(1));
    // `adds(3)` clicks three times; pause is irrelevant, it passes. Break it
    // by taking a click away from the app between steps.
    run_until(&mut app, 400, |world| frames(world) > 12);
    app.world_mut().insert_resource(Count(100));
    run_until(&mut app, 400, finished);
    let state = app.world().resource::<GalleryState>();
    assert_eq!(state.result(1), Some(ScenarioResult::Failed));
}

#[test]
fn the_window_takes_the_scenarios_size() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    let window = primary(&mut app);
    let size = app
        .world()
        .get::<Window>(window)
        .expect("window")
        .resolution
        .size();
    assert_eq!(size, Vec2::new(640.0, 480.0));
}

#[test]
fn pause_holds_the_script_and_step_frame_runs_exactly_one_frame() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 5);
    ask(&mut app, GalleryAction::TogglePlay);
    let held = frames(app.world());
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(frames(app.world()), held, "paused");
    assert!(!app.world().resource::<GalleryState>().is_playing());
    ask(&mut app, GalleryAction::StepFrame);
    assert_eq!(frames(app.world()), held + 1, "one frame");
    app.update();
    assert_eq!(frames(app.world()), held + 1, "and then paused again");
    ask(&mut app, GalleryAction::TogglePlay);
    run_until(&mut app, 400, finished);
    assert_eq!(count(app.world()), 2, "playing on from where it stopped");
}

#[test]
fn step_to_next_step_runs_to_a_step_boundary_and_pauses() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 3);
    ask(&mut app, GalleryAction::StepStep);
    for _ in 0..30 {
        app.update();
    }
    // The first step is a ten-frame glide, so the stop is at the end of it,
    // before the click.
    assert_eq!(frames(app.world()), 10, "the glide is over");
    assert_eq!(count(app.world()), 0, "the click has not happened");
    ask(&mut app, GalleryAction::StepStep);
    for _ in 0..30 {
        app.update();
    }
    assert_eq!(
        frames(app.world()),
        11,
        "the click is one step of one frame"
    );
    assert_eq!(count(app.world()), 1);
}

#[test]
fn slower_speeds_play_fewer_frames_per_update_and_faster_ones_more_without_stale_checks() {
    let frames_after = |speed: Speed, updates: usize| {
        let mut app = gallery_app();
        ask(&mut app, GalleryAction::ToggleLoop);
        ask(&mut app, GalleryAction::SetSpeed(speed));
        ask(&mut app, GalleryAction::Select(0));
        // Past the settling updates, then a fixed number of playing ones.
        for _ in 0..(updates + 3) {
            app.update();
        }
        frames(app.world())
    };
    let quarter = frames_after(Speed::Quarter, 7);
    let half = frames_after(Speed::Half, 7);
    let normal = frames_after(Speed::Normal, 7);
    let double = frames_after(Speed::Double, 7);
    assert!(
        quarter < half && half < normal && normal < double,
        "{quarter} {half} {normal} {double}"
    );
    assert!(double <= normal * 2);

    let mut app = gallery_app();
    ask(&mut app, GalleryAction::SetSpeed(Speed::Double));
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 400, finished);
    assert!(
        app.world().resource::<ScenarioReport>().passed(),
        "{:?}",
        app.world().resource::<ScenarioReport>().failures()
    );
}

#[test]
fn a_looping_gallery_resets_and_plays_again_after_a_pause() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 400, finished);
    assert_eq!(app.world().resource::<Resets>().0, 1);
    run_until(&mut app, 100, |world| world.resource::<Resets>().0 == 2);
    assert_eq!(
        count(app.world()),
        0,
        "the second run starts from a fresh app"
    );
    run_until(&mut app, 400, |world| finished(world) && count(world) == 2);
}

#[test]
fn without_looping_a_finished_scenario_stays_finished_until_asked_again() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::ToggleLoop);
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 400, finished);
    for _ in 0..30 {
        app.update();
    }
    assert_eq!(app.world().resource::<Resets>().0, 1, "no second run");
    assert!(app.world().resource::<GalleryState>().is_finished());
    ask(&mut app, GalleryAction::TogglePlay);
    assert_eq!(app.world().resource::<Resets>().0, 2, "play runs it again");
}

#[test]
fn restart_resets_the_app_and_starts_over() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 400, |world| count(world) == 1);
    ask(&mut app, GalleryAction::Restart);
    assert_eq!(count(app.world()), 0);
    assert_eq!(app.world().resource::<Resets>().0, 2);
    run_until(&mut app, 400, finished);
    assert_eq!(count(app.world()), 2);
}

#[test]
fn moving_and_clicking_the_real_mouse_over_the_app_changes_nothing_during_playback() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    // Play to just before the glide ends, so the script cursor is moving.
    run_until(&mut app, 100, |world| frames(world) >= 4);
    assert!(
        !app.world()
            .resource::<PointerInputSettings>()
            .is_mouse_enabled,
        "Bevy's own mouse picking is off while the script plays"
    );
    let window = primary(&mut app);
    let script_cursor = app
        .world()
        .get::<Window>(window)
        .and_then(Window::cursor_position);
    let _ = script_cursor;
    // The real mouse goes to "Add" and clicks.
    let add = centre_of_button_with(&mut app, "Add");
    real_move(&mut app, window, add);
    real_click(&mut app, window);
    real_key(&mut app, window, KeyCode::KeyA);
    app.update();
    assert_eq!(
        count(app.world()),
        0,
        "the real click did not reach the app"
    );
    assert!(
        !app.world()
            .resource::<ButtonInput<KeyCode>>()
            .pressed(KeyCode::KeyA)
    );
    assert!(
        !app.world()
            .resource::<ButtonInput<MouseButton>>()
            .pressed(MouseButton::Left),
    );
    // What the app saw in Update is the script's cursor, not the real one.
    let seen = app.world().resource::<SeenCursor>().0.expect("a cursor");
    assert!(
        seen.distance(add) > 1.0 || frames(app.world()) > 10,
        "the script's, not the mouse's: {seen} vs {add}"
    );
    // And the script still passes.
    run_until(&mut app, 400, finished);
    assert!(
        app.world().resource::<ScenarioReport>().passed(),
        "{:?}",
        app.world().resource::<ScenarioReport>().failures()
    );
    assert_eq!(count(app.world()), 2);
}

#[test]
fn the_real_cursor_position_is_put_back_after_the_script_cursor_was_shown() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 6);
    let window = primary(&mut app);
    real_move(&mut app, window, Vec2::new(500.0, 400.0));
    app.update();
    // After the whole update the window holds the real position again, which
    // is what keeps `bevy_winit` from moving the operating system's pointer.
    let after = app
        .world()
        .get::<Window>(window)
        .and_then(Window::cursor_position);
    assert_eq!(after, Some(Vec2::new(500.0, 400.0)));
    let seen = app.world().resource::<SeenCursor>().0.expect("a cursor");
    assert!(seen.distance(Vec2::new(500.0, 400.0)) > 1.0, "{seen}");
}

#[test]
fn take_over_pauses_the_script_and_gives_the_app_back_to_the_real_mouse() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);
    ask(&mut app, GalleryAction::TakeOver);
    assert!(app.world().resource::<GalleryState>().is_taken_over());
    app.update();
    assert!(
        app.world()
            .resource::<PointerInputSettings>()
            .is_mouse_enabled
    );
    let held = frames(app.world());
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(frames(app.world()), held, "the script is paused");
    // Let layout settle, then use the real mouse on the real button.
    let window = primary(&mut app);
    let add = centre_of_button_with(&mut app, "Add");
    real_move(&mut app, window, add);
    app.update();
    app.update();
    real_click(&mut app, window);
    app.update();
    assert_eq!(count(app.world()), 1, "the real click reached the app");
    let seen = app.world().resource::<SeenCursor>().0.expect("a cursor");
    assert!(seen.distance(add) < 1.0, "the real cursor is back: {seen}");

    ask(&mut app, GalleryAction::Resume);
    app.update();
    assert!(
        !app.world()
            .resource::<PointerInputSettings>()
            .is_mouse_enabled
    );
    assert!(app.world().resource::<GalleryState>().is_playing());
    run_until(&mut app, 400, |world| frames(world) > held);
}

#[test]
fn taking_over_releases_what_the_script_was_holding() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 12);
    app.world_mut()
        .resource_mut::<rin_bevy_ui_tester::SyntheticInput>()
        .press_button(MouseButton::Left);
    app.update();
    assert!(
        app.world()
            .resource::<ButtonInput<MouseButton>>()
            .pressed(MouseButton::Left)
    );
    ask(&mut app, GalleryAction::TakeOver);
    app.update();
    assert!(
        !app.world()
            .resource::<ButtonInput<MouseButton>>()
            .pressed(MouseButton::Left)
    );
}

#[test]
fn a_script_clicking_a_label_the_gallery_also_uses_finds_only_the_apps() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(2));
    run_until(&mut app, 400, finished);
    let report = app.world().resource::<ScenarioReport>();
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(count(app.world()), 1);
}

#[test]
fn the_scenarios_window_has_its_own_camera_and_the_overlay_stays_with_the_app() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 6);
    let world = app.world_mut();
    let scenarios_camera = world
        .query_filtered::<Entity, With<ScenariosCamera>>()
        .single(world)
        .expect("one Scenarios camera");
    let app_camera = world
        .query_filtered::<Entity, (With<Camera2d>, Without<ScenariosCamera>)>()
        .single(world)
        .expect("the app's camera");
    let overlay: Vec<_> = world
        .query_filtered::<&ComputedUiTargetCamera, With<OverlayRoot>>()
        .iter(world)
        .map(ComputedUiTargetCamera::get)
        .collect();
    assert!(!overlay.is_empty());
    assert!(
        overlay.iter().all(|camera| *camera == Some(app_camera)),
        "the overlay is drawn by the app's camera, not the Scenarios one"
    );
    let on_scenarios_camera = world
        .query::<&ComputedUiTargetCamera>()
        .iter(world)
        .filter(|camera| camera.get() == Some(scenarios_camera))
        .count();
    assert!(
        on_scenarios_camera > 10,
        "the Scenarios UI is on its own camera"
    );
}

#[test]
fn the_scenarios_window_is_a_second_window_with_a_catalogue_of_results() {
    let mut app = gallery_app();
    let primary = primary(&mut app);
    let second = scenarios_window(&mut app);
    assert_ne!(primary, second);
    let title = &app.world().get::<Window>(second).expect("window").title;
    assert_eq!(title, "Scenarios");
    for name in [
        "add 2",
        "add 3",
        "restart button",
        "Counting",
        "Names",
        "Two clicks",
    ] {
        let shown = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .any(|text| text.0 == name);
        assert!(shown, "the window lists {name}");
    }
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 400, finished);
    app.update();
    app.update();
    let marks: Vec<String> = app
        .world_mut()
        .query::<&Text>()
        .iter(app.world())
        .map(|text| text.0.clone())
        .collect();
    assert!(marks.iter().any(|text| text == "PASS"), "{marks:?}");
    assert!(
        marks.iter().any(|text| text == "PASS `Count.0` is 2"),
        "the live list shows each check with its mark: {marks:?}"
    );
}

#[test]
fn real_clicks_on_the_scenarios_windows_buttons_work_during_playback() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);
    app.update();
    app.update();
    let second = scenarios_window(&mut app);
    let pause = centre_of_button_with(&mut app, "Pause");
    real_move(&mut app, second, pause);
    app.update();
    app.update();
    real_click(&mut app, second);
    app.update();
    assert!(
        !app.world().resource::<GalleryState>().is_playing(),
        "the Pause button was clicked with the real mouse"
    );
    assert!(
        !app.world()
            .resource::<ButtonInput<MouseButton>>()
            .pressed(MouseButton::Left),
        "and the app saw no mouse button"
    );
}

/// The catalogue's scroll offset down the list, in logical pixels.
fn catalogue_scroll(app: &mut App) -> f32 {
    app.world_mut()
        .query::<&ScrollPosition>()
        .iter(app.world())
        .map(|position| position.0.y)
        .fold(0.0, f32::max)
}

/// What `bevy_winit` does for a real wheel turned one line down.
fn real_wheel_down(app: &mut App, window: Entity) {
    let turn = MouseWheel {
        unit: MouseScrollUnit::Line,
        x: 0.0,
        y: -1.0,
        phase: TouchPhase::Moved,
        window,
    };
    app.world_mut().write_message(WindowEvent::from(turn));
    app.world_mut().write_message(turn);
    app.update();
}

#[test]
fn the_wheel_scrolls_the_catalogue_before_any_scenario_is_selected() {
    let many = (0..40).map(|index| {
        GalleryEntry::new(Scenario::new(format!("scenario {index}")).setup(spawn_app))
    });
    let mut app =
        app_with(GalleryPlugin::new(Gallery::new().group("Many", many)).with_reset(reset_app));
    app.update();
    assert!(
        !app.world().resource::<GalleryState>().input_is_scripted(),
        "nothing is selected yet"
    );
    let second = scenarios_window(&mut app);
    real_wheel_down(&mut app, second);
    app.update();
    assert!(
        catalogue_scroll(&mut app) > 0.0,
        "the list moved down with the wheel"
    );
}

#[test]
fn closing_the_scenarios_window_ends_the_app() {
    let mut app = gallery_app();
    assert!(app.should_exit().is_none(), "running until a window closes");
    let second = scenarios_window(&mut app);
    app.world_mut().despawn(second);
    app.update();
    assert!(
        app.should_exit().is_some(),
        "the app exits with the gallery"
    );
}

#[test]
fn closing_the_app_window_ends_the_app() {
    let mut app = gallery_app();
    let first = primary(&mut app);
    app.world_mut().despawn(first);
    app.update();
    assert!(app.should_exit().is_some(), "the app exits with its window");
}

#[test]
fn space_and_arrow_keys_in_the_scenarios_window_drive_the_gallery_but_never_reach_the_app() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);
    let second = scenarios_window(&mut app);
    real_key(&mut app, second, KeyCode::Space);
    app.update();
    assert!(
        !app.world().resource::<GalleryState>().is_playing(),
        "Space pauses"
    );
    assert!(
        !app.world()
            .resource::<ButtonInput<KeyCode>>()
            .pressed(KeyCode::Space)
    );
    let held = frames(app.world());
    real_key(&mut app, second, KeyCode::ArrowRight);
    app.update();
    assert_eq!(frames(app.world()), held + 1, "Right steps a frame");
    real_key(&mut app, second, KeyCode::Space);
    app.update();
    assert!(
        app.world().resource::<GalleryState>().is_playing(),
        "Space plays"
    );

    // The same key in the primary window goes nowhere while the script plays.
    let first = primary(&mut app);
    real_key(&mut app, first, KeyCode::Space);
    app.update();
    assert!(app.world().resource::<GalleryState>().is_playing());
}

#[test]
fn export_writes_the_selected_scenario_as_ron_that_loads_again() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("gallery-export-test");
    drop(std::fs::remove_dir_all(&dir));
    let mut app = app_with(
        GalleryPlugin::new(gallery())
            .with_reset(reset_app)
            .with_export_dir(&dir),
    );
    ask(&mut app, GalleryAction::Export);
    assert!(
        app.world()
            .resource::<GalleryState>()
            .status()
            .contains("Select"),
        "nothing to export before a pick"
    );
    ask(&mut app, GalleryAction::Select(0));
    ask(&mut app, GalleryAction::Export);
    let path = dir.join("add_2.ron");
    let text = std::fs::read_to_string(&path).expect("the file was written");
    let registry = Registry::default();
    let loaded = Scenario::from_ron_with(&text, &registry).expect("the file loads again");
    assert_eq!(loaded.name(), "add 2");
    assert_eq!(loaded.script(), adds(2).script(), "{text}");
    assert!(
        app.world()
            .resource::<GalleryState>()
            .status()
            .contains("add_2.ron"),
        "the status says where it went"
    );
}

/// Counts the real mouse button messages the app reads, press and release.
fn count_presses(mut reader: MessageReader<MouseButtonInput>, mut seen: ResMut<SeenPresses>) {
    seen.0 += u32::try_from(reader.read().count()).unwrap_or(u32::MAX);
}

#[test]
fn a_real_click_reaches_the_app_once_and_is_not_replayed_every_update() {
    let mut app = gallery_app();
    app.init_resource::<SeenPresses>()
        .add_systems(Update, count_presses);
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);
    ask(&mut app, GalleryAction::TakeOver);
    app.update();
    let window = primary(&mut app);
    let add = centre_of_button_with(&mut app, "Add");
    real_move(&mut app, window, add);
    app.update();
    real_click(&mut app, window);
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(
        app.world().resource::<SeenPresses>().0,
        2,
        "one press and one release, each seen once"
    );
}

/// The gallery, recording into `dir`; the app's setup is captured as a
/// [`GallerySetup`].
fn recording_app(dir: &std::path::Path) -> App {
    app_with(
        GalleryPlugin::new(gallery())
            .with_reset(reset_app)
            .with_export_dir(dir),
    )
}

/// A fresh directory for one test.
fn fresh_dir(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    drop(std::fs::remove_dir_all(&dir));
    dir
}

#[test]
fn a_session_recorded_in_the_gallery_plays_back_to_the_same_end_state() {
    let dir = fresh_dir("gallery-record");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);
    ask(&mut app, GalleryAction::Record);
    {
        let state = app.world().resource::<GalleryState>();
        assert!(state.is_recording());
        assert!(state.is_taken_over(), "the script lets go of the window");
    }
    app.update();
    app.update();

    // Use the app by hand: three clicks on "Add".
    let window = primary(&mut app);
    let add = centre_of_button_with(&mut app, "Add");
    for step in 1..=8_u8 {
        let t = f32::from(step) / 8.0;
        real_move(&mut app, window, Vec2::new(20.0, 20.0).lerp(add, t));
        app.update();
    }
    for _ in 0..3 {
        real_click(&mut app, window);
        for _ in 0..10 {
            app.update();
        }
    }
    assert_eq!(count(app.world()), 3, "the real clicks reached the app");

    // Using the gallery's own window meanwhile is not part of the session.
    let second = scenarios_window(&mut app);
    let restart = centre_of_button_with(&mut app, "Step frame");
    real_move(&mut app, second, restart);
    app.update();
    app.update();
    real_click(&mut app, second);
    app.update();
    assert_eq!(
        app.world().resource::<Resets>().0,
        1,
        "nothing resets the app while recording"
    );
    assert!(
        app.world()
            .resource::<GalleryState>()
            .status()
            .contains("Stop recording first")
    );

    ask(&mut app, GalleryAction::StopRecording);
    let status = app.world().resource::<GalleryState>().status().to_owned();
    assert!(status.starts_with("Saved"), "{status}");
    assert!(!app.world().resource::<GalleryState>().is_recording());

    let path = dir.join("recording.ron");
    let text = std::fs::read_to_string(&path).expect("the file was written");
    let script = Script::from_ron(&text).expect("it reads back");
    let Some(Setup::App(setup)) = &script.setup else {
        panic!("the app's setup was captured: {text}");
    };
    assert!(
        setup
            .ron()
            .is_ok_and(|ron| ron.starts_with("Counted(count:")),
        "{text}"
    );
    assert_eq!(script.window_size, Some((640, 480)));
    let clicks = script.steps.iter().filter(|s| **s == Step::click()).count();
    assert_eq!(clicks, 3, "{text}");
    assert!(
        script.steps.iter().any(|step| matches!(
            step,
            Step::Glide { target, .. } | Step::MoveTo(target)
                if *target == Target::role(WidgetRole::Button, "Add")
        )),
        "{text}"
    );
    assert!(
        !text.contains("Step frame"),
        "nothing of the gallery's window is in the script: {text}"
    );
    assert!(
        script
            .expect
            .contains(&Check::resource::<Count>(".0").eq(3_u32)),
        "{text}"
    );

    // The file runs like any other: load it and play it; its setup is the
    // app's own value.
    let registry = Registry::default();
    let scenario = Scenario::from_script(script, &registry).expect("the file loads");
    let mut played = scenario.headless_app();
    played.insert_resource(Resets::default());
    let report = scenario.run_in(&mut played);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(played.world().resource::<Count>().0, 3);
}

/// The index of the suggested check on `Count.0`, if there is one.
fn count_suggestion(state: &GalleryState) -> Option<usize> {
    state
        .suggestions()?
        .entries()
        .position(|(check, _)| matches!(check, Check::Resource { path, .. } if path == "Count.0"))
}

#[test]
fn stopping_a_recording_lists_the_suggested_checks_and_a_click_unticks_one_in_the_file() {
    let dir = fresh_dir("gallery-record-checklist");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);
    ask(&mut app, GalleryAction::Record);
    app.update();
    let window = primary(&mut app);
    let add = centre_of_button_with(&mut app, "Add");
    real_move(&mut app, window, add);
    app.update();
    app.update();
    real_click(&mut app, window);
    for _ in 0..5 {
        app.update();
    }
    ask(&mut app, GalleryAction::StopRecording);
    app.update();

    let state = app.world().resource::<GalleryState>();
    let suggestions = state.suggestions().expect("a checklist");
    assert!(!suggestions.is_empty());
    assert!(
        suggestions.entries().all(|(_, ticked)| ticked),
        "all start ticked"
    );
    let index = count_suggestion(state).expect("the count's change is suggested");
    let (check, _) = suggestions.entries().nth(index).expect("listed");
    let row = format!("[x] {check}");
    let path = suggestions.session().path.clone();
    let read = |path: &std::path::Path| {
        Script::from_ron(&std::fs::read_to_string(path).expect("written")).expect("reads back")
    };
    assert_eq!(read(&path).expect.len(), suggestions.len());

    // Untick it with the real mouse, in the Scenarios window.
    let second = scenarios_window(&mut app);
    let at = centre_of_button_with(&mut app, &row);
    real_move(&mut app, second, at);
    app.update();
    app.update();
    real_click(&mut app, second);
    app.update();
    let state = app.world().resource::<GalleryState>();
    let suggestions = state.suggestions().expect("still listed");
    assert!(!suggestions.is_ticked(index), "the click unticked it");
    assert!(state.status().starts_with("Saved"), "{}", state.status());
    let script = read(&path);
    assert_eq!(script.expect, suggestions.ticked());
    assert!(
        !script
            .expect
            .iter()
            .any(|check| matches!(check, Check::Resource { path, .. } if path == "Count.0")),
        "{:?}",
        script.expect
    );

    ask(&mut app, GalleryAction::ToggleSuggestion(index));
    assert_eq!(
        read(&path).expect.len(),
        suggestions_len(&app),
        "ticked again"
    );
    ask(&mut app, GalleryAction::Select(0));
    assert!(
        app.world()
            .resource::<GalleryState>()
            .suggestions()
            .is_none(),
        "the checklist goes once something else is picked"
    );
}

#[test]
fn a_checkpoint_while_recording_in_the_gallery_gets_its_own_checklist_and_a_click_unticks_it() {
    let dir = fresh_dir("gallery-record-checkpoint");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);
    ask(&mut app, GalleryAction::Record);
    assert!(
        app.world()
            .resource::<GalleryState>()
            .status()
            .contains("F8 takes a checkpoint"),
        "the status says how to take one"
    );
    app.update();
    let window = primary(&mut app);
    let add = centre_of_button_with(&mut app, "Add");
    real_move(&mut app, window, add);
    app.update();
    app.update();
    real_click(&mut app, window);
    for _ in 0..5 {
        app.update();
    }
    let start = count(app.world());

    real_key(&mut app, window, KeyCode::F8);
    app.update();
    {
        let world = app.world_mut();
        let mut flash = world.query_filtered::<&Visibility, With<OverlayCheckpointFlash>>();
        let shown = flash.single(world).expect("the overlay's flash");
        assert_ne!(*shown, Visibility::Hidden, "the overlay flashes");
    }
    assert!(
        !app.world()
            .resource::<ButtonInput<KeyCode>>()
            .pressed(KeyCode::F8),
        "the app never sees the checkpoint key"
    );
    real_click(&mut app, window);
    for _ in 0..5 {
        app.update();
    }
    ask(&mut app, GalleryAction::StopRecording);
    app.update();

    let state = app.world().resource::<GalleryState>();
    let suggestions = state.suggestions().expect("checklists");
    assert_eq!(suggestions.checkpoints(), 1);
    let at_checkpoint = Check::resource::<Count>(".0").eq(start);
    let index = suggestions
        .checkpoint_entries(0)
        .position(|(check, _)| *check == at_checkpoint)
        .expect("the count at the checkpoint is suggested there");
    assert!(
        suggestions
            .ticked()
            .contains(&Check::resource::<Count>(".0").eq(start + 1)),
        "the end's list is compared with the checkpoint"
    );
    let path = suggestions.session().path.clone();
    let read = |path: &std::path::Path| {
        Script::from_ron(&std::fs::read_to_string(path).expect("written")).expect("reads back")
    };
    let check_step = Step::check(at_checkpoint.clone());
    assert!(read(&path).steps.contains(&check_step));

    // Untick it with the real mouse, in the Scenarios window.
    let second = scenarios_window(&mut app);
    let row = centre_of_button_with(&mut app, &format!("[x] {at_checkpoint}"));
    real_move(&mut app, second, row);
    app.update();
    app.update();
    real_click(&mut app, second);
    app.update();
    let state = app.world().resource::<GalleryState>();
    let suggestions = state.suggestions().expect("still listed");
    assert!(
        !suggestions.is_checkpoint_ticked(0, index),
        "the click unticked it"
    );
    assert!(state.status().starts_with("Saved"), "{}", state.status());
    let script = read(&path);
    assert!(!script.steps.contains(&check_step), "{:#?}", script.steps);
    assert_eq!(script.expect, suggestions.ticked(), "the end's list stays");

    ask(
        &mut app,
        GalleryAction::ToggleCheckpointSuggestion {
            checkpoint: 0,
            index,
        },
    );
    assert!(read(&path).steps.contains(&check_step), "ticked again");
}

#[test]
fn the_wheel_scrolls_long_checklists_while_the_pointer_is_over_them() {
    let dir = fresh_dir("gallery-record-scroll");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::Record);
    for _ in 0..16 {
        rin_bevy_ui_tester::session::checkpoint(app.world_mut()).expect("taken");
        app.update();
    }
    ask(&mut app, GalleryAction::StopRecording);
    app.update();
    app.update();

    let label = {
        let world = app.world_mut();
        world
            .query::<(Entity, &Text)>()
            .iter(world)
            .find(|(_, text)| text.0 == "Checkpoint 1: nothing changed")
            .expect("a line per checkpoint")
            .0
    };
    let panel = app
        .world()
        .get::<ChildOf>(label)
        .expect("in the details panel")
        .parent();
    let at = centre_of_button_with(&mut app, "Checkpoint 2: nothing changed");
    let second = scenarios_window(&mut app);
    real_move(&mut app, second, at);
    app.update();
    app.update();
    real_wheel_down(&mut app, second);
    app.update();
    let scrolled = |app: &App, entity: Entity| {
        app.world()
            .get::<ScrollPosition>(entity)
            .map_or(0.0, |position| position.0.y)
    };
    assert!(scrolled(&app, panel) > 0.0, "the checklists moved");
    let others = {
        let world = app.world_mut();
        world
            .query::<(Entity, &ScrollPosition)>()
            .iter(world)
            .filter(|(entity, _)| *entity != panel)
            .map(|(_, position)| position.0.y)
            .fold(0.0, f32::max)
    };
    assert!(others <= 0.0, "the catalogue stayed put");
}

/// How many checks the gallery's checklist has.
fn suggestions_len(app: &App) -> usize {
    app.world()
        .resource::<GalleryState>()
        .suggestions()
        .map_or(0, rin_bevy_ui_tester::Suggestions::len)
}

#[test]
fn the_gallerys_buttons_are_never_named_as_targets_in_a_recording() {
    let dir = fresh_dir("gallery-record-tooling");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::Record);
    app.update();
    app.update();
    // The app has no UI yet; click where the gallery's button would be if its
    // window were the primary one.
    let window = primary(&mut app);
    real_move(&mut app, window, Vec2::new(30.0, 30.0));
    app.update();
    real_click(&mut app, window);
    ask(&mut app, GalleryAction::StopRecording);
    let text = std::fs::read_to_string(dir.join("recording.ron")).expect("written");
    assert!(!text.contains("Text("), "{text}");
}

#[test]
fn recording_can_be_started_and_stopped_without_the_buttons_and_reports_where_it_went() {
    let dir = fresh_dir("gallery-record-twice");
    let mut app = recording_app(&dir);
    for round in 1..=2 {
        ask(&mut app, GalleryAction::Record);
        ask(&mut app, GalleryAction::StopRecording);
        let status = app.world().resource::<GalleryState>().status().to_owned();
        let name = if round == 1 {
            "recording.ron".to_owned()
        } else {
            "recording-2.ron".to_owned()
        };
        assert!(status.contains(&name), "{status}");
    }
}

#[test]
fn a_failing_capture_leaves_the_gallery_as_it_was() {
    let dir = fresh_dir("gallery-record-fails");
    let mut app = recording_app(&dir);
    app.insert_resource(CaptureFails);
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);
    ask(&mut app, GalleryAction::Record);
    let state = app.world().resource::<GalleryState>();
    assert!(!state.is_recording());
    assert!(state.is_playing(), "the script carries on");
    assert!(state.status().contains("no way to save this app"));
}

#[test]
fn new_from_the_current_state_records_what_is_on_screen_and_keeps_it() {
    let dir = fresh_dir("gallery-new-current");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::Select(1));
    run_until(&mut app, 200, finished);
    assert_eq!(count(app.world()), 3);
    let resets = app.world().resource::<Resets>().0;

    ask(&mut app, GalleryAction::NewFromCurrent);
    let state = app.world().resource::<GalleryState>();
    assert!(state.is_recording());
    assert!(state.is_taken_over(), "the script lets go of the window");
    assert_eq!(app.world().resource::<Resets>().0, resets, "no reset");
    assert_eq!(count(app.world()), 3, "the state is kept");

    ask(&mut app, GalleryAction::StopRecording);
    let text = std::fs::read_to_string(dir.join("recording.ron")).expect("written");
    let script = Script::from_ron(&text).expect("it reads back");
    let Some(Setup::App(setup)) = &script.setup else {
        panic!("the app's setup was captured: {text}");
    };
    assert!(
        setup
            .ron()
            .is_ok_and(|ron| ron.replace(' ', "").starts_with("Counted(count:3")),
        "{text}"
    );
}

#[test]
fn new_from_an_empty_world_resets_the_app_drops_the_selection_and_records_from_nothing() {
    let dir = fresh_dir("gallery-new-empty");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::Select(1));
    run_until(&mut app, 200, finished);
    let resets = app.world().resource::<Resets>().0;

    ask(&mut app, GalleryAction::NewEmpty);
    let state = app.world().resource::<GalleryState>();
    assert!(state.is_recording());
    assert_eq!(state.selected(), None, "no scenario is selected");
    assert_eq!(state.phase(), GalleryPhase::Idle);
    assert_eq!(app.world().resource::<Resets>().0, resets + 1);
    assert_eq!(count(app.world()), 0, "the app's state was put back");
    let left = app
        .world_mut()
        .query_filtered::<Entity, With<AppRoot>>()
        .iter(app.world())
        .count();
    assert_eq!(left, 0, "nothing of the old scenario is left");
    assert!(
        !app.world().resource::<GalleryState>().input_is_scripted(),
        "the real mouse and keyboard have the app window"
    );

    ask(&mut app, GalleryAction::StopRecording);
    let text = std::fs::read_to_string(dir.join("recording.ron")).expect("written");
    let script = Script::from_ron(&text).expect("it reads back");
    let Some(Setup::App(setup)) = &script.setup else {
        panic!("the app's setup was captured: {text}");
    };
    assert!(
        setup.ron().is_ok_and(|ron| ron
            .replace(' ', "")
            .starts_with("Counted(count:0,form:false")),
        "{text}"
    );
}

#[test]
fn new_is_refused_while_recording_and_its_buttons_ask_for_it() {
    let dir = fresh_dir("gallery-new-refused");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::NewEmpty);
    let resets = app.world().resource::<Resets>().0;
    ask(&mut app, GalleryAction::NewEmpty);
    ask(&mut app, GalleryAction::NewFromCurrent);
    assert_eq!(
        app.world().resource::<Resets>().0,
        resets,
        "no second reset"
    );
    assert!(
        app.world()
            .resource::<GalleryState>()
            .status()
            .contains("Stop recording first")
    );
    ask(&mut app, GalleryAction::StopRecording);

    let second = scenarios_window(&mut app);
    let button = centre_of_button_with(&mut app, "New: empty world");
    real_move(&mut app, second, button);
    app.update();
    app.update();
    real_click(&mut app, second);
    app.update();
    assert!(app.world().resource::<GalleryState>().is_recording());
}

/// How many times the pointer went over a node of the app.
#[derive(Resource, Default)]
struct SeenOvers(u32);

/// Counts the pointer going over the app's buttons or their labels.
fn count_overs(
    over: On<Pointer<Over>>,
    roots: Query<(), With<AppRoot>>,
    mut seen: ResMut<SeenOvers>,
) {
    if roots.contains(over.event_target()) {
        seen.0 += 1;
    }
}

/// The entity of the button labelled `text`.
fn button_entity(app: &mut App, text: &str) -> Entity {
    let world = app.world_mut();
    let label = world
        .query::<(Entity, &Text)>()
        .iter(world)
        .find(|(_, shown)| shown.0 == text)
        .expect("a node shows the text")
        .0;
    world.get::<ChildOf>(label).expect("a button").parent()
}

/// Turns pick mode on, points the real mouse at the button labelled `text`
/// in the app window and clicks it.
fn pick_button(app: &mut App, text: &str) {
    ask(app, GalleryAction::TogglePick);
    assert!(
        app.world().resource::<GalleryState>().is_picking(),
        "pick mode is on"
    );
    let window = primary(app);
    let at = centre_of_button_with(app, text);
    real_move(app, window, at);
    app.update();
    app.update();
    real_click(app, window);
    app.update();
}

#[test]
fn picking_a_disabled_button_while_recording_offers_disabled_and_the_recording_plays_back() {
    let dir = fresh_dir("gallery-pick-record");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::Select(FORM));
    run_until(&mut app, 100, |world| frames(world) >= 2);
    ask(&mut app, GalleryAction::Record);
    app.update();

    pick_button(&mut app, "Save");
    let save = button_entity(&mut app, "Save");
    let highlight = app.world().resource::<PickHighlight>().clone();
    assert_eq!(
        highlight.hovered,
        Some(save),
        "the outline is on the button"
    );
    assert_eq!(highlight.selected, Some(save));
    assert_eq!(highlight.tag, "Role(role: Button, label: \"Save\")");
    let save_target = Target::role(WidgetRole::Button, "Save");
    let wanted = Check::disabled(save_target.clone());
    {
        let state = app.world().resource::<GalleryState>();
        let picked = state.picked().expect("the click picked the button");
        assert_eq!(picked.target(), Some(&save_target));
        assert!(picked.checks().contains(&wanted), "{:?}", picked.checks());
    }
    // The outline is the overlay's, so it is never hit.
    let outlines = {
        let world = app.world_mut();
        world
            .query_filtered::<&Pickable, With<OverlayPickOutline>>()
            .iter(world)
            .copied()
            .collect::<Vec<_>>()
    };
    assert_eq!(outlines.len(), 2);
    assert!(
        outlines
            .iter()
            .all(|pickable| *pickable == Pickable::IGNORE)
    );

    // Choose it with the real mouse, in the Scenarios window.
    let second = scenarios_window(&mut app);
    let spelled = ron::to_string(&wanted).expect("the check is written as RON");
    let row = centre_of_button_with(&mut app, &format!("[+] {spelled}"));
    real_move(&mut app, second, row);
    app.update();
    app.update();
    real_click(&mut app, second);
    app.update();
    let status = app.world().resource::<GalleryState>().status().to_owned();
    assert!(status.starts_with("Added"), "{status}");
    ask(&mut app, GalleryAction::TogglePick);
    ask(&mut app, GalleryAction::StopRecording);

    let path = dir.join("recording.ron");
    let text = std::fs::read_to_string(&path).expect("the file was written");
    assert!(text.contains("Check(Disabled(Role("), "{text}");
    let script = Script::from_ron(&text).expect("it reads back");
    assert!(script.steps.contains(&Step::check(wanted)), "{text}");
    assert!(
        !script.steps.iter().any(|step| *step == Step::click()),
        "the pick click is not part of the recording: {text}"
    );
    // The app's setup brings the form back.
    let scenario = Scenario::from_script(script, &Registry::default()).expect("the file loads");
    let mut played = scenario.headless_app();
    played.insert_resource(Resets::default());
    let report = scenario.run_in(&mut played);
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn picking_does_not_change_the_apps_state() {
    let dir = fresh_dir("gallery-pick-unchanged");
    let mut app = recording_app(&dir);
    app.init_resource::<SeenPresses>()
        .init_resource::<SeenOvers>()
        .add_systems(Update, count_presses)
        .add_observer(count_overs);
    ask(&mut app, GalleryAction::Select(FORM));
    run_until(&mut app, 100, |world| frames(world) >= 2);
    ask(&mut app, GalleryAction::Record);
    app.update();
    let before = rin_bevy_ui_tester::WorldSnapshot::take(app.world(), &NoiseFilter::default());

    pick_button(&mut app, "Add");
    for _ in 0..5 {
        app.update();
    }
    assert!(
        app.world().resource::<GalleryState>().picked().is_some(),
        "the click picked the button"
    );
    assert_eq!(count(app.world()), 0, "the click did not reach the app");
    assert_eq!(app.world().resource::<SeenPresses>().0, 0);
    assert_eq!(
        app.world().resource::<SeenOvers>().0,
        0,
        "the pointer never went over the app's nodes"
    );
    assert_eq!(
        app.world().resource::<SeenCursor>().0,
        None,
        "the app saw no cursor in its window"
    );
    let after = rin_bevy_ui_tester::WorldSnapshot::take(app.world(), &NoiseFilter::default());
    assert_eq!(before, after);

    // Escape in the app window ends pick mode, and the app gets its mouse back.
    let window = primary(&mut app);
    real_key(&mut app, window, KeyCode::Escape);
    app.update();
    assert!(!app.world().resource::<GalleryState>().is_picking());
    assert!(
        !app.world()
            .resource::<ButtonInput<KeyCode>>()
            .pressed(KeyCode::Escape),
        "nor did it see the key"
    );
    app.update();
    assert_eq!(
        app.world().resource::<PickHighlight>(),
        &PickHighlight::default()
    );
    let add = centre_of_button_with(&mut app, "Add");
    real_move(&mut app, window, add);
    app.update();
    app.update();
    real_click(&mut app, window);
    app.update();
    assert_eq!(count(app.world()), 1, "clicks reach the app again");
    ask(&mut app, GalleryAction::StopRecording);
}

#[test]
fn a_check_picked_while_paused_is_added_to_the_scenarios_expect_and_runs_at_its_end() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::ToggleLoop);
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);

    pick_button(&mut app, "Add");
    assert!(
        !app.world().resource::<GalleryState>().is_playing(),
        "pick mode pauses the script"
    );
    let wanted = !Check::disabled(Target::role(WidgetRole::Button, "Add"));
    let index = app
        .world()
        .resource::<GalleryState>()
        .picked()
        .expect("picked")
        .checks()
        .iter()
        .position(|check| *check == wanted)
        .expect("an enabled button offers that it is not disabled");
    ask(&mut app, GalleryAction::AddPickedCheck(index));
    let expects = app
        .world()
        .resource::<Gallery>()
        .get(0)
        .expect("the entry")
        .scenario
        .expects();
    assert!(expects.contains(&wanted), "{expects:?}");
    assert!(
        app.world()
            .resource::<GalleryState>()
            .status()
            .contains("expect"),
        "the status says where it went"
    );

    ask(&mut app, GalleryAction::TogglePlay);
    assert!(!app.world().resource::<GalleryState>().is_picking());
    run_until(&mut app, 400, finished);
    let report = app.world().resource::<ScenarioReport>();
    assert!(report.passed(), "{:?}", report.failures());
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.name == wanted.to_string()),
        "the picked check ran at the end: {:?}",
        report.checks
    );
}

#[test]
fn pick_mode_needs_a_recording_or_a_selected_scenario() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::TogglePick);
    let state = app.world().resource::<GalleryState>();
    assert!(!state.is_picking());
    assert!(
        state.status().contains("Select a scenario"),
        "{}",
        state.status()
    );
}

#[test]
fn a_scenario_whose_setup_fails_says_so_and_plays_no_step() {
    let lost = Scenario::new("lost save")
        .app_setup(GallerySetup::Lost)
        .window_size(640, 480)
        .steps([Step::wait(2), Step::check(Check::shows("Add"))]);
    let mut app =
        app_with(GalleryPlugin::new(Gallery::new().group("Broken", [lost])).with_reset(reset_app));
    ask(&mut app, GalleryAction::Select(0));
    let status = app.world().resource::<GalleryState>().status().to_owned();
    assert_eq!(
        status,
        "Setup failed: the setup `Lost` failed: the save is gone"
    );
    run_until(&mut app, 100, |world| {
        world.resource::<ScenarioReport>().finished
    });
    let report = app.world().resource::<ScenarioReport>();
    assert!(!report.passed());
    assert!(report.setup_failure.is_some(), "{report:?}");
    assert!(report.checks.is_empty(), "no step played: {report:?}");
}

/// A scenario that does nothing and fails its expectation.
fn never_satisfied() -> Scenario {
    Scenario::new("never satisfied")
        .setup(spawn_app)
        .window_size(640, 480)
        .expect_check(Check::resource::<Count>(".0").eq(99_u32))
}

/// The exit the app asked for during the last update, if any.
fn exit_asked(app: &App) -> Option<AppExit> {
    app.world()
        .resource::<Messages<AppExit>>()
        .iter_current_update_messages()
        .next()
        .cloned()
}

/// Runs a gallery of these scenarios in run-all mode until it has a report.
fn run_all(scenarios: impl IntoIterator<Item = Scenario>) -> (App, AppExit) {
    let mut app = app_with(
        GalleryPlugin::new(Gallery::new().group("All", scenarios))
            .with_reset(reset_app)
            .with_run_all(),
    );
    for _ in 0..2_000 {
        if let Some(exit) = exit_asked(&app) {
            return (app, exit);
        }
        app.update();
    }
    panic!("the run never ended");
}

#[test]
fn running_everything_plays_every_scenario_once_and_exits_with_success() {
    let (app, exit) = run_all([adds(2), adds(3)]);
    assert_eq!(exit, AppExit::Success);
    let report = app.world().resource::<GalleryRunReport>();
    assert!(report.passed());
    let names: Vec<_> = report.outcomes.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["add 2", "add 3"]);
    assert_eq!(app.world().resource::<Resets>().0, 2, "each ran once");
    assert!(!app.world().resource::<GalleryState>().is_looping());
}

#[test]
fn running_everything_goes_on_after_a_failure_and_reports_every_one() {
    let (app, exit) = run_all([never_satisfied(), adds(2), never_satisfied()]);
    assert_eq!(exit, AppExit::error());
    let report = app.world().resource::<GalleryRunReport>();
    assert_eq!(report.outcomes.len(), 3, "the later scenarios still ran");
    assert!(!report.passed());
    assert_eq!(report.failed().count(), 2);
    let summary = report.summary();
    assert!(summary.starts_with("1 of 3 scenarios passed"), "{summary}");
    assert!(summary.contains("PASS add 2"), "{summary}");
    assert!(summary.contains("FAIL never satisfied"), "{summary}");
    assert!(summary.contains("`Count.0` is 0, expected 99"), "{summary}");
}

#[test]
fn running_everything_counts_a_failed_setup_as_a_failure() {
    let broken = Scenario::new("broken setup")
        .app_setup(GallerySetup::Lost)
        .window_size(640, 480);
    let (app, exit) = run_all([broken, adds(2)]);
    assert_eq!(exit, AppExit::error());
    let report = app.world().resource::<GalleryRunReport>();
    assert_eq!(report.outcomes.len(), 2);
    assert!(report.failed().any(|outcome| {
        outcome
            .failures
            .iter()
            .any(|line| line.contains("the save is gone"))
    }));
}

#[test]
fn the_controls_do_nothing_while_everything_runs() {
    let mut app = app_with(
        GalleryPlugin::new(gallery())
            .with_reset(reset_app)
            .with_run_all(),
    );
    ask(&mut app, GalleryAction::ToggleLoop);
    ask(&mut app, GalleryAction::TogglePlay);
    let state = app.world().resource::<GalleryState>();
    assert!(state.is_running_all());
    assert!(!state.is_looping());
    assert!(state.is_playing(), "pause was refused");
}

/// Plays the jump to `step` of the first scenario and waits until the
/// gallery has paused there.
fn jumped_to(app: &mut App, step: usize) {
    ask(app, GalleryAction::Select(0));
    ask(app, GalleryAction::JumpTo(step));
    run_until(app, 500, |world| {
        let state = world.resource::<GalleryState>();
        state.jumping_to().is_none() && !state.is_playing()
    });
}

#[test]
fn jumping_to_a_step_lands_on_the_frame_a_straight_run_reaches_before_it() {
    let scenario = adds(2);
    let mut headless = scenario.headless_app();
    headless
        .init_resource::<Count>()
        .world_mut()
        .spawn(Camera2d);
    let (report, trace) = scenario.run_in_traced(&mut headless, NoiseFilter::default());
    assert!(report.passed(), "{report:?}");
    // Steps: 0 glide, 1 click, 2 wait, 3 click, 4 wait, 5 check.
    for step in [1, 3, 5] {
        let mut app = gallery_app();
        jumped_to(&mut app, step);
        let point = trace.before(step).expect("a point before the step");
        assert_eq!(frames(app.world()), point.frame, "step {step}");
        let held = app.world().resource::<GalleryState>().status().to_owned();
        assert!(
            held.contains(&format!("Paused before step {}", step + 1)),
            "{held}"
        );
    }
}

#[test]
fn jumping_to_a_step_leaves_the_app_as_the_steps_before_it_made_it() {
    let mut app = gallery_app();
    jumped_to(&mut app, 3);
    assert_eq!(
        count(app.world()),
        1,
        "the first click happened, not the second"
    );
    // The paused app stays put however long it is looked at.
    for _ in 0..30 {
        app.update();
    }
    assert_eq!(count(app.world()), 1);
}

#[test]
fn playing_on_after_a_jump_finishes_the_scenario_from_there() {
    let mut app = gallery_app();
    jumped_to(&mut app, 3);
    ask(&mut app, GalleryAction::TogglePlay);
    run_until(&mut app, 500, finished);
    assert_eq!(count(app.world()), 2);
    assert!(app.world().resource::<ScenarioReport>().passed());
}

#[test]
fn jumping_from_the_middle_of_a_run_restarts_it_first() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 500, finished);
    assert_eq!(count(app.world()), 2);
    ask(&mut app, GalleryAction::JumpTo(1));
    run_until(&mut app, 500, |world| {
        world.resource::<GalleryState>().jumping_to().is_none()
    });
    assert_eq!(
        count(app.world()),
        0,
        "the app was put back before the jump"
    );
}

#[test]
fn a_jump_is_refused_while_recording() {
    let dir = fresh_dir("gallery-jump-recording");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);
    ask(&mut app, GalleryAction::Record);
    ask(&mut app, GalleryAction::JumpTo(1));
    let state = app.world().resource::<GalleryState>();
    assert!(state.is_recording(), "the recording went on");
    assert_eq!(state.jumping_to(), None);
}

/// Every text on screen, in no particular order.
fn texts(app: &mut App) -> Vec<String> {
    app.world_mut()
        .query::<&Text>()
        .iter(app.world())
        .map(|text| text.0.clone())
        .collect()
}

/// Whether some text on screen is exactly `wanted`.
fn shows(app: &mut App, wanted: &str) -> bool {
    texts(app).iter().any(|text| text == wanted)
}

/// Selects the first entry and shows its trace, as the person would.
fn on_the_trace_tab(app: &mut App) {
    ask(app, GalleryAction::Select(0));
    ask(app, GalleryAction::ShowTab(GalleryTab::Trace));
}

#[test]
fn the_trace_tab_says_there_is_nothing_to_trace_before_a_scenario_is_picked() {
    let mut app = gallery_app();
    ask(&mut app, GalleryAction::ShowTab(GalleryTab::Trace));
    assert!(shows(&mut app, "Pick a scenario to trace it"));
}

#[test]
fn the_trace_tab_follows_the_step_the_script_is_at_while_it_plays() {
    let mut app = gallery_app();
    on_the_trace_tab(&mut app);
    ask(&mut app, GalleryAction::ShowTraceWorld(TraceWorld::Before));
    assert!(app.world().resource::<GalleryState>().is_following_trace());
    run_until(&mut app, 500, |world| frames(world) >= 14);
    ask(&mut app, GalleryAction::TogglePlay);
    app.update();
    let player = app.world().resource::<ScenarioPlayer>();
    let step = player
        .steps_started()
        .checked_sub(1)
        .expect("a step has started");
    assert!(step >= 1, "the script is past its first step");
    let frame = player
        .trace()
        .and_then(|trace| trace.before(step))
        .expect("the step has a point before it")
        .frame;
    assert!(
        shows(&mut app, &format!("UI before the step (frame {frame})")),
        "{:?}",
        texts(&mut app)
    );
}

#[test]
fn the_trace_tab_shows_the_first_failing_step_once_the_run_is_over() {
    let failing = Scenario::new("fails in the middle")
        .setup(spawn_app)
        .window_size(640, 480)
        .steps([
            Step::wait(2),
            Step::check(Check::resource::<Count>(".0").eq(99_u32)),
            Step::wait(2),
        ]);
    let mut app = app_with(
        GalleryPlugin::new(Gallery::new().group("All", [failing]))
            .with_reset(reset_app)
            .with_loop_pause(2),
    );
    on_the_trace_tab(&mut app);
    ask(&mut app, GalleryAction::ToggleLoop);
    run_until(&mut app, 500, finished);
    app.update();
    let report = app.world().resource::<ScenarioReport>();
    let why = report
        .steps
        .iter()
        .find_map(|record| match &record.outcome {
            Outcome::Fail(why) => Some(why.clone()),
            Outcome::Pass | Outcome::Skipped => None,
        })
        .expect("a step failed");
    let all = texts(&mut app);
    assert!(all.iter().any(|text| text == &why), "{all:?}");
    assert!(
        all.iter().any(|text| text.starts_with("FAIL 2.")),
        "the failing step is titled with its mark: {all:?}"
    );
}

#[test]
fn the_trace_tab_lists_what_the_step_looked_at_captured() {
    let capturing = Scenario::new("captures a value")
        .setup(spawn_app)
        .window_size(640, 480)
        .steps([
            Step::wait(2),
            Step::capture("clicks", Measure::resource::<Count>(".0")),
            Step::wait(2),
        ]);
    let mut app = app_with(
        GalleryPlugin::new(Gallery::new().group("All", [capturing]))
            .with_reset(reset_app)
            .with_loop_pause(2),
    );
    on_the_trace_tab(&mut app);
    ask(&mut app, GalleryAction::ToggleLoop);
    run_until(&mut app, 500, finished);
    app.update();
    ask(&mut app, GalleryAction::InspectStep(1));
    let all = texts(&mut app);
    assert!(
        all.iter()
            .any(|text| text.starts_with("captured \"clicks\" = 0 (")),
        "{all:?}"
    );
}

#[test]
fn a_jump_keeps_the_trace_and_looks_at_the_step_jumped_to() {
    let mut app = gallery_app();
    on_the_trace_tab(&mut app);
    ask(&mut app, GalleryAction::ToggleLoop);
    run_until(&mut app, 500, finished);
    assert!(app.world().resource::<GalleryState>().trace().is_some());
    ask(&mut app, GalleryAction::JumpTo(3));
    run_until(&mut app, 500, |world| {
        let state = world.resource::<GalleryState>();
        state.jumping_to().is_none() && !state.is_playing()
    });
    let state = app.world().resource::<GalleryState>();
    assert!(state.trace().is_some(), "the trace of the run is kept");
    assert_eq!(state.trace_step(), Some(3));
    assert!(!state.is_following_trace());
    assert_eq!(state.trace_world(), TraceWorld::Before);
}

#[test]
fn the_trace_tab_redraws_when_the_step_or_the_world_looked_at_changes() {
    let mut app = gallery_app();
    on_the_trace_tab(&mut app);
    ask(&mut app, GalleryAction::ToggleLoop);
    run_until(&mut app, 500, finished);
    app.update();
    ask(&mut app, GalleryAction::InspectStep(1));
    ask(&mut app, GalleryAction::ShowTraceWorld(TraceWorld::After));
    let after = app
        .world()
        .resource::<GalleryState>()
        .trace()
        .and_then(|trace| trace.after(1))
        .expect("the step finished")
        .frame;
    assert!(shows(
        &mut app,
        &format!("UI after the step (frame {after})")
    ));
    ask(&mut app, GalleryAction::ShowTraceWorld(TraceWorld::Before));
    let before = app
        .world()
        .resource::<GalleryState>()
        .trace()
        .and_then(|trace| trace.before(1))
        .expect("the step has a point before it")
        .frame;
    assert!(shows(
        &mut app,
        &format!("UI before the step (frame {before})")
    ));
    assert!(!shows(
        &mut app,
        &format!("UI after the step (frame {after})")
    ));
}

#[test]
fn the_tabs_swap_the_step_list_between_playing_up_to_a_step_and_looking_at_one() {
    let mut app = gallery_app();
    on_the_trace_tab(&mut app);
    // The Trace tab has nothing to list until the script has played a while.
    run_until(&mut app, 500, |world| frames(world) >= 14);
    ask(&mut app, GalleryAction::TogglePlay);
    app.update();
    assert!(shows(&mut app, "Steps (click to look at one)"));
    ask(&mut app, GalleryAction::ShowTab(GalleryTab::Scenario));
    assert!(shows(&mut app, "Steps (click to play up to one and pause)"));
    assert!(!shows(&mut app, "Steps (click to look at one)"));
    ask(&mut app, GalleryAction::ShowTab(GalleryTab::Trace));
    assert!(shows(&mut app, "Steps (click to look at one)"));
}

#[test]
fn selecting_another_entry_forgets_the_trace() {
    let mut app = gallery_app();
    on_the_trace_tab(&mut app);
    ask(&mut app, GalleryAction::ToggleLoop);
    run_until(&mut app, 500, finished);
    assert!(app.world().resource::<GalleryState>().trace().is_some());
    ask(&mut app, GalleryAction::Select(1));
    assert!(app.world().resource::<GalleryState>().trace().is_none());
}

#[test]
fn running_everything_traces_nothing() {
    let (app, _) = run_all([adds(2)]);
    assert!(app.world().resource::<GalleryState>().trace().is_none());
    assert!(app.world().resource::<ScenarioPlayer>().trace().is_none());
}

/// Whether some text on screen starts with `prefix`, as a change line does.
fn lists_change(app: &mut App, prefix: &str) -> bool {
    texts(app).iter().any(|text| text.starts_with(prefix))
}

/// How many changes the Trace tab says its filter hides.
fn hidden_count(app: &mut App) -> Option<usize> {
    texts(app).iter().find_map(|text| {
        text.strip_prefix("Hidden by the filter (")?
            .strip_suffix(')')?
            .parse()
            .ok()
    })
}

/// Plays the first scenario to its end and looks in the Trace tab at the
/// first step that changed the app's `Count`; returns that step.
fn at_the_step_that_counted(app: &mut App) -> usize {
    on_the_trace_tab(app);
    ask(app, GalleryAction::ToggleLoop);
    run_until(app, 500, finished);
    app.update();
    let step = (0..8)
        .find(|step| {
            ask(app, GalleryAction::InspectStep(*step));
            lists_change(app, "resource Count.0")
        })
        .expect("a step changed the count");
    ask(app, GalleryAction::InspectStep(step));
    step
}

#[test]
fn the_trace_tab_lists_the_apps_changes_under_the_apps_filter_and_everything_on_request() {
    let mut app = gallery_app();
    let step = at_the_step_that_counted(&mut app);
    assert!(
        !lists_change(&mut app, "resource Time<Virtual>."),
        "the app's filter leaves Bevy's time out: {:?}",
        texts(&mut app)
    );
    assert!(hidden_count(&mut app).is_some_and(|hidden| hidden > 0));
    ask(&mut app, GalleryAction::UseNoise(NoisePreset::Everything));
    assert!(lists_change(&mut app, "resource Count.0"));
    assert!(
        lists_change(&mut app, "resource Time<Virtual>."),
        "everything lists Bevy's time: {:?}",
        texts(&mut app)
    );
    assert_eq!(hidden_count(&mut app), None);
    // Nothing was played again to change what is listed.
    assert!(finished(app.world()));
    ask(&mut app, GalleryAction::UseNoise(NoisePreset::App));
    ask(&mut app, GalleryAction::InspectStep(step));
    assert!(!lists_change(&mut app, "resource Time<Virtual>."));
}

#[test]
fn hiding_a_type_in_the_trace_tab_moves_its_changes_to_the_hidden_list_and_showing_brings_them_back()
 {
    let mut app = gallery_app();
    at_the_step_that_counted(&mut app);
    let hidden = hidden_count(&mut app).unwrap_or(0);
    // The only type the app's filter lets through in this step is the app's.
    ask(&mut app, GalleryAction::ToggleType(0));
    assert!(!lists_change(&mut app, "resource Count.0"));
    assert_eq!(hidden_count(&mut app), Some(hidden + 1));
    assert_eq!(
        app.world().resource::<GalleryState>().noise_preset(),
        None,
        "the filter is the app's with an edit"
    );
    assert!(
        lists_change(&mut app, "As code: NoiseFilter::default().ignore("),
        "{:?}",
        texts(&mut app)
    );
    ask(&mut app, GalleryAction::ToggleHiddenChanges);
    assert!(lists_change(&mut app, "resource Count.0"));
    assert!(shows(&mut app, "Show"));
    ask(&mut app, GalleryAction::ToggleType(0));
    ask(&mut app, GalleryAction::ToggleHiddenChanges);
    assert!(lists_change(&mut app, "resource Count.0"));
    assert_eq!(hidden_count(&mut app), Some(hidden));
}

#[test]
fn the_trace_tabs_filter_survives_selecting_another_scenario() {
    let mut app = gallery_app();
    on_the_trace_tab(&mut app);
    ask(&mut app, GalleryAction::UseNoise(NoisePreset::Everything));
    ask(&mut app, GalleryAction::Select(1));
    let state = app.world().resource::<GalleryState>();
    assert_eq!(state.noise_preset(), Some(NoisePreset::Everything));
    assert_eq!(state.noise(), &NoiseFilter::nothing());
}

#[test]
fn the_trace_tab_shows_the_filter_in_force_as_code() {
    let mut app = gallery_app();
    at_the_step_that_counted(&mut app);
    assert!(lists_change(&mut app, "As code: NoiseFilter::default()"));
    ask(&mut app, GalleryAction::UseNoise(NoisePreset::Everything));
    assert!(lists_change(&mut app, "As code: NoiseFilter::nothing()"));
}

#[test]
fn a_filter_change_is_accepted_while_recording() {
    let dir = fresh_dir("gallery-noise-recording");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::Select(0));
    ask(&mut app, GalleryAction::Record);
    ask(&mut app, GalleryAction::UseNoise(NoisePreset::Everything));
    let state = app.world().resource::<GalleryState>();
    assert!(state.is_recording());
    assert_eq!(state.noise_preset(), Some(NoisePreset::Everything));
}

#[test]
fn a_type_a_keep_pattern_of_the_app_matches_cannot_be_hidden() {
    let hooks = SessionHooks::default().noise_filter(NoiseFilter::default().keep("Count"));
    let mut app = app_with(
        GalleryPlugin::new(gallery())
            .with_reset(reset_app)
            .with_session_hooks(hooks)
            .with_loop_pause(2),
    );
    at_the_step_that_counted(&mut app);
    ask(&mut app, GalleryAction::ToggleType(0));
    let state = app.world().resource::<GalleryState>();
    assert_eq!(state.status(), "Kept by a keep pattern of the app");
    assert_eq!(state.noise_preset(), Some(NoisePreset::App));
}

/// The default variant and a larger window.
fn default_and_large() -> [Variant; 2] {
    [
        Variant::default(),
        Variant::new("large").window_size(800, 600),
    ]
}

/// A gallery app over `scenarios`, listing `variants`.
fn variant_app(scenarios: impl IntoIterator<Item = Scenario>, variants: [Variant; 2]) -> App {
    app_with(
        GalleryPlugin::new(Gallery::new().group("All", scenarios).variants(variants))
            .with_reset(reset_app)
            .with_loop_pause(2),
    )
}

/// The size of the primary window, in logical pixels.
fn window_size_of(app: &mut App) -> Vec2 {
    let window = primary(app);
    app.world()
        .get::<Window>(window)
        .expect("window")
        .resolution
        .size()
}

#[test]
fn the_variant_picker_runs_the_selected_scenario_in_that_variant() {
    let mut app = variant_app([adds(2)], default_and_large());
    ask(&mut app, GalleryAction::Select(0));
    assert_eq!(window_size_of(&mut app), Vec2::new(640.0, 480.0));
    ask(&mut app, GalleryAction::SelectVariant(1));
    assert_eq!(app.world().resource::<GalleryState>().variant(), 1);
    assert_eq!(window_size_of(&mut app), Vec2::new(800.0, 600.0));
    run_until(&mut app, 600, finished);
    let report = app.world().resource::<ScenarioReport>();
    assert_eq!(report.name, "add 2 [large]");
    assert!(report.passed(), "{report:#?}");
    let state = app.world().resource::<GalleryState>();
    assert_eq!(state.result_in(0, 1), Some(ScenarioResult::Passed));
    assert_eq!(state.result_in(0, 0), None, "the default has not run");
}

#[test]
fn going_back_to_the_default_variant_puts_the_scenarios_window_back() {
    let mut app = variant_app([adds(2)], default_and_large());
    ask(&mut app, GalleryAction::Select(0));
    ask(&mut app, GalleryAction::SelectVariant(1));
    ask(&mut app, GalleryAction::SelectVariant(0));
    assert_eq!(window_size_of(&mut app), Vec2::new(640.0, 480.0));
    assert_eq!(app.world().resource::<ScenarioReport>().name, "add 2");
}

#[test]
fn a_variant_that_does_not_exist_is_ignored() {
    let mut app = variant_app([adds(2)], default_and_large());
    ask(&mut app, GalleryAction::SelectVariant(7));
    assert_eq!(app.world().resource::<GalleryState>().variant(), 0);
}

#[test]
fn running_everything_plays_each_scenario_in_each_variant() {
    let mut app = app_with(
        GalleryPlugin::new(
            Gallery::new()
                .group("All", [adds(2), adds(3)])
                .variants(default_and_large()),
        )
        .with_reset(reset_app)
        .with_run_all(),
    );
    for _ in 0..4_000 {
        if exit_asked(&app).is_some() {
            break;
        }
        app.update();
    }
    let report = app.world().resource::<GalleryRunReport>();
    assert!(report.passed(), "{}", report.summary());
    let names: Vec<_> = report.outcomes.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["add 2", "add 2 [large]", "add 3", "add 3 [large]"]);
}

#[test]
fn a_screen_step_in_a_variant_fails_the_run_naming_the_step() {
    let fixed = Scenario::new("fixed")
        .setup(spawn_app)
        .window_size(640, 480)
        .step(Step::move_to(Target::screen(Vec2::new(10.0, 10.0))));
    let mut app = app_with(
        GalleryPlugin::new(
            Gallery::new()
                .group("All", [fixed])
                .variants(default_and_large()),
        )
        .with_reset(reset_app)
        .with_run_all(),
    );
    for _ in 0..2_000 {
        if exit_asked(&app).is_some() {
            break;
        }
        app.update();
    }
    let report = app.world().resource::<GalleryRunReport>();
    let failed: Vec<_> = report.failed().collect();
    assert_eq!(failed.len(), 1, "{}", report.summary());
    let outcome = failed.first().expect("one failure");
    assert_eq!(outcome.name, "fixed [large]");
    assert!(
        outcome.failures.iter().any(|line| line.contains("step 0")),
        "{:?}",
        outcome.failures
    );
}

#[test]
fn a_scenario_restricted_to_another_variant_is_skipped_not_played() {
    let restricted = adds(2).variants(["default"]);
    let mut app = variant_app([restricted], default_and_large());
    ask(&mut app, GalleryAction::SelectVariant(1));
    ask(&mut app, GalleryAction::Select(0));
    let state = app.world().resource::<GalleryState>();
    assert_eq!(state.result_in(0, 1), Some(ScenarioResult::Skipped));
    assert!(!state.is_playing(), "the player never starts");
    assert!(state.status().contains("Skipped"), "{}", state.status());
    let report = app.world().resource::<ScenarioReport>();
    assert_eq!(report.name, "add 2 [large]");
    assert_eq!(report.frames, 0, "no frame played");
    assert_eq!(count(app.world()), 0, "the app was not played");
}

#[test]
fn running_everything_skips_a_scenario_that_does_not_name_the_variant() {
    let restricted = adds(2).variants(["default"]);
    let mut app = app_with(
        GalleryPlugin::new(
            Gallery::new()
                .group("All", [restricted, adds(3)])
                .variants(default_and_large()),
        )
        .with_reset(reset_app)
        .with_run_all(),
    );
    for _ in 0..4_000 {
        if exit_asked(&app).is_some() {
            break;
        }
        app.update();
    }
    let report = app.world().resource::<GalleryRunReport>();
    assert!(report.passed(), "{}", report.summary());
    let names: Vec<_> = report.outcomes.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["add 2", "add 2 [large]", "add 3", "add 3 [large]"]);
    let skipped = report
        .outcomes
        .iter()
        .filter(|outcome| outcome.skipped())
        .count();
    assert_eq!(skipped, 1, "{}", report.summary());
    let summary = report.summary();
    assert!(
        summary.starts_with("3 of 4 scenarios passed, 1 skipped"),
        "{summary}"
    );
    assert!(summary.contains("SKIP add 2 [large]"), "{summary}");
}

#[test]
fn save_as_setup_while_recording_writes_the_apps_own_setup() {
    let dir = fresh_dir("gallery-save-own");
    let mut app = recording_app(&dir);
    ask(&mut app, GalleryAction::Record);
    ask(&mut app, GalleryAction::SaveAsSetup);
    let state = app.world().resource::<GalleryState>();
    assert!(state.is_recording(), "saving does not stop the recording");
    assert!(state.status().contains("Saved setup"), "{}", state.status());
    assert!(state.status().contains("Counted"), "{}", state.status());
    let scenes = std::fs::read_dir(&dir).map_or(0, |files| {
        files
            .filter_map(Result::ok)
            .filter(|file| file.file_name().to_string_lossy().ends_with(".scene.ron"))
            .count()
    });
    assert_eq!(scenes, 0, "the app's capture wrote its own format");
}

#[test]
fn save_as_setup_writes_the_world_for_an_app_without_a_save_format() {
    let dir = fresh_dir("gallery-save-scene");
    let mut app = recording_app(&dir);
    app.world_mut().insert_resource(CaptureNothing);
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);
    // Taken over, so the script no longer plays, with its two clicks done or
    // not: whatever the count is.
    ask(&mut app, GalleryAction::TakeOver);
    assert!(app.world().resource::<GalleryState>().can_save_as_setup());
    let count = app.world().resource::<Count>().0;
    ask(&mut app, GalleryAction::SaveAsSetup);
    let path = dir.join("add_2.scene.ron");
    let text = std::fs::read_to_string(&path).expect("the world was written");
    assert!(text.contains("Count"), "{text}");
    assert!(
        !text.contains("Button") && !text.contains("Node"),
        "the app's UI is not saved: {text}"
    );
    let status = app.world().resource::<GalleryState>().status().to_owned();
    assert!(status.contains("add_2.scene.ron"), "{status}");
    assert!(status.contains("Scene("), "{status}");

    // A second save does not overwrite the first.
    ask(&mut app, GalleryAction::SaveAsSetup);
    assert!(dir.join("add_2-2.scene.ron").exists());

    // And the saved world is a setup that brings the count back.
    app.world_mut().resource_mut::<Count>().0 = 99;
    let scenario = Scenario::new("from the save")
        .scene_setup("add_2.scene.ron")
        .with_base_dir(&dir);
    scenario.run_setup(app.world_mut()).expect("loads");
    assert_eq!(app.world().resource::<Count>().0, count);
}

#[test]
fn save_as_setup_does_nothing_while_a_script_plays() {
    let dir = fresh_dir("gallery-save-playing");
    let mut app = recording_app(&dir);
    app.world_mut().insert_resource(CaptureNothing);
    ask(&mut app, GalleryAction::Select(0));
    run_until(&mut app, 100, |world| frames(world) >= 4);
    assert!(!finished(app.world()), "the script is still playing");
    let state = app.world().resource::<GalleryState>();
    assert!(state.is_script_playing());
    assert!(!state.can_save_as_setup());

    ask(&mut app, GalleryAction::SaveAsSetup);
    let state = app.world().resource::<GalleryState>();
    assert!(state.is_playing(), "the request does not pause the script");
    assert!(
        !state.status().contains("Saved setup"),
        "{}",
        state.status()
    );
    assert!(
        state.status().contains("Pause the script"),
        "{}",
        state.status()
    );
    assert!(!dir.exists(), "nothing was written");

    // Paused, the same world can be saved.
    ask(&mut app, GalleryAction::TogglePlay);
    let state = app.world().resource::<GalleryState>();
    assert!(!state.is_script_playing());
    assert!(state.can_save_as_setup());
    ask(&mut app, GalleryAction::SaveAsSetup);
    assert!(dir.join("add_2.scene.ron").exists());

    // And so is the finished one.
    ask(&mut app, GalleryAction::TogglePlay);
    run_until(&mut app, 400, finished);
    run_until(&mut app, 10, |world| {
        world.resource::<GalleryState>().is_finished()
    });
    assert!(app.world().resource::<GalleryState>().can_save_as_setup());
}

#[test]
fn save_as_setup_says_why_it_failed() {
    let dir = fresh_dir("gallery-save-fails");
    let mut app = recording_app(&dir);
    app.world_mut().insert_resource(CaptureFails);
    ask(&mut app, GalleryAction::SaveAsSetup);
    let status = app.world().resource::<GalleryState>().status().to_owned();
    assert!(status.contains("Save as setup failed"), "{status}");
    assert!(status.contains("no way to save this app"), "{status}");
}

/// A scenario with a long list of steps, so the details panel has plenty to
/// show.
fn long_scenario() -> Scenario {
    let steps: Vec<Step> = (0..40).map(|_| Step::wait(1)).collect();
    Scenario::new("long").setup(spawn_app).steps(steps)
}

/// A gallery of `entries` scenarios, the first of which is [`long_scenario`].
fn gallery_of(entries: u32) -> Gallery {
    let mut all = vec![GalleryEntry::new(long_scenario())];
    all.extend((1..entries).map(|at| GalleryEntry::new(adds(at))));
    Gallery::new().group("All", all)
}

/// The heights, in logical pixels, of the catalogue and of the details panel:
/// the two scrolling lists of the Scenarios window, top to bottom.
fn panel_heights(app: &mut App) -> (f32, f32) {
    let world = app.world_mut();
    let mut lists: Vec<(f32, f32)> = world
        .query::<(&Node, &ComputedNode, &UiGlobalTransform)>()
        .iter(world)
        .filter(|(style, ..)| style.overflow.y == OverflowAxis::Scroll)
        .map(|(_, node, transform)| {
            (
                transform.affine().translation.y,
                node.size().y * node.inverse_scale_factor(),
            )
        })
        .collect();
    lists.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(lists.len(), 2, "the catalogue and the details panel");
    (lists[0].1, lists[1].1)
}

/// Updates a few times so the heights set from one layout have been laid out.
fn settle(app: &mut App) {
    for _ in 0..6 {
        app.update();
    }
}

#[test]
fn a_long_catalogue_and_a_long_list_of_steps_share_the_window_half_and_half() {
    let mut app = app_with(GalleryPlugin::new(gallery_of(60)).with_reset(reset_app));
    ask(&mut app, GalleryAction::Select(0));
    settle(&mut app);
    let (catalogue, details) = panel_heights(&mut app);
    assert!(
        catalogue > 100.0 && details > 100.0,
        "{catalogue} {details}"
    );
    assert!(
        (catalogue - details).abs() < 2.0,
        "{catalogue} vs {details}"
    );
}

#[test]
fn a_long_catalogue_leaves_a_selected_scenarios_details_a_fifth_at_least() {
    let mut app = app_with(GalleryPlugin::new(gallery_of(60)).with_reset(reset_app));
    // Nothing selected: the details panel needs almost nothing.
    settle(&mut app);
    let (catalogue, details) = panel_heights(&mut app);
    let share = details / (catalogue + details);
    assert!((share - 0.2).abs() < 0.01, "the details panel had {share}");
}

#[test]
fn a_short_catalogue_gives_the_rest_to_a_long_list_of_steps() {
    let mut app = app_with(GalleryPlugin::new(gallery_of(2)).with_reset(reset_app));
    ask(&mut app, GalleryAction::Select(0));
    settle(&mut app);
    let (catalogue, details) = panel_heights(&mut app);
    assert!(details > catalogue * 1.5, "{catalogue} vs {details}");
    let share = catalogue / (catalogue + details);
    assert!(share >= 0.19, "the catalogue had {share}");
}
