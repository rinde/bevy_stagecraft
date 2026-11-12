//! A checkpoint saved as a setup for other scenarios.
//!
//! In the gallery, **Save as setup** (while recording, paused, taken over or
//! finished, but not while a script plays, when the button is disabled)
//! writes the world as it is to a file next to the scenarios, and any
//! scenario can start from it with `setup: Scene("name.scene.ron")` (see
//! [`Setup::Scene`]).
//!
//! The file is Bevy's own world format (a `DynamicWorld`, the dynamic scene of
//! earlier versions) holding the app's reflected resources and entities. What
//! belongs to the test tooling or to the UI is left out, because the scenario
//! brings its own UI (its setup closure, or the app's setup after this one):
//!
//! * every type of Bevy's crates and of this one, as a [`NoiseFilter`]
//!   decides (the default leaves them all out), so no layout, transform or
//!   picking state, and no gallery or overlay state;
//! * every entity that is a UI node, a camera or a window, or that sits below
//!   one, and everything marked [`NotUnderTest`](crate::NotUnderTest);
//! * entities left with no component after that.
//!
//! What is saved needs `#[derive(Reflect)]` and `#[reflect(Component)]` or
//! `#[reflect(Resource)]` (and `Default`, as Bevy's world format needs to
//! read it back), as for checks on resources. A component that refers to
//! another entity is saved with that entity's id: keep both in the file, or
//! the reference dangles when it is loaded.
//!
//! The file is the same on every run of the same app: the resources and the
//! components of each entity are written sorted by type path, the entities by
//! id. Inside one component or resource the order is the value's own: a
//! `HashMap` or `HashSet` field is written in hash order, which can differ
//! between runs, so use an ordered collection (`BTreeMap`, `Vec`) for what is
//! saved if the file must be the same every time.
//!
//! An app with its own save format does not use this: its
//! [`AppSetup::capture`](crate::AppSetup::capture) writes the file and names
//! it in the setup it returns, and [`save_setup`] prefers that. Name what it
//! writes `"{stem}.{extension}"` so that a second save gets another `stem`
//! rather than overwriting the first.

use core::{any::TypeId, fmt};
use std::{
    fs,
    io::{ErrorKind, Write as _},
    path::{Path, PathBuf},
};

use bevy::{
    asset::{AssetPath, AssetServer, LoadFromPath, UntypedHandle},
    camera::Camera,
    ecs::{
        entity::{Entity, EntityHashMap},
        hierarchy::ChildOf,
        reflect::AppTypeRegistry,
        world::World,
    },
    window::Window,
    world_serialization::{
        DynamicWorld, DynamicWorldBuilder, WorldFilter, serde::WorldDeserializer,
    },
};
use serde::de::DeserializeSeed as _;

use crate::{
    check::suggest::NoiseFilter,
    extension::{Setup, capture_setup},
    targets::{is_tooling, is_ui_node},
};

/// The end of a saved world's file name, which makes a `.ron` file a setup
/// and not a scenario: [`crate::ron_files`] skips such files.
pub const SCENE_SUFFIX: &str = ".scene.ron";

/// Saving or loading a world failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveError(String);

impl SaveError {
    /// An error with this message.
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for SaveError {}

/// What a saved world's file is called for `stem`: `"{stem}.scene.ron"`.
#[must_use]
pub fn scene_file_name(stem: &str) -> String {
    format!("{stem}{SCENE_SUFFIX}")
}

/// Whether `name` is the name of a saved world (see [`SCENE_SUFFIX`]).
#[must_use]
pub fn is_scene_file(name: &str) -> bool {
    name.ends_with(SCENE_SUFFIX)
}

/// Whether `entity` belongs to the app's data and not to its UI or to the
/// tooling: it and nothing above it is a UI node, a camera, a window or
/// marked [`NotUnderTest`](crate::NotUnderTest).
fn is_app_data(world: &World, entity: Entity) -> bool {
    if is_tooling(world, entity) {
        return false;
    }
    let mut current = entity;
    loop {
        let Ok(here) = world.get_entity(current) else {
            return false;
        };
        if is_ui_node(world, current) || here.contains::<Camera>() || here.contains::<Window>() {
            return false;
        }
        match here.get::<ChildOf>() {
            Some(parent) => current = parent.parent(),
            None => return true,
        }
    }
}

/// The types in `world`'s registry that `noise` leaves out.
fn ignored_types(world: &World, noise: &NoiseFilter) -> Vec<TypeId> {
    let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
        return Vec::new();
    };
    registry
        .read()
        .iter()
        .filter(|registration| noise.ignores_path(registration.type_info().type_path()))
        .map(bevy::reflect::TypeRegistration::type_id)
        .collect()
}

