//! Scenarios as data: the serializable [`Script`], the [`Registry`] of types it is read against,
//! and RON reading and writing.
//!
//! Closures cannot be serialized, so whatever is the app's own is a value of one of its
//! reflected enums, written inside `App(..)` and read against the registry's types when the
//! script loads. A value or type the registry cannot read is an error, never a silent skip.
//!
//! A file that uses a form or optional field added after version 3 first shipped (`Group`,
//! `Capture`, `action_timeout`, `scale_factor`, `variants`) is a newer version.

use alloc::sync::Arc;
use core::fmt;

use bevy::{
    ecs::{
        reflect::{AppTypeRegistry, ReflectComponent, ReflectResource},
        world::World,
    },
    reflect::{
        FromReflect, GetTypeRegistration, ReflectDeserialize, ReflectFromReflect, ReflectSerialize,
        TypeData, TypeRegistration, TypeRegistry, TypeRegistryArc, Typed,
        std_traits::ReflectDefault,
    },
    state::{
        reflect::ReflectState,
        state::{State, States},
    },
};
use ron::{Options, extensions::Extensions, ser::PrettyConfig};
use serde::{Deserialize, Serialize};

pub use crate::extension::{Setup, View};
use crate::{
    check::{Check, NO_STATE_PROBE, Problem, did_you_mean},
    extension::{
        AppType, Extension, SeveralAppTypes, app_type, bad_app_value, no_app_type, several_message,
    },
    scenario::{Scenario, Step, with_group_path},
    targets::Target,
};

/// The script format this crate writes. Files older than [`OLDEST_FORMAT_VERSION`] or newer
/// than this are refused. Additions to version 3 only add forms, so older files still load.
pub const FORMAT_VERSION: u32 = 3;

/// The oldest script format this crate still reads.
pub const OLDEST_FORMAT_VERSION: u32 = 1;

const fn check_version(found: u32) -> Result<(), LoadError> {
    if found >= OLDEST_FORMAT_VERSION && found <= FORMAT_VERSION {
        Ok(())
    } else {
        Err(LoadError::UnsupportedVersion {
            found,
            supported: FORMAT_VERSION,
        })
    }
}

/// ```
/// use bevy_stagecraft::script::Script;
///
/// let script = Script::from_ron(
///     "(format_version: 2, name: \"demo\", steps: [Click(Left), Wait(2), Check(Shows(\"1\"))])",
/// )
/// .unwrap();
/// assert_eq!(script.steps.len(), 3);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    pub format_version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_size: Option<(u32, u32)>,
    /// The window keeps its logical size. Must be finite and above zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_factor: Option<f32>,
    /// Frames a step waits for its target to be actionable; [`crate::DEFAULT_ACTION_TIMEOUT`]
    /// when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_timeout: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<Setup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<View>,
    /// Names of the variants the scenario runs in; empty means every variant, and a runner
    /// skips the others. The default variant answers to `default`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<String>,
    /// Labels a runner filters by (`RIN_SCENARIO_TAGS`, the gallery's tag filter).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip: Option<String>,
    /// A known problem: not run, and listed apart from the skipped ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixme: Option<String>,
    /// Passes only if the scenario fails, so a fixed bug is noticed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub should_fail: Option<String>,
    pub steps: Vec<Step>,
    /// A recorded session writes the checks it suggests here (see [`crate::check::suggest`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect: Vec<Check>,
}

impl Script {
    /// Only the format version is checked here; [`Scenario::from_script`] reads the rest.
    ///
    /// # Errors
    ///
    /// A syntax error with its position, or [`LoadError::UnsupportedVersion`].
    pub fn from_ron(text: &str) -> Result<Self, LoadError> {
        /// Just the version, so a newer file is refused as such even when the rest would not
        /// parse.
        #[derive(Debug, Deserialize)]
        struct Version {
            format_version: u32,
        }
        let options = ron_options();
        let version: Version = options
            .from_str(text)
            .map_err(|error| LoadError::Syntax(error.to_string()))?;
        check_version(version.format_version)?;
        options.from_str(text).map_err(|error| {
            let message = error.to_string();
            if message.contains("`expect_state`") {
                LoadError::Syntax(format!("{message}: {NO_STATE_PROBE}"))
            } else {
                LoadError::Syntax(message)
            }
        })
    }

