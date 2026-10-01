//! Scenario definition: setup, view, steps and checks.
//!
//! Steps and targets are plain data and checks are referenced by name, so a
//! serialised form can be added later without reshaping any of it.

use alloc::sync::Arc;
use core::{fmt, ops::BitOr};

use bevy::{
    ecs::world::World,
    input::{keyboard::KeyCode, mouse::MouseButton},
    math::Vec2,
};

use crate::{probe::Probe, targets::Target};

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
#[derive(Debug, Clone, PartialEq)]
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
        modifiers: Modifiers,
    },
    /// Release the key, then the modifiers.
    KeyUp {
        /// The key.
        key: KeyCode,
        /// The modifiers to let go of with it.
        modifiers: Modifiers,
    },
    /// Press and release the key, with the modifiers held around it, all in
    /// one frame. Bevy sees the key as just pressed and just released at once.
    Tap {
        /// The key.
        key: KeyCode,
        /// The modifiers held around it.
        modifiers: Modifiers,
    },
    /// Do nothing for this many frames.
    Wait(u16),
    /// Run the check of this name, which the scenario defines. Takes no
    /// frames: it sees the world as the previous step left it.
    Check(String),
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

    /// Run the check called `name`.
    #[must_use]
    pub fn check(name: impl Into<String>) -> Self {
        Self::Check(name.into())
    }

    /// Show `text` as the caption.
    #[must_use]
    pub fn caption(text: impl Into<String>) -> Self {
        Self::Caption(text.into())
    }
}

/// What a hook does to the world.
type HookFn = dyn Fn(&mut World) + Send + Sync;

/// What a check does with a probe.
type CheckFn = dyn Fn(&Probe<'_>) -> CheckResult + Send + Sync;

/// A check with its name.
#[derive(Clone)]
pub struct NamedCheck {
    /// The name steps and reports refer to it by.
    pub name: String,
    /// The check itself.
    run: Arc<CheckFn>,
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
    /// The script.
    steps: Vec<Step>,
    /// Checks that [`Step::Check`] refers to.
    checks: Vec<NamedCheck>,
    /// Checks run once the script has finished.
    expects: Vec<NamedCheck>,
}

impl fmt::Debug for Scenario {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scenario")
            .field("name", &self.name)
            .field("setup", &self.setup.is_some())
            .field("view", &self.view.is_some())
            .field("steps", &self.steps)
            .field("checks", &self.checks)
            .field("expects", &self.expects)
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
            steps: Vec::new(),
            checks: Vec::new(),
            expects: Vec::new(),
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

    /// Defines a check that a [`Step::Check`] of the same name runs.
    #[must_use]
    pub fn check<R: IntoCheckResult>(
        mut self,
        name: impl Into<String>,
        check: impl Fn(&Probe<'_>) -> R + Send + Sync + 'static,
    ) -> Self {
        self.checks.push(named(name, check));
        self
    }

    /// Defines a check that runs once the script has finished.
    #[must_use]
    pub fn expect<R: IntoCheckResult>(
        mut self,
        name: impl Into<String>,
        check: impl Fn(&Probe<'_>) -> R + Send + Sync + 'static,
    ) -> Self {
        self.expects.push(named(name, check));
        self
    }

    /// The scenario's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The script.
    #[must_use]
    pub fn script(&self) -> &[Step] {
        &self.steps
    }

    /// The checks [`Step::Check`] can refer to.
    #[must_use]
    pub fn checks(&self) -> &[NamedCheck] {
        &self.checks
    }

    /// The checks that run at the end.
    #[must_use]
    pub fn expects(&self) -> &[NamedCheck] {
        &self.expects
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

/// Wraps a check closure with its name.
fn named<R: IntoCheckResult>(
    name: impl Into<String>,
    check: impl Fn(&Probe<'_>) -> R + Send + Sync + 'static,
) -> NamedCheck {
    NamedCheck {
        name: name.into(),
        run: Arc::new(move |probe| check(probe).into_check_result()),
    }
}

/// The pointer position a script starts from when nothing has moved it.
pub(crate) const ORIGIN: Vec2 = Vec2::ZERO;
