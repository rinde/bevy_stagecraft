//! Saved types need `Reflect`, `Default` and `#[reflect(Component)]` or
//! `#[reflect(Resource)]`; use ordered collections, as hash order differs between runs.

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

/// Makes a `.ron` file a setup, not a scenario: [`crate::ron_files`] skips it.
pub const SCENE_SUFFIX: &str = ".scene.ron";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveError(String);

impl SaveError {
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

#[must_use]
pub fn scene_file_name(stem: &str) -> String {
    format!("{stem}{SCENE_SUFFIX}")
}

#[must_use]
pub fn is_scene_file(name: &str) -> bool {
    name.ends_with(SCENE_SUFFIX)
}

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

/// # Errors
///
/// [`SaveError`] when the world has no type registry or a value cannot be written.
pub fn scene_ron(world: &World, noise: &NoiseFilter) -> Result<String, SaveError> {
    let saved = extract(world, noise)?;
    let registry = world
        .get_resource::<AppTypeRegistry>()
        .ok_or_else(|| SaveError::new("the world has no type registry"))?;
    saved
        .serialize(&registry.read())
        .map_err(|error| SaveError::new(format!("the world cannot be written: {error}")))
}

/// Resources are replaced, entities are spawned anew.
///
/// # Errors
///
/// [`SaveError`] when the text is not a saved world, names a type `world` does not
/// know, or the world has no type registry.
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

/// # Errors
///
/// [`SaveError`] when the file cannot be read or does not load.
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

fn resolve(base_dir: Option<&Path>, file: &str) -> PathBuf {
    let path = Path::new(file);
    match base_dir {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path.to_path_buf(),
    }
}

/// Not `"{stem}.ron"`: the scenario the save is named for sits there already. A
/// listing error is never "free", as it could hide a file the save would overwrite.
fn stem_is_taken(dir: &Path, stem: &str) -> Result<bool, SaveError> {
    stem_is_used(dir, stem, false)
}

/// A recorded session counts `"{stem}.ron"`: it has yet to write that file and
/// must not replace one.
fn stem_is_used(dir: &Path, stem: &str, scenario_counts: bool) -> Result<bool, SaveError> {
    let prefix = format!("{stem}.");
    let scenario = format!("{stem}.ron");
    let unreadable =
        |error: std::io::Error| SaveError::new(format!("cannot list {}: {error}", dir.display()));
    for entry in fs::read_dir(dir).map_err(unreadable)? {
        let name = entry.map_err(unreadable)?.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && (scenario_counts || name != scenario) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn numbered_stem(name: &str, number: u32) -> String {
    if number <= 1 {
        name.to_owned()
    } else {
        format!("{name}-{number}")
    }
}

fn free_stem_number(dir: &Path, name: &str, from: u32) -> Result<u32, SaveError> {
    let mut number = from.max(1);
    while stem_is_taken(dir, &numbered_stem(name, number))? {
        number = number.checked_add(1).ok_or_else(|| {
            SaveError::new(format!("no free name for {name} in {}", dir.display()))
        })?;
    }
    Ok(number)
}

/// Counts the scenario file and orphaned captures, so neither is overwritten.
///
/// # Errors
///
/// A directory that cannot be listed is an error and never a free stem.
pub(crate) fn free_session_stem(dir: &Path, name: &str) -> Result<String, SaveError> {
    let mut number = 1_u32;
    loop {
        let stem = numbered_stem(name, number);
        if !stem_is_used(dir, &stem, true)? {
            return Ok(stem);
        }
        number = number.checked_add(1).ok_or_else(|| {
            SaveError::new(format!("no free name for {name} in {}", dir.display()))
        })?;
    }
}

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

#[derive(Debug, Clone, PartialEq)]
pub struct SavedSetup {
    pub setup: Setup,
    pub path: Option<PathBuf>,
}

/// The app's own [`AppSetup::capture`](crate::AppSetup::capture) goes first; if it
/// returns a setup the crate writes nothing. `stem` is numbered when `dir` holds
/// `"{stem}.{extension}"`.
///
/// # Errors
///
/// [`SaveError`] when the directory or file cannot be written, the world cannot
/// be saved, the directory cannot be listed, or the app's capture fails.
///
/// # Races
///
/// The world is created exclusively. The app's capture must create its files
/// exclusively too, and error rather than replace one.
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
