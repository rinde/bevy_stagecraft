//! The scenarios a gallery offers, grouped.

use bevy::ecs::resource::Resource;

use crate::scenario::Scenario;

/// One scenario in the catalogue, with the sentence shown beside it.
#[derive(Debug, Clone)]
pub struct GalleryEntry {
    /// The scenario.
    pub scenario: Scenario,
    /// What it shows, in a sentence.
    pub description: String,
}

impl GalleryEntry {
    /// An entry with no description.
    #[must_use]
    pub const fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            description: String::new(),
        }
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
}

impl Gallery {
    /// An empty catalogue.
    #[must_use]
    pub const fn new() -> Self {
        Self { groups: Vec::new() }
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
