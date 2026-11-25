//! Scenario definition: setup, view, steps and checks.
//!
//! Steps, targets and checks are plain data, so a scenario converts to a serializable
//! [`Script`] and back. The setup and view closures are Rust only and stay out of the script.
//!
//! A [`Step::Group`] is only a label: [`flatten`] gives the leaf steps with their path of
//! names, and everything that plays, numbers or reports steps works on that flat list.

use alloc::sync::Arc;
use core::{fmt, ops::BitOr};
use std::path::{Path, PathBuf};

use bevy::{
    ecs::world::World,
    input::{keyboard::KeyCode, mouse::MouseButton},
    math::Vec2,
    reflect::{GetTypeRegistration, Reflect, TypePath},
    text::RemSize,
};
use serde::{Deserialize, Serialize};

use crate::{
    annotation::Disabled,
    check::{Check, Measure},
    extension::{AppSetup, AppView, Setup, View},
    script::{FORMAT_VERSION, LoadError, Registry, Script},
    targets::{DEFAULT_ACTION_TIMEOUT, Target},
};

pub type CheckResult = Result<(), String>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    bits: u8,
}

impl Modifiers {
    pub const NONE: Self = Self { bits: 0 };
    pub const SHIFT: Self = Self { bits: 1 };
    pub const CTRL: Self = Self { bits: 2 };
    pub const ALT: Self = Self { bits: 4 };
    pub const SUPER: Self = Self { bits: 8 };

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.bits & other.bits == other.bits
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }

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

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde passes a reference"
)]
const fn is_first_finger(finger: &u8) -> bool {
    *finger == 0
}

fn no_modifiers<T: Default + PartialEq>(modifiers: &T) -> bool {
    *modifiers == T::default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Modifier {
    Shift,
    Ctrl,
    Alt,
    Super,
}

impl Modifier {
    const ALL: [(Self, Modifiers); 4] = [
        (Self::Shift, Modifiers::SHIFT),
        (Self::Ctrl, Modifiers::CTRL),
        (Self::Alt, Modifiers::ALT),
        (Self::Super, Modifiers::SUPER),
    ];
}

/// Written as a list such as `[Ctrl, Shift]`, not raw bits, so files stay hand-editable.
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
/// An input step takes one frame; `Check`, `Capture`, `Caption` and `Group` take none. A step
/// acting on a widget waits first, as Playwright does, until it is actionable or the action
/// timeout runs out; positions never wait.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Step {
    MoveTo(Target),
    /// Eased path over `frames` frames, at human speed. The target is worked out when the
    /// step starts.
    Glide {
        target: Target,
        frames: u16,
    },
    /// Several pointer moves in one frame, as a high-rate mouse delivers them; leaves the
    /// pointer at the last position.
    MoveBurst(Vec<Vec2>),
    Press(MouseButton),
    Release(MouseButton),
    Click(MouseButton),
    /// Takes `frames + 2` frames.
    Drag {
        from: Target,
        to: Target,
        frames: u16,
    },
    /// The finger is not the mouse: the cursor and `Hovered` stay where they are.
    TouchDown {
        at: Target,
        #[serde(default, skip_serializing_if = "is_first_finger")]
        finger: u8,
    },
    TouchMove {
        to: Target,
        #[serde(default, skip_serializing_if = "is_first_finger")]
        finger: u8,
    },
    TouchUp {
        #[serde(default, skip_serializing_if = "is_first_finger")]
        finger: u8,
    },
    /// Ends the pointer with no click. On Bevy 0.20 widgets see `Pointer<Out>`, never
    /// `Pointer<Cancel>`, as on a device; an app that must react reads `Touches`
    /// (`just_canceled`).
    TouchCancel {
        #[serde(default, skip_serializing_if = "is_first_finger")]
        finger: u8,
    },
    /// Two frames, because Bevy only clicks an entity a pointer hovered the frame before.
    TouchTap(Target),
    /// Takes `frames + 2` frames.
    TouchDrag {
        from: Target,
        to: Target,
        frames: u16,
    },
    Wheel {
        x: f32,
        y: f32,
    },
    Key {
        key: KeyCode,
        #[serde(default, skip_serializing_if = "no_modifiers")]
        modifiers: Modifiers,
    },
    KeyUp {
        key: KeyCode,
        #[serde(default, skip_serializing_if = "no_modifiers")]
        modifiers: Modifiers,
    },
    /// All in one frame: Bevy sees the key as just pressed and just released at once.
    Tap {
        key: KeyCode,
        #[serde(default, skip_serializing_if = "no_modifiers")]
        modifiers: Modifiers,
    },
    /// One character a frame; where the US layout needs Shift it goes down and up in that same
    /// frame. With `into`, the target is clicked first, waiting as a [`Step::Click`] would.
    Type {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        into: Option<Target>,
    },
    Wait(u16),
    /// Sees the world as the previous step left it. A failure does not stop the script.
    Check(Check),
    /// Keeps what `value` reads now as `name`, for later checks to compare with by
    /// `Captured("name")`. The name is unique, and a check may use it only after the capture
    /// (`expect`, which runs last, may use any). An unreadable measurement stops the script.
    Capture {
        name: String,
        value: Measure,
    },
    Caption(String),
    /// Waits at most `frames` for the target instead of the action timeout. Any step that does
    /// not wait for a target is a load error: `Timeout(120, MoveTo(Text("Go")))`.
    Timeout(u16, Box<Self>),
    /// Acts at once, with no waiting or checks (Playwright's `force`): `Force(Click(Left))`
    /// clicks a disabled button.
    Force(Box<Self>),
    /// Only a name for reports: plays frame for frame as its steps would, and steps are
    /// numbered across groups as one flat list.
    Group {
        /// Not empty: a blank name is refused when a script is loaded, but [`Step::group`] does
        /// not check it.
        name: String,
        steps: Vec<Self>,
    },
}

