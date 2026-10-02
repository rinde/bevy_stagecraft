//! A plain Bevy UI shared by the RON tests.

#![allow(
    dead_code,
    unreachable_pub,
    reason = "each test target uses some of this"
)]

use bevy::{
    prelude::*,
    ui::px,
    ui_widgets::{Activate, Button},
};
use rin_bevy_ui_tester::{Registry, ResolvedTarget};

/// How many times the button was activated.
#[derive(Resource, Default)]
pub struct Activations(pub u32);

/// Spawns a camera and a "Pause" button that counts activations.
pub fn spawn_ui(world: &mut World) {
    world.init_resource::<Activations>();
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
            BackgroundColor(Color::WHITE),
            children![Text::new("Pause")],
        ))
        .observe(|_: On<Activate>, mut count: ResMut<Activations>| count.0 += 1);
}

/// The registry the RON scenarios are loaded with.
pub fn registry() -> Registry {
    let mut registry = Registry::default();
    registry.setups.register("plain_ui", spawn_ui);
    registry
        .checks
        .register("activated once", |probe: &rin_bevy_ui_tester::Probe<'_>| {
            probe.resource::<Activations>().is_some_and(|a| a.0 == 1)
        });
    registry.targets.register("pause_button", |_world, _arg| {
        Ok(ResolvedTarget::at(Vec2::new(150.0, 50.0)))
    });
    registry
}
