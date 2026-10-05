//! What an app plugs into session recording.

use alloc::sync::Arc;
use core::fmt;
use std::path::Path;

use bevy::{ecs::world::World, input::keyboard::KeyCode, math::Vec2};

use crate::{
    check::suggest::{NoiseFilter, WorldSnapshot},
    script::Setup,
    targets::Target,
};

/// What the setup hook does: write what is needed to get back to the current
/// state into the directory (names starting with the stem), and say how the
/// script refers to it.
type SetupFn = dyn Fn(&mut World, &Path, &str) -> Result<Option<Setup>, String> + Send + Sync;

/// What the view hook does.
type ViewFn = dyn Fn(&World) -> Option<String> + Send + Sync;

/// What the target hook does.
type TargetFn = dyn Fn(&World, Vec2) -> Option<Target> + Send + Sync;

/// The key that takes a checkpoint while recording, unless the app sets
/// another with [`SessionHooks::checkpoint_key`].
pub const DEFAULT_CHECKPOINT_KEY: KeyCode = KeyCode::F8;

/// The app's side of session recording. Every hook is optional.
#[derive(Clone)]
pub struct SessionHooks {
    /// Captures the setup.
    setup: Option<Arc<SetupFn>>,
    /// Names the view.
    view: Option<Arc<ViewFn>>,
    /// Resolves a position to an app-defined target.
    target: Option<Arc<TargetFn>>,
    /// What the snapshots that suggest checks leave out.
    noise: NoiseFilter,
    /// The key that takes a checkpoint.
    checkpoint: KeyCode,
}

impl Default for SessionHooks {
    fn default() -> Self {
        Self {
            setup: None,
            view: None,
            target: None,
            noise: NoiseFilter::default(),
            checkpoint: DEFAULT_CHECKPOINT_KEY,
        }
    }
}

impl fmt::Debug for SessionHooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionHooks")
            .field("capture_setup", &self.setup.is_some())
            .field("capture_view", &self.view.is_some())
            .field("resolve_target", &self.target.is_some())
            .field("noise_filter", &self.noise)
            .field("checkpoint_key", &self.checkpoint)
            .finish()
    }
}

impl SessionHooks {
    /// No hooks.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs when recording starts, to capture the app's setup. It gets the
    /// directory the script goes to and the script's file name without its
    /// extension, so it can write `"{stem}.save.ron"` next to it, and returns
    /// what the script's `setup` should say: for such a file,
    /// `Some(Setup::File(format!("{stem}.save.ron")))`.
    ///
    /// # Errors
    ///
    /// Return a message when the setup cannot be captured; recording then
    /// does not start.
    #[must_use]
    pub fn capture_setup(
        mut self,
        hook: impl Fn(&mut World, &Path, &str) -> Result<Option<Setup>, String> + Send + Sync + 'static,
    ) -> Self {
        self.setup = Some(Arc::new(hook));
        self
    }

    /// Names the view the app is in when recording starts, as a script's
    /// `view` does: it must match a view registered in
    /// [`crate::Registry::views`] when the file is loaded.
    #[must_use]
    pub fn capture_view(
        mut self,
        hook: impl Fn(&World) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.view = Some(Arc::new(hook));
        self
    }

    /// Offered every click that landed on no labelled UI node: return the
    /// app's own target for the position, or `None` to keep `screen(..)`.
    #[must_use]
    pub fn resolve_target(
        mut self,
        hook: impl Fn(&World, Vec2) -> Option<Target> + Send + Sync + 'static,
    ) -> Self {
        self.target = Some(Arc::new(hook));
        self
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

    /// Runs the setup hook.
    pub(super) fn run_capture_setup(
        &self,
        world: &mut World,
        dir: &Path,
        stem: &str,
    ) -> Result<Option<Setup>, String> {
        self.setup
            .as_ref()
            .map_or(Ok(None), |hook| hook(world, dir, stem))
    }

    /// Runs the view hook.
    pub(super) fn run_capture_view(&self, world: &World) -> Option<String> {
        self.view.as_ref().and_then(|hook| hook(world))
    }

    /// Runs the target hook.
    pub(super) fn run_resolve_target(&self, world: &World, position: Vec2) -> Option<Target> {
        self.target.as_ref().and_then(|hook| hook(world, position))
    }

    /// A snapshot of `world`, as the noise filter allows.
    pub(super) fn snapshot(&self, world: &World) -> WorldSnapshot {
        WorldSnapshot::take(world, &self.noise)
    }
}
