//! The gallery: the app under test in one window, the scenario list in a second.
//!
//! The script owns the primary window's real input until **Take over**, and the
//! app's reset hook must not despawn the gallery's, overlay's or Scenarios cameras.

use alloc::sync::Arc;
use std::path::PathBuf;

use bevy::{
    app::{App, Plugin, PostUpdate, PreUpdate, Startup, Update},
    ecs::{
        schedule::{IntoScheduleConfigs, SystemSet},
        world::World,
    },
    picking::input::PointerInputSettings,
};

mod catalogue;
mod control;
mod input;
mod pick;
mod state;
mod ui;
mod watch;

pub use catalogue::{Gallery, GalleryEntry, GalleryGroup, WATCH_INTERVAL};
pub use input::scenarios_pointer;
pub use state::{
    GalleryAction, GalleryPhase, GalleryRunReport, GalleryState, GalleryTab, HiddenList,
    NoisePreset, PendingStep, Picking, Recording, ScenarioOutcome, ScenarioResult, Speed,
    Suggestions, TraceWorld,
};
pub use ui::{GalleryToolbar, SCENARIOS_WINDOW_SIZE, ScenariosCamera, ScenariosWindow};

use crate::{
    input::{SyntheticInputPlugin, SyntheticInputSystems},
    overlay::CursorOverlayPlugin,
    player::{ExternallyDriven, ScenarioPlayerPlugin, ScenarioSystems},
    session::{SessionConfig, SessionHooks, SessionPlugin, SessionSystems},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reload {
    Changed(usize),
    Added(usize),
}

pub const DEFAULT_LOOP_PAUSE: u16 = 60;

type ResetFn = dyn Fn(&mut World) + Send + Sync;

#[derive(bevy::ecs::resource::Resource, Clone)]
pub(crate) struct GalleryReset(pub(crate) Arc<ResetFn>);

impl core::fmt::Debug for GalleryReset {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("GalleryReset(..)")
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GallerySystems {
    /// Runs before `SyntheticInput` is drained so synthetic input is never caught.
    Input,
    Requests,
    Drive,
}

/// Plays a catalog of scenarios in a second window, in a loop.
///
/// Needs windowing and rendering plugins. Adds [`SyntheticInputPlugin`],
/// [`ScenarioPlayerPlugin`] and [`CursorOverlayPlugin`] unless present.
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_stagecraft::{Gallery, GalleryPlugin, Scenario};
///
/// App::new()
///     .add_plugins(DefaultPlugins)
///     .add_plugins(
///         GalleryPlugin::new(Gallery::new().group("Basics", [Scenario::new("empty")]))
///             .with_reset(|_world| { /* put the app's state back */ }),
///     )
///     .run();
/// ```
#[derive(Debug, Clone)]
pub struct GalleryPlugin {
    gallery: Gallery,
    reset: Option<GalleryReset>,
    export_dir: PathBuf,
    loop_pause: u16,
    session_hooks: SessionHooks,
    run_all: bool,
    tag: Option<String>,
}

impl GalleryPlugin {
    #[must_use]
    pub fn new(gallery: Gallery) -> Self {
        Self {
            gallery,
            reset: None,
            export_dir: PathBuf::from("gallery-export"),
            loop_pause: DEFAULT_LOOP_PAUSE,
            session_hooks: SessionHooks::default(),
            run_all: false,
            tag: None,
        }
    }

    /// Plays every entry once, failing ones included, then exits with
    /// [`bevy::app::AppExit::error`] if any failed. The controls do nothing
    /// meanwhile.
    #[must_use]
    pub const fn with_run_all(mut self) -> Self {
        self.run_all = true;
        self
    }

    /// Starts with the catalog filtered by `tag` ([`GalleryAction::SelectTag`]).
    /// A tag no entry carries filters nothing.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Ignored when the app added its own [`SessionPlugin`].
    #[must_use]
    pub fn with_session_hooks(mut self, hooks: SessionHooks) -> Self {
        self.session_hooks = hooks;
        self
    }

    /// Runs before the scenario's own setup on every run.
    #[must_use]
    pub fn with_reset(mut self, reset: impl Fn(&mut World) + Send + Sync + 'static) -> Self {
        self.reset = Some(GalleryReset(Arc::new(reset)));
        self
    }

    #[must_use]
    pub fn with_export_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.export_dir = dir.into();
        self
    }

    #[must_use]
    pub const fn with_loop_pause(mut self, updates: u16) -> Self {
        self.loop_pause = updates;
        self
    }
}

impl Plugin for GalleryPlugin {
    fn build(&self, app: &mut App) {
        disable_window_tabbing();
        if !app.is_plugin_added::<SyntheticInputPlugin>() {
            app.add_plugins(SyntheticInputPlugin);
        }
        if !app.is_plugin_added::<ScenarioPlayerPlugin>() {
            app.add_plugins(ScenarioPlayerPlugin);
        }
        if !app.is_plugin_added::<CursorOverlayPlugin>() {
            app.add_plugins(CursorOverlayPlugin);
        }
        if !app.is_plugin_added::<SessionPlugin>() {
            app.add_plugins(
                SessionPlugin::new(self.export_dir.clone()).with_hooks(self.session_hooks.clone()),
            );
        }
        let noise = app.world().get_resource::<SessionConfig>().map_or_else(
            || self.session_hooks.noise().clone(),
            |config| config.hooks.noise().clone(),
        );
        let mut state = GalleryState::new(self.export_dir.clone(), self.loop_pause, noise);
        if let Some(tag) = &self.tag {
            state.tag = self.gallery.tags().iter().position(|known| known == tag);
        }
        if self.run_all {
            state.begin_run_all();
        }
        app.init_resource::<PointerInputSettings>()
            .init_resource::<input::ScenariosScroll>()
            .init_resource::<input::RealCursor>()
            .init_resource::<input::PickClicks>()
            .insert_resource(ExternallyDriven)
            .insert_resource(self.gallery.clone())
            .insert_resource(state);
        if let Some(reset) = &self.reset {
            app.insert_resource(reset.clone());
        }
        app.configure_sets(
            PreUpdate,
            (
                GallerySystems::Input,
                GallerySystems::Requests,
                GallerySystems::Drive,
            )
                .chain()
                .before(SyntheticInputSystems::Drain),
        )
        .configure_sets(
            PreUpdate,
            GallerySystems::Drive.in_set(ScenarioSystems::Play),
        )
        // Input the gallery removed must not reach a recording.
        .configure_sets(
            PreUpdate,
            (
                SessionSystems::Record.after(GallerySystems::Requests),
                SessionSystems::Checkpoint.after(GallerySystems::Input),
            ),
        )
        .add_systems(Startup, (ui::spawn_window, input::spawn_scenarios_pointer))
        .add_systems(
            PreUpdate,
            (
                (input::save_real_cursor, input::route_real_input)
                    .chain()
                    .in_set(GallerySystems::Input),
                (control::apply_requests, pick::update)
                    .chain()
                    .in_set(GallerySystems::Requests),
                control::drive.in_set(GallerySystems::Drive),
                input::assert_script_cursor.after(SyntheticInputSystems::Drain),
            ),
        )
        .add_systems(
            Update,
            (
                input::exit_with_either_window,
                watch::reload_changed_files,
                ui::rebuild_catalogue.after(watch::reload_changed_files),
                ui::place_scenarios_window,
                ui::scroll_catalogue,
                ui::split_panels,
                ui::style_controls,
                ui::filter_entries,
                ui::update_labels,
                ui::update_details,
            ),
        )
        .add_systems(PostUpdate, input::restore_real_cursor);
    }
}

/// Runs before winit creates the first window; the setting is process-wide.
#[cfg(target_os = "macos")]
fn disable_window_tabbing() {
    let Some(main_thread) = objc2::MainThreadMarker::new() else {
        bevy::log::warn!("not on the main thread: macOS may put the Scenarios window in a tab");
        return;
    };
    objc2_app_kit::NSWindow::setAllowsAutomaticWindowTabbing(false, main_thread);
}

#[cfg(not(target_os = "macos"))]
const fn disable_window_tabbing() {}
