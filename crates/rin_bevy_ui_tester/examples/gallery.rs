//! The gallery over a small plain-UI app.
//!
//! ```text
//! cargo run --example gallery --features gallery
//! ```
//!
//! The app is a counter with a "+1" button, a reset button and a switch. Its
//! scenarios are in the **Scenarios** window; pick one to watch it play, loop,
//! pause, step, or take over the app with the real mouse.

use bevy::{ecs::system::IntoObserverSystem, prelude::*, ui::px, ui_widgets::Activate};
use rin_bevy_ui_tester::{Gallery, GalleryEntry, GalleryPlugin, Scenario, Step, Target};

/// The size of the app's window, in logical pixels.
const WINDOW: (u32, u32) = (640, 360);

/// What the app counts.
#[derive(Resource, Default)]
struct Counter(u32);

/// The switch.
#[derive(Resource, Default)]
struct Switch(bool);

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
    on_press: impl IntoObserverSystem<Activate, (), M>,
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

/// A scenario of this app with its window size.
fn scenario(name: &str) -> Scenario {
    Scenario::new(name)
        .setup(spawn_app)
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
            Step::check("three"),
        ])
        .check("three", |probe| probe.has_text("Count: 3"))
        .expect("the counter says three", |probe| {
            probe.resource::<Counter>().is_some_and(|c| c.0 == 3)
        });
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
        .expect("the counter is back to zero", |probe| {
            probe.resource::<Counter>().is_some_and(|c| c.0 == 0)
        });
    let switch = scenario("Flip the switch")
        .steps([
            Step::caption("The switch turns on"),
            Step::glide(Target::text("Switch"), 40),
            Step::wait(10),
            Step::click(),
            Step::wait(30),
        ])
        .expect("the switch is on", |probe| {
            probe.resource::<Switch>().is_some_and(|s| s.0)
        });
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

fn main() -> AppExit {
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
        .add_plugins(GalleryPlugin::new(gallery()).with_reset(reset_app))
        .run()
}
