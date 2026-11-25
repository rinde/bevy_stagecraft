use core::fmt;
use std::path::Path;

use bevy::{
    ecs::world::World,
    reflect::{CreateTypeData, GetTypeRegistration, Reflect, TypePath, TypeRegistry},
};
use serde::{Deserialize, Serialize, de::Error as _};

use super::{AppSetupValue, AppValue, SetupPoint};
use crate::{check::Problem, save::load_scene_file};

/// A setup the app defines, on one reflected enum marked `#[reflect(AppSetup)]`.
///
/// One type per app: a second one is a load error naming both.
///
/// ```
/// use bevy::prelude::*;
/// use bevy_stagecraft::{AppSetup, ReflectAppSetup, Scenario};
///
/// #[derive(Resource)]
/// struct Score(u32);
///
/// #[derive(Reflect, Debug, Clone)]
/// #[reflect(AppSetup)]
/// enum GameSetup {
///     /// A game with this score.
///     ScoreAt(u32),
/// }
///
/// impl AppSetup for GameSetup {
///     fn build(&self, world: &mut World) -> Result<(), String> {
///         match self {
///             Self::ScoreAt(score) => world.insert_resource(Score(*score)),
///         }
///         Ok(())
///     }
/// }
///
/// let scenario = Scenario::new("from three").app_setup(GameSetup::ScoreAt(3));
/// assert!(scenario.to_ron().unwrap().contains("setup: App(ScoreAt(3))"));
/// ```
///
/// A variant that names a file is the app's to find: the crate passes the name along untouched.
///
/// A session recording and the gallery's **Save as setup** call [`AppSetup::capture`]; where it
/// returns `None`, the crate saves the world in Bevy's own format as a `Scene(..)` setup.
pub trait AppSetup {
    /// # Errors
    ///
    /// Why it cannot, such as a save file that is missing; the scenario
    /// then fails without playing a step.
    fn build(&self, world: &mut World) -> Result<(), String>;

    /// The setup that gets back to the state `world` is in now, for a session recording;
    /// `None` records no setup.
    ///
    /// May write files into `dir` named `"{stem}.{extension}"`. Create them exclusively
    /// (`OpenOptions::create_new`): the gallery's choice of `stem` can race with another save.
    ///
    /// `Self: Sized` keeps the trait usable as `dyn AppSetup`.
    ///
    /// # Errors
    ///
    /// Why the state cannot be captured; the recording then does not start.
    fn capture(_world: &mut World, _dir: &Path, _stem: &str) -> Result<Option<Self>, String>
    where
        Self: Sized,
    {
        Ok(None)
    }
}

type CaptureFn = fn(&mut World, &Path, &str) -> Result<Option<Box<dyn Reflect>>, String>;

/// The type data `#[reflect(AppSetup)]` registers.
///
/// By hand: `#[reflect_trait]` can only cast a value, and [`AppSetup::capture`] has no value.
#[derive(Clone)]
pub struct ReflectAppSetup {
    get: fn(&dyn Reflect) -> Option<&dyn AppSetup>,
    capture: CaptureFn,
}

impl ReflectAppSetup {
    #[must_use]
    pub fn get<'a>(&self, value: &'a dyn Reflect) -> Option<&'a dyn AppSetup> {
        (self.get)(value)
    }

    /// # Errors
    ///
    /// What the app's capture says.
    pub fn capture(
        &self,
        world: &mut World,
        dir: &Path,
        stem: &str,
    ) -> Result<Option<Box<dyn Reflect>>, String> {
        (self.capture)(world, dir, stem)
    }
}

impl<T: AppSetup + Reflect> CreateTypeData<T> for ReflectAppSetup {
    fn create_type_data((): ()) -> Self {
        Self {
            get: |value| {
                value
                    .downcast_ref::<T>()
                    .map(|value| -> &dyn AppSetup { value })
            },
            capture: |world, dir, stem| {
                Ok(T::capture(world, dir, stem)?
                    .map(|setup| -> Box<dyn Reflect> { Box::new(setup) }))
            },
        }
    }
}

impl fmt::Debug for ReflectAppSetup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReflectAppSetup").finish_non_exhaustive()
    }
}

const NO_NAMED_SETUP: &str = "setups are no longer looked up by name or file: \
    declare the app's setups as one enum with `#[derive(Reflect)]` and \
    `#[reflect(AppSetup)]`, and write one as `App(..)`, such as \
    `setup: App(SavedWorld(\"start.ron\"))`; a world saved from the gallery is \
    `setup: Scene(\"start.scene.ron\")`";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub enum Setup {
    App(AppSetupValue),
    /// A world saved with **Save as setup** in the gallery, by file name: next to the script's
    /// own when the script was loaded from a file.
    Scene(String),
}

#[derive(Deserialize)]
#[serde(remote = "Setup", rename = "Setup")]
enum Written {
    App(AppSetupValue),
    Scene(String),
}

/// Refuses the old forms with a hint to write `App(..)` or `Scene(..)`.
impl<'de> Deserialize<'de> for Setup {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Written::deserialize(deserializer)
            .map_err(|error| D::Error::custom(format!("{error}: {NO_NAMED_SETUP}")))
    }
}

impl Setup {
    #[must_use]
    pub fn app<T: AppSetup + Reflect + GetTypeRegistration + TypePath>(setup: T) -> Self {
        Self::App(AppValue::of(setup))
    }

    pub(crate) fn build(&self, world: &mut World, base_dir: Option<&Path>) -> Result<(), String> {
        match self {
            Self::App(value) => value.build(world),
            Self::Scene(file) => load_scene_file(world, base_dir, file)
                .map_err(|error| format!("the setup `Scene({file:?})` failed: {error}")),
        }
    }

    pub(crate) fn type_against(&mut self, types: &TypeRegistry) -> Result<(), Problem> {
        match self {
            Self::App(value) => value.type_against(types),
            Self::Scene(_) => Ok(()),
        }
    }
}

impl AppValue<SetupPoint> {
    pub(crate) fn build(&self, world: &mut World) -> Result<(), String> {
        let typed = self.typed_in(world)?.into_owned();
        let (value, data) = typed
            .typed_parts()
            .ok_or_else(|| format!("the app setup `{self}` has no type"))?;
        let setup = data
            .get(value)
            .ok_or_else(|| format!("`{self}` is not an `AppSetup`"))?;
        setup
            .build(world)
            .map_err(|error| format!("the setup `{self}` failed: {error}"))
    }
}
