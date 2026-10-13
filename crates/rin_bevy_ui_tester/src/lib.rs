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
pub mod check;
pub mod extension;
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
pub mod save;
pub mod scenario;
pub mod script;
pub mod session;
pub mod targets;
pub mod trace;
pub mod variant;

#[cfg(feature = "record")]
pub use capture::{CaptureError, Frame};
pub use check::{
    AppCheck, Check, Comparison, CountOf, FieldOf, Measure, ReflectAppCheck, TextMatch, Unit,
    Value,
    pick::{
        AppCheckForm, FieldValue, Picked, PickedForm, app_check_forms, checks_that_hold, node_at,
        subject_of, target_alternatives,
    },
    suggest::{DEFAULT_IGNORED, NoiseFilter, SnapshotChange, WorldSnapshot, suggest_checks},
    tree_snapshot,
};
pub use extension::{
    AppCheckValue, AppField, AppSetup, AppSetupValue, AppTarget, AppTargetValue, AppType, AppValue,
    AppVariant, AppView, AppViewValue, Extension, ReflectAppSetup, ReflectAppTarget,
    ReflectAppView, Setup, SeveralAppTypes, View, app_type,
};
pub use files::{
    FileOutcome, FileResult, load_ron_file, ron_files, run_ron_files, run_ron_tests,
    run_ron_tests_with_variants, run_ron_variants,
};
#[cfg(feature = "gallery")]
pub use gallery::{
    Gallery, GalleryAction, GalleryEntry, GalleryGroup, GalleryPlugin, GalleryRunReport,
    GalleryState, GallerySystems, GalleryTab, NoisePreset, ScenarioOutcome, ScenarioResult,
    ScenariosCamera, ScenariosWindow, Speed, Suggestions, TraceWorld,
};
pub use headless::{DEFAULT_SCALE_FACTOR, DEFAULT_WINDOW_SIZE, FRAME_STEP, HeadlessPlugins};
pub use input::{
    Keystroke, SyntheticEvent, SyntheticInput, SyntheticInputPlugin, SyntheticInputSystems,
    keystroke,
};
pub use overlay::{
    CursorOverlayPlugin, OverlayCamera, OverlaySystems, OverlayTheme, TargetDescriptions,
    checkpoint_text, checks_text, current_step_text, describe_step, describe_target,
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
pub use save::{SCENE_SUFFIX, SaveError, SavedSetup, save_setup};
pub use scenario::{
    CheckResult, FlatStep, GROUP_SEPARATOR, Modifiers, Scenario, Step, flatten, with_group_path,
};
pub use script::{
    ExportError, FORMAT_VERSION, LoadError, OLDEST_FORMAT_VERSION, Registry, Script, export_ron,
};
pub use session::{
    Checkpoint, CheckpointTaken, DEFAULT_CHECKPOINT_KEY, LoggedSteps, RecordedEvent, RecordedKind,
    SavedSession, SessionConfig, SessionError, SessionHooks, SessionPlugin, SessionRecorder,
    SessionSystems, StepOptions, read_log, steps_from_events,
};
pub use targets::{
    DEFAULT_ACTION_TIMEOUT, NotUnderTest, ResolvedTarget, Target, WidgetRole, entity_of, resolve,
};
pub use trace::{
    ScenarioTrace, StepTrace, StepView, TargetTrace, TraceCaptured, TraceCheck, TracePoint, UiLine,
    step_title,
};
pub use variant::{DEFAULT_VARIANT_NAME, Variant};
