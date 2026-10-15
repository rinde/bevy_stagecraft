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
//! # Jumping to a step
//!
//! Clicking a step in the selected scenario's list ([`GalleryAction::JumpTo`])
//! restarts the scenario and plays it, one script frame per update, up to the
//! start of that step, where it pauses. No snapshot of the app is stored: the
//! world is the one a straight run has before the step, on the same frame
//! ([`crate::ScenarioTrace::before`]), so the live app can be inspected there.
//!
//! # The Trace tab
//!
//! The **Trace** tab of the Scenarios window ([`GalleryTab::Trace`], or
//! [`GalleryAction::ShowTab`]) shows what the run did, step by step, as the
//! recorder's `trace.html` does and in the same words ([`crate::StepView`]):
//! the step's number and description, why it failed, the targets it looked up
//! and what it found, the checks it ran, what it changed in the reflected
//! state, and the app's UI tree before or after it
//! ([`GalleryAction::ShowTraceWorld`]).
//!
//! The gallery keeps a trace of every run it plays (not of **Run all**, which
//! is for CI). **Follow** (on to begin with, [`GalleryAction::ToggleTraceFollow`])
//! shows the step being played, and once the run is over the first that
//! failed. A click on a step in the list, or on the arrows, shows that step
//! instead ([`GalleryAction::InspectStep`]) and turns following off; a jump
//! to a step shows that step as the world was before it. The run's whole
//! trace is stored when it ends, and kept across a jump and a loop, since a
//! run is deterministic; selecting or restarting drops it. The tab redraws
//! only when what it shows has changed, which includes the scenarios'
//! revision (see [`GalleryState`]).
//!
//! ## What the changes leave out
//!
//! The trace holds every reflected type; the tab filters the **changes** it
//! lists when it draws them ([`GalleryState::noise`]), so the filter can
//! change at once, without playing the scenario again or disturbing a paused
//! jump. It starts as the filter the app set for the gallery
//! ([`crate::SessionHooks::noise_filter`], by default
//! [`crate::NoiseFilter::default`]). Each change about a reflected type
//! (`resource`, `count`, `state`) has a **Hide** button
//! ([`GalleryAction::ToggleType`]); those the filter leaves out are counted,
//! and **List** opens them, each with a **Show** button. **App default** and
//! **Everything** ([`GalleryAction::UseNoise`], [`NoisePreset`]) switch
//! the whole filter. The tab ends with the filter in force as Rust
//! ([`crate::NoiseFilter::to_code`]), to paste into the app's hooks. Hiding a
//! type a broader `keep` pattern of the app matches does nothing and says so
//! in the status line. The filter is for the whole gallery session and every
//! scenario; it changes the Trace tab and nothing else (not `trace.html`, a
//! recording's suggested checks or pick mode), and is accepted whatever the
//! gallery is doing.
//!
//! # Running everything
//!
//! [`GalleryPlugin::with_run_all`] plays the whole catalogue once, front to
//! back, with nobody at the controls: each entry is reset, set up and played
//! as if selected, whether or not the one before passed. When the last has
//! finished the outcome is logged (one `PASS` or `FAIL` line per scenario,
//! the failures under it), kept in the [`GalleryRunReport`] resource, and the
//! app exits, closing both windows, with an error code if anything failed.
//!
//! # Recording
//!
//! **Record** starts a [session recording](crate::session) from what is on
//! screen: the script pauses, the primary window is taken over, and the real
//! mouse and keyboard of that window are logged until **Stop recording**,
//! which writes a `.ron` file into the export directory. Nothing in the
//! Scenarios window is recorded, and its nodes are never named as targets.
//!
//! **New: empty world** ([`GalleryAction::NewEmpty`]) starts a recording of a
//! new scenario from nothing: the selected scenario is dropped, the app's reset
//! hook puts its state back and the session starts on what is left.
//! **New: current state** ([`GalleryAction::NewFromCurrent`]) starts it from the
//! app as it is, which is what **Record** does too.
//!
//! While recording, the checkpoint key (F8 unless the session hooks set
//! another) takes a [checkpoint](crate::session#checkpoints): the overlay
//! flashes and names it, and the app never sees the key.
//!
//! When recording stops, the checks the world's changes suggest (see
//! [`crate::check::suggest`]) are listed in the Scenarios window as
//! checklists, all ticked, in a fixed order: one per checkpoint, then one for
//! the end. Clicking one ticks or unticks it and writes the file again, a
//! checkpoint's ticked checks as `Check` steps where it was taken and the
//! end's as its `expect` ([`GalleryAction::ToggleCheckpointSuggestion`] and
//! [`GalleryAction::ToggleSuggestion`] do the same from code). The wheel
//! scrolls the lists while the pointer is over them. They stay until another
//! scenario is selected or recorded.
//! Selecting, restarting, playing or stepping is refused while recording.
//!
//! # Pick mode
//!
//! **Pick** (or [`GalleryAction::TogglePick`]) works while recording, and
//! with a scenario selected, which it pauses. The app window's mouse and
//! keyboard then go to the gallery: the overlay outlines the node under the
//! real pointer with a tag naming its best target (see
//! [`crate::overlay::PickHighlight`]), and a click picks it instead of
//! reaching the app. While the person has the window the app sees no cursor
//! on it, so hovering changes nothing either. Escape in the app window ends
//! it, as do playing on, stepping, restarting, selecting, taking over and
//! starting or stopping a recording.
//!
//! The Scenarios window lists what [`crate::check::pick`] finds for the
//! picked node: its targets, best first, the checks that hold for it now,
//! and a form for each variant of the app's check type. Clicking a check
//! takes it: a `Check` step at this point of the recording
//! ([`crate::session::add_check`]), or else an expectation of the selected
//! scenario, in the catalogue and in the run being played.
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

/// What a look at the watched files changed in the catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reload {
    /// The entry at this index was read again, or its file went. Whether it
    /// now loads is [`GalleryEntry::error`].
    Changed(usize),
    /// A new entry at this index, which moved every later entry up by one.
    Added(usize),
}

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
    /// Whether to play the whole catalogue once and exit.
    run_all: bool,
    /// The tag the catalogue starts filtered by.
    tag: Option<String>,
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
            run_all: false,
            tag: None,
        }
    }

    /// Plays every entry once, front to back, with no input needed, then ends
    /// the app, closing both windows. Every scenario runs, failing ones
    /// included. The outcome is logged, kept as a [`GalleryRunReport`]
    /// resource, and the app exits with [`bevy::app::AppExit::error`] if any
    /// scenario failed, so `App::run` returns it. The controls do nothing
    /// meanwhile; closing a window still ends the app.
    #[must_use]
    pub const fn with_run_all(mut self) -> Self {
        self.run_all = true;
        self
    }

    /// Starts with the catalogue filtered by `tag` (see
    /// [`GalleryAction::SelectTag`]), for example to run only the entries
    /// tagged `bridge` with [`GalleryPlugin::with_run_all`]. A tag no entry
    /// carries filters nothing.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
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
        // The Trace tab starts from the one filter the app has set for the
        // gallery: the hooks of the session plugin in use, which may be the
        // app's own.
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
        // Real input the gallery removed must not end up in a recording.
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
