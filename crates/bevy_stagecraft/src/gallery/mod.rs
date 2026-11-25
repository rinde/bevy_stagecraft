//! The gallery: the app under test in one window, a list of scenarios in a
//! second one (feature `gallery`).
//!
//! # Two windows
//!
//! The Scenarios window has its own UI camera ([`ScenariosCamera`]), which is
//! never the default UI camera: the app's UI and the overlay stay on the
//! primary window's. Name the overlay's camera with [`crate::OverlayCamera`]
//! when a scenario renders somewhere unusual. Everything the gallery spawns
//! carries [`crate::NotUnderTest`].
//!
//! # Real input while a scenario plays
//!
//! The script owns the primary window until **Take over**:
//!
//! * real mouse, keyboard and touch messages aimed at it are removed before
//!   `InputSystems`, and `PointerInputSettings::is_mouse_enabled` is off;
//! * the synthetic cursor position is put on the window each update and the
//!   real one restored before `bevy_winit` compares them. An app that reads
//!   `Window::cursor_position` in `First`, `Last` or after `PostUpdate` sees
//!   the real position;
//! * the Scenarios window's mouse drives its own pointer
//!   ([`scenarios_pointer`]); its touch input does not work during playback.
//!
//! Space plays or pauses, Right steps one frame, Down steps to the next step,
//! Left restarts. Those keys are read from the Scenarios window only and
//! removed afterwards.
//!
//! # Jumping to a step
//!
//! [`GalleryAction::JumpTo`] restarts the scenario and plays it up to the start
//! of the step, where it pauses. No snapshot is stored: the world is the one a
//! straight run has there ([`crate::ScenarioTrace::before`]).
//!
//! # The Trace tab
//!
//! [`GalleryTab::Trace`] shows what the run did, as the recorder's
//! `trace.html` does ([`crate::StepView`]). Runs are traced except **Run all**.
//! **Follow** ([`GalleryAction::ToggleTraceFollow`]) shows the step being
//! played, then the first that failed; [`GalleryAction::InspectStep`] shows
//! another and turns it off. The trace survives a jump and a loop; selecting
//! or restarting drops it.
//!
//! The changes list is filtered when drawn ([`GalleryState::noise`]), starting
//! from [`crate::SessionHooks::noise_filter`]. [`GalleryAction::ToggleType`]
//! hides or shows a type and [`GalleryAction::UseNoise`] switches the whole
//! filter ([`NoisePreset`]). Hiding a type that a broader `keep` pattern
//! matches does nothing. The filter lasts for the gallery session and affects
//! the Trace tab only.
//!
//! # Running everything
//!
//! [`GalleryPlugin::with_run_all`] plays the whole catalog once, passing or
//! not, logs one `PASS` or `FAIL` line per scenario, keeps a
//! [`GalleryRunReport`] and exits with an error code if anything failed.
//!
//! # Recording
//!
//! **Record** starts a [session recording](crate::session) from what is on
//! screen and logs the primary window's real mouse and keyboard until **Stop
//! recording**, which writes a `.ron` file into the export directory. Nothing
//! in the Scenarios window is recorded. [`GalleryAction::NewEmpty`] starts from
//! the reset app, [`GalleryAction::NewFromCurrent`] from the app as it is.
//!
//! The checkpoint key (F8 by default) takes a [checkpoint](crate::session#checkpoints)
//! and is hidden from the app. When recording stops, the checks
//! [`crate::check::suggest`] finds are listed, all ticked; toggling one writes
//! the file again ([`GalleryAction::ToggleCheckpointSuggestion`],
//! [`GalleryAction::ToggleSuggestion`]). Selecting, restarting, playing and
//! stepping are refused while recording.
//!
//! # Pick mode
//!
//! [`GalleryAction::TogglePick`] works while recording or with a scenario
//! selected, which it pauses. The app window's input then goes to the gallery:
//! the overlay outlines the node under the pointer
//! ([`crate::overlay::PickHighlight`]) and a click picks it. Escape in the app
//! window, playing on, stepping, restarting, selecting, taking over and
//! starting or stopping a recording end it.
//!
//! The Scenarios window lists what [`crate::check::pick`] finds for the node;
//! clicking a check adds a `Check` step to the recording
//! ([`crate::session::add_check`]) or else an expectation of the selected
//! scenario.
//!
//! The app's reset hook must not despawn the gallery's, the overlay's or the
//! Scenarios window's cameras.

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
    /// Shifts every later entry up by one.
    Added(usize),
}

/// Updates a loop waits before starting again, unless configured.
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
/// Needs the app's windowing and rendering plugins (for example
/// `DefaultPlugins`). Adds [`SyntheticInputPlugin`], [`ScenarioPlayerPlugin`]
/// and [`CursorOverlayPlugin`] unless present.
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

/// Runs before winit creates the first window, or "Prefer tabs: Always" hides
/// the app under test behind a tab. The setting is process-wide.
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
