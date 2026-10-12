//! Scenario definition: setup, view, steps and checks.
//!
//! Steps, targets and checks are plain data, so a scenario converts to a
//! serialisable [`Script`] and back (see [`crate::script`]). The setup and
//! view closures are the exception: they are Rust only, so they stay out of
//! the script.

use alloc::sync::Arc;
use core::{fmt, ops::BitOr};
use std::path::{Path, PathBuf};

use bevy::{
    ecs::world::World,
    input::{keyboard::KeyCode, mouse::MouseButton},
    math::Vec2,
    reflect::{GetTypeRegistration, Reflect, TypePath},
};
use serde::{Deserialize, Serialize};

use crate::{
    check::Check,
    extension::{AppSetup, AppView, Setup, View},
    script::{FORMAT_VERSION, LoadError, Registry, Script},
    targets::{DEFAULT_ACTION_TIMEOUT, Target},
};

/// Whether a check passed; the error says what was wrong.
pub type CheckResult = Result<(), String>;

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

    /// The set without the modifiers in `other`.
    #[must_use]
    pub(crate) const fn without(self, other: Self) -> Self {
        Self {
            bits: self.bits & !other.bits,
        }
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
/// otherwise; [`Step::Check`], [`Step::Caption`] and [`Step::Group`] take none.
///
/// A step that acts on a target that names a widget first waits, a frame at
/// a time, until the target can be acted on, as Playwright does: it exists
/// (exactly one widget), is visible, is stable (it did not move during the
/// last frame) and receives the pointer (nothing covers it). A press
/// ([`Step::Click`] or [`Step::Press`]) right after a step that moved the
/// pointer to a widget also waits for that widget to be enabled and still
/// to receive the pointer, and so does the start of a [`Step::Drag`]. The
/// wait lasts at most the scenario's action timeout
/// ([`Scenario::action_timeout`], [`DEFAULT_ACTION_TIMEOUT`] frames unless
/// it says), or the step's own ([`Step::Timeout`]); when it runs out the
/// step fails, naming the first condition that did not hold. Positions name
/// no widget and never wait. [`Step::Force`] acts at once, without checks.
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
    /// Move the pointer to each position within one frame, in order: several
    /// pointer moves for the app to read at once, as a high-rate mouse
    /// delivers them. Takes one frame, and leaves the pointer at the last
    /// position. Useful for regression-testing a handler whose every move
    /// counts, such as a drag.
    MoveBurst(Vec<Vec2>),
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
    /// Type the text into whatever has keyboard focus, one character a
    /// frame: each character is a key press and release with its logical key
    /// and its text. Where the US layout needs Shift (see
    /// [`crate::keystroke`]), it goes down and up in that same frame, around
    /// the key, which is what makes a character one frame. With `into`, the
    /// target is clicked first, which focuses a text field; the click waits
    /// for the target as a [`Step::Click`] on it would. Takes one frame per
    /// character, plus the click's.
    Type {
        /// What to type.
        text: String,
        /// The widget to click before typing, if the focus is not already
        /// where the text should go.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        into: Option<Target>,
    },
    /// Do nothing for this many frames.
    Wait(u16),
    /// Run the check. Takes no frames: it sees the world as the previous step
    /// left it. A failed check fails the step but does not stop the script.
    Check(Check),
    /// Show this caption from here on, for the recorder and the gallery.
    Caption(String),
    /// The step, waiting at most this many frames for its target to be
    /// actionable instead of the scenario's action timeout. Wraps a
    /// [`Step::MoveTo`], [`Step::Glide`], [`Step::Drag`], [`Step::Click`],
    /// [`Step::Press`] or a [`Step::Type`] with an `into`:
    /// `Timeout(120, MoveTo(Text("Go")))`.
    Timeout(u16, Box<Self>),
    /// The step at once, with no waiting and no check that its target can be
    /// acted on or ends up under the pointer: Playwright's `force`. Wraps the
    /// same steps as [`Step::Timeout`]: `Force(Click(Left))` clicks a
    /// disabled button.
    Force(Box<Self>),
    /// A named run of steps, such as `"raise the bridge"`. Groups nest. A
    /// group is only a name for the reports: it plays frame for frame as its
    /// steps would without it, and takes no frame of its own. A failure
    /// inside names the group path (see [`GROUP_SEPARATOR`]), and the
    /// steps are numbered across groups as if they were one flat list.
    Group {
        /// What the group is called; not empty. A blank name is refused when
        /// a script is loaded; [`Step::group`] in Rust does not check it.
        name: String,
        /// The steps in it, which may be groups.
        steps: Vec<Self>,
    },
}

