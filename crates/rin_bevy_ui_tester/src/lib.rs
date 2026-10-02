//! Scripted pointer and keyboard scenarios for Bevy UI.
//!
//! Scenarios run headless in `cargo test`, are recorded offscreen for review
//! (feature `record`), and play back in a gallery (feature `gallery`).
//!
//! The one rule: this crate **knows nothing about any game**. It takes Bevy
//! types only.

extern crate alloc;

pub mod files;
pub mod headless;
pub mod input;
pub mod player;
pub mod probe;
pub mod scenario;
pub mod script;
pub mod targets;

pub use files::{FileResult, load_ron_file, ron_files, run_ron_files, run_ron_tests};
pub use headless::{DEFAULT_WINDOW_SIZE, FRAME_STEP, HeadlessPlugins};
pub use input::{SyntheticEvent, SyntheticInput, SyntheticInputPlugin, SyntheticInputSystems};
pub use player::{
    CheckKind, CheckRecord, Outcome, ScenarioPlayer, ScenarioPlayerPlugin, ScenarioReport,
    ScenarioSystems, StepRecord,
};
pub use probe::Probe;
pub use scenario::{CheckResult, IntoCheckResult, Modifiers, NamedCheck, Scenario, Step};
pub use script::{
    CheckRegistry, ExportError, FORMAT_VERSION, HookRegistry, LoadError, Registry, Script, Setup,
    export_ron,
};
pub use targets::{ResolvedTarget, Target, TargetResolvers, resolve};
