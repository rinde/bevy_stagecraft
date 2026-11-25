//! Checks as data, spelled out in the script. Anything not built in is one of
//! the app's own checks, written inside `App(..)` (see [`AppCheck`]).

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
/// In Rust the same checks are typed; `!check` wraps one in [`Check::Not`].
///
/// ```
/// use bevy_stagecraft::{Check, Target};
///
/// let check = !Check::hovered(Target::text("OK"));
/// assert_eq!(check.to_string(), "not: Text(\"OK\") is under the pointer");
/// #[derive(bevy::prelude::Component, bevy::prelude::Reflect)]
/// struct Ghost;
/// assert_eq!(Check::count::<Ghost>().eq(1).to_string(), "count of `Ghost` is 1");
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Check {
    App(AppCheckValue),
    Shows(String),
    /// A target that is only a position names no widget, so this fails.
    Hovered(Target),
    /// Disabled entities are not counted.
    Count {
        with: Vec<String>,
        is: Comparison,
    },
    /// `path` may be the type alone, for the whole resource.
    Resource {
        path: String,
        unit: Unit,
        is: Comparison,
    },
    /// Falls back to the nearest node above that has it, so a label finds its
    /// slider.
    Component {
        target: Target,
        path: String,
        unit: Unit,
        is: Comparison,
    },
    /// Needs `register_type_state`.
    InState {
        state: String,
        value: Value,
    },
    Disabled(Target),
    Checked(Target),
    /// A tab is selected when its list's `SelectedTab` names it.
    Selected(Target),
    Pressed(Target),
    Focused(Target),
    Visible(Target),
    /// Ignores `Pickable`. Only nodes that paint cover, so an empty full-window
    /// container does not.
    OnTop(Target),
    Exists(Target),
    /// Reads a `TextInput`'s value, not its label.
    TextOf(Target, TextMatch),
    /// Partial: siblings may be left out, their order must hold.
    Tree {
        of: Target,
        matches: String,
    },
    Screenshot {
        name: String,
        of: Option<Target>,
        tolerance: Tolerance,
    },
    /// A check that cannot run at all still fails.
    Not(Box<Self>),
    /// An empty list holds.
    All(Vec<Self>),
    /// An empty list fails.
    Any(Vec<Self>),
    Within(u16, Box<Self>),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(remote = "Check", rename = "Check")]
enum Spelled {
    App(AppCheckValue),
    Shows(String),
    Hovered(Target),
    Count {
        with: Vec<String>,
        is: Comparison,
    },
    Resource {
        path: String,
        #[serde(default, skip_serializing_if = "Unit::is_physical")]
        unit: Unit,
        is: Comparison,
    },
    Component {
        target: Target,
        path: String,
        #[serde(default, skip_serializing_if = "Unit::is_physical")]
        unit: Unit,
        is: Comparison,
    },
    InState {
        state: String,
        value: Value,
    },
    Disabled(Target),
    Checked(Target),
    Selected(Target),
    Pressed(Target),
    Focused(Target),
    Visible(Target),
    OnTop(Target),
    Exists(Target),
    TextOf(Target, TextMatch),
    Tree {
        of: Target,
        matches: String,
    },
    Screenshot {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Option<Target>,
        #[serde(default, skip_serializing_if = "Tolerance::is_default")]
        tolerance: Tolerance,
    },
    Not(Box<Check>),
    All(Vec<Check>),
    Any(Vec<Check>),
    Within(u16, Box<Check>),
}

const FORMS: &str = "a check: App(..), Shows(..), Hovered(..), Count(..), Resource(..), \
    Component(..), InState(..), Disabled(..), Checked(..), Selected(..), Pressed(..), \
    Focused(..), Visible(..), OnTop(..), Exists(..), TextOf(..), Tree(..), Screenshot(..), Not(..), All(..), Any(..) or Within(..)";

const BY_NAME: &str = "checks are no longer looked up by name: declare the app's checks as \
    one enum with `#[derive(Reflect)]` and `#[reflect(AppCheck)]`, and write one as \
    `App(..)`, such as `Check(App(StationNamed(\"Alpha\")))`";