/// The world's reflected resources and app entities as a [`DynamicWorld`],
/// everything in it in a fixed order.
///
/// # Errors
///
/// [`SaveError`] when the world has no type registry.
fn extract(world: &World, noise: &NoiseFilter) -> Result<DynamicWorld, SaveError> {
    let registry = world
        .get_resource::<AppTypeRegistry>()
        .ok_or_else(|| SaveError::new("the world has no type registry"))?;
    let mut filter = WorldFilter::allow_all();
    for type_id in ignored_types(world, noise) {
        filter = filter.deny_by_id(type_id);
    }
    let entities: Vec<Entity> = world
        .iter_entities()
        .map(|entity| entity.id())
        .filter(|&entity| is_app_data(world, entity))
        .collect();
    let mut saved = DynamicWorldBuilder::from_world(world, &registry.read())
        .with_component_filter(filter.clone())
        .with_resource_filter(filter)
        .extract_entities(entities.into_iter())
        .extract_resources()
        .remove_empty_entities()
        .build();
    let path_of = |value: &dyn bevy::reflect::PartialReflect| {
        value
            .get_represented_type_info()
            .map_or_else(String::new, |info| info.type_path().to_owned())
    };
    saved.resources.sort_by_key(|value| path_of(value.as_ref()));
    for entity in &mut saved.entities {
        entity
            .components
            .sort_by_key(|value| path_of(value.as_ref()));
    }
    saved.entities.sort_by_key(|entity| entity.entity);
    Ok(saved)
}

/// The world's app data as the text of a saved world; see the
/// [module documentation](self) for what that is.
///
/// # Errors
///
/// [`SaveError`] when the world has no type registry or something in it
/// cannot be written (a value whose type does not derive `Reflect` fully).
pub fn scene_ron(world: &World, noise: &NoiseFilter) -> Result<String, SaveError> {
    let saved = extract(world, noise)?;
    let registry = world
        .get_resource::<AppTypeRegistry>()
        .ok_or_else(|| SaveError::new("the world has no type registry"))?;
    saved
        .serialize(&registry.read())
        .map_err(|error| SaveError::new(format!("the world cannot be written: {error}")))
}

/// Puts a saved world's resources and entities into `world`: resources are
/// replaced, entities are spawned anew.
///
/// # Errors
///
/// [`SaveError`] when the text is not a saved world, names a type `world`
/// does not know, or the world has no type registry.
pub fn load_scene_ron(world: &mut World, text: &str) -> Result<(), SaveError> {
    let registry = world
        .get_resource::<AppTypeRegistry>()
        .ok_or_else(|| SaveError::new("the world has no type registry"))?
        .clone();
    let mut loader = Loader(world.get_resource::<AssetServer>().cloned());
    let mut deserializer = ron::de::Deserializer::from_str(text)
        .map_err(|error| SaveError::new(format!("not a saved world: {error}")))?;
    let saved = WorldDeserializer {
        type_registry: &registry.read(),
        load_from_path: &mut loader,
    }
    .deserialize(&mut deserializer)
    .map_err(|error| SaveError::new(format!("not a saved world: {error}")))?;
    saved
        .write_to_world_with(world, &mut EntityHashMap::default(), &registry.read())
        .map_err(|error| SaveError::new(format!("the saved world cannot be loaded: {error}")))
}

/// How a saved world's handles are loaded: by the world's asset server, or,
/// where it has none, as the default handle of their type.
struct Loader(Option<AssetServer>);

