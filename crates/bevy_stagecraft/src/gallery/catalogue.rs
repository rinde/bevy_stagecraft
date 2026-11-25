//! Watched directories are polled every [`WATCH_INTERVAL`] updates by
//! modification time and length.

use alloc::sync::Arc;
use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use bevy::ecs::resource::Resource;

use super::Reload;
use crate::{
    files::{load_ron_file, ron_files},
    scenario::Scenario,
    script::Registry,
    variant::Variant,
};

pub const WATCH_INTERVAL: u32 = 30;

type FileStamp = (Option<SystemTime>, u64);

fn stamp_of(path: &Path) -> Option<FileStamp> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok(), meta.len()))
}

#[derive(Clone)]
struct Prepare(Arc<dyn Fn(Scenario) -> Scenario + Send + Sync>);

impl core::fmt::Debug for Prepare {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Prepare(..)")
    }
}

#[derive(Debug, Clone)]
struct WatchedDir {
    group: usize,
    dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct GalleryEntry {
    pub scenario: Scenario,
    pub description: String,
    source: Option<PathBuf>,
    error: Option<String>,
    /// A placeholder that cannot be played: no version of the file has loaded.
    unloaded: bool,
    stamp: Option<FileStamp>,
    /// `error` comes from the directory scan, not from loading the file, so the
    /// next scan that finds the file reads it again whatever its stamp.
    scan_error: bool,
}

impl GalleryEntry {
    #[must_use]
    pub const fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            description: String::new(),
            source: None,
            error: None,
            unloaded: false,
            stamp: None,
            scan_error: false,
        }
    }

    fn from_file(path: &Path, registry: &Registry, prepare: Option<&Prepare>) -> Self {
        let name = path
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        let mut entry = Self::new(Scenario::new(name));
        entry.source = Some(path.to_path_buf());
        entry.unloaded = true;
        entry.reload(registry, prepare);
        entry
    }

    /// A file that fails to load keeps the last good scenario.
    fn reload(&mut self, registry: &Registry, prepare: Option<&Prepare>) {
        let Some(path) = self.source.clone() else {
            return;
        };
        self.stamp = stamp_of(&path);
        self.scan_error = false;
        match load_ron_file(&path, registry) {
            Ok(scenario) => {
                self.scenario = match prepare {
                    Some(prepare) => (prepare.0)(scenario),
                    None => scenario,
                };
                self.error = None;
                self.unloaded = false;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    #[must_use]
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// Why the file does not load; the scenario is then the last version that did.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    #[must_use]
    pub const fn is_unloaded(&self) -> bool {
        self.unloaded
    }

    #[must_use]
    pub fn described(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}

impl From<Scenario> for GalleryEntry {
    fn from(scenario: Scenario) -> Self {
        Self::new(scenario)
    }
}

#[derive(Debug, Clone)]
pub struct GalleryGroup {
    pub name: String,
    pub entries: Vec<GalleryEntry>,
}

/// The catalog: groups of scenarios. [`crate::GalleryAction`] addresses an
/// entry by its index counted across all groups.
#[derive(Resource, Debug, Clone, Default)]
pub struct Gallery {
    groups: Vec<GalleryGroup>,
    /// Empty means entries are played as written.
    variants: Vec<Variant>,
    watched: Vec<WatchedDir>,
    registry: Option<Registry>,
    prepare: Option<Prepare>,
    /// Bumped on each change to the catalog so the UI is rebuilt.
    revision: u32,
}

impl Gallery {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            groups: Vec::new(),
            variants: Vec::new(),
            watched: Vec::new(),
            registry: None,
            prepare: None,
            revision: 0,
        }
    }

    /// Watches `dir` (not `*.scene.ron`); a deleted file or failed scan keeps
    /// its entries, marked. Call [`Gallery::prepare_files`] first.
    #[must_use]
    pub fn directory(
        mut self,
        name: impl Into<String>,
        dir: impl Into<PathBuf>,
        registry: &Registry,
    ) -> Self {
        let dir = dir.into();
        let entries: Vec<GalleryEntry> = ron_files(&dir)
            .unwrap_or_default()
            .iter()
            .map(|path| GalleryEntry::from_file(path, registry, self.prepare.as_ref()))
            .collect();
        self.watched.push(WatchedDir {
            group: self.groups.len(),
            dir,
        });
        self.registry = Some(registry.clone());
        self.groups.push(GalleryGroup {
            name: name.into(),
            entries,
        });
        self
    }

    /// Applies to every scenario [`Gallery::directory`] reads, on reload too.
    /// Set it before the directories.
    #[must_use]
    pub fn prepare_files(
        mut self,
        prepare: impl Fn(Scenario) -> Scenario + Send + Sync + 'static,
    ) -> Self {
        self.prepare = Some(Prepare(Arc::new(prepare)));
        self
    }

    #[must_use]
    pub(crate) const fn is_watching(&self) -> bool {
        !self.watched.is_empty()
    }

    #[must_use]
    pub(crate) const fn revision(&self) -> u32 {
        self.revision
    }

    pub(crate) fn poll_files(&mut self) -> Vec<Reload> {
        let mut reloads = Vec::new();
        let Some(registry) = self.registry.clone() else {
            return reloads;
        };
        let prepare = self.prepare.clone();
        for watched in self.watched.clone() {
            let scan = ron_files(&watched.dir);
            let before: usize = self
                .groups
                .iter()
                .take(watched.group)
                .map(|group| group.entries.len())
                .sum();
            let Some(group) = self.groups.get_mut(watched.group) else {
                continue;
            };
            let files = match scan {
                Ok(files) => files,
                Err(error) => {
                    // A failed scan says nothing about the files.
                    let message = format!("The directory could not be read: {error}");
                    for (at, entry) in group.entries.iter_mut().enumerate() {
                        if entry.source.is_some() && entry.error.as_ref() != Some(&message) {
                            entry.error = Some(message.clone());
                            entry.scan_error = true;
                            reloads.push(Reload::Changed(before + at));
                        }
                    }
                    continue;
                }
            };
            for (at, entry) in group.entries.iter_mut().enumerate() {
                let Some(path) = entry.source.clone() else {
                    continue;
                };
                if !files.contains(&path) {
                    let gone = "The file is gone".to_owned();
                    if entry.error.as_ref() != Some(&gone) {
                        entry.error = Some(gone);
                        entry.scan_error = true;
                        reloads.push(Reload::Changed(before + at));
                    }
                } else if entry.scan_error || stamp_of(&path) != entry.stamp {
                    entry.reload(&registry, prepare.as_ref());
                    reloads.push(Reload::Changed(before + at));
                }
            }
            for path in files {
                let known = group
                    .entries
                    .iter()
                    .any(|entry| entry.source.as_deref() == Some(path.as_path()));
                if !known {
                    let at = before + group.entries.len();
                    group
                        .entries
                        .push(GalleryEntry::from_file(&path, &registry, prepare.as_ref()));
                    reloads.push(Reload::Added(at));
                }
            }
        }
        if !reloads.is_empty() {
            self.revision = self.revision.wrapping_add(1);
        }
        reloads
    }

    #[must_use]
    pub fn group<E: Into<GalleryEntry>>(
        mut self,
        name: impl Into<String>,
        entries: impl IntoIterator<Item = E>,
    ) -> Self {
        self.groups.push(GalleryGroup {
            name: name.into(),
            entries: entries.into_iter().map(Into::into).collect(),
        });
        self
    }

    /// List [`Variant::default`] to keep the entries as written. Name variants
    /// differently ([`Variant::new`]): equal names give identical rows.
    #[must_use]
    pub fn variants(mut self, variants: impl IntoIterator<Item = Variant>) -> Self {
        self.variants = variants.into_iter().collect();
        self
    }

    #[must_use]
    pub fn variant(&self, index: usize) -> Option<&Variant> {
        self.variants.get(index)
    }

    #[must_use]
    pub fn variant_list(&self) -> &[Variant] {
        &self.variants
    }

    /// One when there are no variants.
    #[must_use]
    pub fn variant_count(&self) -> usize {
        self.variants.len().max(1)
    }

    #[must_use]
    pub fn tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .entries()
            .flat_map(|(_, entry)| entry.scenario.tag_names().iter().cloned())
            .collect();
        tags.sort();
        tags.dedup();
        tags
    }

    #[must_use]
    pub fn groups(&self) -> &[GalleryGroup] {
        &self.groups
    }

    /// Pairs each entry with its index counted across all groups.
    pub fn entries(&self) -> impl Iterator<Item = (usize, &GalleryEntry)> {
        self.groups
            .iter()
            .flat_map(|group| group.entries.iter())
            .enumerate()
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&GalleryEntry> {
        self.entries()
            .find_map(|(at, entry)| (at == index).then_some(entry))
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut GalleryEntry> {
        self.groups
            .iter_mut()
            .flat_map(|group| group.entries.iter_mut())
            .nth(index)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.groups.iter().map(|group| group.entries.len()).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "tests may panic")]

    use super::*;
    use crate::Step;

    #[test]
    fn a_file_restored_with_the_same_stamp_is_reloaded_and_reported() {
        let dir =
            std::env::temp_dir().join(format!("rin-catalogue-restored-{}", std::process::id()));
        drop(fs::remove_dir_all(&dir));
        fs::create_dir_all(&dir).expect("a temp directory");
        let file = dir.join("a.ron");
        let away = dir.join("elsewhere");
        let text = Scenario::new("a")
            .steps([Step::wait(1)])
            .to_ron()
            .expect("a scenario exports");
        fs::write(&file, text).expect("written");
        let mut gallery = Gallery::new().directory("Files", &dir, &Registry::default());
        let revision = gallery.revision();

        fs::rename(&file, &away).expect("moved away");
        let reloads = gallery.poll_files();
        assert!(matches!(reloads.as_slice(), [Reload::Changed(0)]));
        assert_eq!(
            gallery.get(0).and_then(GalleryEntry::error),
            Some("The file is gone")
        );
        assert_ne!(gallery.revision(), revision, "the catalogue is told");

        // A rename keeps the modification time and length: the same stamp.
        fs::rename(&away, &file).expect("moved back");
        let revision = gallery.revision();
        let reloads = gallery.poll_files();
        assert!(matches!(reloads.as_slice(), [Reload::Changed(0)]));
        assert!(gallery.get(0).is_some_and(|entry| entry.error().is_none()));
        assert_ne!(gallery.revision(), revision, "the catalogue is told again");
        drop(fs::remove_dir_all(&dir));
    }
}