/// ASCII, as the bundled font draws nothing else.
pub const GROUP_SEPARATOR: &str = " > ";

#[derive(Debug, Clone, PartialEq)]
pub struct FlatStep<'a> {
    pub group: Vec<&'a str>,
    pub step: &'a Step,
}

#[must_use]
pub fn flatten(steps: &[Step]) -> Vec<FlatStep<'_>> {
    let mut out = Vec::new();
    flatten_into(steps, &mut Vec::new(), &mut out);
    out
}

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

#[must_use]
pub fn with_group_path(group: &[String], text: &str) -> String {
    if group.is_empty() {
        return text.to_owned();
    }
    format!("{}{GROUP_SEPARATOR}{text}", group.join(GROUP_SEPARATOR))
}

impl Step {
    #[must_use]
    pub fn group(name: impl Into<String>, steps: impl IntoIterator<Item = Self>) -> Self {
        Self::Group {
            name: name.into(),
            steps: steps.into_iter().collect(),
        }
    }

    #[must_use]
    pub const fn move_to(target: Target) -> Self {
        Self::MoveTo(target)
    }

    #[must_use]
    pub const fn glide(target: Target, frames: u16) -> Self {
        Self::Glide { target, frames }
    }

    #[must_use]
    pub fn move_burst(positions: impl IntoIterator<Item = Vec2>) -> Self {
        Self::MoveBurst(positions.into_iter().collect())
    }

    #[must_use]
    pub const fn click() -> Self {
        Self::Click(MouseButton::Left)
    }

    #[must_use]
    pub const fn press() -> Self {
        Self::Press(MouseButton::Left)
    }

    #[must_use]
    pub const fn release() -> Self {
        Self::Release(MouseButton::Left)
    }

    #[must_use]
    pub const fn drag(from: Target, to: Target, frames: u16) -> Self {
        Self::Drag { from, to, frames }
    }

    #[must_use]
    pub const fn touch_down(at: Target) -> Self {
        Self::TouchDown { at, finger: 0 }
    }

    #[must_use]
    pub const fn touch_move(to: Target) -> Self {
        Self::TouchMove { to, finger: 0 }
    }

    #[must_use]
    pub const fn touch_up() -> Self {
        Self::TouchUp { finger: 0 }
    }

    #[must_use]
    pub const fn touch_cancel() -> Self {
        Self::TouchCancel { finger: 0 }
    }

    #[must_use]
    pub const fn touch_tap(target: Target) -> Self {
        Self::TouchTap(target)
    }

    #[must_use]
    pub const fn touch_drag(from: Target, to: Target, frames: u16) -> Self {
        Self::TouchDrag { from, to, frames }
    }

    /// The same touch step for another finger; other steps are unchanged.
    #[must_use]
    pub const fn with_finger(mut self, id: u8) -> Self {
        match &mut self {
            Self::TouchDown { finger, .. }
            | Self::TouchMove { finger, .. }
            | Self::TouchUp { finger }
            | Self::TouchCancel { finger } => *finger = id,
            _ => {}
        }
        self
    }

