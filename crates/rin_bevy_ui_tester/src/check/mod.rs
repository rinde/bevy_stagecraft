//! Checks as data: what a [`Step::Check`](crate::Step::Check) or an
//! expectation asks of the world.
//!
//! The common questions are built in and spelled out in the script itself, so
//! a `.ron` file reads on its own: is this text shown, is this under the
//! pointer, how many entities have a component, what a resource or a
//! component field holds, which state the app is in, whether a widget is
//! disabled, checked, pressed, focused or visible, what a target's text says.
//! Types and fields are found through Bevy's type registry and written as
//! short type paths with a reflect path after them (`Editor.tool`). Anything
//! else is one of the app's own checks: a value of the one enum it marks
//! with `#[reflect(AppCheck)]` (see [`AppCheck`]), written inside `App(..)`.

mod app;
mod capture;
pub mod pick;
mod reflect;
pub mod screenshot;
pub mod suggest;
mod tree;
mod value;
mod widget;

use alloc::collections::BTreeMap;
use core::{fmt, ops::Not};

use bevy::{
    ecs::{
        component::{Component, ComponentId},
        entity::Entity,
        hierarchy::ChildOf,
        resource::Resource,
        world::World,
    },
    reflect::{GetTypeRegistration, Reflect, TypePath, TypeRegistry},
    state::state::States,
};
use ron::value::RawValue;
use serde::{Deserialize, Serialize, de::DeserializeSeed};

pub use self::screenshot::{ScreenshotRecord, ScreenshotResult, ScreenshotSettings, Tolerance};
pub use self::{
    app::{AppCheck, ReflectAppCheck},
    capture::Measure,
    value::{Comparison, TextMatch, Unit, Value},
};
use self::{
    reflect::{Reflected, Role, find_type},
    value::{Comparable, Counted},
    widget::WidgetState,
};
use crate::{
    extension::{
        AppCheckValue, Extension, NO_CUSTOM_TARGET, bad_app_value, no_app_type, several_message,
    },
    player::Outcome,
    probe::{Probe, check_hover},
    scenario::CheckResult,
    targets::{Lookup, Target, resolve},
};

/// What a check asks of the world.
///
/// In a script every check is spelled out, the app's own inside `App(..)`:
///
/// ```ron
/// Check(App(StationNamed("Alpha"))),
/// Check(Shows("Score: 3")),
/// Check(Hovered(Text("OK"))),
/// Check(Count(with: ["Ghost"], is: Eq(1))),
/// Check(Resource(path: "Editor.tool", is: Eq(Lay))),
/// Check(Component(target: Text("Speed"), path: "SliderValue.0", is: Gt(0.5))),
/// Check(InState(state: "Mode", value: Editing)),
/// Check(Disabled(Text("Save"))),
/// Check(TextOf(Text("Score"), StartsWith("Score: "))),
/// Check(Tree(of: Name("Toolbar"), matches: r#"
///     - panel:
///       - button "Pause"
///       - slider "Speed" [value=0.5]
/// "#)),
/// Check(All([Shows("Saved"), Not(Shows("Error"))])),
/// Check(Within(30, Shows("Saved"))),
/// ```
///
/// In Rust the same checks are typed, and type paths are only for files.
/// `!check` wraps a check in [`Check::Not`]. Every check is data: there is no
/// check written as a closure, so every scenario can be written to a file.
///
/// ```
/// use rin_bevy_ui_tester::{Check, Target};
///
/// let check = !Check::hovered(Target::text("OK"));
/// assert_eq!(check.to_string(), "not: Text(\"OK\") is under the pointer");
/// ///
/// #[derive(bevy::prelude::Component, bevy::prelude::Reflect)]
/// struct Ghost;
/// assert_eq!(Check::count::<Ghost>().eq(1).to_string(), "count of `Ghost` is 1");
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Check {
    /// One of the app's own checks: a value of the type the app marks with
    /// `#[reflect(AppCheck)]` (see [`AppCheck`]). Written as
    /// `App(StationNamed("Alpha"))`.
    App(AppCheckValue),
    /// Some UI node of the app shows exactly this text.
    Shows(String),
    /// The target's widget, or something below it, is under the pointer. A
    /// target that is only a position names no widget, and the check fails.
    Hovered(Target),
    /// How many entities of the app have every one of these components.
    /// Disabled entities are left out, as queries leave them out.
    Count {
        /// The components, by registered type path.
        with: Vec<String>,
        /// How the count must compare.
        is: Comparison,
    },
    /// A field of a reflected resource: the resource's type path, then a
    /// reflect path into it (`Editor.tool`), or the type alone for the whole
    /// resource.
    Resource {
        /// The type and the field.
        path: String,
        /// Whether a physical value is read as logical. See [`Unit`].
        unit: Unit,
        /// How the field must compare.
        is: Comparison,
    },
    /// A field of a reflected component on the target's node or, failing
    /// that, on the nearest node above it that has the component, so a label
    /// finds the slider it sits in.
    Component {
        /// Where the component is.
        target: Target,
        /// The component's type path, then a reflect path into it.
        path: String,
        /// Whether a physical value is read as logical. See [`Unit`].
        unit: Unit,
        /// How the field must compare.
        is: Comparison,
    },
    /// The current value of a Bevy `State<S>`. The state type must be
    /// registered as a state (`register_type_state`).
    InState {
        /// The state's type path.
        state: String,
        /// The value it must have.
        value: Value,
    },
    /// The target's widget carries `InteractionDisabled`.
    Disabled(Target),
    /// The target's widget carries `Checked`.
    Checked(Target),
    /// The target's widget carries `Selected`, as a list item or a swatch
    /// that is the chosen one, or a tab its list's `SelectedTab` names.
    Selected(Target),
    /// The target's widget carries `Pressed`.
    Pressed(Target),
    /// The target's widget, or something below it, has input focus.
    Focused(Target),
    /// The target's widget is visible in the hierarchy and takes up room on
    /// screen.
    Visible(Target),
    /// The target's widget is painted topmost at its middle: nothing drawn
    /// later covers it, whatever the covering node's `Pickable` says. This is
    /// the question of what the person sees, where the player's own wait
    /// asks what the pointer reaches (a popup that lets the pointer through
    /// still covers what lies below). Only nodes that paint something count
    /// as covering (a background that is not transparent, an image or text),
    /// so an empty full-window container does not.
    OnTop(Target),
    /// The target can be found: exactly one widget matches it (for a text
    /// target, exactly one node shows the text).
    Exists(Target),
    /// The text the target's widget shows, which is every text node at or
    /// below it in order, joined by spaces, matches.
    TextOf(Target, TextMatch),
    /// The UI at and below the target's node matches an expected tree. The
    /// tree is indented lines of role, label and states, as
    /// [`tree_snapshot`] writes it, and partial: it lists only what matters,
    /// siblings may be left out, their order must hold. A mismatch fails
    /// with a line diff.
    ///
    /// ```text
    /// - panel:
    ///   - button "Pause"
    ///   - slider "Speed" [value=0.5]
    /// ```
    Tree {
        /// The node whose tree is read.
        of: Target,
        /// The expected tree.
        matches: String,
    },
    /// The frame, or the target's node rectangle of it, looks like the
    /// baseline `<scenario>/<name>.png` next to the scenario's files, within
    /// the tolerance. Needs the recorder (feature `record`); anywhere else the
    /// check is skipped. A missing baseline fails and is written; on a
    /// mismatch the picture and a diff are written beside the baseline.
    /// `RIN_UPDATE_SNAPSHOTS=1` rewrites every baseline. Written
    /// `Screenshot(name: "home", of: Name("Toolbar"), tolerance: (colour: 4))`.
    Screenshot {
        /// The baseline's file name, without `.png`.
        name: String,
        /// The node whose rectangle is compared; the whole frame when `None`.
        of: Option<Target>,
        /// How far the picture may be from the baseline.
        tolerance: Tolerance,
    },
    /// The check inside does not hold. A check that cannot run at all, such
    /// as a name nobody defined, still fails.
    Not(Box<Self>),
    /// Every check inside holds; an empty list holds.
    All(Vec<Self>),
    /// At least one check inside holds; an empty list fails.
    Any(Vec<Self>),
    /// The check inside holds now or on one of the next this many frames,
    /// like Playwright's `expect.poll`: a step waits for something without a
    /// guessed `Wait`. The step takes the frames it waits, and no more.
    Within(u16, Box<Self>),
}

