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
use rin_bevy_ui_tester::{
    AppCheck, CheckResult, Probe, ReflectAppCheck, Registry, ResolvedTarget, StateProbe, Target,
    resolve,
};

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

/// The test UI's own checks, found by Bevy with no registration call.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(AppCheck)]
pub enum UiCheck {
    /// The button was activated exactly once.
    ActivatedOnce,
    /// The button was activated exactly this many times.
    Activated(u32),
    /// The target can be found on screen.
    Found(Target),
    /// The button was activated at least `at_least` and at most `at_most`
    /// times.
    ActivatedBetween {
        /// The fewest activations.
        at_least: u32,
        /// The most activations.
        at_most: u32,
    },
}

impl AppCheck for UiCheck {
    fn run(&self, probe: &Probe) -> CheckResult {
        let count = || {
            probe
                .resource::<Activations>()
                .map(|count| count.0)
                .ok_or_else(|| "nothing counts activations".to_owned())
        };
        match self {
            Self::ActivatedOnce => Self::Activated(1).run(probe),
            Self::Activated(wanted) => match count()? {
                found if found == *wanted => Ok(()),
                found => Err(format!("activated {found} times, not {wanted}")),
            },
            Self::Found(target) => resolve(probe.world(), target).map(|_| ()),
            Self::ActivatedBetween { at_least, at_most } => {
                let found = count()?;
                if (*at_least..=*at_most).contains(&found) {
                    Ok(())
                } else {
                    Err(format!("activated {found} times"))
                }
            }
        }
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
    registry.targets.register("pause_button", |_world, _arg| {
        Ok(ResolvedTarget::at(Vec2::new(150.0, 50.0)))
    });
    registry
}