    /// # Errors
    ///
    /// [`ExportError`] if serialization fails.
    pub fn to_ron(&self) -> Result<String, ExportError> {
        export_ron(self)
    }
}

pub(crate) fn ron_options() -> Options {
    Options::default().with_default_extension(Extensions::IMPLICIT_SOME)
}

/// # Errors
///
/// [`ExportError`] if serialization fails.
pub fn export_ron(script: &Script) -> Result<String, ExportError> {
    let config = PrettyConfig::new().extensions(Extensions::IMPLICIT_SOME);
    ron_options()
        .to_string_pretty(script, config)
        .map_err(|error| ExportError(error.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportError(String);

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot write the script as RON: {}", self.0)
    }
}

impl core::error::Error for ExportError {}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadError {
    /// For a mistake inside a spelled-out [`Check`] the position is where the whole check ends,
    /// as checks are read whole.
    Syntax(String),
    UnsupportedVersion {
        found: u32,
        supported: u32,
    },
    UnknownType {
        name: String,
        used_by: String,
        suggestions: Vec<String>,
    },
    UnknownPath {
        path: String,
        used_by: String,
        message: String,
        suggestions: Vec<String>,
    },
    BadValue {
        value: String,
        used_by: String,
        message: String,
    },
    SeveralAppTypes {
        extension: Extension,
        used_by: String,
        types: Vec<String>,
    },
    NoAppType {
        extension: Extension,
        used_by: String,
    },
    BadAppValue {
        extension: Extension,
        used_by: String,
        value: String,
        type_name: &'static str,
        message: String,
        variants: Vec<String>,
    },
    BadCheck {
        used_by: String,
        message: String,
    },
    Invalid {
        used_by: String,
        message: String,
    },
    Io {
        path: String,
        message: String,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(message) => write!(f, "invalid script: {message}"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "script format version {found} is not supported; this crate reads versions {OLDEST_FORMAT_VERSION} to {supported}"
            ),
            Self::Io { path, message } => write!(f, "cannot read {path}: {message}"),
            Self::UnknownType {
                name,
                used_by,
                suggestions,
            } => write!(
                f,
                "{used_by} names type `{name}`, which is not registered{}",
                did_you_mean(suggestions)
            ),
            Self::UnknownPath {
                used_by,
                message,
                suggestions,
                ..
            } => write!(f, "{used_by}: {message}{}", did_you_mean(suggestions)),
            Self::BadValue {
                value,
                used_by,
                message,
            } => write!(f, "{used_by}: `{value}` does not fit: {message}"),
            Self::SeveralAppTypes {
                extension,
                used_by,
                types,
            } => write!(f, "{used_by}: {}", several_message(*extension, types)),
            Self::NoAppType { extension, used_by } => {
                write!(f, "{used_by}: {}", no_app_type(*extension))
            }
            Self::BadAppValue {
                used_by,
                value,
                type_name,
                message,
                variants,
                ..
            } => write!(
                f,
                "{used_by}: {}",
                bad_app_value(value, type_name, message, variants)
            ),
            Self::BadCheck { used_by, message } | Self::Invalid { used_by, message } => {
                write!(f, "{used_by}: {message}")
            }
        }
    }
}

impl core::error::Error for LoadError {}

/// The types a script is read against when it loads, so a script is checked whole with no
/// world at hand.
#[derive(Debug, Clone)]
pub struct Registry {
    /// Starts with every type that derives `Reflect`, as Bevy registers them itself; add a type
    /// it cannot find (a generic one) with [`Registry::register_type`].
    pub types: TypeRegistryArc,
}

impl Default for Registry {
    fn default() -> Self {
        let types = TypeRegistryArc::default();
        types.write().register_derived_types();
        Self { types }
    }
}

impl Registry {
    pub fn register_type<T: GetTypeRegistration>(&mut self) {
        self.types.write().register::<T>();
    }

    pub fn register_state<S: States + FromReflect + GetTypeRegistration + Typed>(&mut self) {
        let mut types = self.types.write();
        types.register::<S>();
        types.register::<State<S>>();
        types.register_type_data::<S, ReflectState>();
    }