/// [`Check`] with every variant written out by name: serde's derived form,
/// which [`Check`]'s own impls add the bare name to. A mirror rather than a
/// derive on [`Check`] itself, so the derived functions stay private.
#[derive(Debug, Serialize, Deserialize)]
#[serde(remote = "Check", rename = "Check")]
enum Spelled {
    /// See [`Check::App`].
    App(AppCheckValue),
    /// See [`Check::Shows`].
    Shows(String),
    /// See [`Check::Hovered`].
    Hovered(Target),
    /// See [`Check::Count`].
    Count {
        /// See [`Check::Count`].
        with: Vec<String>,
        /// See [`Check::Count`].
        is: Comparison,
    },
    /// See [`Check::Resource`].
    Resource {
        /// See [`Check::Resource`].
        path: String,
        /// See [`Check::Resource`].
        #[serde(default, skip_serializing_if = "Unit::is_physical")]
        unit: Unit,
        /// See [`Check::Resource`].
        is: Comparison,
    },
    /// See [`Check::Component`].
    Component {
        /// See [`Check::Component`].
        target: Target,
        /// See [`Check::Component`].
        path: String,
        /// See [`Check::Component`].
        #[serde(default, skip_serializing_if = "Unit::is_physical")]
        unit: Unit,
        /// See [`Check::Component`].
        is: Comparison,
    },
    /// See [`Check::InState`].
    InState {
        /// See [`Check::InState`].
        state: String,
        /// See [`Check::InState`].
        value: Value,
    },
    /// See [`Check::Disabled`].
    Disabled(Target),
    /// See [`Check::Checked`].
    Checked(Target),
    /// See [`Check::Selected`].
    Selected(Target),
    /// See [`Check::Pressed`].
    Pressed(Target),
    /// See [`Check::Focused`].
    Focused(Target),
    /// See [`Check::Visible`].
    Visible(Target),
    /// See [`Check::OnTop`].
    OnTop(Target),
    /// See [`Check::Exists`].
    Exists(Target),
    /// See [`Check::TextOf`].
    TextOf(Target, TextMatch),
    /// See [`Check::Tree`].
    Tree {
        /// See [`Check::Tree`].
        of: Target,
        /// See [`Check::Tree`].
        matches: String,
    },
    /// See [`Check::Screenshot`].
    Screenshot {
        /// See [`Check::Screenshot`].
        name: String,
        /// See [`Check::Screenshot`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Option<Target>,
        /// See [`Check::Screenshot`].
        #[serde(default, skip_serializing_if = "Tolerance::is_default")]
        tolerance: Tolerance,
    },
    /// See [`Check::Not`].
    Not(Box<Check>),
    /// See [`Check::All`].
    All(Vec<Check>),
    /// See [`Check::Any`].
    Any(Vec<Check>),
    /// See [`Check::Within`].
    Within(u16, Box<Check>),
}

/// The forms a check can take, for a check that does not parse.
const FORMS: &str = "a check: App(..), Shows(..), Hovered(..), Count(..), Resource(..), \
    Component(..), InState(..), Disabled(..), Checked(..), Selected(..), Pressed(..), \
    Focused(..), Visible(..), OnTop(..), Exists(..), TextOf(..), Tree(..), Screenshot(..), Not(..), All(..), Any(..) or Within(..)";

/// What a file that refers to a check by name, as format versions 1 and 2
/// could, is told.
const BY_NAME: &str = "checks are no longer looked up by name: declare the app's checks as \
    one enum with `#[derive(Reflect)]` and `#[reflect(AppCheck)]`, and write one as \
    `App(..)`, such as `Check(App(StationNamed(\"Alpha\")))`";

/// What a file that checks the app's state as text, as the state probe of
/// earlier versions could, is told.
pub(crate) const NO_STATE_PROBE: &str = "the state probe is gone: publish the state as a \
    reflected resource (`#[derive(Resource, Reflect)]` with `#[reflect(Resource)]`) and check \
    it with `Resource(path: \"Type.field\", is: Eq(..))`";

impl Serialize for Check {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Spelled::serialize(self, serializer)
    }
}

/// Reads the check's RON text whole, then parses it. Values inside checks
/// are kept as written, which only RON can give, so checks are read from
/// RON only.
impl<'de> Deserialize<'de> for Check {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        parse(raw.trim().get_ron()).map_err(serde::de::Error::custom)
    }
}

