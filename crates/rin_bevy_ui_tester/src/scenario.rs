//! Scenario definition: setup, view, steps and checks.
//!
//! Steps, targets and checks are plain data, and checks defined in Rust are
//! referred to by name, so a scenario converts to a serialisable [`Script`]
//! and back (see [`crate::script`]).

use alloc::{collections::BTreeMap, sync::Arc};
use core::{fmt, ops::BitOr};

use bevy::{
    ecs::world::World,
    input::{keyboard::KeyCode, mouse::MouseButton},
    math::Vec2,
};
use serde::{Deserialize, Serialize};

use crate::{
    check::Check,
    probe::Probe,
    script::{FORMAT_VERSION, LoadError, Registry, Script, Setup, StateProbe},
    targets::Target,
};

/// Whether a check passed; the error says what was wrong.
pub type CheckResult = Result<(), String>;

/// Anything a check may return: a plain `bool`, `()`, or a [`CheckResult`]
/// when the failure needs explaining.
pub trait IntoCheckResult {
    /// Converts into a [`CheckResult`].
    ///
    /// # Errors
    ///
    /// The check's failure message, when it did not hold.
    fn into_check_result(self) -> CheckResult;
}

impl IntoCheckResult for bool {
    fn into_check_result(self) -> CheckResult {
        if self {
            Ok(())
        } else {
            Err("the check returned false".into())
        }
    }
}

impl IntoCheckResult for () {
    fn into_check_result(self) -> CheckResult {
        Ok(())
    }
}

impl IntoCheckResult for CheckResult {
    fn into_check_result(self) -> CheckResult {
        self
    }
}

/// Modifier keys held around a key press, as a set. Combine with `|`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// One bit per modifier; see the associated constants.
    bits: u8,
}

impl Modifiers {
    /// No modifiers.
    pub const NONE: Self = Self { bits: 0 };
    /// Left shift.
    pub const SHIFT: Self = Self { bits: 1 };
    /// Left control.
    pub const CTRL: Self = Self { bits: 2 };
    /// Left alt.
    pub const ALT: Self = Self { bits: 4 };
    /// Left super (the logo key).
    pub const SUPER: Self = Self { bits: 8 };

    /// Whether every modifier in `other` is in this set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.bits & other.bits == other.bits
    }

    /// Whether no modifier is in the set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }

    /// The keys to hold, in a fixed order.
    #[must_use]
    pub fn keys(self) -> Vec<KeyCode> {
        [
            (Self::SHIFT, KeyCode::ShiftLeft),
            (Self::CTRL, KeyCode::ControlLeft),
            (Self::ALT, KeyCode::AltLeft),
            (Self::SUPER, KeyCode::SuperLeft),
        ]
        .into_iter()
        .filter_map(|(modifier, key)| self.contains(modifier).then_some(key))
        .collect()
    }
}

/// For serde: leave the modifiers out of a file when there are none.
fn no_modifiers<T: Default + PartialEq>(modifiers: &T) -> bool {
    *modifiers == T::default()
}

/// One modifier key, the readable serialised form of a [`Modifiers`] set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Modifier {
    /// Shift.
    Shift,
    /// Control.
    Ctrl,
    /// Alt.
    Alt,
    /// The logo key.
    Super,
}

impl Modifier {
    /// Every modifier, in the order sets list them.
    const ALL: [(Self, Modifiers); 4] = [
        (Self::Shift, Modifiers::SHIFT),
        (Self::Ctrl, Modifiers::CTRL),
        (Self::Alt, Modifiers::ALT),
        (Self::Super, Modifiers::SUPER),
    ];
}

/// Written as a list such as `[Ctrl, Shift]`, never as the raw bits, so a
/// file stays easy to edit by hand.
impl Serialize for Modifiers {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let list: Vec<Modifier> = Modifier::ALL
            .iter()
            .filter(|(_, set)| self.contains(*set))
            .map(|(modifier, _)| *modifier)
            .collect();
        list.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Modifiers {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let list = Vec::<Modifier>::deserialize(deserializer)?;
        Ok(list
            .into_iter()
            .filter_map(|modifier| {
                Modifier::ALL
                    .iter()
                    .find(|(candidate, _)| *candidate == modifier)
                    .map(|(_, set)| *set)
            })
            .fold(Self::NONE, |all, one| all | one))
    }
}

impl BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }
}

