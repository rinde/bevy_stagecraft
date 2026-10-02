//! What an app plugs into session recording.

use alloc::sync::Arc;
use core::fmt;
use std::path::Path;

use bevy::{ecs::world::World, math::Vec2};

use crate::{script::Setup, script::StateProbe, targets::Target};

/// What the setup hook does: write what is needed to get back to the current
/// state into the directory (names starting with the stem), and say how the
/// script refers to it.
type SetupFn = dyn Fn(&mut World, &Path, &str) -> Result<Option<Setup>, String> + Send + Sync;

/// What the view hook does.
type ViewFn = dyn Fn(&World) -> Option<String> + Send + Sync;

/// What the target hook does.
type TargetFn = dyn Fn(&World, Vec2) -> Option<Target> + Send + Sync;

/// The app's side of session recording. Every hook is optional.
#[derive(Clone, Default)]
pub struct SessionHooks {
    /// Captures the setup.
    setup: Option<Arc<SetupFn>>,
    /// Names the view.
    view: Option<Arc<ViewFn>>,
    /// Resolves a position to an app-defined target.
    target: Option<Arc<TargetFn>>,
    /// Reads the app's state.
    state: Option<StateProbe>,
}

impl fmt::Debug for SessionHooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionHooks")
            .field("capture_setup", &self.setup.is_some())
            .field("capture_view", &self.view.is_some())
            .field("resolve_target", &self.target.is_some())
            .field("state", &self.state.is_some())
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

    /// Reads the app's state as `key` and `value` text. What differs between
    /// start and stop is written into the script as `expect_state`.
    #[must_use]
    pub fn state(mut self, probe: StateProbe) -> Self {
        self.state = Some(probe);
        self
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

    /// The app's state, if it has a probe.
    pub(super) fn snapshot(
        &self,
        world: &World,
    ) -> Option<alloc::collections::BTreeMap<String, String>> {
        self.state.as_ref().map(|probe| probe.snapshot(world))
    }
}