/// Reads one check from its RON text.
fn parse(text: &str) -> Result<Check, String> {
    /// Reads a check spelled out, through the mirror.
    #[derive(Debug)]
    struct SpelledSeed;

    impl<'de> DeserializeSeed<'de> for SpelledSeed {
        type Value = Check;

        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Check, D::Error> {
            Spelled::deserialize(deserializer)
        }
    }

    let options = crate::script::ron_options();
    if let Ok(name) = options.from_str::<String>(text) {
        return Err(format!("`\"{name}\"`: {BY_NAME}"));
    }
    options.from_str_seed(text, SpelledSeed).map_err(|error| {
        let message = error.code.to_string();
        // A check nested in another, or a target inside, has said this
        // already.
        if message.contains(FORMS)
            || message.contains(BY_NAME)
            || message.contains(NO_CUSTOM_TARGET)
        {
            message
        } else if ["Named", "Custom"]
            .iter()
            .any(|old| message.contains(&format!("variant named `{old}` in enum `Check`")))
        {
            format!("{message}: {BY_NAME}")
        } else if message.contains("variant named `State` in enum `Check`") {
            format!("{message}: {NO_STATE_PROBE}")
        } else {
            format!("{message} ({FORMS})")
        }
    })
}

/// A count of entities, waiting for what it must compare to: see
/// [`Check::count`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct CountOf {
    /// The components, by type path.
    with: Vec<String>,
}

impl CountOf {
    /// Counts only entities that also have `C`.
    pub fn and<C: Component + TypePath>(mut self) -> Self {
        self.with.push(C::short_type_path().to_owned());
        self
    }

    /// The count compares as `comparison` says.
    #[must_use]
    pub fn is(self, comparison: Comparison) -> Check {
        Check::Count {
            with: self.with,
            is: comparison,
        }
    }

    /// The count is `count`.
    #[must_use]
    pub fn eq(self, count: usize) -> Check {
        self.is(Comparison::Eq(Value::of(&count)))
    }

    /// The count is not `count`.
    #[must_use]
    pub fn ne(self, count: usize) -> Check {
        self.is(Comparison::Ne(Value::of(&count)))
    }

    /// The count is less than `count`.
    #[must_use]
    pub fn lt(self, count: usize) -> Check {
        self.is(Comparison::Lt(Value::of(&count)))
    }

    /// The count is at most `count`.
    #[must_use]
    pub fn le(self, count: usize) -> Check {
        self.is(Comparison::Le(Value::of(&count)))
    }

    /// The count is more than `count`.
    #[must_use]
    pub fn gt(self, count: usize) -> Check {
        self.is(Comparison::Gt(Value::of(&count)))
    }

    /// The count is at least `count`.
    #[must_use]
    pub fn ge(self, count: usize) -> Check {
        self.is(Comparison::Ge(Value::of(&count)))
    }

    /// The count is from `low` to `high`, both included.
    #[must_use]
    pub fn between(self, low: usize, high: usize) -> Check {
        self.is(Comparison::Between(Value::of(&low), Value::of(&high)))
    }
}

/// A reflected field, waiting for what it must compare to: see
/// [`Check::resource`] and [`Check::component`].
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct FieldOf {
    /// Where the component is; `None` for a resource.
    target: Option<Target>,
    /// The type path and the field.
    path: String,
    /// Whether a physical value is read as logical.
    unit: Unit,
}

impl FieldOf {
    /// Reads the field as logical pixels: a physical value is divided by the
    /// primary window's scale factor. See [`Unit`].
    pub const fn logical(mut self) -> Self {
        self.unit = Unit::Logical;
        self
    }

    /// The field compares as `comparison` says.
    #[must_use]
    pub fn is(self, comparison: Comparison) -> Check {
        match self.target {
            Some(target) => Check::Component {
                target,
                path: self.path,
                unit: self.unit,
                is: comparison,
            },
            None => Check::Resource {
                path: self.path,
                unit: self.unit,
                is: comparison,
            },
        }
    }

    /// The field equals `value`.
    #[must_use]
    pub fn eq<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Eq(Value::of(&value)))
    }

    /// The field does not equal `value`.
    #[must_use]
    pub fn ne<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Ne(Value::of(&value)))
    }

    /// The field is less than `value`.
    #[must_use]
    pub fn lt<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Lt(Value::of(&value)))
    }

    /// The field is at most `value`.
    #[must_use]
    pub fn le<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Le(Value::of(&value)))
    }

    /// The field is more than `value`.
    #[must_use]
    pub fn gt<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Gt(Value::of(&value)))
    }

    /// The field is at least `value`.
    #[must_use]
    pub fn ge<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Ge(Value::of(&value)))
    }

    /// The field is from `low` to `high`, both included.
    #[must_use]
    pub fn between<T: Reflect + GetTypeRegistration + TypePath>(self, low: T, high: T) -> Check {
        self.is(Comparison::Between(Value::of(&low), Value::of(&high)))
    }
}

/// `Editor` and `tool` as `Editor.tool`; a path that starts with its own
/// separator (`.0`, `[2]`) is kept as it is.
fn join_path(type_path: &str, field: &str) -> String {
    if field.is_empty() || field.starts_with(['.', '[', '#']) {
        format!("{type_path}{field}")
    } else {
        format!("{type_path}.{field}")
    }
}

impl Check {
    /// One of the app's own checks, for example
    /// `Check::app(RailCheck::StationNamed("Alpha".into()))`.
    #[must_use]
    pub fn app<T: AppCheck + Reflect + GetTypeRegistration + TypePath>(check: T) -> Self {
        Self::App(AppCheckValue::of(check))
    }

    /// Some UI node of the app shows exactly `text`.
    #[must_use]
    pub fn shows(text: impl Into<String>) -> Self {
        Self::Shows(text.into())
    }

    /// The widget `target` names is under the pointer.
    #[must_use]
    pub const fn hovered(target: Target) -> Self {
        Self::Hovered(target)
    }

    /// How many entities have `C` (and whatever [`CountOf::and`] adds), for
    /// example `Check::count::<Ghost>().eq(1)`.
    pub fn count<C: Component + TypePath>() -> CountOf {
        CountOf {
            with: vec![C::short_type_path().to_owned()],
        }
    }

    /// How many entities have every component named in `with`, by type path.
    pub fn count_of(with: impl IntoIterator<Item = impl Into<String>>) -> CountOf {
        CountOf {
            with: with.into_iter().map(Into::into).collect(),
        }
    }

    /// The field `path` of the resource `R`, for example
    /// `Check::resource::<Editor>("tool").eq(Tool::Lay)`. An empty path is
    /// the whole resource.
    pub fn resource<R: Resource + TypePath>(path: &str) -> FieldOf {
        FieldOf {
            target: None,
            path: join_path(R::short_type_path(), path),
            unit: Unit::Physical,
        }
    }

