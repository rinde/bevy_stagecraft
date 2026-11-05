//! A checkpoint saved as a setup for other scenarios, on a plain Bevy UI: a
//! "+1" button counting into a resource.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

use std::{fs, path::PathBuf};

use bevy::{
    prelude::*,
    ui::px,
    ui_widgets::{Activate, Button},
};
use bevy_stagecraft::{
    AppSetup, Check, HeadlessPlugins, LoadError, NoiseFilter, NotUnderTest, ReflectAppSetup,
    Registry, SCENE_SUFFIX, Scenario, Setup, Step, Target, load_ron_file, ron_files, save_setup,
};

/// What the "+1" button counts: the state worth saving.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Counter(u32);

/// A thing the app keeps, as an entity.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct Item(String);

/// Marks the button, which is UI and must not be saved even so.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct IncrementButton;

/// Makes the app's own setup save in its own format.
#[derive(Resource)]
struct OwnFormat;

/// The app's own setup type, with a save format of its own.
#[derive(Reflect, Debug, Clone)]
#[reflect(AppSetup)]
enum SaveSetup {
    /// The counter, from a file of its own format in the scenario's directory.
    Own(String),
}

impl AppSetup for SaveSetup {
    fn build(&self, world: &mut World) -> Result<(), String> {
        let Self::Own(file) = self;
        let text = fs::read_to_string(file).map_err(|error| error.to_string())?;
        let count = text.trim().parse().map_err(|_| "not a number".to_owned())?;
        world.insert_resource(Counter(count));
        Ok(())
    }

    fn capture(
        world: &mut World,
        dir: &std::path::Path,
        stem: &str,
    ) -> Result<Option<Self>, String> {
        if world.get_resource::<OwnFormat>().is_none() {
            return Ok(None);
        }
        let count = world
            .get_resource::<Counter>()
            .map_or(0, |counter| counter.0);
        let file = dir.join(format!("{stem}.own"));
        fs::write(&file, count.to_string()).map_err(|error| error.to_string())?;
        Ok(Some(Self::Own(file.display().to_string())))
    }
}

