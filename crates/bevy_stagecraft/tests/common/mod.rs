//! A plain Bevy UI shared by the RON tests, with its own setups, views,
//! targets and checks.

#![allow(
    dead_code,
    unreachable_pub,
    reason = "each test target uses some of this"
)]

use bevy::{
    ecs::{
        intern::Interned,
        schedule::{ScheduleLabel, SingleThreadedExecutor},
    },
    prelude::*,
    ui::px,
    ui_widgets::{Activate, Button},
};
use bevy_stagecraft::{
    AppCheck, AppSetup, AppTarget, AppView, CheckResult, Probe, ReflectAppCheck, ReflectAppSetup,
    ReflectAppTarget, ReflectAppView, Registry, ResolvedTarget, Target, resolve,
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

/// The test UI's own setups, found by Bevy with no registration call.
#[derive(Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(AppSetup)]
pub enum UiSetup {
    /// A camera and the "Pause" button.
    PlainUi,
    /// The same, with the button activated this many times already.
    Activated(u32),
    /// A setup that cannot be built: the save it names is missing.
    MissingSave(String),
}

impl AppSetup for UiSetup {
    fn build(&self, world: &mut World) -> Result<(), String> {
        match self {
            Self::PlainUi => spawn_ui(world),
            Self::Activated(count) => {
                spawn_ui(world);
                world.insert_resource(Activations(*count));
            }
            Self::MissingSave(file) => return Err(format!("there is no save `{file}`")),
        }
        Ok(())
    }
}

/// Which of the test UI's views was arranged last.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct Arranged(pub UiView);

/// The test UI's own views, found by Bevy with no registration call.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(AppView)]
pub enum UiView {
    /// As spawned.
    Plain,
    /// Magnified this many times.
    Zoomed(f32),
}

impl AppView for UiView {
    fn arrange(&self, world: &mut World) {
        world.insert_resource(Arranged(self.clone()));
    }
}

/// The test UI's own targets, found by Bevy with no registration call.
#[derive(Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(AppTarget)]
pub enum UiTarget {
    /// Where the "Pause" button is spawned, as a bare position.
    PauseButton,
    /// The node showing exactly this text, as its own widget.
    Showing(String),
}

impl AppTarget for UiTarget {
    fn resolve(&self, world: &World) -> Result<ResolvedTarget, String> {
        match self {
            Self::PauseButton => Ok(ResolvedTarget::at(Vec2::new(150.0, 50.0))),
            Self::Showing(text) => resolve(world, &Target::text(text.clone())),
        }
    }
}

/// The registry the RON scenarios are loaded with: the default one, as
/// every type the tests read is found by Bevy.
pub fn registry() -> Registry {
    Registry::default()
}

/// Fixed seeds for shuffling a schedule: never the clock, so a failure
/// reproduces from the seed it names.
pub const SHUFFLE_SEEDS: [u64; 4] = [1, 2, 3, 0x5eed_cafe];

/// The schedules the crate adds systems to, by name.
pub fn shuffled_schedules() -> [(&'static str, Interned<dyn ScheduleLabel>); 4] {
    [
        ("PreUpdate", PreUpdate.intern()),
        ("Update", Update.intern()),
        ("PostUpdate", PostUpdate.intern()),
        ("Last", Last.intern()),
    ]
}

/// Has `schedule` run its systems in an order shuffled by `seed` wherever
/// nothing orders them, on the single-threaded executor so the order is the
/// seed's alone.
pub fn shuffle(app: &mut App, schedule: Interned<dyn ScheduleLabel>, seed: u64) {
    app.edit_schedule(schedule, |schedule| {
        let mut settings = schedule.get_build_settings();
        settings.shuffle_seed = Some(seed);
        schedule.set_build_settings(settings);
        schedule.set_executor(SingleThreadedExecutor::new());
    });
}