    #[must_use]
    pub const fn wheel(x: f32, y: f32) -> Self {
        Self::Wheel { x, y }
    }

    #[must_use]
    pub const fn tap(key: KeyCode) -> Self {
        Self::Tap {
            key,
            modifiers: Modifiers::NONE,
        }
    }

    #[must_use]
    pub const fn tap_with(key: KeyCode, modifiers: Modifiers) -> Self {
        Self::Tap { key, modifiers }
    }

    #[must_use]
    pub fn type_text(text: impl Into<String>) -> Self {
        Self::Type {
            text: text.into(),
            into: None,
        }
    }

    #[must_use]
    pub fn type_into(text: impl Into<String>, into: Target) -> Self {
        Self::Type {
            text: text.into(),
            into: Some(into),
        }
    }

    #[must_use]
    pub const fn wait(frames: u16) -> Self {
        Self::Wait(frames)
    }

    #[must_use]
    pub fn check(check: impl Into<Check>) -> Self {
        Self::Check(check.into())
    }

    #[must_use]
    pub fn capture(name: impl Into<String>, value: Measure) -> Self {
        Self::Capture {
            name: name.into(),
            value,
        }
    }

    #[must_use]
    pub fn caption(text: impl Into<String>) -> Self {
        Self::Caption(text.into())
    }

    #[must_use]
    pub fn with_timeout(self, frames: u16) -> Self {
        Self::Timeout(frames, Box::new(self))
    }

    #[must_use]
    pub fn forced(self) -> Self {
        Self::Force(Box::new(self))
    }

    #[must_use]
    pub fn inner(&self) -> &Self {
        match self {
            Self::Timeout(_, inner) | Self::Force(inner) => inner.inner(),
            other => other,
        }
    }

    #[must_use]
    pub const fn waits_for_target(&self) -> bool {
        matches!(
            self,
            Self::MoveTo(_)
                | Self::Glide { .. }
                | Self::Drag { .. }
                | Self::TouchDown { .. }
                | Self::TouchMove { .. }
                | Self::TouchTap(_)
                | Self::TouchDrag { .. }
                | Self::Click(_)
                | Self::Press(_)
                | Self::Type { into: Some(_), .. }
        )
    }
}

type HookFn = dyn Fn(&mut World) + Send + Sync;

/// A scenario: a name, how its world is set up and shown, a script and checks.
///
/// The app's own setup and view run before any closures.
///
/// ```
/// use bevy_stagecraft::{Check, Scenario, Step, Target};
/// use bevy::math::Vec2;
///
/// let scenario = Scenario::new("empty")
///     .steps([Step::move_to(Target::screen(Vec2::ZERO)), Step::wait(3)])
///     .expect_check(!Check::shows("Error"));
/// assert_eq!(scenario.script().len(), 2);
/// ```
#[derive(Clone)]
pub struct Scenario {
    name: String,
    setup: Option<Arc<HookFn>>,
    view: Option<Arc<HookFn>>,
    window: Option<(u32, u32)>,
    scale_factor: Option<f32>,
    /// Set only by a variant, so it is not in the script.
    rem_size: Option<f32>,
    action_timeout: Option<u16>,
    app_setup: Option<Setup>,
    base_dir: Option<PathBuf>,
    app_view: Option<View>,
    variants: Vec<String>,
    tags: Vec<String>,
    skip: Option<String>,
    fixme: Option<String>,
    should_fail: Option<String>,
    steps: Vec<Step>,
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
            .field("rem_size", &self.rem_size)
            .field("action_timeout", &self.action_timeout)
            .field("app_setup", &self.app_setup)
            .field("base_dir", &self.base_dir)
            .field("app_view", &self.app_view)
            .field("variants", &self.variants)
            .field("tags", &self.tags)
            .field("skip", &self.skip)
            .field("fixme", &self.fixme)
            .field("should_fail", &self.should_fail)
            .field("steps", &self.steps)
            .field("expects", &self.expects)
            .finish()
    }
}