    /// The field `path` of the component `C` on `target`'s node, or on the
    /// nearest node above it that has one.
    pub fn component<C: Component + TypePath>(target: Target, path: &str) -> FieldOf {
        FieldOf {
            target: Some(target),
            path: join_path(C::short_type_path(), path),
            unit: Unit::Physical,
        }
    }

    /// The app is in `state`.
    #[must_use]
    pub fn in_state<S: States + Reflect + GetTypeRegistration + TypePath>(state: S) -> Self {
        Self::InState {
            state: S::short_type_path().to_owned(),
            value: Value::of(&state),
        }
    }

    /// The target's widget is disabled.
    #[must_use]
    pub const fn disabled(target: Target) -> Self {
        Self::Disabled(target)
    }

    /// The target's widget is checked.
    #[must_use]
    pub const fn checked(target: Target) -> Self {
        Self::Checked(target)
    }

    /// The target's widget is selected.
    #[must_use]
    pub const fn selected(target: Target) -> Self {
        Self::Selected(target)
    }

    /// The target's widget is pressed.
    #[must_use]
    pub const fn pressed(target: Target) -> Self {
        Self::Pressed(target)
    }

    /// The target's widget, or something below it, has input focus.
    #[must_use]
    pub const fn focused(target: Target) -> Self {
        Self::Focused(target)
    }

    /// The target's widget is visible.
    #[must_use]
    pub const fn visible(target: Target) -> Self {
        Self::Visible(target)
    }

    /// The target's widget is painted topmost at its middle: see
    /// [`Check::OnTop`].
    #[must_use]
    pub const fn on_top(target: Target) -> Self {
        Self::OnTop(target)
    }

    /// The target can be found.
    #[must_use]
    pub const fn exists(target: Target) -> Self {
        Self::Exists(target)
    }

    /// The text of the target's widget matches.
    #[must_use]
    pub const fn text_of(target: Target, matching: TextMatch) -> Self {
        Self::TextOf(target, matching)
    }

    /// The UI at and below the target's node matches the expected `tree`:
    /// see [`Check::Tree`].
    #[must_use]
    pub fn tree(of: Target, tree: impl Into<String>) -> Self {
        Self::Tree {
            of,
            matches: tree.into(),
        }
    }

    /// Every one of `checks` holds.
    #[must_use]
    pub fn all(checks: impl IntoIterator<Item = impl Into<Self>>) -> Self {
        Self::All(checks.into_iter().map(Into::into).collect())
    }

    /// The whole frame looks like the baseline `name` (`<name>.png`, in the
    /// scenario's directory), within the default [`Tolerance`]. See
    /// [`Check::Screenshot`].
    #[must_use]
    pub fn screenshot(name: impl Into<String>) -> Self {
        Self::Screenshot {
            name: name.into(),
            of: None,
            tolerance: Tolerance::default(),
        }
    }

    /// The rectangle `target`'s node covers looks like the baseline `name`.
    #[must_use]
    pub fn screenshot_of(name: impl Into<String>, target: Target) -> Self {
        Self::Screenshot {
            name: name.into(),
            of: Some(target),
            tolerance: Tolerance::default(),
        }
    }

    /// This screenshot check with another tolerance; any other check is
    /// returned as it is.
    #[must_use]
    pub const fn with_tolerance(mut self, tolerance: Tolerance) -> Self {
        if let Self::Screenshot { tolerance: own, .. } = &mut self {
            *own = tolerance;
        }
        self
    }

    /// At least one of `checks` holds.
    #[must_use]
    pub fn any(checks: impl IntoIterator<Item = impl Into<Self>>) -> Self {
        Self::Any(checks.into_iter().map(Into::into).collect())
    }

    /// `check` holds now or on one of the next `frames` frames.
    #[must_use]
    pub fn within(frames: u16, check: impl Into<Self>) -> Self {
        Self::Within(frames, Box::new(check.into()))
    }

    /// This check and every check inside it, depth first, outermost first.
    pub(crate) fn parts(&self) -> Vec<&Self> {
        let mut parts = Vec::new();
        let mut stack = vec![self];
        while let Some(check) = stack.pop() {
            parts.push(check);
            match check {
                Self::Not(inner) | Self::Within(_, inner) => stack.push(inner),
                Self::All(checks) | Self::Any(checks) => stack.extend(checks.iter().rev()),
                _ => {}
            }
        }
        parts
    }

    /// The names of the captured values this check and the checks inside it
    /// compare with, in order, once each place they are used.
    pub(crate) fn captured_names(&self) -> Vec<&str> {
        self.parts()
            .into_iter()
            .flat_map(|part| match part {
                Self::Count { is, .. } | Self::Resource { is, .. } | Self::Component { is, .. } => {
                    is.captured_names()
                }
                Self::InState { value, .. } => value.captured_name().into_iter().collect(),
                _ => Vec::new(),
            })
            .collect()
    }

    /// The target this check, not counting the checks inside it, looks at.
    const fn target_mut(&mut self) -> Option<&mut Target> {
        match self {
            Self::Hovered(target)
            | Self::Component { target, .. }
            | Self::Disabled(target)
            | Self::Checked(target)
            | Self::Selected(target)
            | Self::Pressed(target)
            | Self::Focused(target)
            | Self::Visible(target)
            | Self::OnTop(target)
            | Self::Exists(target)
            | Self::TextOf(target, _)
            | Self::Tree { of: target, .. } => Some(target),
            Self::Screenshot { of, .. } => of.as_mut(),
            _ => None,
        }
    }

    /// Reads every app check in this check, and in the checks inside it, as
    /// the app's check type in `types`, and every app target they look at as
    /// the app's target type.
    pub(crate) fn type_app_values(&mut self, types: &TypeRegistry) -> Result<(), Problem> {
        match self {
            Self::App(value) => value.type_against(types),
            Self::Not(inner) | Self::Within(_, inner) => inner.type_app_values(types),
            Self::All(checks) | Self::Any(checks) => checks
                .iter_mut()
                .try_for_each(|check| check.type_app_values(types)),
            leaf => leaf
                .target_mut()
                .map_or(Ok(()), |target| target.type_against(types)),
        }
    }

