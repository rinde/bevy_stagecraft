//! The scenarios a gallery offers, grouped.

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

/// How many updates pass between looks at the watched files.
pub const WATCH_INTERVAL: u32 = 30;

/// What a file looked like when last read: when it was written and how long
/// it was, which together tell an edit from no edit without reading it.
type FileStamp = (Option<SystemTime>, u64);

/// The stamp of the file at `path`, or `None` if it cannot be looked at.
fn stamp_of(path: &Path) -> Option<FileStamp> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok(), meta.len()))
}

/// What the gallery does to a scenario it has read from a file, such as
/// sizing its window, so a reload gives the same result as the first load.
#[derive(Clone)]
struct Prepare(Arc<dyn Fn(Scenario) -> Scenario + Send + Sync>);

impl core::fmt::Debug for Prepare {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Prepare(..)")
    }
}

/// A directory whose `.ron` files make up one group of the catalogue.
#[derive(Debug, Clone)]
struct WatchedDir {
    /// Where the group is among the groups.
    group: usize,
    /// The directory.
    dir: PathBuf,
}

/// One scenario in the catalogue, with the sentence shown beside it.
#[derive(Debug, Clone)]
pub struct GalleryEntry {
    /// The scenario.
    pub scenario: Scenario,
    /// What it shows, in a sentence.
    pub description: String,
    /// The file it was read from, which the gallery reads again when it
    /// changes (see [`Gallery::directory`]).
    source: Option<PathBuf>,
    /// Why the file does not load, where it does not. The scenario is then
    /// the last version that did, if there was one.
    error: Option<String>,
    /// Whether no version of the file has loaded yet, so the scenario is only
    /// a placeholder that cannot be played.
    unloaded: bool,
    /// How the file looked when it was last read.
    stamp: Option<FileStamp>,
}

