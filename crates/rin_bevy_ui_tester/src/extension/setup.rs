//! The world a scenario starts from, as the app's own reflected enum.

use core::fmt;
use std::path::Path;

use bevy::{
    ecs::world::World,
    reflect::{FromType, GetTypeRegistration, Reflect, TypePath, TypeRegistry},
};
use serde::{Deserialize, Serialize, de::Error as _};

use super::{AppSetupValue, AppValue, SetupPoint};
use crate::{check::Problem, save::load_scene_file};

/// A setup the app defines, on one reflected enum of its own: what builds
/// the world a scenario starts from (spawning the UI, loading a save).
///
/// Derive `Reflect`, add `#[reflect(AppSetup)]` (with [`ReflectAppSetup`] in
/// scope) and implement this trait; Bevy registers the type by itself. One
/// type per app: a second type with `#[reflect(AppSetup)]` is a load error
/// naming both. A script writes a value as `setup: App(..)`.
///
/// ```
/// use bevy::prelude::*;
/// use rin_bevy_ui_tester::{AppSetup, ReflectAppSetup, Scenario};
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
/// A variant that names a file (a save, say) is the app's to find: the crate
/// passes the name along untouched, so resolve it against the directory the
/// app keeps its scenarios in.
///
/// # Capturing a setup while recording
///
/// When a session recording starts (see [`crate::session`]), the crate calls
/// [`AppSetup::capture`] to turn the app's current state into a setup, which
/// the recorded script then starts from. The gallery's **Save as setup** calls
/// it too, to save a checkpoint as a setup for other scenarios; where it
/// returns `None`, the crate saves the world in Bevy's own format instead (a
/// built-in `Scene(..)` setup, see [`crate::save`]).
pub trait AppSetup {
    /// Builds the scenario's starting state in `world`.
    ///
    /// # Errors
    ///
    /// Why it cannot, such as a save file that is missing; the scenario
    /// then fails without playing a step.
    fn build(&self, world: &mut World) -> Result<(), String>;

    /// The setup that gets back to the state `world` is in now, for a session
    /// recording that starts here. It may write what it needs into `dir`, in
    /// files named `"{stem}.{extension}"` (`stem` is the recorded script's
    /// file name without its extension), so `"{stem}.save.ron"` sits next to
    /// the script. **Save as setup** in the gallery picks a `stem` that no
    /// such file in `dir` has yet (other than the script's own
    /// `"{stem}.ron"`), so a second save does not overwrite the first; a file
    /// named any other way is not looked for. `None`, the default, records no
    /// setup.
    ///
    /// `Self: Sized` keeps the trait usable as `dyn AppSetup`; the recorder
    /// reaches this function through [`ReflectAppSetup`].
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

/// What a recording calls to capture the app's setup: [`AppSetup::capture`],
/// with the setup boxed as reflection.
type CaptureFn = fn(&mut World, &Path, &str) -> Result<Option<Box<dyn Reflect>>, String>;

/// The type data `#[reflect(AppSetup)]` registers: how to see a reflected
/// value as an [`AppSetup`], and how to capture one of its type.
///
/// Written by hand rather than with `#[reflect_trait]`, which can only cast a
/// value to the trait: [`AppSetup::capture`] has no value to cast, so its
/// function pointer is kept here, made for the concrete type when Bevy
/// registers it.
#[derive(Clone)]
pub struct ReflectAppSetup {
    /// Casts a value of the registered type to the trait.
    get: fn(&dyn Reflect) -> Option<&dyn AppSetup>,
    /// The registered type's [`AppSetup::capture`].
    capture: CaptureFn,
}

impl ReflectAppSetup {
    /// `value` as an [`AppSetup`], when it is of the registered type.
    #[must_use]
    pub fn get<'a>(&self, value: &'a dyn Reflect) -> Option<&'a dyn AppSetup> {
        (self.get)(value)
    }

    /// The registered type's [`AppSetup::capture`].
    ///
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

impl<T: AppSetup + Reflect> FromType<T> for ReflectAppSetup {
    fn from_type() -> Self {
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

// By hand: function pointers say nothing worth printing.
impl fmt::Debug for ReflectAppSetup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReflectAppSetup").finish_non_exhaustive()
    }
}

/// What a file that refers to a setup by name, as format versions 1 to 3
/// could, is told.
const NO_NAMED_SETUP: &str = "setups are no longer looked up by name or file: \
    declare the app's setups as one enum with `#[derive(Reflect)]` and \
    `#[reflect(AppSetup)]`, and write one as `App(..)`, such as \
    `setup: App(SavedWorld(\"start.ron\"))`; a world saved from the gallery is \
    `setup: Scene(\"start.scene.ron\")`";

/// Where a scenario's starting state comes from, as a script holds it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub enum Setup {
    /// One of the app's own setups: a value of the type the app marks with
    /// `#[reflect(AppSetup)]` (see [`AppSetup`]). Written as
    /// `App(SavedWorld("start.ron"))`.
    App(AppSetupValue),
    /// A world saved with **Save as setup** in the gallery (or
    /// [`save_setup`](crate::save::save_setup)): the name of its file, which is
    /// next to the script's own when the script was loaded from a file (see
    /// [`Scenario::base_dir`](crate::Scenario::base_dir)). Loading it puts the
    /// saved resources and entities into the world (see [`crate::save`]).
    /// Written as `Scene("start.scene.ron")`.
    Scene(String),
}

/// [`Setup`] as serde's derive reads it, which [`Setup`]'s own impl adds the
/// hint for the old forms to.
#[derive(Deserialize)]
#[serde(remote = "Setup", rename = "Setup")]
enum Written {
    /// See [`Setup::App`].
    App(AppSetupValue),
    /// See [`Setup::Scene`].
    Scene(String),
}

/// Any form but `App(..)` and `Scene(..)` is one of the old ones, so the error
/// says what to write instead.
impl<'de> Deserialize<'de> for Setup {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Written::deserialize(deserializer)
            .map_err(|error| D::Error::custom(format!("{error}: {NO_NAMED_SETUP}")))
    }
}

impl Setup {
    /// One of the app's own setups, for example
    /// `Setup::app(RailSetup::SavedWorld("start.ron".into()))`.
    #[must_use]
    pub fn app<T: AppSetup + Reflect + GetTypeRegistration + TypePath>(setup: T) -> Self {
        Self::App(AppValue::of(setup))
    }

    /// Builds the setup in `world`; a saved world's file is looked for in
    /// `base_dir`.
    pub(crate) fn build(&self, world: &mut World, base_dir: Option<&Path>) -> Result<(), String> {
        match self {
            Self::App(value) => value.build(world),
            Self::Scene(file) => load_scene_file(world, base_dir, file)
                .map_err(|error| format!("the setup `Scene({file:?})` failed: {error}")),
        }
    }

    /// Reads the app's value as its setup type in `types`.
    pub(crate) fn type_against(&mut self, types: &TypeRegistry) -> Result<(), Problem> {
        match self {
            Self::App(value) => value.type_against(types),
            Self::Scene(_) => Ok(()),
        }
    }
}

impl AppValue<SetupPoint> {
    /// Builds the setup in `world`, reading it as the app's type first if
    /// it is not yet.
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