/// Spawns the counter's UI, which a scenario that starts from a saved world
/// brings itself.
fn spawn_ui(world: &mut World) {
    world.init_resource::<Counter>();
    world.spawn(Camera2d);
    world
        .spawn((
            Button,
            IncrementButton,
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
}

/// A headless app whose counter reads `count`, with two items and its UI.
fn app_at(count: u32) -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default().with_window_size(640, 480));
    app.finish();
    app.cleanup();
    spawn_ui(app.world_mut());
    app.world_mut().insert_resource(Counter(count));
    app.world_mut().spawn(Item("sword".into()));
    app.world_mut().spawn(Item("shield".into()));
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

/// What the default filter saves of `app`, as the file's text.
fn saved_text(app: &mut App, dir: &std::path::Path, stem: &str) -> String {
    let saved = save_setup(app.world_mut(), dir, stem, &NoiseFilter::default()).expect("saves");
    fs::read_to_string(saved.path.expect("a file was written")).expect("reads")
}

#[test]
fn a_counter_saved_at_a_checkpoint_is_another_scenarios_setup() {
    let dir = dir("save-setup-counter");
    let mut app = app_at(3);
    let saved = save_setup(app.world_mut(), &dir, "three", &NoiseFilter::default()).expect("saves");
    assert_eq!(saved.setup, Setup::Scene("three.scene.ron".into()));
    assert_eq!(saved.path, Some(dir.join("three.scene.ron")));

    let scenario = Scenario::new("from three")
        .scene_setup("three.scene.ron")
        .with_base_dir(&dir)
        .setup(spawn_ui)
        .steps([
            Step::check(Check::resource::<Counter>(".0").eq(3_u32)),
            Step::move_to(Target::text("+1")),
            Step::click(),
            Step::wait(2),
        ])
        .expect_check(Check::resource::<Counter>(".0").eq(4_u32))
        .expect_check(Check::count::<Item>().eq(2));
    let report = scenario.run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_saved_setup_loads_from_the_scenarios_file_next_to_it() {
    let dir = dir("save-setup-file");
    let mut app = app_at(7);
    save_setup(app.world_mut(), &dir, "seven", &NoiseFilter::default()).expect("saves");
    let scenario = Scenario::new("from seven")
        .scene_setup("seven.scene.ron")
        .expect_check(Check::resource::<Counter>(".0").eq(7_u32));
    let ron = scenario.to_ron().expect("exports");
    assert!(ron.contains("setup: Scene(\"seven.scene.ron\")"), "{ron}");
    fs::write(dir.join("from-seven.ron"), &ron).expect("writes the scenario");

    // The scenario's file is next to the saved world, so the name finds it
    // whatever the working directory is.
    let loaded = load_ron_file(dir.join("from-seven.ron"), &Registry::default()).expect("loads");
    assert_eq!(loaded.base_dir(), Some(dir.as_path()));
    let report = loaded.setup(spawn_ui).run_headless();
    assert!(report.passed(), "{:?}", report.failures());

    // The saved world is a setup and not a scenario to play.
    let files = ron_files(&dir).expect("lists");
    assert_eq!(files, [dir.join("from-seven.ron")]);
    assert!(dir.join(format!("seven{SCENE_SUFFIX}")).exists());
}

#[test]
fn the_saved_file_is_the_same_on_every_run() {
    let dir = dir("save-setup-stable");
    let first = saved_text(&mut app_at(5), &dir, "first");
    let second = saved_text(&mut app_at(5), &dir, "second");
    assert_eq!(first, second);
    let again = saved_text(&mut app_at(5), &dir, "third");
    assert_eq!(first, again);
    assert!(first.contains("Counter"), "{first}");
}

#[test]
fn the_saved_file_holds_the_apps_data_and_not_its_ui() {
    let dir = dir("save-setup-excludes");
    let mut app = app_at(1);
    // Tooling and a parentless marker on a node's child are no app data.
    app.world_mut().spawn((Item("tool".into()), NotUnderTest));
    let text = saved_text(&mut app, &dir, "data");
    assert!(text.contains("Counter"), "{text}");
    assert!(text.contains("sword") && text.contains("shield"), "{text}");
    for left_out in [
        "IncrementButton",
        "tool",
        "Node",
        "Camera",
        "Window",
        "Text",
    ] {
        assert!(!text.contains(left_out), "{left_out} in {text}");
    }
}

#[test]
fn a_saved_world_is_loaded_into_a_world_that_has_other_values() {
    let dir = dir("save-setup-overwrite");
    let mut app = app_at(9);
    save_setup(app.world_mut(), &dir, "nine", &NoiseFilter::default()).expect("saves");

    let mut other = app_at(0);
    let scenario = Scenario::new("over zero")
        .scene_setup("nine.scene.ron")
        .with_base_dir(&dir);
    scenario.run_setup(other.world_mut()).expect("loads");
    assert_eq!(other.world().resource::<Counter>().0, 9);
    let items = other
        .world_mut()
        .query::<&Item>()
        .iter(other.world())
        .count();
    // The two it had, and the two the file brought: a load adds entities.
    assert_eq!(items, 4);
}

#[test]
fn a_missing_saved_world_fails_the_scenario_naming_the_file() {
    let dir = dir("save-setup-missing");
    fs::create_dir_all(&dir).expect("creates");
    let report = Scenario::new("no file")
        .scene_setup("gone.scene.ron")
        .with_base_dir(&dir)
        .steps([Step::wait(1)])
        .run_headless();
    assert!(!report.passed());
    let failures = report.failures();
    let [failure] = failures.as_slice() else {
        panic!("one failure: {failures:?}");
    };
    assert!(failure.starts_with("setup: "), "{failure}");
    assert!(failure.contains("gone.scene.ron"), "{failure}");
}

#[test]
fn a_file_that_is_not_a_saved_world_fails_the_scenario() {
    let dir = dir("save-setup-garbage");
    fs::create_dir_all(&dir).expect("creates");
    fs::write(dir.join("bad.scene.ron"), "this is not a world").expect("writes");
    let report = Scenario::new("garbage")
        .scene_setup("bad.scene.ron")
        .with_base_dir(&dir)
        .run_headless();
    assert!(!report.passed());
    assert!(
        report
            .failures()
            .iter()
            .any(|failure| failure.contains("not a saved world")),
        "{:?}",
        report.failures()
    );
}

#[test]
fn saving_never_overwrites_an_earlier_save() {
    let dir = dir("save-setup-numbered");
    let mut app = app_at(2);
    let first = save_setup(app.world_mut(), &dir, "again", &NoiseFilter::default()).expect("saves");
    app.world_mut().insert_resource(Counter(8));
    let second =
        save_setup(app.world_mut(), &dir, "again", &NoiseFilter::default()).expect("saves");
    assert_eq!(first.setup, Setup::Scene("again.scene.ron".into()));
    assert_eq!(second.setup, Setup::Scene("again-2.scene.ron".into()));
    let kept = fs::read_to_string(dir.join("again.scene.ron")).expect("reads");
    assert!(kept.contains('2') && !kept.contains('8'), "{kept}");
}

#[test]
fn a_second_save_of_an_app_with_its_own_format_does_not_overwrite_the_first() {
    let dir = dir("save-setup-own-twice");
    let mut app = app_at(1);
    app.world_mut().insert_resource(OwnFormat);
    save_setup(app.world_mut(), &dir, "mine", &NoiseFilter::default()).expect("saves");
    app.world_mut().insert_resource(Counter(9));
    save_setup(app.world_mut(), &dir, "mine", &NoiseFilter::default()).expect("saves");
    assert_eq!(
        fs::read_to_string(dir.join("mine.own")).expect("reads"),
        "1"
    );
    assert_eq!(
        fs::read_to_string(dir.join("mine-2.own")).expect("reads"),
        "9"
    );
}

#[test]
fn saving_next_to_the_scenario_it_is_named_for_keeps_that_name() {
    let dir = dir("save-setup-beside-scenario");
    fs::create_dir_all(&dir).expect("a directory");
    fs::write(dir.join("again.ron"), "(scenario)").expect("writes");
    let mut app = app_at(2);
    let saved = save_setup(app.world_mut(), &dir, "again", &NoiseFilter::default()).expect("saves");
    assert_eq!(saved.setup, Setup::Scene("again.scene.ron".into()));
}

#[test]
fn an_app_with_its_own_save_format_saves_through_its_setup() {
    let dir = dir("save-setup-own");
    let mut app = app_at(6);
    app.world_mut().insert_resource(OwnFormat);
    let saved = save_setup(app.world_mut(), &dir, "mine", &NoiseFilter::default()).expect("saves");
    assert_eq!(saved.path, None, "the crate wrote no file of its own");
    let Setup::App(value) = &saved.setup else {
        panic!("the app's setup: {:?}", saved.setup);
    };
    assert!(value.to_string().starts_with("Own("), "{value}");
    assert!(dir.join("mine.own").exists());
    assert!(!dir.join("mine.scene.ron").exists());

    // And the setup it made builds the counter back, written in a script.
    let script = format!(
        "(format_version: 3, name: \"own\", setup: App({value}), steps: [], \
         expect: [Resource(path: \"Counter.0\", is: Eq(6))])"
    );
    let report = Scenario::from_ron(&script)
        .expect("loads")
        .setup(spawn_ui)
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn an_old_file_that_names_a_setup_is_told_the_new_forms() {
    let error = Scenario::from_ron(
        "(format_version: 3, name: \"old\", setup: Named(\"start\"), steps: [], expect: [])",
    )
    .expect_err("refused");
    let LoadError::Syntax(message) = &error else {
        panic!("a syntax error: {error:?}");
    };
    assert!(message.contains("Scene("), "{message}");
}