impl LoadFromPath for Loader {
    fn load_from_path_erased(
        &mut self,
        type_id: TypeId,
        path: AssetPath<'static>,
    ) -> UntypedHandle {
        self.0.as_mut().map_or_else(
            || UntypedHandle::default_for_type(type_id),
            |server| server.load_from_path_erased(type_id, path),
        )
    }
}

/// Reads the file `file` and loads it as a saved world into `world`: resolved
/// against `base_dir` when it is relative and there is one.
///
/// # Errors
///
/// [`SaveError`] when the file cannot be read (naming the path tried) or does
/// not load; see [`load_scene_ron`].
pub fn load_scene_file(
    world: &mut World,
    base_dir: Option<&Path>,
    file: &str,
) -> Result<(), SaveError> {
    let path = resolve(base_dir, file);
    let text = fs::read_to_string(&path)
        .map_err(|error| SaveError::new(format!("cannot read {}: {error}", path.display())))?;
    load_scene_ron(world, &text)
        .map_err(|error| SaveError::new(format!("{}: {error}", path.display())))
}

/// Where the file `file` of a scenario is: next to the scenario's file when
/// it is relative and the scenario came from a file, else as given.
fn resolve(base_dir: Option<&Path>, file: &str) -> PathBuf {
    let path = Path::new(file);
    match base_dir {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path.to_path_buf(),
    }
}

/// Whether `dir` holds a file a save with this `stem` would write or has
/// written: `"{stem}.{extension}"`, which covers the saved world and what an
/// app's own [`AppSetup::capture`](crate::AppSetup::capture) writes. Not
/// `"{stem}.ron"`: that is the scenario the save is named for, and sits there
/// to begin with.
///
/// # Errors
///
/// A directory that cannot be listed, or an entry that cannot be read, is an
/// error and never "free": an incomplete listing could hide the very file a
/// save would then overwrite.
fn stem_is_taken(dir: &Path, stem: &str) -> Result<bool, SaveError> {
    let prefix = format!("{stem}.");
    let scenario = format!("{stem}.ron");
    let unreadable =
        |error: std::io::Error| SaveError::new(format!("cannot list {}: {error}", dir.display()));
    for entry in fs::read_dir(dir).map_err(unreadable)? {
        let name = entry.map_err(unreadable)?.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name != scenario {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The stem `name` numbered `number`: `name` for 1, then `name-2`, `name-3`.
fn numbered_stem(name: &str, number: u32) -> String {
    if number <= 1 {
        name.to_owned()
    } else {
        format!("{name}-{number}")
    }
}

/// The first number from `from` whose stem nothing saved in `dir` uses yet
/// (see [`stem_is_taken`]).
fn free_stem_number(dir: &Path, name: &str, from: u32) -> Result<u32, SaveError> {
    let mut number = from.max(1);
    while stem_is_taken(dir, &numbered_stem(name, number))? {
        number = number.checked_add(1).ok_or_else(|| {
            SaveError::new(format!("no free name for {name} in {}", dir.display()))
        })?;
    }
    Ok(number)
}

/// Writes `text` as the saved world of `stem` numbered `number` or, when that
/// file exists by now, of the next free number; never over an existing file.
/// Gives the file's name and path.
fn write_new_scene(
    dir: &Path,
    stem: &str,
    mut number: u32,
    text: &str,
) -> Result<(String, PathBuf), SaveError> {
    loop {
        let name = scene_file_name(&numbered_stem(stem, number));
        let path = dir.join(&name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(text.as_bytes()).map_err(|error| {
                    SaveError::new(format!("cannot write {}: {error}", path.display()))
                })?;
                return Ok((name, path));
            }
            // Another save took the name since the check: go on to the next.
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                number = free_stem_number(dir, stem, number.saturating_add(1))?;
            }
            Err(error) => {
                return Err(SaveError::new(format!(
                    "cannot write {}: {error}",
                    path.display()
                )));
            }
        }
    }
}

/// A saved setup: the setup that gets back to the state saved, and the files
/// it wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedSetup {
    /// The setup to put in a script: [`Setup::Scene`] for a saved world, or
    /// what the app's own [`AppSetup::capture`](crate::AppSetup::capture)
    /// made.
    pub setup: Setup,
    /// The saved world's file, when the setup is [`Setup::Scene`].
    pub path: Option<PathBuf>,
}

