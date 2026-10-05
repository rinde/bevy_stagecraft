//! What an app tunes about session recording.
//!
//! The app's setup, view and targets are not hooks here: they come from its
//! own reflected types (see [`crate::extension`]), through
//! [`AppSetup::capture`](crate::AppSetup::capture),
//! [`AppView::current`](crate::AppView::current) and
//! [`AppTarget::at`](crate::AppTarget::at).

use bevy::{ecs::world::World, input::keyboard::KeyCode};

use crate::check::suggest::{NoiseFilter, WorldSnapshot};

/// The key that takes a checkpoint while recording, unless the app sets
/// another with [`SessionHooks::checkpoint_key`].
pub const DEFAULT_CHECKPOINT_KEY: KeyCode = KeyCode::F8;

/// The app's settings for session recording. Every one is optional.
///
/// What the recording starts from, the view it names and the targets it
/// makes of clicks on no label are the app's own: see
/// [`AppSetup::capture`](crate::AppSetup::capture),
/// [`AppView::current`](crate::AppView::current) and
/// [`AppTarget::at`](crate::AppTarget::at).
#[derive(Debug, Clone)]
pub struct SessionHooks {
    /// What the snapshots that suggest checks leave out.
    noise: NoiseFilter,
    /// The key that takes a checkpoint.
    checkpoint: KeyCode,
}

impl Default for SessionHooks {
    fn default() -> Self {
        Self {
            noise: NoiseFilter::default(),
            checkpoint: DEFAULT_CHECKPOINT_KEY,
        }
    }
}

impl SessionHooks {
    /// The defaults: Bevy's own types left out, F8 for checkpoints.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets what the world snapshots taken at start and stop leave out, which
    /// decides the checks a recording suggests (see [`crate::check::suggest`]). The
    /// default leaves out Bevy's own types; [`NoiseFilter::nothing`] leaves
    /// out nothing.
    #[must_use]
    pub fn noise_filter(mut self, filter: NoiseFilter) -> Self {
        self.noise = filter;
        self
    }

    /// Sets the key that takes a checkpoint while recording, F8
    /// ([`DEFAULT_CHECKPOINT_KEY`]) unless set. Pick a key the app does not
    /// use: while a session is recorded, the key never reaches the app.
    #[must_use]
    pub const fn checkpoint_key(mut self, key: KeyCode) -> Self {
        self.checkpoint = key;
        self
    }

    /// The key that takes a checkpoint while recording.
    #[must_use]
    pub const fn checkpoint(&self) -> KeyCode {
        self.checkpoint
    }

    /// What the snapshots leave out.
    #[must_use]
    pub const fn noise(&self) -> &NoiseFilter {
        &self.noise
    }

    /// A snapshot of `world`, as the noise filter allows.
    pub(super) fn snapshot(&self, world: &World) -> WorldSnapshot {
        WorldSnapshot::take(world, &self.noise)
    }
}
