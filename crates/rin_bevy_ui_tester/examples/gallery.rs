//! The gallery over a small plain-UI app.
//!
//! ```text
//! cargo run --example gallery --features gallery
//! ```
//!
//! The app is a counter with a "+1" button, a reset button and a switch. Its
//! scenarios are in the **Scenarios** window; pick one to watch it play, loop,
//! pause, step, or take over the app with the real mouse.
//!
//! **Record** saves what you do by hand as a `.ron` file in `recordings/`,
//! starting from the counter and switch as they were when you pressed it:
//! the app's own setup, `setup: App(Counter(count: 2, switch: true))`. Press F8 in
//! the app while recording to take a checkpoint, which flashes the window.
//! When you stop, the Scenarios window lists the checks the changes suggest
//! (the counter's new value, the new text, the switch's fact), one list per
//! checkpoint and one for the end; untick the ones you do not want and the
//! file is written again without them. Files in
//! that directory show up under "Recorded", and when one is edited and saved
//! the gallery reads it again (and restarts it if it is selected); a file that
//! does not load shows why in the catalogue.
//! With `--record` the session starts straight away and is saved when the app
//! window is closed:
//!
//! ```text
//! cargo run --example gallery --features gallery -- --record
//! ```
//!
//! With `--run-all` nobody has to touch anything: every scenario plays once,
//! in order, and the app closes itself. The outcome is logged, and the exit
//! code is non-zero if any scenario failed (`just gallery-test`):
//!
//! ```text
//! cargo run --example gallery --features gallery -- --run-all
//! ```

use std::path::Path;

use bevy::{
    app::AppExit, ecs::system::IntoObserverSystem, prelude::*, ui::px, ui_widgets::Activate,
};
use rin_bevy_ui_tester::{
    AppCheck, AppSetup, Check, CheckResult, Gallery, GalleryEntry, GalleryPlugin, Probe,
    ReflectAppCheck, ReflectAppSetup, Registry, Scenario, SessionRecorder, Step, Target, session,
};

/// Where recordings are written, and read from at startup.
const RECORDINGS: &str = "recordings";

/// The size of the app's window, in logical pixels.
const WINDOW: (u32, u32) = (640, 360);

/// What the app counts. Reflected, so a recording can suggest checks on it.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Counter(u32);

/// The switch. Reflected, like the counter.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Switch(bool);

/// The app's own checks. Bevy finds the type by itself, and a scenario that
/// uses them can be exported as RON and loaded again.
#[derive(Reflect, Debug, Clone)]
#[reflect(AppCheck)]
enum CounterCheck {
    /// The counter is at this value.
    CounterIs(u32),
    /// The switch is on.
    SwitchOn,
}

impl AppCheck for CounterCheck {
    fn run(&self, probe: &Probe) -> CheckResult {
        match self {
            Self::CounterIs(wanted) => match probe.resource::<Counter>() {
                Some(counter) if counter.0 == *wanted => Ok(()),
                Some(counter) => Err(format!("the counter is at {}", counter.0)),
                None => Err("there is no counter".into()),
            },
            Self::SwitchOn => {
                if probe.resource::<Switch>().is_some_and(|switch| switch.0) {
                    Ok(())
                } else {
                    Err("the switch is off".into())
                }
            }
        }
    }

    /// What a recording may suggest of the app's own checks.
    fn facts(probe: &Probe) -> Vec<Self> {
        if probe.resource::<Switch>().is_some_and(|switch| switch.0) {
            vec![Self::SwitchOn]
        } else {
            Vec::new()
        }
    }
}

/// Where the app's scenarios start from. Bevy finds the type by itself, and a
/// recording captures the app's state as one of these.
#[derive(Reflect, Debug, Clone)]
#[reflect(AppSetup)]
enum CounterSetup {
    /// The app with the counter and the switch at these values.
    Counter {
        /// The counter.
        count: u32,
        /// Whether the switch is on.
        switch: bool,
    },
}

impl AppSetup for CounterSetup {
    fn build(&self, world: &mut World) -> Result<(), String> {
        let Self::Counter { count, switch } = self;
        spawn_app(world);
        world.insert_resource(Counter(*count));
        world.insert_resource(Switch(*switch));
        Ok(())
    }

    /// The counter and the switch as they are when recording starts.
    fn capture(world: &mut World, _dir: &Path, _stem: &str) -> Result<Option<Self>, String> {
        Ok(Some(Self::Counter {
            count: world.get_resource::<Counter>().map_or(0, |c| c.0),
            switch: world.get_resource::<Switch>().is_some_and(|s| s.0),
        }))
    }
}

/// Marks everything the app spawned, so the reset hook can remove it.
#[derive(Component)]
struct AppRoot;

/// The text that shows the counter.
#[derive(Component)]
struct CounterText;

/// The text that shows the switch.
#[derive(Component)]
struct SwitchText;

/// Spawns a button labelled `label` at `top`, running `on_press` when pressed.
fn button<M>(
    world: &mut World,
    label: &str,
    top: i32,
    on_press: impl IntoObserverSystem<Activate, M>,
) {
    world
        .spawn((
            AppRoot,
            bevy::ui_widgets::Button,
            Node {
                position_type: PositionType::Absolute,
                left: px(40),
                top: px(top),
                width: px(200),
                height: px(48),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..Node::default()
            },
            BackgroundColor(Color::srgb(0.25, 0.3, 0.45)),
            children![Text::new(label)],
        ))
        .observe(on_press);
}