/// Saves the state of `world` as a setup in `dir`, for scenarios that sit in
/// the same directory.
///
/// The app's own [`AppSetup::capture`](crate::AppSetup::capture) goes first:
/// when it returns a setup, that is the result and the crate writes nothing
/// (the app wrote what it needs in files named `"{stem}.{extension}"`).
/// Otherwise the world is written as `"{stem}.scene.ron"` and the setup is
/// `Scene` of that name. Either way `stem` is numbered (`stem-2`, `stem-3`)
/// when `dir` already holds any file named `"{stem}.{extension}"` (other than
/// the scenario's own `"{stem}.ron"`), so an earlier save is not overwritten.
///
/// # Errors
///
/// [`SaveError`] when the directory or file cannot be written, the world
/// cannot be saved, the directory cannot be listed (a stem is never taken to
/// be free when the listing fails), or the app's capture fails.
///
/// # Races
///
/// The saved world is created exclusively (`create_new`): when another save
/// took the file between the check and the write, the next number is tried, so
/// a scene is never overwritten. The crate cannot do the same for an app's own
/// files, which [`AppSetup::capture`](crate::AppSetup::capture) writes: the
/// contract is that it creates them exclusively too and returns an error
/// rather than replacing a file that exists.
pub fn save_setup(
    world: &mut World,
    dir: &Path,
    stem: &str,
    noise: &NoiseFilter,
) -> Result<SavedSetup, SaveError> {
    fs::create_dir_all(dir)
        .map_err(|error| SaveError::new(format!("cannot create {}: {error}", dir.display())))?;
    let number = free_stem_number(dir, stem, 1)?;
    let captured = capture_setup(world, dir, &numbered_stem(stem, number)).map_err(|error| {
        SaveError::new(format!("the app's setup could not be captured: {error}"))
    })?;
    if let Some(setup) = captured {
        return Ok(SavedSetup { setup, path: None });
    }
    let text = scene_ron(world, noise)?;
    let (name, path) = write_new_scene(dir, stem, number, &text)?;
    Ok(SavedSetup {
        setup: Setup::Scene(name),
        path: Some(path),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "tests may panic")]

    use super::*;

    /// A fresh empty directory for one test.
    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stagecraft-save-unit-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a directory");
        dir
    }

    #[test]
    fn a_directory_that_cannot_be_listed_is_an_error_and_not_a_free_stem() {
        let dir = dir("unlistable");
        let file = dir.join("not-a-directory");
        fs::write(&file, "x").expect("writes");
        let error = stem_is_taken(&file, "a").expect_err("cannot list a file");
        assert!(error.to_string().contains("cannot list"), "{error}");
        assert!(free_stem_number(&file, "a", 1).is_err());
    }

    #[test]
    fn a_taken_stem_is_found_and_the_scenario_itself_does_not_count() {
        let dir = dir("taken");
        fs::write(dir.join("a.ron"), "x").expect("writes");
        assert!(!stem_is_taken(&dir, "a").expect("lists"));
        fs::write(dir.join("a.own"), "x").expect("writes");
        assert!(stem_is_taken(&dir, "a").expect("lists"));
        assert_eq!(free_stem_number(&dir, "a", 1).expect("free"), 2);
    }

    #[test]
    fn a_scene_taken_by_a_racing_save_is_not_overwritten() {
        let dir = dir("race");
        // The check said stem 1 was free; then another save took it.
        fs::write(dir.join("a.scene.ron"), "first").expect("writes");
        let (name, _) = write_new_scene(&dir, "a", 1, "second").expect("writes");
        assert_eq!(name, "a-2.scene.ron");
        assert_eq!(
            fs::read_to_string(dir.join("a.scene.ron")).expect("reads"),
            "first"
        );
        assert_eq!(
            fs::read_to_string(dir.join("a-2.scene.ron")).expect("reads"),
            "second"
        );
    }

    #[test]
    fn a_write_that_fails_for_another_reason_is_an_error() {
        let dir = dir("unwritable");
        let error = write_new_scene(&dir.join("missing"), "a", 1, "x").expect_err("no directory");
        assert!(error.to_string().contains("cannot write"), "{error}");
    }
}