    /// Checks, from type information alone, that the types, fields and
    /// values this check names, not counting the checks inside it, are
    /// there and fit.
    pub(crate) fn problem(&self, types: &TypeRegistry) -> Result<(), Problem> {
        match self {
            Self::Count { with, is } => {
                if with.is_empty() {
                    return Err(Problem::Other(
                        "a count needs at least one component".into(),
                    ));
                }
                for name in with {
                    find_type(types, name, Role::Counted)?;
                }
                reflect::check_count(is)
            }
            Self::Resource { path, is, unit } => {
                let field = field_registration(types, path, Role::Resource)?;
                if unit.is_logical() {
                    reflect::check_logical(field)?;
                }
                reflect::check_comparison(types, field, is)
            }
            Self::Component { path, is, unit, .. } => {
                let field = field_registration(types, path, Role::Component)?;
                if unit.is_logical() {
                    reflect::check_logical(field)?;
                }
                reflect::check_comparison(types, field, is)
            }
            Self::InState { state, value } => {
                let registration = find_type(types, state, Role::State)?;
                reflect::read_value(types, registration, value).map(|_| ())
            }
            Self::Tree { matches, .. } => tree::parse(matches).map(|_| ()).map_err(Problem::Other),
            Self::Screenshot {
                name, tolerance, ..
            } => screenshot::name_problem(name)
                .or_else(|| tolerance.problem())
                .map_or(Ok(()), |message| Err(Problem::Other(message))),
            _ => Ok(()),
        }
    }

    /// Whether this check, or one inside it, compares a screenshot, which
    /// needs a picture taken before it is polled.
    pub(crate) fn has_screenshot(&self) -> bool {
        self.parts()
            .into_iter()
            .any(|part| matches!(part, Self::Screenshot { .. }))
    }

    /// Runs the check against `world`, which has waited `waited` frames for
    /// it: `None` while a [`Check::Within`] inside still waits for it to
    /// hold. A check that cannot run here (a screenshot without the
    /// recorder) is [`Outcome::Skipped`].
    pub(crate) fn poll(
        &self,
        world: &World,
        captures: &CapturedValues,
        waited: u32,
    ) -> Option<Outcome> {
        match self.verdict(world, captures, waited) {
            Verdict::Holds => Some(Outcome::Pass),
            Verdict::Fails(message) | Verdict::Broken(message) => Some(Outcome::Fail(message)),
            Verdict::Pending(_) => None,
            Verdict::Unavailable(_) => Some(Outcome::Skipped),
        }
    }

    /// What the check comes to on this frame.
    fn verdict(&self, world: &World, captures: &CapturedValues, waited: u32) -> Verdict {
        match self {
            Self::Not(inner) => match inner.verdict(world, captures, waited) {
                Verdict::Holds => Verdict::Fails(format!("{inner} holds, but should not")),
                Verdict::Fails(_) => Verdict::Holds,
                other => other,
            },
            Self::All(checks) => {
                let verdicts: Vec<Verdict> = checks
                    .iter()
                    .map(|check| check.verdict(world, captures, waited))
                    .collect();
                first_broken(&verdicts)
                    .or_else(|| verdicts.iter().find(|v| matches!(v, Verdict::Fails(_))))
                    .or_else(|| verdicts.iter().find(|v| matches!(v, Verdict::Pending(_))))
                    .or_else(|| {
                        verdicts
                            .iter()
                            .find(|v| matches!(v, Verdict::Unavailable(_)))
                    })
                    .cloned()
                    .unwrap_or(Verdict::Holds)
            }
            Self::Any(checks) => {
                let verdicts: Vec<Verdict> = checks
                    .iter()
                    .map(|check| check.verdict(world, captures, waited))
                    .collect();
                if let Some(broken) = first_broken(&verdicts) {
                    return broken.clone();
                }
                if verdicts.iter().any(|v| matches!(v, Verdict::Holds)) {
                    return Verdict::Holds;
                }
                if let Some(pending) = verdicts.iter().find(|v| matches!(v, Verdict::Pending(_))) {
                    return pending.clone();
                }
                if let Some(skipped) = verdicts
                    .iter()
                    .find(|v| matches!(v, Verdict::Unavailable(_)))
                {
                    return skipped.clone();
                }
                let reasons: Vec<&str> = verdicts.iter().map(Verdict::message).collect();
                Verdict::Fails(if reasons.is_empty() {
                    "there is no check to hold".into()
                } else {
                    format!("none holds: {}", reasons.join("; "))
                })
            }
            Self::Within(frames, inner) => match inner.verdict(world, captures, waited) {
                Verdict::Fails(message) if waited < u32::from(*frames) => Verdict::Pending(message),
                Verdict::Fails(message) => {
                    Verdict::Fails(format!("{message} (still after {frames} frames)"))
                }
                other => other,
            },
            screen @ Self::Screenshot { .. } => screenshot::verdict(world, screen),
            leaf => match leaf.evaluate(world, captures) {
                Ok(Ok(())) => Verdict::Holds,
                Ok(Err(message)) => Verdict::Fails(message),
                Err(message) => Verdict::Broken(message),
            },
        }
    }

