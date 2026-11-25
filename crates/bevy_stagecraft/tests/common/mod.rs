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

#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
pub struct Activations(pub u32);

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

#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(AppCheck)]
pub enum UiCheck {
    ActivatedOnce,
    Activated(u32),
    Found(Target),
    ActivatedBetween { at_least: u32, at_most: u32 },
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

#[derive(Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(AppSetup)]
pub enum UiSetup {
    PlainUi,
    Activated(u32),
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

#[derive(Resource, Debug, Clone, PartialEq)]
pub struct Arranged(pub UiView);

#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(AppView)]
pub enum UiView {
    Plain,
    Zoomed(f32),
}

impl AppView for UiView {
    fn arrange(&self, world: &mut World) {
        world.insert_resource(Arranged(self.clone()));
    }
}

#[derive(Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(AppTarget)]
pub enum UiTarget {
    PauseButton,
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

pub fn registry() -> Registry {
    Registry::default()
}

pub const SHUFFLE_SEEDS: [u64; 4] = [1, 2, 3, 0x5eed_cafe];

pub fn shuffled_schedules() -> [(&'static str, Interned<dyn ScheduleLabel>); 4] {
    [
        ("PreUpdate", PreUpdate.intern()),
        ("Update", Update.intern()),
        ("PostUpdate", PostUpdate.intern()),
        ("Last", Last.intern()),
    ]
}

pub fn shuffle(app: &mut App, schedule: Interned<dyn ScheduleLabel>, seed: u64) {
    app.edit_schedule(schedule, |schedule| {
        let mut settings = schedule.get_build_settings();
        settings.shuffle_seed = Some(seed);
        schedule.set_build_settings(settings);
        schedule.set_executor(SingleThreadedExecutor::new());
    });
}