/// One thing a scenario does. Time is counted in frames of 1/60 s.
///
/// Every step that touches input takes one frame, except where it says
/// otherwise; [`Step::Check`] and [`Step::Caption`] take none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Step {
    /// Jump the pointer to the target.
    MoveTo(Target),
    /// Move the pointer to the target along an eased path over `frames`
    /// frames, at human speed. The target is worked out when the step starts.
    Glide {
        /// Where to end up.
        target: Target,
        /// How many frames the movement takes; at least one.
        frames: u16,
    },
    /// Press a mouse button.
    Press(MouseButton),
    /// Release a mouse button.
    Release(MouseButton),
    /// Press and release a mouse button within one frame.
    Click(MouseButton),
    /// Move to `from`, press the left button, glide to `to` over `frames`
    /// frames, and release. Takes `frames + 2` frames.
    Drag {
        /// Where the drag starts.
        from: Target,
        /// Where the drag ends.
        to: Target,
        /// How many frames the movement takes; at least one.
        frames: u16,
    },
    /// Turn the mouse wheel, in pixels, at the pointer.
    Wheel {
        /// Horizontal amount.
        x: f32,
        /// Vertical amount.
        y: f32,
    },
    /// Press the modifiers, then the key, and leave them held.
    Key {
        /// The key.
        key: KeyCode,
        /// The modifiers held with it.
        #[serde(default, skip_serializing_if = "no_modifiers")]
        modifiers: Modifiers,
    },
    /// Release the key, then the modifiers.
    KeyUp {
        /// The key.
        key: KeyCode,
        /// The modifiers to let go of with it.
        #[serde(default, skip_serializing_if = "no_modifiers")]
        modifiers: Modifiers,
    },
    /// Press and release the key, with the modifiers held around it, all in
    /// one frame. Bevy sees the key as just pressed and just released at once.
    Tap {
        /// The key.
        key: KeyCode,
        /// The modifiers held around it.
        #[serde(default, skip_serializing_if = "no_modifiers")]
        modifiers: Modifiers,
    },
    /// Do nothing for this many frames.
    Wait(u16),
    /// Run the check. Takes no frames: it sees the world as the previous step
    /// left it. A failed check fails the step but does not stop the script.
    Check(Check),
    /// Show this caption from here on, for the recorder and the gallery.
    Caption(String),
}

impl Step {
    /// Jump the pointer to `target`.
    #[must_use]
    pub const fn move_to(target: Target) -> Self {
        Self::MoveTo(target)
    }

    /// Glide the pointer to `target` over `frames` frames.
    #[must_use]
    pub const fn glide(target: Target, frames: u16) -> Self {
        Self::Glide { target, frames }
    }

    /// Click the left mouse button.
    #[must_use]
    pub const fn click() -> Self {
        Self::Click(MouseButton::Left)
    }

    /// Press the left mouse button.
    #[must_use]
    pub const fn press() -> Self {
        Self::Press(MouseButton::Left)
    }

    /// Release the left mouse button.
    #[must_use]
    pub const fn release() -> Self {
        Self::Release(MouseButton::Left)
    }

    /// Drag with the left button from `from` to `to` over `frames` frames.
    #[must_use]
    pub const fn drag(from: Target, to: Target, frames: u16) -> Self {
        Self::Drag { from, to, frames }
    }

    /// Turn the wheel by `x` and `y` pixels.
    #[must_use]
    pub const fn wheel(x: f32, y: f32) -> Self {
        Self::Wheel { x, y }
    }

    /// Tap `key` with no modifiers.
    #[must_use]
    pub const fn tap(key: KeyCode) -> Self {
        Self::Tap {
            key,
            modifiers: Modifiers::NONE,
        }
    }

    /// Tap `key` with `modifiers` held.
    #[must_use]
    pub const fn tap_with(key: KeyCode, modifiers: Modifiers) -> Self {
        Self::Tap { key, modifiers }
    }

    /// Do nothing for `frames` frames.
    #[must_use]
    pub const fn wait(frames: u16) -> Self {
        Self::Wait(frames)
    }

    /// Run `check`: a [`Check`], or the name of a check defined in Rust.
    #[must_use]
    pub fn check(check: impl Into<Check>) -> Self {
        Self::Check(check.into())
    }

    /// Show `text` as the caption.
    #[must_use]
    pub fn caption(text: impl Into<String>) -> Self {
        Self::Caption(text.into())
    }
}

/// What a hook does to the world.
pub(crate) type HookFn = dyn Fn(&mut World) + Send + Sync;

/// What a check does with a probe.
type CheckFn = dyn Fn(&Probe<'_>) -> CheckResult + Send + Sync;

/// A check with its name.
#[derive(Clone)]
pub struct NamedCheck {
    /// The name steps and reports refer to it by.
    pub name: String,
    /// The check itself.
    pub(crate) run: Arc<CheckFn>,
}

impl fmt::Debug for NamedCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NamedCheck")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl NamedCheck {
    /// Runs the check against `probe`.
    ///
    /// # Errors
    ///
    /// The check's own explanation when it fails.
    pub fn run(&self, probe: &Probe<'_>) -> CheckResult {
        (self.run)(probe)
    }
}