    /// Runs a check with no checks inside it. The outer error means it could
    /// not run at all, which [`Check::Not`] passes on rather than inverting.
    fn evaluate(&self, world: &World, captures: &CapturedValues) -> Result<CheckResult, String> {
        let probe = Probe::new(world);
        match self {
            Self::App(value) => value.run(probe),
            Self::Shows(text) => Ok(if probe.has_text(text) {
                Ok(())
            } else {
                Err(format!("no UI node shows \"{text}\""))
            }),
            Self::Hovered(target) => {
                // Only a target that names widgets by what they are can match
                // several; the error from `resolve` would not tell that apart
                // from a miss.
                if !matches!(
                    target,
                    Target::Screen(_) | Target::World(_) | Target::App(_)
                ) && let Lookup::Ambiguous(message) = widget::node(world, target)
                {
                    return Err(message);
                }
                let resolved = match resolve(world, target) {
                    Ok(resolved) => resolved,
                    Err(message) => return Ok(Err(message)),
                };
                let widget = resolved.widget.ok_or_else(|| {
                    format!("{target:?} is a position, not a widget, so it cannot be hovered")
                })?;
                Ok(check_hover(world, widget, &format!("{target:?}")))
            }
            Self::Count { .. }
            | Self::Resource { .. }
            | Self::Component { .. }
            | Self::InState { .. } => {
                let types = reflect::app_types(world)?;
                self.evaluate_reflected(world, &types.read(), captures)
            }
            Self::Disabled(target) => widget_state(world, target, WidgetState::Disabled),
            Self::Checked(target) => widget_state(world, target, WidgetState::Checked),
            Self::Selected(target) => widget_state(world, target, WidgetState::Selected),
            Self::Pressed(target) => widget_state(world, target, WidgetState::Pressed),
            Self::Focused(target) => widget_state(world, target, WidgetState::Focused),
            Self::Visible(target) => widget_state(world, target, WidgetState::Visible),
            Self::OnTop(target) => widget_state(world, target, WidgetState::OnTop),
            Self::Exists(target) => widget_state(world, target, WidgetState::Exists),
            Self::TextOf(target, matching) => {
                let node = match widget::node(world, target) {
                    Lookup::Found(node) => node,
                    Lookup::Missing(message) => return Ok(Err(message)),
                    Lookup::Ambiguous(message) | Lookup::NotANode(message) => return Err(message),
                };
                let text = widget::text_of(world, widget::widget(world, node));
                Ok(if matching.matches(&text) {
                    Ok(())
                } else {
                    Err(format!("{target:?} shows \"{text}\""))
                })
            }
            Self::Tree { of, matches } => {
                let node = match widget::node(world, of) {
                    Lookup::Found(node) => node,
                    Lookup::Missing(message) => return Ok(Err(message)),
                    Lookup::Ambiguous(message) | Lookup::NotANode(message) => return Err(message),
                };
                let expected = tree::parse(matches)?;
                Ok(expected.compare(world, node).map_err(|diff| {
                    format!("the tree of {of:?} differs (- expected, + actual):\n{diff}")
                }))
            }
            Self::Screenshot { .. } => {
                Err("a screenshot is judged from a picture, not read from the world".into())
            }
            Self::Not(_) | Self::All(_) | Self::Any(_) | Self::Within(..) => {
                Err("a check with checks inside it is not a leaf".into())
            }
        }
    }

    /// Runs a check that reads the world through reflection, with the app's
    /// type registry.
    fn evaluate_reflected(
        &self,
        world: &World,
        types: &TypeRegistry,
        captures: &CapturedValues,
    ) -> Result<CheckResult, String> {
        match self {
            Self::Count { with, is } => {
                let mut registrations = Vec::with_capacity(with.len());
                for name in with {
                    registrations.push(
                        find_type(types, name, Role::Counted)
                            .map_err(|problem| problem.message())?,
                    );
                }
                if registrations.is_empty() {
                    return Err("a count needs at least one component".into());
                }
                let count = reflect::count(world, &registrations);
                let is = self.resolve_is(is, captures)?;
                compare(&is, &Counted(count), &count.to_string(), || self.subject())
            }
            Self::Resource { path, is, unit } => {
                let is = self.resolve_is(is, captures)?;
                let (root, field) = match root_of(world, types, None, path)? {
                    Ok(found) => found,
                    Err(message) => return Ok(Err(message)),
                };
                compare_field(world, types, root, path, field, &is, *unit, || {
                    self.subject()
                })
            }
            Self::Component {
                target,
                path,
                is,
                unit,
            } => {
                let is = self.resolve_is(is, captures)?;
                let (root, field) = match root_of(world, types, Some(target), path)? {
                    Ok(found) => found,
                    Err(message) => return Ok(Err(message)),
                };
                compare_field(world, types, root, path, field, &is, *unit, || {
                    self.subject()
                })
            }
            Self::InState { state, value } => {
                let value = value
                    .resolve(captures)
                    .map_err(|message| format!("{}: {message}", self.subject()))?;
                let registration =
                    find_type(types, state, Role::State).map_err(|problem| problem.message())?;
                let Some(current) = reflect::state(world, registration) else {
                    return Ok(Err(format!("the app is in no `{state}` state")));
                };
                let measured = Reflected {
                    value: current.as_partial_reflect(),
                    registration,
                    types,
                };
                Ok(if measured.equals(&value)? {
                    Ok(())
                } else {
                    Err(format!(
                        "{} is {}, expected {value}",
                        self.subject(),
                        measured.describe()
                    ))
                })
            }
            _ => Err("this check does not read through reflection".into()),
        }
    }

    /// `is` with the captured values it refers to looked up; a value that
    /// cannot be names the check.
    fn resolve_is(&self, is: &Comparison, captures: &CapturedValues) -> Result<Comparison, String> {
        is.resolve(captures)
            .map_err(|message| format!("{}: {message}", self.subject()))
    }

    /// What a measuring check measures, in words, such as ``count of `Ghost` ``.
    fn subject(&self) -> String {
        match self {
            Self::Count { with, .. } => {
                let names: Vec<String> = with.iter().map(|name| format!("`{name}`")).collect();
                format!("count of {}", names.join(" and "))
            }
            Self::Resource { path, unit, .. } => logical(*unit, &format!("`{path}`")),
            Self::Component {
                target, path, unit, ..
            } => logical(*unit, &format!("`{path}` of {target:?}")),
            Self::InState { state, .. } => format!("state `{state}`"),
            other => other.to_string(),
        }
    }
}

/// `subject`, marked as read in logical pixels when `unit` says so.
fn logical(unit: Unit, subject: &str) -> String {
    if unit.is_logical() {
        format!("logical {subject}")
    } else {
        subject.to_owned()
    }
}

/// The reflected value a resource or component path starts from, and the
/// field path after its type: the resource, or the component on `target`'s
/// node or, failing that, the nearest node above it that has one.
///
/// The outer error says the path cannot be used at all (a type or field that
/// cannot exist, a target that is ambiguous); the inner one that the world
/// does not have it now, which is a failure and may change.
fn root_of<'w, 'p>(
    world: &'w World,
    types: &TypeRegistry,
    target: Option<&Target>,
    path: &'p str,
) -> Result<Result<(&'w dyn Reflect, &'p str), String>, String> {
    let (type_name, field) = reflect::split_path(path);
    let Some(target) = target else {
        let registration =
            find_type(types, type_name, Role::Resource).map_err(|problem| problem.message())?;
        // A field the type cannot have is a mistake, not a state the world
        // may yet reach.
        reflect::field_type(types, registration, path, field)
            .map_err(|problem| problem.message())?;
        return Ok(reflect::resource(world, registration)
            .map(|resource| (resource, field))
            .ok_or_else(|| format!("there is no `{type_name}` resource")));
    };
    let registration =
        find_type(types, type_name, Role::Component).map_err(|problem| problem.message())?;
    reflect::field_type(types, registration, path, field).map_err(|problem| problem.message())?;
    let node = match widget::node(world, target) {
        Lookup::Found(node) => node,
        Lookup::Missing(message) => return Ok(Err(message)),
        Lookup::Ambiguous(message) | Lookup::NotANode(message) => return Err(message),
    };
    Ok(self_and_ancestors(world, node)
        .into_iter()
        .find_map(|entity| reflect::component(world, registration, entity))
        .map(|component| (component, field))
        .ok_or_else(|| format!("neither {target:?} nor anything above it has a `{type_name}`")))
}