    /// # Errors
    ///
    /// [`SeveralAppTypes`] when more than one does.
    pub fn app_type(&self, extension: Extension) -> Result<Option<AppType>, SeveralAppTypes> {
        app_type(&self.types.read(), extension)
    }

    #[must_use]
    pub fn with_types_of(mut self, world: &World) -> Self {
        if let Some(types) = world.get_resource::<AppTypeRegistry>() {
            self.types = types.0.clone();
        }
        self
    }

    /// A type the app already registered keeps its registration and gains the type data the
    /// checks need.
    pub fn install(&self, world: &mut World) {
        let app = world.get_resource_or_init::<AppTypeRegistry>();
        // Sharing one registry, the app already has every type; locking it
        // twice would deadlock.
        if !Arc::ptr_eq(&app.0.internal, &self.types.internal) {
            let ours = self.types.read();
            let mut theirs = app.write();
            for registration in ours.iter() {
                if let Some(existing) = theirs.get_mut(registration.type_id()) {
                    fill_type_data(existing, registration);
                } else {
                    theirs.add_registration(registration.clone());
                }
            }
        }
    }
}

/// Copies type data the checks read that the app's registration lacks: without, say,
/// [`ReflectState`] a check would load and still fail when it runs.
fn fill_type_data(theirs: &mut TypeRegistration, ours: &TypeRegistration) {
    fn fill<T: TypeData + Clone>(theirs: &mut TypeRegistration, ours: &TypeRegistration) {
        if !theirs.contains::<T>()
            && let Some(data) = ours.data::<T>()
        {
            theirs.insert(data.clone());
        }
    }
    fill::<ReflectState>(theirs, ours);
    fill::<ReflectComponent>(theirs, ours);
    fill::<ReflectResource>(theirs, ours);
    fill::<ReflectFromReflect>(theirs, ours);
    fill::<ReflectDeserialize>(theirs, ours);
    fill::<ReflectSerialize>(theirs, ours);
    fill::<ReflectDefault>(theirs, ours);
}

pub(crate) fn build_scenario(
    mut script: Script,
    registry: &Registry,
) -> Result<Scenario, LoadError> {
    check_version(script.format_version)?;
    if let Some(scale_factor) = script.scale_factor {
        check_scale_factor(scale_factor).map_err(|message| LoadError::Invalid {
            used_by: "the scale factor".to_owned(),
            message,
        })?;
    }
    let types = registry.types.read();
    if let Some(setup) = &mut script.setup {
        setup
            .type_against(&types)
            .map_err(|problem| load_error(problem, "the setup", false))?;
    }
    if let Some(view) = &mut script.view {
        view.type_against(&types)
            .map_err(|problem| load_error(problem, "the view", false))?;
    }
    let mut captured = Vec::new();
    resolve_steps(
        &mut script.steps,
        &types,
        &mut 0,
        &mut Vec::new(),
        &mut captured,
    )?;
    for check in &mut script.expect {
        resolve_check(&types, check, "the expect list")?;
        // The expect list runs last, so it sees every capture.
        known_captures(check, &captured, "the expect list", "the script")?;
    }
    drop(types);
    Ok(Scenario::assemble(script))
}

pub(crate) fn check_scale_factor(scale_factor: f32) -> Result<(), String> {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        Ok(())
    } else {
        Err(format!(
            "the scale factor is {scale_factor}; it must be finite and above zero"
        ))
    }
}