/// What separates the names of the groups around a step from each other and
/// from the step in reports: `raise the bridge > lay the first run`. ASCII,
/// as the bundled font draws nothing else.
pub const GROUP_SEPARATOR: &str = " > ";

/// A step that plays, with the names of the groups around it, outermost first.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatStep<'a> {
    /// The groups it is in.
    pub group: Vec<&'a str>,
    /// The step itself, never a [`Step::Group`].
    pub step: &'a Step,
}

/// The steps that play, in order, with their groups: `steps` with every
/// [`Step::Group`] replaced by what is inside it. A report numbers its steps
/// by this list.
#[must_use]
pub fn flatten(steps: &[Step]) -> Vec<FlatStep<'_>> {
    let mut out = Vec::new();
    flatten_into(steps, &mut Vec::new(), &mut out);
    out
}

/// [`flatten`], adding to `out` the steps of `steps` inside the groups in `path`.
fn flatten_into<'a>(steps: &'a [Step], path: &mut Vec<&'a str>, out: &mut Vec<FlatStep<'a>>) {
    for step in steps {
        if let Step::Group { name, steps } = step {
            path.push(name);
            flatten_into(steps, path, out);
            path.pop();
        } else {
            out.push(FlatStep {
                group: path.clone(),
                step,
            });
        }
    }
}

/// `text` with the group path in front, such as `raise the bridge > Click`;
/// just `text` outside any group.
#[must_use]
pub fn with_group_path(group: &[String], text: &str) -> String {
    if group.is_empty() {
        return text.to_owned();
    }
    format!("{}{GROUP_SEPARATOR}{text}", group.join(GROUP_SEPARATOR))
}

impl Step {
    /// A group called `name` of `steps`.
    #[must_use]
    pub fn group(name: impl Into<String>, steps: impl IntoIterator<Item = Self>) -> Self {
        Self::Group {
            name: name.into(),
            steps: steps.into_iter().collect(),
        }
    }

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

