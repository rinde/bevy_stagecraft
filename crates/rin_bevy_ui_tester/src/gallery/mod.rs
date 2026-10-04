//! The gallery: the app under test in one window, a list of scenarios in a
//! second one (feature `gallery`).
//!
//! [`GalleryPlugin`] goes into an ordinary windowed app, next to whatever
//! plugins the app has. Picking a scenario in the **Scenarios** window calls
//! the app's reset hook, runs the scenario's setup and view, resizes the
//! primary window to the scenario's window size and plays it with the cursor
//! overlay on top.
//!
//! # Two windows
//!
//! The primary window holds the app, untouched. The Scenarios window has its
//! own UI camera ([`ScenariosCamera`]), which renders to that window only and
//! so is never the default UI camera: the app's UI and the overlay keep going
//! to the camera of the primary window. A scenario whose cameras render
//! somewhere unusual can name the overlay's camera with
//! [`crate::OverlayCamera`].
//!
//! Everything the gallery spawns carries [`crate::NotUnderTest`], so a script
//! that clicks a button labelled "Reset" never finds the gallery's.
//!
//! # Real input while a scenario plays
//!
//! The script owns the primary window until **Take over**. In that time:
//!
//! * the real mouse, keyboard and touch messages aimed at the primary window
//!   are removed before `InputSystems`, and
//!   `PointerInputSettings::is_mouse_enabled` is off;
//! * the synthetic cursor position is put on the primary window each update,
//!   so the app and the overlay see it, and the real one is put back before
//!   `bevy_winit` compares them, so the operating system's pointer is never
//!   moved. An app that reads `Window::cursor_position` in `First`, `Last` or
//!   after `PostUpdate` sees the real position, not the script's;
//! * input to the Scenarios window keeps working. Bevy's mouse picking is off
//!   for every window, so the Scenarios window's mouse drives a pointer of its
//!   own ([`scenarios_pointer`]); touch input to that window does not work
//!   during playback.
//!
//! Space, the arrow keys and the buttons read only that window's keyboard
//! messages, which are removed afterwards so the app never sees them: Space
//! plays or pauses, Right steps one frame, Down steps to the next step, Left
//! restarts.
//!
//! # Recording
//!
//! **Record** starts a [session recording](crate::session) from what is on
//! screen: the script pauses, the primary window is taken over, and the real
//! mouse and keyboard of that window are logged until **Stop recording**,
//! which writes a `.ron` file into the export directory. Nothing in the
//! Scenarios window is recorded, and its nodes are never named as targets.
//! Selecting, restarting, playing or stepping is refused while recording.
//!
//! The app's reset hook must put the app's own state back and nothing else:
//! it must not despawn the gallery's, the overlay's or the cameras of the
//! Scenarios window.

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
mod state;
mod ui;

pub use catalogue::{Gallery, GalleryEntry, GalleryGroup};
pub use input::scenarios_pointer;
pub use state::{GalleryAction, GalleryPhase, GalleryState, PendingStep, Recording, Speed};
pub use ui::{GalleryToolbar, SCENARIOS_WINDOW_SIZE, ScenariosCamera, ScenariosWindow};

use crate::{
    input::{SyntheticInputPlugin, SyntheticInputSystems},
    overlay::CursorOverlayPlugin,
    player::{ExternallyDriven, ScenarioPlayerPlugin, ScenarioSystems},
    session::{SessionHooks, SessionPlugin, SessionSystems},
};

/// How many updates a loop waits before starting again, unless configured.
pub const DEFAULT_LOOP_PAUSE: u16 = 60;

/// The app's reset hook.
type ResetFn = dyn Fn(&mut World) + Send + Sync;

/// What puts the app's state back for a fresh run.
#[derive(bevy::ecs::resource::Resource, Clone)]
pub(crate) struct GalleryReset(pub(crate) Arc<ResetFn>);

impl core::fmt::Debug for GalleryReset {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("GalleryReset(..)")
    }
}

/// System sets of the gallery, all in `PreUpdate` unless noted.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GallerySystems {
    /// Sorts the real input: drops what is aimed at the primary window while a
    /// scenario plays, and routes the Scenarios window's. Before
    /// `SyntheticInput` is drained, so synthetic input is never caught.
    Input,
    /// Carries out the requests made by buttons, keys and code.
    Requests,
    /// Advances the script, paced by the mode and the speed.
    Drive,
}