/// `index` counts steps that are not groups, as the report does; `captured` collects the names
/// `Capture` steps give, which a check may use only after one.
fn resolve_steps(
    steps: &mut [Step],
    types: &TypeRegistry,
    index: &mut usize,
    path: &mut Vec<String>,
    captured: &mut Vec<String>,
) -> Result<(), LoadError> {
    for step in steps {
        if let Step::Group { name, steps } = step {
            if name.trim().is_empty() {
                return Err(LoadError::Invalid {
                    used_by: with_group_path(path, &format!("step {index}")),
                    message: "a `Group` needs a name".into(),
                });
            }
            path.push(name.clone());
            resolve_steps(steps, types, index, path, captured)?;
            path.pop();
            continue;
        }
        let used_by = with_group_path(path, &format!("step {index}"));
        *index += 1;
        check_wrapper(step, &used_by)?;
        for target in step_targets(step) {
            target
                .type_against(types)
                .map_err(|problem| load_error(problem, &used_by, false))?;
        }
        match step {
            Step::Check(check) => {
                resolve_check(types, check, &used_by)?;
                known_captures(check, captured, &used_by, "this step")?;
            }
            Step::Capture { name, value } => {
                if captured.contains(name) {
                    return Err(LoadError::Invalid {
                        used_by,
                        message: format!(
                            "`{name}` is captured twice; a captured name is used once in a script"
                        ),
                    });
                }
                value
                    .problem(types)
                    .map_err(|problem| load_error(problem, &used_by, false))?;
                captured.push(name.clone());
            }
            _ => {}
        }
    }
    Ok(())
}

fn known_captures(
    check: &Check,
    captured: &[String],
    used_by: &str,
    before: &str,
) -> Result<(), LoadError> {
    for name in check.captured_names() {
        if !captured.iter().any(|known| known == name) {
            return Err(LoadError::BadCheck {
                used_by: used_by.to_owned(),
                message: format!(
                    "nothing is captured as {name:?} before {before}; add `Capture(name: {name:?}, value: ..)` earlier"
                ),
            });
        }
    }
    Ok(())
}

fn resolve_check(types: &TypeRegistry, check: &mut Check, used_by: &str) -> Result<(), LoadError> {
    check
        .type_app_values(types)
        .map_err(|problem| load_error(problem, used_by, true))?;
    for part in check.parts() {
        part.problem(types)
            .map_err(|problem| load_error(problem, used_by, true))?;
    }
    Ok(())
}

fn load_error(problem: Problem, used_by: &str, in_check: bool) -> LoadError {
    let used_by = used_by.to_owned();
    match problem {
        Problem::UnknownType { name, suggestions } => LoadError::UnknownType {
            name,
            used_by,
            suggestions,
        },
        Problem::UnknownPath {
            path,
            message,
            suggestions,
        } => LoadError::UnknownPath {
            path,
            used_by,
            message,
            suggestions,
        },
        Problem::BadValue { value, message } => LoadError::BadValue {
            value,
            used_by,
            message,
        },
        Problem::SeveralAppTypes { extension, types } => LoadError::SeveralAppTypes {
            extension,
            used_by,
            types,
        },
        Problem::NoAppType(extension) => LoadError::NoAppType { extension, used_by },
        Problem::BadAppValue {
            extension,
            written,
            type_name,
            message,
            variants,
        } => LoadError::BadAppValue {
            extension,
            used_by,
            value: written,
            type_name,
            message,
            variants,
        },
        Problem::Other(message) if in_check => LoadError::BadCheck { used_by, message },
        Problem::Other(message) => LoadError::Invalid { used_by, message },
    }
}

fn step_targets(step: &mut Step) -> Vec<&mut Target> {
    match step {
        Step::MoveTo(target)
        | Step::Glide { target, .. }
        | Step::Type {
            into: Some(target), ..
        }
        | Step::TouchDown { at: target, .. }
        | Step::TouchMove { to: target, .. }
        | Step::TouchTap(target) => vec![target],
        Step::Drag { from, to, .. } | Step::TouchDrag { from, to, .. } => vec![from, to],
        Step::Capture { value, .. } => value.target_mut().into_iter().collect(),
        Step::Timeout(_, inner) | Step::Force(inner) => step_targets(inner),
        _ => Vec::new(),
    }
}

fn check_wrapper(step: &Step, used_by: &str) -> Result<(), LoadError> {
    let (Step::Timeout(_, inner) | Step::Force(inner)) = step else {
        return Ok(());
    };
    if inner.waits_for_target() {
        return Ok(());
    }
    Err(LoadError::Invalid {
        used_by: used_by.to_owned(),
        message: format!(
            "`Timeout(..)` and `Force(..)` wrap one MoveTo, Glide, Drag, Click, Press, Type with an `into`, or TouchDown, TouchMove, TouchTap or TouchDrag, not {inner:?}; \
             a check waits with `Check(Within(frames, ..))`"
        ),
    })
}