pub(crate) const NO_STATE_PROBE: &str = "the state probe is gone: publish the state as a \
    reflected resource (`#[derive(Resource, Reflect)]` with `#[reflect(Resource)]`) and check \
    it with `Resource(path: \"Type.field\", is: Eq(..))`";

impl Serialize for Check {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Spelled::serialize(self, serializer)
    }
}

/// Values are kept as written, which only RON can give, so checks are read from
/// RON only.
impl<'de> Deserialize<'de> for Check {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        parse(raw.trim().get_ron()).map_err(serde::de::Error::custom)
    }
}

fn parse(text: &str) -> Result<Check, String> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct CountOf {
    with: Vec<String>,
}

impl CountOf {
    pub fn and<C: Component + TypePath>(mut self) -> Self {
        self.with.push(C::short_type_path().to_owned());
        self
    }

    #[must_use]
    pub fn is(self, comparison: Comparison) -> Check {
        Check::Count {
            with: self.with,
            is: comparison,
        }
    }

    #[must_use]
    pub fn eq(self, count: usize) -> Check {
        self.is(Comparison::Eq(Value::of(&count)))
    }

    #[must_use]
    pub fn ne(self, count: usize) -> Check {
        self.is(Comparison::Ne(Value::of(&count)))
    }

    #[must_use]
    pub fn lt(self, count: usize) -> Check {
        self.is(Comparison::Lt(Value::of(&count)))
    }

    #[must_use]
    pub fn le(self, count: usize) -> Check {
        self.is(Comparison::Le(Value::of(&count)))
    }

    #[must_use]
    pub fn gt(self, count: usize) -> Check {
        self.is(Comparison::Gt(Value::of(&count)))
    }

    #[must_use]
    pub fn ge(self, count: usize) -> Check {
        self.is(Comparison::Ge(Value::of(&count)))
    }

    #[must_use]
    pub fn between(self, low: usize, high: usize) -> Check {
        self.is(Comparison::Between(Value::of(&low), Value::of(&high)))
    }
}

#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct FieldOf {
    target: Option<Target>,
    path: String,
    unit: Unit,
}

impl FieldOf {
    pub const fn logical(mut self) -> Self {
        self.unit = Unit::Logical;
        self
    }

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

    #[must_use]
    pub fn eq<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Eq(Value::of(&value)))
    }

    #[must_use]
    pub fn ne<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Ne(Value::of(&value)))
    }

    #[must_use]
    pub fn lt<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Lt(Value::of(&value)))
    }

    #[must_use]
    pub fn le<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Le(Value::of(&value)))
    }

    #[must_use]
    pub fn gt<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Gt(Value::of(&value)))
    }

    #[must_use]
    pub fn ge<T: Reflect + GetTypeRegistration + TypePath>(self, value: T) -> Check {
        self.is(Comparison::Ge(Value::of(&value)))
    }

    #[must_use]
    pub fn between<T: Reflect + GetTypeRegistration + TypePath>(self, low: T, high: T) -> Check {
        self.is(Comparison::Between(Value::of(&low), Value::of(&high)))
    }
}

/// A path that starts with its own separator (`.0`, `[2]`) is kept as it is.
fn join_path(type_path: &str, field: &str) -> String {
    if field.is_empty() || field.starts_with(['.', '[', '#']) {
        format!("{type_path}{field}")
    } else {
        format!("{type_path}.{field}")
    }
}

impl Check {
    #[must_use]
    pub fn app<T: AppCheck + Reflect + GetTypeRegistration + TypePath>(check: T) -> Self {
        Self::App(AppCheckValue::of(check))
    }

    #[must_use]
    pub fn shows(text: impl Into<String>) -> Self {
        Self::Shows(text.into())
    }

    #[must_use]
    pub const fn hovered(target: Target) -> Self {
        Self::Hovered(target)
    }

    pub fn count<C: Component + TypePath>() -> CountOf {
        CountOf {
            with: vec![C::short_type_path().to_owned()],
        }
    }

    pub fn count_of(with: impl IntoIterator<Item = impl Into<String>>) -> CountOf {
        CountOf {
            with: with.into_iter().map(Into::into).collect(),
        }
    }