/// What a check that takes an argument does with a probe.
type CustomCheckFn = dyn Fn(&Probe<'_>, &str) -> CheckResult + Send + Sync;

/// A check that takes an argument, with its name: what [`Check::Custom`]
/// refers to.
#[derive(Clone)]
pub struct CustomCheck {
    /// The name checks refer to it by.
    pub name: String,
    /// The check itself.
    pub(crate) run: Arc<CustomCheckFn>,
}

impl fmt::Debug for CustomCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomCheck")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl CustomCheck {
    /// Runs the check against `probe`, asking it about `argument`.
    ///
    /// # Errors
    ///
    /// The check's own explanation when it fails.
    pub fn run(&self, probe: &Probe<'_>, argument: &str) -> CheckResult {
        (self.run)(probe, argument)
    }
}

/// A scenario: a name, hooks the app gives meaning to, a script and checks.
///
/// ```
/// use rin_bevy_ui_tester::{Scenario, Step, Target};
/// use bevy::math::Vec2;
///
/// let scenario = Scenario::new("empty")
///     .steps([Step::move_to(Target::screen(Vec2::ZERO)), Step::wait(3)])
///     .expect("nothing broke", |_probe| true);
/// assert_eq!(scenario.script().len(), 2);
/// ```
#[derive(Clone)]
pub struct Scenario {
    /// What the scenario is called.
    name: String,
    /// Builds the world the script runs against.
    setup: Option<Arc<HookFn>>,
    /// Arranges how the world is shown (camera, window); the recorder and the
    /// gallery decide what that means.
    view: Option<Arc<HookFn>>,
    /// The size of the window the scenario wants, in logical pixels.
    window: Option<(u32, u32)>,
    /// The setup as the script refers to it, for the app to resolve.
    setup_ref: Option<Setup>,
    /// The name the view hook is referred to by in a script.
    view_ref: Option<String>,
    /// The script.
    steps: Vec<Step>,
    /// What [`Check::Named`] refers to, one per name.
    checks: Vec<NamedCheck>,
    /// What [`Check::Custom`] refers to, one per name.
    custom_checks: Vec<CustomCheck>,
    /// Checks run once the script has finished, before those of
    /// `state_expects`.
    expects: Vec<Check>,
    /// The script's `expect_state` entries, run as [`Check::State`] at the
    /// end and kept apart so the script converts back.
    state_expects: BTreeMap<String, String>,
    /// Reads the app's state for [`Check::State`].
    state: Option<StateProbe>,
}

impl fmt::Debug for Scenario {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scenario")
            .field("name", &self.name)
            .field("setup", &self.setup.is_some())
            .field("view", &self.view.is_some())
            .field("window", &self.window)
            .field("setup_ref", &self.setup_ref)
            .field("view_ref", &self.view_ref)
            .field("steps", &self.steps)
            .field("checks", &self.checks)
            .field("custom_checks", &self.custom_checks)
            .field("expects", &self.expects)
            .field("state_expects", &self.state_expects)
            .field("state", &self.state)
            .finish()
    }
}