/// Builds the app's UI.
fn spawn_app(world: &mut World) {
    world.spawn((AppRoot, Camera2d));
    button(
        world,
        "+1",
        40,
        |_: On<Activate>, mut counter: ResMut<Counter>| {
            counter.0 += 1;
        },
    );
    button(
        world,
        "Reset counter",
        110,
        |_: On<Activate>, mut counter: ResMut<Counter>| counter.0 = 0,
    );
    button(
        world,
        "Switch",
        180,
        |_: On<Activate>, mut switch: ResMut<Switch>| {
            switch.0 = !switch.0;
        },
    );
    world.spawn((
        AppRoot,
        CounterText,
        Text::new("Count: 0"),
        Node {
            position_type: PositionType::Absolute,
            left: px(280),
            top: px(52),
            ..Node::default()
        },
    ));
    world.spawn((
        AppRoot,
        SwitchText,
        Text::new("Switch: off"),
        Node {
            position_type: PositionType::Absolute,
            left: px(280),
            top: px(192),
            ..Node::default()
        },
    ));
}

/// Puts the app's state back for a fresh run.
fn reset_app(world: &mut World) {
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, With<AppRoot>>()
        .iter(world)
        .collect();
    for root in roots {
        world.despawn(root);
    }
    world.insert_resource(Counter(0));
    world.insert_resource(Switch(false));
}

/// Keeps the texts in step with the state.
fn show_state(
    counter: Res<Counter>,
    switch: Res<Switch>,
    mut counts: Query<&mut Text, (With<CounterText>, Without<SwitchText>)>,
    mut switches: Query<&mut Text, (With<SwitchText>, Without<CounterText>)>,
) {
    for mut text in &mut counts {
        text.0 = format!("Count: {}", counter.0);
    }
    for mut text in &mut switches {
        text.0 = format!("Switch: {}", if switch.0 { "on" } else { "off" });
    }
}

/// A scenario of this app, from a fresh start, with its window size.
fn scenario(name: &str) -> Scenario {
    Scenario::new(name)
        .app_setup(CounterSetup::Counter {
            count: 0,
            switch: false,
        })
        .window_size(WINDOW.0, WINDOW.1)
}

/// The catalogue.
fn gallery() -> Gallery {
    let count_to_three = scenario("Count to three")
        .steps([
            Step::caption("Three presses of +1"),
            Step::glide(Target::text("+1"), 30),
            Step::click(),
            Step::wait(20),
            Step::click(),
            Step::wait(20),
            Step::click(),
            Step::wait(20),
            Step::check(Check::shows("Count: 3")),
        ])
        .expect_check(Check::app(CounterCheck::CounterIs(3)));
    let reset = scenario("Count, then reset")
        .steps([
            Step::caption("Two presses, then Reset counter"),
            Step::glide(Target::text("+1"), 30),
            Step::click(),
            Step::wait(15),
            Step::click(),
            Step::wait(15),
            Step::glide(Target::text("Reset counter"), 40),
            Step::wait(10),
            Step::click(),
            Step::wait(20),
        ])
        .expect_check(Check::app(CounterCheck::CounterIs(0)));
    let switch = scenario("Flip the switch")
        .steps([
            Step::caption("The switch turns on"),
            Step::glide(Target::text("Switch"), 40),
            Step::wait(10),
            Step::click(),
            Step::wait(30),
        ])
        .expect_check(Check::app(CounterCheck::SwitchOn));
    Gallery::new()
        .group(
            "Counter",
            [
                GalleryEntry::new(count_to_three).described("Press +1 three times."),
                GalleryEntry::new(reset).described("Count up, then reset."),
            ],
        )
        .group(
            "Switch",
            [GalleryEntry::new(switch).described("Turn the switch on.")],
        )
}

/// With `--record`: starts a session as soon as the app is up.
fn record_from_the_command_line(mut recorder: ResMut<SessionRecorder>) {
    if std::env::args().any(|arg| arg == "--record") {
        recorder.request_start("Recorded session");
    }
}

/// Saves a session still in progress when the app is closing.
fn save_on_exit(world: &mut World) {
    let closing = world
        .get_resource::<Messages<AppExit>>()
        .is_some_and(|messages| !messages.is_empty());
    let recording = world
        .get_resource::<SessionRecorder>()
        .is_some_and(SessionRecorder::is_recording);
    if closing && recording {
        match session::stop(world) {
            Ok(saved) => info!("{}", saved.describe()),
            Err(error) => error!("{error}"),
        }
    }
}

fn main() -> AppExit {
    // The directory is watched: a recording saved here, or a file edited in
    // an editor, shows up in the gallery without a restart. Its setup is the
    // app's own, so nothing but the window size is added to what the file says.
    let gallery = gallery()
        .prepare_files(|scenario| scenario.window_size(WINDOW.0, WINDOW.1))
        .directory("Recorded", RECORDINGS, &Registry::default());
    let mut plugin = GalleryPlugin::new(gallery)
        .with_reset(reset_app)
        .with_export_dir(RECORDINGS);
    if std::env::args().any(|arg| arg == "--run-all") {
        plugin = plugin.with_run_all();
    }
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "App under test".into(),
                resolution: bevy::window::WindowResolution::new(WINDOW.0, WINDOW.1),
                ..Window::default()
            }),
            ..WindowPlugin::default()
        }))
        .init_resource::<Counter>()
        .init_resource::<Switch>()
        .add_systems(Update, show_state)
        .add_systems(Startup, record_from_the_command_line)
        .add_systems(Last, save_on_exit)
        .add_plugins(plugin)
        .run()
}