    pub fn resource<R: Resource + TypePath>(path: &str) -> FieldOf {
        FieldOf {
            target: None,
            path: join_path(R::short_type_path(), path),
            unit: Unit::Physical,
        }
    }

    pub fn component<C: Component + TypePath>(target: Target, path: &str) -> FieldOf {
        FieldOf {
            target: Some(target),
            path: join_path(C::short_type_path(), path),
            unit: Unit::Physical,
        }
    }

    #[must_use]
    pub fn in_state<S: States + Reflect + GetTypeRegistration + TypePath>(state: S) -> Self {
        Self::InState {
            state: S::short_type_path().to_owned(),
            value: Value::of(&state),
        }
    }

    #[must_use]
    pub const fn disabled(target: Target) -> Self {
        Self::Disabled(target)
    }

    #[must_use]
    pub const fn checked(target: Target) -> Self {
        Self::Checked(target)
    }

    #[must_use]
    pub const fn selected(target: Target) -> Self {
        Self::Selected(target)
    }

    #[must_use]
    pub const fn pressed(target: Target) -> Self {
        Self::Pressed(target)
    }

    #[must_use]
    pub const fn focused(target: Target) -> Self {
        Self::Focused(target)
    }

    #[must_use]
    pub const fn visible(target: Target) -> Self {
        Self::Visible(target)
    }

    #[must_use]
    pub const fn on_top(target: Target) -> Self {
        Self::OnTop(target)
    }

    #[must_use]
    pub const fn exists(target: Target) -> Self {
        Self::Exists(target)
    }

    #[must_use]
    pub const fn text_of(target: Target, matching: TextMatch) -> Self {
        Self::TextOf(target, matching)
    }

    #[must_use]
    pub fn tree(of: Target, tree: impl Into<String>) -> Self {
        Self::Tree {
            of,
            matches: tree.into(),
        }
    }

    #[must_use]
    pub fn all(checks: impl IntoIterator<Item = impl Into<Self>>) -> Self {
        Self::All(checks.into_iter().map(Into::into).collect())
    }

    #[must_use]
    pub fn screenshot(name: impl Into<String>) -> Self {
        Self::Screenshot {
            name: name.into(),
            of: None,
            tolerance: Tolerance::default(),
        }
    }

    #[must_use]
    pub fn screenshot_of(name: impl Into<String>, target: Target) -> Self {
        Self::Screenshot {
            name: name.into(),
            of: Some(target),
            tolerance: Tolerance::default(),
        }
    }

    #[must_use]
    pub const fn with_tolerance(mut self, tolerance: Tolerance) -> Self {
        if let Self::Screenshot { tolerance: own, .. } = &mut self {
            *own = tolerance;
        }
        self
    }

    #[must_use]
    pub fn any(checks: impl IntoIterator<Item = impl Into<Self>>) -> Self {
        Self::Any(checks.into_iter().map(Into::into).collect())
    }

    #[must_use]
    pub fn within(frames: u16, check: impl Into<Self>) -> Self {
        Self::Within(frames, Box::new(check.into()))
    }

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

    pub(crate) fn has_screenshot(&self) -> bool {
        self.parts()
            .into_iter()
            .any(|part| matches!(part, Self::Screenshot { .. }))
    }

    /// `None` while a [`Check::Within`] inside still waits.
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

    /// The outer error means the check could not run at all, which
    /// [`Check::Not`] passes on rather than inverting.
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

    fn resolve_is(&self, is: &Comparison, captures: &CapturedValues) -> Result<Comparison, String> {
        is.resolve(captures)
            .map_err(|message| format!("{}: {message}", self.subject()))
    }

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

fn logical(unit: Unit, subject: &str) -> String {
    if unit.is_logical() {
        format!("logical {subject}")
    } else {
        subject.to_owned()
    }
}

/// The outer error says the path cannot be used at all; the inner one that the
/// world does not have it now, which may change.
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

/// `None` when the type is registered but no entity ever had it.
pub(crate) fn component_id(world: &World, name: &str) -> Result<Option<ComponentId>, String> {
    let registry = reflect::app_types(world)?;
    let id = find_type(&registry.read(), name, Role::Component)
        .map(bevy::reflect::TypeRegistration::type_id)
        .map_err(|problem| problem.message())?;
    Ok(world.components().get_id(id))
}

pub(crate) fn check_component_type(types: &TypeRegistry, name: &str) -> Result<(), Problem> {
    find_type(types, name, Role::Component).map(|_| ())
}

pub(crate) fn close_matches<'a>(
    wanted: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    reflect::close_names(wanted, candidates)
}