/// The component id of the reflected component called `name` in `world`'s
/// app, `None` when the type is registered but no entity ever had it: for
/// [`Target::Component`].
pub(crate) fn component_id(world: &World, name: &str) -> Result<Option<ComponentId>, String> {
    let registry = reflect::app_types(world)?;
    let id = find_type(&registry.read(), name, Role::Component)
        .map(bevy::reflect::TypeRegistration::type_id)
        .map_err(|problem| problem.message())?;
    Ok(world.components().get_id(id))
}

/// Checks, from type information alone, that `name` is a reflected
/// component in `types`: for [`Target::Component`] when a script loads.
pub(crate) fn check_component_type(types: &TypeRegistry, name: &str) -> Result<(), Problem> {
    find_type(types, name, Role::Component).map(|_| ())
}

/// The candidates closest to `wanted`, closest first, for a hint.
pub(crate) fn close_matches<'a>(
    wanted: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    reflect::close_names(wanted, candidates)
}

/// What names the widget `entity`: its text without a text input's own
/// value, which a target never finds the field by.
pub(crate) fn label_of(world: &World, entity: Entity) -> String {
    widget::label_of(world, entity)
}

/// The tree of the UI at and below the node `target` names, written as
/// [`Check::Tree`] reads it; what pick mode offers to snapshot a node.
///
/// # Errors
///
/// Says why when `target` names no one node.
pub fn tree_snapshot(world: &World, target: &Target) -> Result<String, String> {
    match widget::node(world, target) {
        Lookup::Found(node) => Ok(tree::snapshot(world, node)),
        Lookup::Missing(message) | Lookup::Ambiguous(message) | Lookup::NotANode(message) => {
            Err(message)
        }
    }
}

/// The widget `node` belongs to: the nearest widget at or above it,
/// otherwise `node` itself.
pub(crate) fn widget_of(world: &World, node: Entity) -> Entity {
    widget::widget(world, node)
}

/// A value a `Capture` step took: RON text and the name of its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedValue {
    /// The value, written through its reflected form, as a literal in a
    /// script would be.
    ron: String,
    /// The type's short path, for messages.
    type_name: String,
}

impl CapturedValue {
    /// The value as RON.
    pub(crate) fn ron(&self) -> &str {
        &self.ron
    }

    /// The type's short path.
    pub(crate) fn type_name(&self) -> &str {
        &self.type_name
    }

    /// A value of type `type_name` written as `ron`.
    pub(crate) const fn new(ron: String, type_name: String) -> Self {
        Self { ron, type_name }
    }
}

/// Everything captured so far in a run, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CapturedValues {
    /// The values, ordered by name.
    values: BTreeMap<String, CapturedValue>,
}

impl CapturedValues {
    /// What was captured as `name`, if anything.
    pub(crate) fn get(&self, name: &str) -> Option<&CapturedValue> {
        self.values.get(name)
    }

    /// Keeps `value` as `name`, replacing what was there.
    pub(crate) fn insert(&mut self, name: String, value: CapturedValue) {
        self.values.insert(name, value);
    }
}

/// Something a check names that is not there, or not usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Problem {
    /// No registered type has this name.
    UnknownType {
        /// The name as written.
        name: String,
        /// Registered names close to it, closest first.
        suggestions: Vec<String>,
    },
    /// The type has no such field.
    UnknownPath {
        /// The path as written, type included.
        path: String,
        /// What is wrong with it.
        message: String,
        /// Field names close to the one written, closest first.
        suggestions: Vec<String>,
    },
    /// A value does not fit the type it is compared with.
    BadValue {
        /// The value as written.
        value: String,
        /// Why it does not fit.
        message: String,
    },
    /// More than one registered type is the app's type for an extension
    /// point.
    SeveralAppTypes {
        /// The extension point.
        extension: Extension,
        /// Their type paths, in order.
        types: Vec<String>,
    },
    /// No registered type is the app's type for an extension point.
    NoAppType(Extension),
    /// A value does not read as the app's type for its extension point.
    BadAppValue {
        /// The extension point.
        extension: Extension,
        /// The value as written.
        written: String,
        /// The app's type, by its short type path.
        type_name: &'static str,
        /// Why it does not read.
        message: String,
        /// The type's variants, as a file writes them.
        variants: Vec<String>,
    },
    /// Anything else that stops the check from running.
    Other(String),
}

impl Problem {
    /// The problem in words, for a check that cannot run.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::UnknownType { name, suggestions } => {
                format!(
                    "no type `{name}` is registered{}",
                    did_you_mean(suggestions)
                )
            }
            Self::UnknownPath {
                message,
                suggestions,
                ..
            } => format!("{message}{}", did_you_mean(suggestions)),
            Self::BadValue { value, message } => format!("`{value}` does not fit: {message}"),
            Self::SeveralAppTypes { extension, types } => several_message(*extension, types),
            Self::NoAppType(extension) => no_app_type(*extension),
            Self::BadAppValue {
                written,
                type_name,
                message,
                variants,
                ..
            } => bad_app_value(written, type_name, message, variants),
            Self::Other(message) => message.clone(),
        }
    }
}

/// A hint such as ``; did you mean `Ghost`?``, or nothing when no name is
/// close.
pub(crate) fn did_you_mean(suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        return String::new();
    }
    let names: Vec<String> = suggestions.iter().map(|name| format!("`{name}`")).collect();
    format!("; did you mean {}?", names.join(" or "))
}

/// The registration of the field a `Type.field` path leads to, the type
/// playing `role`.
fn field_registration<'r>(
    types: &'r TypeRegistry,
    path: &str,
    role: Role,
) -> Result<&'r bevy::reflect::TypeRegistration, Problem> {
    let (type_name, field) = reflect::split_path(path);
    let registration = find_type(types, type_name, role)?;
    reflect::field_type(types, registration, path, field)
}