impl Scenario {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            setup: None,
            view: None,
            window: None,
            scale_factor: None,
            rem_size: None,
            action_timeout: None,
            app_setup: None,
            base_dir: None,
            app_view: None,
            variants: Vec::new(),
            tags: Vec::new(),
            skip: None,
            fixme: None,
            should_fail: None,
            steps: Vec::new(),
            expects: Vec::new(),
        }
    }

    /// Runs after the app's setup and is not part of the script: a file holds the app's setup
    /// only.
    #[must_use]
    pub fn setup(mut self, hook: impl Fn(&mut World) + Send + Sync + 'static) -> Self {
        self.setup = Some(Arc::new(hook));
        self
    }

    /// Headless runs do not call it; the recorder and the gallery do. Not part of the script.
    #[must_use]
    pub fn view(mut self, hook: impl Fn(&mut World) + Send + Sync + 'static) -> Self {
        self.view = Some(Arc::new(hook));
        self
    }

    #[must_use]
    pub fn app_setup<T: AppSetup + Reflect + GetTypeRegistration + TypePath>(
        mut self,
        setup: T,
    ) -> Self {
        self.app_setup = Some(Setup::app(setup));
        self
    }

    /// `file` is looked for in [`Scenario::base_dir`] when there is one.
    #[must_use]
    pub fn scene_setup(mut self, file: impl Into<String>) -> Self {
        self.app_setup = Some(Setup::Scene(file.into()));
        self
    }

    /// Without one, a relative name is relative to the working directory.
    #[must_use]
    pub fn with_base_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.base_dir = Some(dir.into());
        self
    }

    #[must_use]
    pub fn base_dir(&self) -> Option<&Path> {
        self.base_dir.as_deref()
    }

    #[must_use]
    pub fn app_view<T: AppView + Reflect + GetTypeRegistration + TypePath>(
        mut self,
        view: T,
    ) -> Self {
        self.app_view = Some(View::app(view));
        self
    }

    #[must_use]
    pub const fn window_size(mut self, width: u32, height: u32) -> Self {
        self.window = Some((width, height));
        self
    }

    /// The window keeps its logical size. A value that is not finite and above zero is refused
    /// when a script holding it loads, and ignored by the headless app.
    #[must_use]
    pub const fn scale_factor(mut self, scale_factor: f32) -> Self {
        self.scale_factor = Some(scale_factor);
        self
    }

    /// A runner [skips](crate::Variant::skips) the other variants rather than failing. The
    /// default variant answers to `default`; empty runs in every variant.
    #[must_use]
    pub fn variants(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.variants = names.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn variant_names(&self) -> &[String] {
        &self.variants
    }

    /// `run_ron_tests` reads `RIN_SCENARIO_TAGS` (see [`crate::TagFilter`]).
    #[must_use]
    pub fn tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn tag_names(&self) -> &[String] {
        &self.tags
    }

    #[must_use]
    pub fn skip(mut self, reason: impl Into<String>) -> Self {
        self.skip = Some(reason.into());
        self
    }

    /// Not run, and listed apart from the skipped ones.
    #[must_use]
    pub fn fixme(mut self, reason: impl Into<String>) -> Self {
        self.fixme = Some(reason.into());
        self
    }

    /// Passes only if it fails; a run that passes is reported as a failure, so a fixed bug is
    /// noticed.
    #[must_use]
    pub fn should_fail(mut self, reason: impl Into<String>) -> Self {
        self.should_fail = Some(reason.into());
        self
    }

    #[must_use]
    pub fn skip_reason(&self) -> Option<&str> {
        self.skip.as_deref()
    }

    #[must_use]
    pub fn fixme_reason(&self) -> Option<&str> {
        self.fixme.as_deref()
    }

    #[must_use]
    pub fn should_fail_reason(&self) -> Option<&str> {
        self.should_fail.as_deref()
    }

    /// Skip wins over fixme when both are set.
    #[must_use]
    pub fn disabled(&self) -> Option<Disabled> {
        self.skip
            .clone()
            .map(Disabled::Skip)
            .or_else(|| self.fixme.clone().map(Disabled::Fixme))
    }

    /// A step can still set its own with [`Step::Timeout`].
    #[must_use]
    pub const fn action_timeout(mut self, frames: u16) -> Self {
        self.action_timeout = Some(frames);
        self
    }

    #[must_use]
    pub fn action_timeout_frames(&self) -> u16 {
        self.action_timeout.unwrap_or(DEFAULT_ACTION_TIMEOUT)
    }

    #[must_use]
    pub fn step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    #[must_use]
    pub fn steps(mut self, steps: impl IntoIterator<Item = Step>) -> Self {
        self.steps.extend(steps);
        self
    }

    #[must_use]
    pub fn expect_check(mut self, check: impl Into<Check>) -> Self {
        self.expects.push(check.into());
        self
    }

    #[cfg(feature = "gallery")]
    pub(crate) fn push_expect(&mut self, check: Check) {
        self.expects.push(check);
    }

    pub(crate) fn in_environment(
        &self,
        name: String,
        window: Option<(u32, u32)>,
        scale_factor: Option<f32>,
        rem_size: Option<f32>,
        setup: Option<&Setup>,
        view: Option<&View>,
    ) -> Self {
        let mut varied = self.clone();
        varied.name = name;
        varied.window = window.or(varied.window);
        varied.scale_factor = scale_factor.or(varied.scale_factor);
        varied.rem_size = rem_size.or(varied.rem_size);
        if let Some(setup) = setup {
            varied.app_setup = Some(setup.clone());
        }
        if let Some(view) = view {
            varied.app_view = Some(view.clone());
        }
        varied
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn window(&self) -> Option<(u32, u32)> {
        self.window
    }

    #[must_use]
    pub const fn scale(&self) -> Option<f32> {
        self.scale_factor
    }

    #[must_use]
    pub const fn rem_size(&self) -> Option<f32> {
        self.rem_size
    }

    /// Returns what the world had, for [`restore_rem_size`]; changes nothing where the rem size
    /// is unset or not finite and above zero.
    pub(crate) fn fit_rem_size(&self, world: &mut World) -> Option<RemSize> {
        let rem = self.rem_size.filter(|rem| check_rem_size(*rem).is_ok())?;
        let before = world.get_resource::<RemSize>().copied().unwrap_or_default();
        world.insert_resource(RemSize(rem));
        Some(before)
    }

    #[must_use]
    pub const fn setup_value(&self) -> Option<&Setup> {
        self.app_setup.as_ref()
    }

    #[must_use]
    pub const fn view_value(&self) -> Option<&View> {
        self.app_view.as_ref()
    }

    #[must_use]
    pub fn script(&self) -> &[Step] {
        &self.steps
    }

    #[must_use]
    pub fn expects(&self) -> Vec<Check> {
        self.expects.clone()
    }

    /// Whether the scenario had a setup.
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

    /// Whether the scenario had a view.
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
    /// Closures cannot be written out, so only the app's own setup and view are in it.
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
            variants: self.variants.clone(),
            tags: self.tags.clone(),
            skip: self.skip.clone(),
            fixme: self.fixme.clone(),
            should_fail: self.should_fail.clone(),
            steps: self.steps.clone(),
            expect: self.expects.clone(),
        }
    }

    /// # Errors
    ///
    /// [`LoadError`] for an unsupported format version, an `App(..)` value that does not read
    /// as the app's type, or a check naming a type or field `registry` lacks.
    pub fn from_script(script: Script, registry: &Registry) -> Result<Self, LoadError> {
        crate::script::build_scenario(script, registry)
    }

    /// Chain `.expect_check(..)` to add checks to a recorded file.
    ///
    /// # Errors
    ///
    /// See [`Scenario::from_script`]; also a syntax error in the text.
    pub fn from_ron(text: &str) -> Result<Self, LoadError> {
        Self::from_ron_with(text, &Registry::default())
    }

    /// # Errors
    ///
    /// See [`Scenario::from_script`]; also a syntax error in the text.
    pub fn from_ron_with(text: &str, registry: &Registry) -> Result<Self, LoadError> {
        Self::from_script(Script::from_ron(text)?, registry)
    }

    /// # Errors
    ///
    /// [`crate::script::ExportError`] if serialization fails.
    pub fn to_ron(&self) -> Result<String, crate::script::ExportError> {
        crate::script::export_ron(&self.to_script())
    }

    pub(crate) fn assemble(script: Script) -> Self {
        Self {
            name: script.name,
            setup: None,
            view: None,
            window: script.window_size,
            scale_factor: script.scale_factor,
            rem_size: None,
            action_timeout: script.action_timeout,
            app_setup: script.setup,
            base_dir: None,
            app_view: script.view,
            variants: script.variants,
            tags: script.tags,
            skip: script.skip,
            fixme: script.fixme,
            should_fail: script.should_fail,
            steps: script.steps,
            expects: script.expect,
        }
    }
}

pub(crate) const ORIGIN: Vec2 = Vec2::ZERO;

pub(crate) fn restore_rem_size(world: &mut World, before: Option<RemSize>) {
    if let Some(before) = before {
        world.insert_resource(before);
    }
}

pub(crate) fn check_rem_size(rem_size: f32) -> Result<(), String> {
    if rem_size.is_finite() && rem_size > 0.0 {
        Ok(())
    } else {
        Err(format!(
            "the rem size is {rem_size}; it must be finite and above zero"
        ))
    }
}