pub(crate) fn label_of(world: &World, entity: Entity) -> String {
    widget::label_of(world, entity)
}

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

pub(crate) fn widget_of(world: &World, node: Entity) -> Entity {
    widget::widget(world, node)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedValue {
    ron: String,
    type_name: String,
}

impl CapturedValue {
    pub(crate) fn ron(&self) -> &str {
        &self.ron
    }

    pub(crate) fn type_name(&self) -> &str {
        &self.type_name
    }

    pub(crate) const fn new(ron: String, type_name: String) -> Self {
        Self { ron, type_name }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CapturedValues {
    values: BTreeMap<String, CapturedValue>,
}

impl CapturedValues {
    pub(crate) fn get(&self, name: &str) -> Option<&CapturedValue> {
        self.values.get(name)
    }

    pub(crate) fn insert(&mut self, name: String, value: CapturedValue) {
        self.values.insert(name, value);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Problem {
    UnknownType {
        name: String,
        suggestions: Vec<String>,
    },
    UnknownPath {
        path: String,
        message: String,
        suggestions: Vec<String>,
    },
    BadValue {
        value: String,
        message: String,
    },
    SeveralAppTypes {
        extension: Extension,
        types: Vec<String>,
    },
    NoAppType(Extension),
    BadAppValue {
        extension: Extension,
        written: String,
        type_name: &'static str,
        message: String,
        variants: Vec<String>,
    },
    Other(String),
}

impl Problem {
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

pub(crate) fn did_you_mean(suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        return String::new();
    }
    let names: Vec<String> = suggestions.iter().map(|name| format!("`{name}`")).collect();
    format!("; did you mean {}?", names.join(" or "))
}

fn field_registration<'r>(
    types: &'r TypeRegistry,
    path: &str,
    role: Role,
) -> Result<&'r bevy::reflect::TypeRegistration, Problem> {
    let (type_name, field) = reflect::split_path(path);
    let registration = find_type(types, type_name, role)?;
    reflect::field_type(types, registration, path, field)
}

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

fn compare(
    is: &Comparison,
    measured: &dyn Comparable,
    actual: &str,
    subject: impl Fn() -> String,
) -> Result<CheckResult, String> {
    let holds = is.test(measured).map_err(|message| {
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

/// `Visible` and `OnTop` are about the named node itself; the other states
/// climb to the widget.
fn widget_state(world: &World, target: &Target, state: WidgetState) -> Result<CheckResult, String> {
    match widget::node(world, target) {
        Lookup::Found(node) => Ok(widget::in_state(
            world,
            if state.is_about_the_node() {
                node
            } else if state == WidgetState::OnTop {
                widget::painted_subject(world, node)
            } else {
                widget::widget(world, node)
            },
            state,
            &format!("{target:?}"),
        )),
        Lookup::Missing(message) => Ok(Err(message)),
        Lookup::Ambiguous(message) | Lookup::NotANode(message) => Err(message),
    }
}

fn self_and_ancestors(world: &World, entity: Entity) -> Vec<Entity> {
    let mut chain = vec![entity];
    let mut current = entity;
    while let Some(parent) = world.get::<ChildOf>(current) {
        current = parent.parent();
        chain.push(current);
    }
    chain
}

fn first_broken(verdicts: &[Verdict]) -> Option<&Verdict> {
    verdicts.iter().find(|v| matches!(v, Verdict::Broken(_)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    Holds,
    Fails(String),
    Pending(String),
    Broken(String),
    Unavailable(String),
}

impl Verdict {
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

fn list(checks: &[Check]) -> String {
    checks
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

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