impl GalleryEntry {
    /// An entry with no description.
    #[must_use]
    pub const fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            description: String::new(),
            source: None,
            error: None,
            unloaded: false,
            stamp: None,
        }
    }

    /// An entry read from `path`. A file that does not load still makes an
    /// entry, which shows [`GalleryEntry::error`] and cannot be played.
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

    /// Reads the file again. A scenario that loads replaces the one held; a
    /// file that does not leaves it as it was and keeps the reason.
    fn reload(&mut self, registry: &Registry, prepare: Option<&Prepare>) {
        let Some(path) = self.source.clone() else {
            return;
        };
        self.stamp = stamp_of(&path);
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

    /// The file this entry was read from, if it was.
    #[must_use]
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// Why the entry's file does not load now, if it does not. The scenario
    /// is then the last version that did.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether the file has never loaded, so there is nothing to play yet.
    #[must_use]
    pub const fn is_unloaded(&self) -> bool {
        self.unloaded
    }

    /// Sets the description.
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

/// A named run of entries.
#[derive(Debug, Clone)]
pub struct GalleryGroup {
    /// The group's heading.
    pub name: String,
    /// Its entries, in the order shown.
    pub entries: Vec<GalleryEntry>,
}

/// The catalogue: groups of scenarios. An entry is addressed by its index
/// counted across all groups in order, which is how [`crate::GalleryAction`]
/// names one.
#[derive(Resource, Debug, Clone, Default)]
pub struct Gallery {
    /// The groups in the order shown.
    groups: Vec<GalleryGroup>,
    /// The environments an entry can be played in, one chosen at a time. Empty
    /// means only as written.
    variants: Vec<Variant>,
    /// The directories whose files are read again when they change.
    watched: Vec<WatchedDir>,
    /// The registry files are read against.
    registry: Option<Registry>,
    /// What is done to a scenario read from a file.
    prepare: Option<Prepare>,
    /// Counts changes to the catalogue itself, so what shows it is rebuilt.
    revision: u32,
}

impl Gallery {
    /// An empty catalogue.
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

    /// Adds a group of the `.ron` scenarios in `dir` (not the `*.scene.ron`
    /// worlds), read against `registry`, and **watches** the directory: while
    /// the gallery runs, a file that is saved is read again and replaces its
    /// entry, a new file is added to the group, and the selected scenario
    /// restarts if its file changed. A file that does not load shows its
    /// error in the catalogue and leaves the last version that loaded in
    /// place (one that never loaded cannot be played). An unreadable
    /// directory gives an empty group, and a file that is deleted keeps its
    /// entry, marked.
    ///
    /// Files are looked at every [`WATCH_INTERVAL`] updates, so this needs no
    /// file-watching dependency. Call [`Gallery::prepare_files`] first if the
    /// scenarios need anything done to them.
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

    /// Sets what is done to a scenario read from a file by
    /// [`Gallery::directory`], such as giving it a window size, on every
    /// reload as on the first load. Set it before the directories it applies
    /// to.
    #[must_use]
    pub fn prepare_files(
        mut self,
        prepare: impl Fn(Scenario) -> Scenario + Send + Sync + 'static,
    ) -> Self {
        self.prepare = Some(Prepare(Arc::new(prepare)));
        self
    }

    /// Whether any directory is watched.
    #[must_use]
    pub(crate) const fn is_watching(&self) -> bool {
        !self.watched.is_empty()
    }

    /// Counts changes to the catalogue itself.
    #[must_use]
    pub(crate) const fn revision(&self) -> u32 {
        self.revision
    }

    /// Looks at the watched directories and reads again what changed.
    pub(crate) fn poll_files(&mut self) -> Vec<Reload> {
        let mut reloads = Vec::new();
        let Some(registry) = self.registry.clone() else {
            return reloads;
        };
        let prepare = self.prepare.clone();
        for watched in self.watched.clone() {
            let files = ron_files(&watched.dir).unwrap_or_default();
            let before: usize = self
                .groups
                .iter()
                .take(watched.group)
                .map(|group| group.entries.len())
                .sum();
            let Some(group) = self.groups.get_mut(watched.group) else {
                continue;
            };
            for (at, entry) in group.entries.iter_mut().enumerate() {
                let Some(path) = entry.source.clone() else {
                    continue;
                };
                if !files.contains(&path) {
                    let gone = "The file is gone".to_owned();
                    if entry.error.as_ref() != Some(&gone) {
                        entry.error = Some(gone);
                        reloads.push(Reload::Changed(before + at));
                    }
                } else if stamp_of(&path) != entry.stamp {
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

    /// Adds a group.
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

    /// Offers every entry in these environments (window size, scale factor,
    /// theme), with a picker to choose one; a run of the whole catalogue plays
    /// every entry in each. The first is chosen to begin with. List
    /// [`Variant::default`] to keep the entries as written among them.
    ///
    /// Name the variants differently ([`Variant::new`]): two with one name
    /// give rows that cannot be told apart.
    #[must_use]
    pub fn variants(mut self, variants: impl IntoIterator<Item = Variant>) -> Self {
        self.variants = variants.into_iter().collect();
        self
    }

    /// The variant at `index`, if the catalogue lists variants and has one
    /// there. Without any, entries are played as written.
    #[must_use]
    pub fn variant(&self, index: usize) -> Option<&Variant> {
        self.variants.get(index)
    }

    /// The variants offered, in order.
    #[must_use]
    pub fn variant_list(&self) -> &[Variant] {
        &self.variants
    }

    /// How many ways there are to play an entry: the variants listed, or one
    /// (as written) when there are none.
    #[must_use]
    pub fn variant_count(&self) -> usize {
        self.variants.len().max(1)
    }

    /// Every tag the entries' scenarios carry, sorted and without repeats:
    /// what the gallery's tag row offers.
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

    /// The groups, in order.
    #[must_use]
    pub fn groups(&self) -> &[GalleryGroup] {
        &self.groups
    }

    /// Every entry across all groups, in order, with its index.
    pub fn entries(&self) -> impl Iterator<Item = (usize, &GalleryEntry)> {
        self.groups
            .iter()
            .flat_map(|group| group.entries.iter())
            .enumerate()
    }

    /// The entry at `index`, counted across all groups.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&GalleryEntry> {
        self.entries()
            .find_map(|(at, entry)| (at == index).then_some(entry))
    }

    /// The entry at `index`, counted across all groups, to change.
    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut GalleryEntry> {
        self.groups
            .iter_mut()
            .flat_map(|group| group.entries.iter_mut())
            .nth(index)
    }

    /// How many entries there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.groups.iter().map(|group| group.entries.len()).sum()
    }

    /// Whether there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