/// Plays a catalogue of scenarios in a second window, in a loop.
///
/// Add it to an app that has its windowing and rendering plugins (for example
/// `DefaultPlugins`). It adds [`SyntheticInputPlugin`], [`ScenarioPlayerPlugin`]
/// and [`CursorOverlayPlugin`] unless the app already has them.
///
/// ```no_run
/// use bevy::prelude::*;
/// use rin_bevy_ui_tester::{Gallery, GalleryPlugin, Scenario};
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
    /// What is on offer.
    gallery: Gallery,
    /// Puts the app's state back before each run.
    reset: Option<GalleryReset>,
    /// Where **Export** writes.
    export_dir: PathBuf,
    /// Updates between loops.
    loop_pause: u16,
    /// What the app plugs into **Record**.
    session_hooks: SessionHooks,
}

impl GalleryPlugin {
    /// A gallery of `gallery`, with no reset hook, exporting to
    /// `gallery-export` under the working directory.
    #[must_use]
    pub fn new(gallery: Gallery) -> Self {
        Self {
            gallery,
            reset: None,
            export_dir: PathBuf::from("gallery-export"),
            loop_pause: DEFAULT_LOOP_PAUSE,
            session_hooks: SessionHooks::default(),
        }
    }

    /// Sets what the app plugs into **Record**: capturing its setup, naming
    /// targets, and reading its state. Recordings go to the export directory.
    /// Ignored when the app added its own [`SessionPlugin`].
    #[must_use]
    pub fn with_session_hooks(mut self, hooks: SessionHooks) -> Self {
        self.session_hooks = hooks;
        self
    }

    /// Sets the hook that puts the app's state back before every run: despawn
    /// what the last run spawned, reset resources. It runs before the
    /// scenario's own setup.
    #[must_use]
    pub fn with_reset(mut self, reset: impl Fn(&mut World) + Send + Sync + 'static) -> Self {
        self.reset = Some(GalleryReset(Arc::new(reset)));
        self
    }

    /// Sets the directory **Export** writes `.ron` files to.
    #[must_use]
    pub fn with_export_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.export_dir = dir.into();
        self
    }

    /// Sets how many updates a looping gallery waits between runs.
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
        app.init_resource::<PointerInputSettings>()
            .init_resource::<input::ScenariosScroll>()
            .init_resource::<input::RealCursor>()
            .insert_resource(ExternallyDriven)
            .insert_resource(self.gallery.clone())
            .insert_resource(GalleryState::new(self.export_dir.clone(), self.loop_pause));
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
        // Real input the gallery removed must not end up in a recording.
        .configure_sets(
            PreUpdate,
            SessionSystems::Record.after(GallerySystems::Requests),
        )
        .add_systems(Startup, (ui::spawn_window, input::spawn_scenarios_pointer))
        .add_systems(
            PreUpdate,
            (
                (input::save_real_cursor, input::route_real_input)
                    .chain()
                    .in_set(GallerySystems::Input),
                control::apply_requests.in_set(GallerySystems::Requests),
                control::drive.in_set(GallerySystems::Drive),
                input::assert_script_cursor.after(SyntheticInputSystems::Drain),
            ),
        )
        .add_systems(
            Update,
            (
                input::exit_with_either_window,
                ui::place_scenarios_window,
                ui::scroll_catalogue,
                ui::style_controls,
                ui::update_labels,
                ui::update_details,
            ),
        )
        .add_systems(PostUpdate, input::restore_real_cursor);
    }
}

/// Stops macOS from merging the gallery's two windows into one tabbed window.
///
/// With "Prefer tabs" set to "Always" in the system settings, a new window
/// opens as a tab over the existing one, hiding the app under test. This must
/// run before winit creates the first window; plugins are built on the main
/// thread, which `MainThreadMarker` checks. The setting is process-wide.
#[cfg(target_os = "macos")]
fn disable_window_tabbing() {
    let Some(main_thread) = objc2::MainThreadMarker::new() else {
        bevy::log::warn!("not on the main thread: macOS may put the Scenarios window in a tab");
        return;
    };
    objc2_app_kit::NSWindow::setAllowsAutomaticWindowTabbing(false, main_thread);
}

/// Window tabbing is a macOS feature; elsewhere there is nothing to turn off.
#[cfg(not(target_os = "macos"))]
const fn disable_window_tabbing() {}