    /// Send the pointer to each of `positions` within one frame.
    #[must_use]
    pub fn move_burst(positions: impl IntoIterator<Item = Vec2>) -> Self {
        Self::MoveBurst(positions.into_iter().collect())
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

    /// Type `text` into whatever has keyboard focus.
    #[must_use]
    pub fn type_text(text: impl Into<String>) -> Self {
        Self::Type {
            text: text.into(),
            into: None,
        }
    }

    /// Click `into`, then type `text` (see [`Step::Type`]).
    #[must_use]
    pub fn type_into(text: impl Into<String>, into: Target) -> Self {
        Self::Type {
            text: text.into(),
            into: Some(into),
        }
    }

    /// Do nothing for `frames` frames.
    #[must_use]
    pub const fn wait(frames: u16) -> Self {
        Self::Wait(frames)
    }

    /// Run `check`.
    #[must_use]
    pub fn check(check: impl Into<Check>) -> Self {
        Self::Check(check.into())
    }

    /// Show `text` as the caption.
    #[must_use]
    pub fn caption(text: impl Into<String>) -> Self {
        Self::Caption(text.into())
    }

    /// This step, waiting at most `frames` frames for its target to be
    /// actionable (see [`Step::Timeout`]).
    #[must_use]
    pub fn with_timeout(self, frames: u16) -> Self {
        Self::Timeout(frames, Box::new(self))
    }

    /// This step at once, without the actionability checks (see
    /// [`Step::Force`]).
    #[must_use]
    pub fn forced(self) -> Self {
        Self::Force(Box::new(self))
    }

    /// The step inside any [`Step::Timeout`] and [`Step::Force`] around it.
    #[must_use]
    pub fn inner(&self) -> &Self {
        match self {
            Self::Timeout(_, inner) | Self::Force(inner) => inner.inner(),
            other => other,
        }
    }

    /// Whether [`Step::Timeout`] and [`Step::Force`] may wrap this step: one
    /// that acts on a target, or presses where one sent the pointer.
    #[must_use]
    pub const fn waits_for_target(&self) -> bool {
        matches!(
            self,
            Self::MoveTo(_)
                | Self::Glide { .. }
                | Self::Drag { .. }
                | Self::Click(_)
                | Self::Press(_)
                | Self::Type { into: Some(_), .. }
        )
    }
}

/// What a setup or view closure does to the world.
type HookFn = dyn Fn(&mut World) + Send + Sync;

/// A scenario: a name, how its world is set up and shown, a script and
/// checks.
///
/// The setup and the view are the app's own values (see
/// [`crate::extension`]), which a file holds, or closures, which only a
/// scenario in Rust can hold; a scenario may have both, and the app's value
/// runs first. Every check is data, so a scenario's checks always go into a
/// file.
///
/// ```
/// use rin_bevy_ui_tester::{Check, Scenario, Step, Target};
/// use bevy::math::Vec2;
///
/// let scenario = Scenario::new("empty")
///     .steps([Step::move_to(Target::screen(Vec2::ZERO)), Step::wait(3)])
///     .expect_check(!Check::shows("Error"));
/// assert_eq!(scenario.script().len(), 2);
/// ```
#[derive(Clone)]
pub struct Scenario {
    /// What the scenario is called.
    name: String,
    /// A closure that builds the world the script runs against, after the
    /// app's setup.
    setup: Option<Arc<HookFn>>,
    /// A closure that arranges how the world is shown (camera, window), after
    /// the app's view; the recorder and the gallery call it.
    view: Option<Arc<HookFn>>,
    /// The size of the window the scenario wants, in logical pixels.
    window: Option<(u32, u32)>,
    /// The scale factor of the window the scenario wants: physical pixels per
    /// logical pixel.
    scale_factor: Option<f32>,
    /// How many frames a step waits for its target to be actionable, when
    /// the scenario says.
    action_timeout: Option<u16>,
    /// The app's setup, which the script holds.
    app_setup: Option<Setup>,
    /// The directory the scenario's file is in, where a setup that names a
    /// file looks for it. `None` for a scenario built in code.
    base_dir: Option<PathBuf>,
    /// The app's view, which the script holds.
    app_view: Option<View>,
    /// The script.
    steps: Vec<Step>,
    /// Checks run once the script has finished.
    expects: Vec<Check>,
}

impl fmt::Debug for Scenario {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scenario")
            .field("name", &self.name)
            .field("setup", &self.setup.is_some())
            .field("view", &self.view.is_some())
            .field("window", &self.window)
            .field("scale_factor", &self.scale_factor)
            .field("action_timeout", &self.action_timeout)
            .field("app_setup", &self.app_setup)
            .field("base_dir", &self.base_dir)
            .field("app_view", &self.app_view)
            .field("steps", &self.steps)
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
            window: None,
            scale_factor: None,
            action_timeout: None,
            app_setup: None,
            base_dir: None,
            app_view: None,
            steps: Vec::new(),
            expects: Vec::new(),
        }
    }

    /// Sets a closure that builds the world, for example spawning a UI. It
    /// runs after the app's setup, if there is one, and is not part of the
    /// script: a file holds the app's setup only ([`Scenario::app_setup`]).
    #[must_use]
    pub fn setup(mut self, hook: impl Fn(&mut World) + Send + Sync + 'static) -> Self {
        self.setup = Some(Arc::new(hook));
        self
    }

    /// Sets a closure that arranges how the world is shown. Headless runs do
    /// not call it; the recorder and the gallery do, after the app's view.
    /// Like [`Scenario::setup`], it is not part of the script.
    #[must_use]
    pub fn view(mut self, hook: impl Fn(&mut World) + Send + Sync + 'static) -> Self {
        self.view = Some(Arc::new(hook));
        self
    }

    /// Sets the app's setup: a value of its `#[reflect(AppSetup)]` enum (see
    /// [`AppSetup`]), which an exported script holds as `setup: App(..)`.
    #[must_use]
    pub fn app_setup<T: AppSetup + Reflect + GetTypeRegistration + TypePath>(
        mut self,
        setup: T,
    ) -> Self {
        self.app_setup = Some(Setup::app(setup));
        self
    }

    /// Starts from a world saved with **Save as setup** in the gallery (see
    /// [`crate::save`]): `file` is its name, looked for in
    /// [`Scenario::base_dir`] when there is one. An exported script holds it
    /// as `setup: Scene("file")`.
    #[must_use]
    pub fn scene_setup(mut self, file: impl Into<String>) -> Self {
        self.app_setup = Some(Setup::Scene(file.into()));
        self
    }

    /// Sets the directory a setup that names a file (see
    /// [`Scenario::scene_setup`]) looks for it in: the scenario's file's
    /// directory, as [`crate::load_ron_file`] sets it. Without one, a relative
    /// name is relative to the working directory.
    #[must_use]
    pub fn with_base_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.base_dir = Some(dir.into());
        self
    }

    /// The directory the scenario's setup looks for the file it names in, if
    /// the scenario was loaded from a file or given one.
    #[must_use]
    pub fn base_dir(&self) -> Option<&Path> {
        self.base_dir.as_deref()
    }

    /// Sets the app's view: a value of its `#[reflect(AppView)]` enum (see
    /// [`AppView`]), which an exported script holds as `view: App(..)`.
    #[must_use]
    pub fn app_view<T: AppView + Reflect + GetTypeRegistration + TypePath>(
        mut self,
        view: T,
    ) -> Self {
        self.app_view = Some(View::app(view));
        self
    }

    /// Sets the window size the scenario wants, in logical pixels.
    #[must_use]
    pub const fn window_size(mut self, width: u32, height: u32) -> Self {
        self.window = Some((width, height));
        self
    }

    /// Sets the scale factor of the window the scenario wants: how many
    /// physical pixels make one logical pixel. The window keeps its logical
    /// size, so the layout stays the same and everything renders at that many
    /// times the pixels. A value that is not finite and above zero is refused
    /// when a script holding it loads, and ignored by the headless app.
    #[must_use]
    pub const fn scale_factor(mut self, scale_factor: f32) -> Self {
        self.scale_factor = Some(scale_factor);
        self
    }

    /// Sets how many frames a step waits for its target to be actionable
    /// before it fails, instead of [`DEFAULT_ACTION_TIMEOUT`]. A step can
    /// still set its own with [`Step::Timeout`].
    #[must_use]
    pub const fn action_timeout(mut self, frames: u16) -> Self {
        self.action_timeout = Some(frames);
        self
    }

    /// How many frames a step waits for its target to be actionable unless
    /// it says otherwise.
    #[must_use]
    pub fn action_timeout_frames(&self) -> u16 {
        self.action_timeout.unwrap_or(DEFAULT_ACTION_TIMEOUT)
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

    /// Runs `check` once the script has finished, for example
    /// `Check::shows("Saved")`.
    #[must_use]
    pub fn expect_check(mut self, check: impl Into<Check>) -> Self {
        self.expects.push(check.into());
        self
    }

    /// Adds `check` to the checks that run at the end, as
    /// [`Scenario::expect_check`] does, in place.
    #[cfg(feature = "gallery")]
    pub(crate) fn push_expect(&mut self, check: Check) {
        self.expects.push(check);
    }

    /// A copy called `name` that runs in another environment: whichever of
    /// the window size, the scale factor, the app's setup and the app's view
    /// is given replaces the scenario's own. The steps and checks stay.
    pub(crate) fn in_environment(
        &self,
        name: String,
        window: Option<(u32, u32)>,
        scale_factor: Option<f32>,
        setup: Option<&Setup>,
        view: Option<&View>,
    ) -> Self {
        let mut varied = self.clone();
        varied.name = name;
        varied.window = window.or(varied.window);
        varied.scale_factor = scale_factor.or(varied.scale_factor);
        if let Some(setup) = setup {
            varied.app_setup = Some(setup.clone());
        }
        if let Some(view) = view {
            varied.app_view = Some(view.clone());
        }
        varied
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

    /// The scale factor the scenario wants, if it says.
    #[must_use]
    pub const fn scale(&self) -> Option<f32> {
        self.scale_factor
    }

    /// The app's setup, which the script carries, if any.
    #[must_use]
    pub const fn setup_value(&self) -> Option<&Setup> {
        self.app_setup.as_ref()
    }

    /// The app's view, which the script carries, if any.
    #[must_use]
    pub const fn view_value(&self) -> Option<&View> {
        self.app_view.as_ref()
    }

    /// The script.
    #[must_use]
    pub fn script(&self) -> &[Step] {
        &self.steps
    }

    /// The checks that run at the end, in order.
    #[must_use]
    pub fn expects(&self) -> Vec<Check> {
        self.expects.clone()
    }

    /// Builds the scenario's world: the app's setup, then the setup
    /// closure, each if there is one. Returns whether there was either.
    ///
    /// # Errors
    ///
    /// Why the app's setup failed; the closure does not run then.
    pub fn run_setup(&self, world: &mut World) -> Result<bool, String> {
        if let Some(setup) = &self.app_setup {
            setup.build(world, self.base_dir.as_deref())?;
        }
        if let Some(hook) = &self.setup {
            hook(world);
        }
        Ok(self.app_setup.is_some() || self.setup.is_some())
    }

    /// Arranges the view: the app's view, then the view closure, each if
    /// there is one. Returns whether there was either.
    ///
    /// # Errors
    ///
    /// Why the app's view could not be arranged, such as a value that does
    /// not read as the app's view type; the closure does not run then.
    pub fn run_view(&self, world: &mut World) -> Result<bool, String> {
        if let Some(view) = &self.app_view {
            view.arrange(world)?;
        }
        if let Some(hook) = &self.view {
            hook(world);
        }
        Ok(self.app_view.is_some() || self.view.is_some())
    }
}

impl Scenario {
    /// The serialisable form of this scenario. Closures cannot be written
    /// out, so the setup and view closures ([`Scenario::setup`],
    /// [`Scenario::view`]) are not part of the script, while the app's own
    /// setup and view are.
    #[must_use]
    pub fn to_script(&self) -> Script {
        Script {
            format_version: FORMAT_VERSION,
            name: self.name.clone(),
            window_size: self.window,
            scale_factor: self.scale_factor,
            action_timeout: self.action_timeout,
            setup: self.app_setup.clone(),
            view: self.app_view.clone(),
            steps: self.steps.clone(),
            expect: self.expects.clone(),
        }
    }

    /// Builds a scenario from `script`, reading its setup, view, targets and
    /// checks inside `App(..)` as the app's types in [`Registry::types`], and
    /// checking the types and fields its checks name.
    ///
    /// # Errors
    ///
    /// [`LoadError`] when the format version is not one this crate reads,
    /// when a value inside `App(..)` does not read as the app's type, or when
    /// a check names a type or field that `registry` does not have. Nothing
    /// is ever skipped.
    pub fn from_script(script: Script, registry: &Registry) -> Result<Self, LoadError> {
        crate::script::build_scenario(script, registry)
    }

    /// Reads a scenario from RON text, with a default [`Registry`]: every
    /// type that derives `Reflect`, the app's own setup, view, target and
    /// check types among them. Chain `.expect_check(..)` to add checks to a
    /// recorded file.
    ///
    /// # Errors
    ///
    /// See [`Scenario::from_script`]; also a syntax error in the text.
    pub fn from_ron(text: &str) -> Result<Self, LoadError> {
        Self::from_ron_with(text, &Registry::default())
    }

    /// Reads a scenario from RON text, against the types in `registry`.
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

    /// Puts a scenario together from a script the loader has read.
    pub(crate) fn assemble(script: Script) -> Self {
        Self {
            name: script.name,
            setup: None,
            view: None,
            window: script.window_size,
            scale_factor: script.scale_factor,
            action_timeout: script.action_timeout,
            app_setup: script.setup,
            base_dir: None,
            app_view: script.view,
            steps: script.steps,
            expects: script.expect,
        }
    }
}

/// The pointer position a script starts from when nothing has moved it.
pub(crate) const ORIGIN: Vec2 = Vec2::ZERO;
