//! What an app tunes about session recording.

use bevy::{ecs::world::World, input::keyboard::KeyCode};

use crate::check::suggest::{NoiseFilter, WorldSnapshot};

pub const DEFAULT_CHECKPOINT_KEY: KeyCode = KeyCode::F8;

/// The app's settings for session recording.
#[derive(Debug, Clone)]
pub struct SessionHooks {
    noise: NoiseFilter,
    checkpoint: KeyCode,
    ask_names: bool,
}

impl Default for SessionHooks {
    fn default() -> Self {
        Self {
            noise: NoiseFilter::default(),
            checkpoint: DEFAULT_CHECKPOINT_KEY,
            ask_names: true,
        }
    }
}

impl SessionHooks {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets what the start and stop snapshots leave out, which decides the checks suggested.
    #[must_use]
    pub fn noise_filter(mut self, filter: NoiseFilter) -> Self {
        self.noise = filter;
        self
    }

    /// Sets the checkpoint key. It never reaches the app while recording, so pick an unused one.
    #[must_use]
    pub const fn checkpoint_key(mut self, key: KeyCode) -> Self {
        self.checkpoint = key;
        self
    }

    /// Sets whether the checkpoint key asks for a name, capturing typed keys meanwhile.
    ///
    /// Only [`CursorOverlayPlugin`](crate::overlay::CursorOverlayPlugin) draws the prompt;
    /// without it, pass `false`.
    #[must_use]
    pub const fn checkpoint_names(mut self, ask: bool) -> Self {
        self.ask_names = ask;
        self
    }

    #[must_use]
    pub const fn asks_names(&self) -> bool {
        self.ask_names
    }

    #[must_use]
    pub const fn checkpoint(&self) -> KeyCode {
        self.checkpoint
    }

    #[must_use]
    pub const fn noise(&self) -> &NoiseFilter {
        &self.noise
    }

    pub(super) fn snapshot(&self, world: &World) -> WorldSnapshot {
        WorldSnapshot::take(world, &self.noise)
    }
}