/// Compares the field at `field` in `root` as `is` says. `path` is the whole
/// path, for messages; `unit: Logical` reads a physical number as logical.
fn compare_field(
    world: &World,
    types: &TypeRegistry,
    root: &dyn Reflect,
    path: &str,
    field: &str,
    is: &Comparison,
    unit: Unit,
    subject: impl Fn() -> String,
) -> Result<CheckResult, String> {
    // A field of another enum variant than the current one fails the check.
    let value = match reflect::field_value(root, path, field) {
        Ok(value) => value,
        Err(message) => return Ok(Err(message)),
    };
    let registration = value
        .get_represented_type_info()
        .and_then(|info| types.get(info.type_id()))
        .ok_or_else(|| format!("the type of `{path}` is not registered"))?;
    let scaled = unit
        .is_logical()
        .then(|| reflect::to_logical(world, value))
        .transpose()?;
    let measured = Reflected {
        value: scaled.as_deref().unwrap_or(value),
        registration,
        types,
    };
    compare(is, &measured, &measured.describe(), subject)
}

/// Whether `measured` compares as `is` says; the failure says what was
/// found, as `{subject} is {actual}, expected {expected}`.
fn compare(
    is: &Comparison,
    measured: &dyn Comparable,
    actual: &str,
    subject: impl Fn() -> String,
) -> Result<CheckResult, String> {
    let holds = is.test(measured).map_err(|message| {
        // A captured value is named in the message, so the check should be.
        if is.is_captured() {
            format!("{}: {message}", subject())
        } else {
            message
        }
    })?;
    Ok(if holds {
        Ok(())
    } else {
        Err(format!(
            "{} is {actual}, expected {}",
            subject(),
            is.expected()
        ))
    })
}

/// Whether the widget `target` names is in `state`.
fn widget_state(world: &World, target: &Target, state: WidgetState) -> Result<CheckResult, String> {
    match widget::node(world, target) {
        Lookup::Found(node) => Ok(widget::in_state(
            world,
            widget::widget(world, node),
            state,
            &format!("{target:?}"),
        )),
        Lookup::Missing(message) => Ok(Err(message)),
        Lookup::Ambiguous(message) | Lookup::NotANode(message) => Err(message),
    }
}

/// `entity` and every entity above it, nearest first.
fn self_and_ancestors(world: &World, entity: Entity) -> Vec<Entity> {
    let mut chain = vec![entity];
    let mut current = entity;
    while let Some(parent) = world.get::<ChildOf>(current) {
        current = parent.parent();
        chain.push(current);
    }
    chain
}

/// The first verdict that says the check cannot run, if any.
fn first_broken(verdicts: &[Verdict]) -> Option<&Verdict> {
    verdicts.iter().find(|v| matches!(v, Verdict::Broken(_)))
}

/// What a check comes to on one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// It holds.
    Holds,
    /// It does not hold, and why.
    Fails(String),
    /// It does not hold yet, and why, but a [`Check::Within`] still waits.
    Pending(String),
    /// It cannot run at all, and why; nothing inverts or waits that out.
    Broken(String),
    /// It cannot run here (no renderer for a screenshot), and why; the check
    /// is skipped rather than failed.
    Unavailable(String),
}

impl Verdict {
    /// The reason, or nothing when the check holds.
    fn message(&self) -> &str {
        match self {
            Self::Holds => "",
            Self::Fails(message)
            | Self::Pending(message)
            | Self::Broken(message)
            | Self::Unavailable(message) => message,
        }
    }
}

/// `checks` in words, separated by semicolons.
fn list(checks: &[Check]) -> String {
    checks
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// The check in words, which is also its name in reports.
impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::App(value) => write!(f, "{value}"),
            Self::Shows(text) => write!(f, "\"{text}\" is shown"),
            Self::Hovered(target) => write!(f, "{target:?} is under the pointer"),
            Self::Count { is, .. } | Self::Resource { is, .. } | Self::Component { is, .. } => {
                write!(f, "{} {is}", self.subject())
            }
            Self::InState { value, .. } => write!(f, "{} is {value}", self.subject()),
            Self::Disabled(target) => write!(f, "{target:?} {}", WidgetState::Disabled.words()),
            Self::Checked(target) => write!(f, "{target:?} {}", WidgetState::Checked.words()),
            Self::Selected(target) => write!(f, "{target:?} {}", WidgetState::Selected.words()),
            Self::Pressed(target) => write!(f, "{target:?} {}", WidgetState::Pressed.words()),
            Self::Focused(target) => write!(f, "{target:?} {}", WidgetState::Focused.words()),
            Self::Visible(target) => write!(f, "{target:?} {}", WidgetState::Visible.words()),
            Self::OnTop(target) => write!(f, "{target:?} {}", WidgetState::OnTop.words()),
            Self::Exists(target) => write!(f, "{target:?} {}", WidgetState::Exists.words()),
            Self::TextOf(target, matching) => write!(f, "text of {target:?} {matching}"),
            Self::Screenshot {
                name,
                of: None,
                tolerance,
            } => write!(f, "screenshot `{name}` matches ({tolerance})"),
            Self::Screenshot {
                name,
                of: Some(target),
                tolerance,
            } => write!(f, "screenshot `{name}` of {target:?} matches ({tolerance})"),
            Self::Tree { of, .. } => write!(f, "tree of {of:?} matches"),
            Self::Not(inner) => write!(f, "not: {inner}"),
            Self::All(checks) => write!(f, "all of [{}]", list(checks)),
            Self::Any(checks) => write!(f, "any of [{}]", list(checks)),
            Self::Within(frames, inner) => write!(f, "within {frames} frames: {inner}"),
        }
    }
}

impl Not for Check {
    type Output = Self;

    fn not(self) -> Self {
        Self::Not(Box::new(self))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may panic")]

    use super::*;

    #[test]
    fn a_misspelt_target_is_not_taken_for_a_check_by_name() {
        let message = parse("Hovered(Custm(resolver: \"a\", argument: \"b\"))").unwrap_err();
        assert!(!message.contains("by name"), "{message}");
    }

    #[test]
    fn a_custom_target_inside_a_check_says_what_to_write_instead() {
        let message = parse("Hovered(Custom(resolver: \"a\", argument: \"b\"))").unwrap_err();
        assert!(message.contains(NO_CUSTOM_TARGET), "{message}");
        assert!(!message.contains(FORMS), "{message}");
    }

    #[test]
    fn the_forms_that_named_a_check_say_what_to_write_instead() {
        for text in [
            "\"saved\"",
            "Named(\"saved\")",
            "Custom(check: \"a\", argument: \"b\")",
        ] {
            let message = parse(text).unwrap_err();
            assert!(message.contains(BY_NAME), "{text}: {message}");
        }
    }
}
