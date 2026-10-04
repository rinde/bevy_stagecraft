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
use rin_bevy_ui_tester::{Probe, Registry, ResolvedTarget, StateProbe};

/// How many times the button was activated.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
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

/// Whether the button was activated exactly as many times as `argument`
/// says.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "the signature register_custom takes"
)]
pub fn activated(probe: &Probe<'_>, argument: &str) -> Result<(), String> {
    let wanted: u32 = argument
        .parse()
        .map_err(|_| format!("`{argument}` is not a count"))?;
    match probe.resource::<Activations>() {
        Some(count) if count.0 == wanted => Ok(()),
        Some(count) => Err(format!("activated {} times, not {wanted}", count.0)),
        None => Err("nothing counts activations".into()),
    }
}

/// Reads the activation count as the app's state, under `activations`.
pub fn state_probe() -> StateProbe {
    StateProbe::new(|world| {
        world
            .get_resource::<Activations>()
            .map(|count| ("activations".to_owned(), count.0.to_string()))
            .into_iter()
            .collect()
    })
}

/// The registry the RON scenarios are loaded with.
pub fn registry() -> Registry {
    let mut registry = Registry::default().with_state(state_probe());
    registry.setups.register("plain_ui", spawn_ui);
    registry
        .checks
        .register("activated once", |probe: &Probe<'_>| {
            probe.resource::<Activations>().is_some_and(|a| a.0 == 1)
        });
    registry.checks.register_custom("activated", activated);
    registry.register_type::<Activations>();
    registry.register_type::<Button>();
    registry.targets.register("pause_button", |_world, _arg| {
        Ok(ResolvedTarget::at(Vec2::new(150.0, 50.0)))
    });
    registry
}
