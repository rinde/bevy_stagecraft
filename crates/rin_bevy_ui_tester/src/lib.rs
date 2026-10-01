//! Scripted pointer and keyboard scenarios for Bevy UI.
//!
//! Scenarios run headless in `cargo test`, are recorded offscreen for review
//! (feature `record`), and play back in a gallery (feature `gallery`).
//!
//! The one rule: this crate **knows nothing about any game**. It takes Bevy
//! types only.

extern crate alloc;

pub mod headless;
pub mod input;
pub mod player;
pub mod probe;
pub mod scenario;
pub mod targets;

pub use headless::{DEFAULT_WINDOW_SIZE, FRAME_STEP, HeadlessPlugins};
pub use input::{SyntheticEvent, SyntheticInput, SyntheticInputPlugin, SyntheticInputSystems};
pub use player::{
    CheckKind, CheckRecord, Outcome, ScenarioPlayer, ScenarioPlayerPlugin, ScenarioReport,
    ScenarioSystems, StepRecord,
};
pub use probe::Probe;
pub use scenario::{CheckResult, IntoCheckResult, Modifiers, NamedCheck, Scenario, Step};
pub use targets::{ResolvedTarget, Target, TargetResolvers, resolve};
