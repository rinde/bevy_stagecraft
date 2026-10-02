//! Scripted pointer and keyboard scenarios for Bevy UI.
//!
//! Scenarios run headless in `cargo test`, are recorded offscreen for review
//! (feature `record`), and play back in a gallery (feature `gallery`).
//!
//! The one rule: this crate **knows nothing about any game**. It takes Bevy
//! types only.

extern crate alloc;

#[cfg(feature = "record")]
pub mod capture;
pub mod files;
#[cfg(feature = "gallery")]
pub mod gallery;
pub mod headless;
#[cfg(feature = "record")]
pub mod index;
pub mod input;
pub mod overlay;
pub mod player;
pub mod probe;
#[cfg(feature = "record")]
pub mod recorder;
pub mod scenario;
pub mod script;
pub mod targets;

#[cfg(feature = "record")]
pub use capture::{CaptureError, Frame};
pub use files::{FileResult, load_ron_file, ron_files, run_ron_files, run_ron_tests};
#[cfg(feature = "gallery")]
pub use gallery::{
    Gallery, GalleryAction, GalleryEntry, GalleryGroup, GalleryPlugin, GalleryState,
    GallerySystems, ScenariosCamera, ScenariosWindow, Speed,
};
pub use headless::{DEFAULT_WINDOW_SIZE, FRAME_STEP, HeadlessPlugins};
pub use input::{SyntheticEvent, SyntheticInput, SyntheticInputPlugin, SyntheticInputSystems};
pub use overlay::{
    CursorOverlayPlugin, OverlayCamera, OverlaySystems, OverlayTheme, TargetDescriptions,
    checks_text, current_step_text, describe_step, describe_target,
};
pub use player::{
    CheckKind, CheckRecord, Outcome, ScenarioPlayer, ScenarioPlayerPlugin, ScenarioReport,
    ScenarioSystems, StepRecord,
};
pub use probe::Probe;
#[cfg(feature = "record")]
pub use recorder::{
    Capture, CaptureMeta, Captures, RecordError, Recorder, RecorderPlugins, RecorderSystems,
    RecorderTarget, Recording, ScenarioRecording,
};
pub use scenario::{CheckResult, IntoCheckResult, Modifiers, NamedCheck, Scenario, Step};
pub use script::{
    CheckRegistry, ExportError, FORMAT_VERSION, HookRegistry, LoadError, Registry, Script, Setup,
    export_ron,
};
pub use targets::{NotUnderTest, ResolvedTarget, Target, TargetResolvers, resolve};