impl Scenario {
    /// An empty scenario called `name`.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            setup: None,
            view: None,
            window: None,
            setup_ref: None,
            view_ref: None,
            steps: Vec::new(),
            checks: Vec::new(),
            custom_checks: Vec::new(),
            expects: Vec::new(),
            state_expects: BTreeMap::new(),
            state: None,
        }
    }

    /// Sets the hook that builds the world, for example spawning a UI.
    #[must_use]
    pub fn setup(mut self, hook: impl Fn(&mut World) + Send + Sync + 'static) -> Self {
        self.setup = Some(Arc::new(hook));
        self
    }

    /// Sets the hook that arranges how the world is shown. Headless runs do
    /// not call it; the recorder and the gallery do.
    #[must_use]
    pub fn view(mut self, hook: impl Fn(&mut World) + Send + Sync + 'static) -> Self {
        self.view = Some(Arc::new(hook));
        self
    }

    /// Sets the setup hook and gives it a name, so an exported script refers
    /// to it as [`Setup::Named`] and an app that registers a hook of that name
    /// gets the same setup when it loads the file.
    #[must_use]
    pub fn setup_named(
        mut self,
        name: impl Into<String>,
        hook: impl Fn(&mut World) + Send + Sync + 'static,
    ) -> Self {
        self.setup = Some(Arc::new(hook));
        self.setup_ref = Some(Setup::Named(name.into()));
        self
    }

    /// Sets the view hook and gives it a name, like [`Scenario::setup_named`].
    #[must_use]
    pub fn view_named(
        mut self,
        name: impl Into<String>,
        hook: impl Fn(&mut World) + Send + Sync + 'static,
    ) -> Self {
        self.view = Some(Arc::new(hook));
        self.view_ref = Some(name.into());
        self
    }

    /// Refers to a setup the app resolves itself, for example
    /// `Setup::File("saves/start.ron".into())`. The crate only stores it; no
    /// setup hook is set.
    #[must_use]
    pub fn setup_ref(mut self, setup: Setup) -> Self {
        self.setup_ref = Some(setup);
        self
    }

    /// Sets the window size the scenario wants, in logical pixels.
    #[must_use]
    pub const fn window_size(mut self, width: u32, height: u32) -> Self {
        self.window = Some((width, height));
        self
    }

    /// Appends one step.
    #[must_use]
    pub fn step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    /// Appends steps.
    #[must_use]
    pub fn steps(mut self, steps: impl IntoIterator<Item = Step>) -> Self {
        self.steps.extend(steps);
        self
    }

    /// Defines the check that [`Check::Named`] of the same name runs, for
    /// example from `Step::check(name)`. Defining a name again replaces the
    /// check.
    #[must_use]
    pub fn check<R: IntoCheckResult>(
        mut self,
        name: impl Into<String>,
        check: impl Fn(&Probe<'_>) -> R + Send + Sync + 'static,
    ) -> Self {
        self.define(named(name, check));
        self
    }

    /// Defines a check under `name` and runs it once the script has finished.
    /// The name is shared with [`Scenario::check`], so a step can run the
    /// same check part-way through.
    #[must_use]
    pub fn expect<R: IntoCheckResult>(
        mut self,
        name: impl Into<String>,
        check: impl Fn(&Probe<'_>) -> R + Send + Sync + 'static,
    ) -> Self {
        let check = named(name, check);
        self.expects.push(Check::Named(check.name.clone()));
        self.define(check);
        self
    }

    /// Runs `check` once the script has finished, for example
    /// `Check::shows("Saved")`.
    #[must_use]
    pub fn expect_check(mut self, check: impl Into<Check>) -> Self {
        self.expects.push(check.into());
        self
    }

    /// Defines the check that [`Check::Custom`] of the same name runs: the
    /// closure gets the check's argument. Defining a name again replaces the
    /// check.
    #[must_use]
    pub fn custom_check<R: IntoCheckResult>(
        mut self,
        name: impl Into<String>,
        check: impl Fn(&Probe<'_>, &str) -> R + Send + Sync + 'static,
    ) -> Self {
        let check = custom(name, check);
        self.custom_checks
            .retain(|defined| defined.name != check.name);
        self.custom_checks.push(check);
        self
    }

    /// Sets the probe that [`Check::State`] reads the app's state with.
    #[must_use]
    pub fn with_state(mut self, probe: StateProbe) -> Self {
        self.state = Some(probe);
        self
    }

    /// Adds `check` to the named checks, replacing one of the same name.
    fn define(&mut self, check: NamedCheck) {
        self.checks.retain(|defined| defined.name != check.name);
        self.checks.push(check);
    }

    /// The scenario's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The window size the scenario wants, if it says.
    #[must_use]
    pub const fn window(&self) -> Option<(u32, u32)> {
        self.window
    }

    /// The setup reference the script carries, if any.
    #[must_use]
    pub const fn setup_reference(&self) -> Option<&Setup> {
        self.setup_ref.as_ref()
    }

    /// The name of the view hook the script carries, if any.
    #[must_use]
    pub fn view_reference(&self) -> Option<&str> {
        self.view_ref.as_deref()
    }

    /// The script.
    #[must_use]
    pub fn script(&self) -> &[Step] {
        &self.steps
    }

    /// The checks [`Check::Named`] can refer to.
    #[must_use]
    pub fn checks(&self) -> &[NamedCheck] {
        &self.checks
    }

    /// The checks [`Check::Custom`] can refer to.
    #[must_use]
    pub fn custom_checks(&self) -> &[CustomCheck] {
        &self.custom_checks
    }

    /// The checks that run at the end, in order: the expect list, then one
    /// [`Check::State`] per `expect_state` entry.
    #[must_use]
    pub fn expects(&self) -> Vec<Check> {
        self.expects
            .iter()
            .cloned()
            .chain(
                self.state_expects
                    .iter()
                    .map(|(key, value)| Check::state(key, value)),
            )
            .collect()
    }

    /// The probe [`Check::State`] reads the app's state with, if any.
    #[must_use]
    pub const fn state_probe(&self) -> Option<&StateProbe> {
        self.state.as_ref()
    }

    /// Runs the setup hook, if there is one. Returns whether there was.
    pub fn run_setup(&self, world: &mut World) -> bool {
        self.setup.as_ref().is_some_and(|hook| {
            hook(world);
            true
        })
    }

    /// Runs the view hook, if there is one. Returns whether there was.
    pub fn run_view(&self, world: &mut World) -> bool {
        self.view.as_ref().is_some_and(|hook| {
            hook(world);
            true
        })
    }
}

impl Scenario {
    /// The serialisable form of this scenario. Closures cannot be written
    /// out, so of the code only names survive: the hooks set with
    /// [`Scenario::setup_named`] and [`Scenario::view_named`], and the names
    /// that [`Check::Named`] and [`Check::Custom`] refer to. Anything set
    /// with the plain [`Scenario::setup`] or [`Scenario::view`] is not part
    /// of the script.
    #[must_use]
    pub fn to_script(&self) -> Script {
        Script {
            format_version: FORMAT_VERSION,
            name: self.name.clone(),
            window_size: self.window,
            setup: self.setup_ref.clone(),
            view: self.view_ref.clone(),
            steps: self.steps.clone(),
            expect: self.expects.clone(),
            expect_state: self.state_expects.clone(),
        }
    }

    /// Builds a scenario from `script`, finding every named check, setup,
    /// view and target resolver in `registry`.
    ///
    /// # Errors
    ///
    /// [`LoadError`] when the format version is not one this crate reads,
    /// when the script names a check, setup, view or target that `registry`
    /// does not have, or when it checks the app's state and `registry` has no
    /// state probe. Nothing is ever skipped.
    pub fn from_script(script: Script, registry: &Registry) -> Result<Self, LoadError> {
        crate::script::build_scenario(script, registry)
    }

    /// Reads a scenario from RON text, with the generic checks only. Chain
    /// `.expect(..)` to add Rust assertions to a recorded file.
    ///
    /// # Errors
    ///
    /// See [`Scenario::from_script`]; also a syntax error in the text.
    pub fn from_ron(text: &str) -> Result<Self, LoadError> {
        Self::from_ron_with(text, &Registry::default())
    }

    /// Reads a scenario from RON text, resolving names in `registry`.
    ///
    /// # Errors
    ///
    /// See [`Scenario::from_script`]; also a syntax error in the text.
    pub fn from_ron_with(text: &str, registry: &Registry) -> Result<Self, LoadError> {
        Self::from_script(Script::from_ron(text)?, registry)
    }

    /// The scenario as pretty RON; see [`Scenario::to_script`].
    ///
    /// # Errors
    ///
    /// [`crate::script::ExportError`] if serialisation fails.
    pub fn to_ron(&self) -> Result<String, crate::script::ExportError> {
        crate::script::export_ron(&self.to_script())
    }

    /// Puts a scenario together from a script and what the loader resolved.
    pub(crate) fn assemble(
        script: Script,
        setup: Option<Arc<HookFn>>,
        view: Option<Arc<HookFn>>,
        checks: Vec<NamedCheck>,
        custom_checks: Vec<CustomCheck>,
        state: Option<StateProbe>,
    ) -> Self {
        Self {
            name: script.name,
            setup,
            view,
            window: script.window_size,
            setup_ref: script.setup,
            view_ref: script.view,
            steps: script.steps,
            checks,
            custom_checks,
            expects: script.expect,
            state_expects: script.expect_state,
            state,
        }
    }
}

/// Wraps a check closure with its name.
pub(crate) fn named<R: IntoCheckResult>(
    name: impl Into<String>,
    check: impl Fn(&Probe<'_>) -> R + Send + Sync + 'static,
) -> NamedCheck {
    NamedCheck {
        name: name.into(),
        run: Arc::new(move |probe| check(probe).into_check_result()),
    }
}

/// Wraps a check closure that takes an argument with its name.
pub(crate) fn custom<R: IntoCheckResult>(
    name: impl Into<String>,
    check: impl Fn(&Probe<'_>, &str) -> R + Send + Sync + 'static,
) -> CustomCheck {
    CustomCheck {
        name: name.into(),
        run: Arc::new(move |probe, argument| check(probe, argument).into_check_result()),
    }
}

/// The pointer position a script starts from when nothing has moved it.
pub(crate) const ORIGIN: Vec2 = Vec2::ZERO;
