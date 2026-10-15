//! Scenarios as data: the serialisable [`Script`], the [`Registry`] of types
//! it is read against, and reading and writing RON.
//!
//! A scenario written in Rust and a `.ron` file are two spellings of the same
//! [`Script`]. Closures cannot be serialised, so whatever is the app's own
//! (its setups, views, targets and checks) is a value of one of its reflected
//! enums (see [`crate::extension`]), written inside `App(..)` and read
//! against the registry's types when the script loads. Nothing is looked up
//! by a name. Loading a script whose values or types the registry cannot
//! read is an error, never a silent skip.

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

/// The script format this crate writes. A file of a version older than
/// [`OLDEST_FORMAT_VERSION`] or newer than this is refused rather than
/// guessed at.
///
/// Version 2 added checks spelled out as data ([`Check`]); version 3 added
/// the built-in checks that read the world through reflection (counts,
/// resources, components, states, widget states, text), `All`, `Any` and
/// `Within`, and the app's own checks as `App(..)`. Version 3 also dropped
/// checks referred to by name (a bare `"name"`, `Named(..)` and
/// `Custom(..)`), then setups, views and targets referred to by name or
/// resolver (`Named(..)` and `File(..)` setups, a view's bare name and
/// `Custom(resolver: .., argument: ..)` targets), which became the app's own
/// values inside `App(..)` as well: a file that uses an old form is refused
/// with a load error that says what to write instead. The locator targets
/// (`Component(..)`, `Name(..)`, `Role(..)`, `Within(..)` and `Nth(..)`)
/// came later still in version 3, and so did the script's `action_timeout`
/// and the `Timeout(..)` and `Force(..)` steps that go with steps waiting
/// for their targets to be actionable, the `scale_factor`, the `Type(..)` step
/// that types text, the `Group(name: .., steps: [..])` step that names a run
/// of steps, and the `Capture(name: .., value: ..)` step with the
/// `Captured(..)` operand of a comparison: they only add forms, so every file
/// that loaded before still does. The `variants` field came later still in
/// version 3 and also only adds a form, as do `tags`, `skip`, `fixme` and
/// `should_fail`.
/// Any other older file still loads.
pub const FORMAT_VERSION: u32 = 3;

/// The oldest script format this crate still reads.
pub const OLDEST_FORMAT_VERSION: u32 = 1;

/// Refuses a format version this crate does not read.
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

/// A scenario as plain data: what is written to and read from a `.ron` file.
///
/// ```
/// use rin_bevy_ui_tester::script::Script;
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
    /// The format the script is written in, from [`OLDEST_FORMAT_VERSION`]
    /// to [`FORMAT_VERSION`].
    pub format_version: u32,
    /// What the scenario is called.
    pub name: String,
    /// The window size the scenario wants, in logical pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_size: Option<(u32, u32)>,
    /// The scale factor of the window the scenario wants: physical pixels per
    /// logical pixel. The window keeps its logical size. Must be finite and
    /// above zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_factor: Option<f32>,
    /// How many frames a step waits for its target to be actionable before
    /// it fails; [`crate::DEFAULT_ACTION_TIMEOUT`] when left out. A step sets
    /// its own with `Timeout(frames, ..)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_timeout: Option<u16>,
    /// Where the starting state comes from: one of the app's setups, or a saved
    /// world (`Scene("..")`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<Setup>,
    /// How the world is shown: one of the app's views.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<View>,
    /// The names of the variants the scenario runs in (see
    /// [`Variant::name`](crate::Variant::name)); empty, the default, means
    /// every variant. A runner skips a file and variant not listed rather
    /// than failing it. The default variant answers to `default`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<String>,
    /// Labels a runner filters by (`RIN_SCENARIO_TAGS`, the gallery's tag
    /// filter); empty, the default, means none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Why the scenario is not run, when it is not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip: Option<String>,
    /// Why the scenario is a known problem and not run, when it is; listed
    /// apart from the skipped ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixme: Option<String>,
    /// Why the scenario is expected to fail, when it is: it passes only if it
    /// fails, so a fixed bug is noticed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub should_fail: Option<String>,
    /// What the scenario does.
    pub steps: Vec<Step>,
    /// Checks that run once the steps have finished. A recorded session
    /// writes the checks it suggests here (see [`crate::check::suggest`]), for the
    /// person to keep, edit or delete.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect: Vec<Check>,
}

impl Script {
    /// Parses RON text. Files may leave out the `Some(..)` around optional
    /// fields. Only the format version is checked here; the app's values and
    /// the types checks name are read by [`Scenario::from_script`].
    ///
    /// # Errors
    ///
    /// A syntax error with its position, or [`LoadError::UnsupportedVersion`].
    pub fn from_ron(text: &str) -> Result<Self, LoadError> {
        /// Just the version, so a file from a newer format is refused as such
        /// even when the rest of it would not parse.
        #[derive(Debug, Deserialize)]
        struct Version {
            /// The file's format version.
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

    /// The script as pretty RON.
    ///
    /// # Errors
    ///
    /// [`ExportError`] if serialisation fails.
    pub fn to_ron(&self) -> Result<String, ExportError> {
        export_ron(self)
    }
}

/// How RON is read: optional fields may omit `Some(..)`.
pub(crate) fn ron_options() -> Options {
    Options::default().with_default_extension(Extensions::IMPLICIT_SOME)
}

/// Writes any `script` out as pretty RON, for the gallery's export and the
/// recorder. [`Script::from_ron`] reads it back unchanged.
///
/// # Errors
///
/// [`ExportError`] if serialisation fails.
pub fn export_ron(script: &Script) -> Result<String, ExportError> {
    let config = PrettyConfig::new().extensions(Extensions::IMPLICIT_SOME);
    ron_options()
        .to_string_pretty(script, config)
        .map_err(|error| ExportError(error.to_string()))
}

/// Writing a script out failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportError(String);

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot write the script as RON: {}", self.0)
    }
}

impl core::error::Error for ExportError {}

/// Why a script could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadError {
    /// The text is not valid RON for a [`Script`]; the message has the
    /// position. For a mistake inside a spelled out [`Check`] that is the
    /// position where the whole check ends, as a check is read whole before
    /// it is parsed.
    Syntax(String),
    /// The file is of a format version this crate does not read.
    UnsupportedVersion {
        /// The version in the file.
        found: u32,
        /// The newest version this crate reads, [`FORMAT_VERSION`].
        supported: u32,
    },
    /// A check names a type that is not in [`Registry::types`], or that is
    /// not the kind of type the check needs.
    UnknownType {
        /// The type as written.
        name: String,
        /// Where it was named, for example `step 3`.
        used_by: String,
        /// Registered types with close names, closest first.
        suggestions: Vec<String>,
    },
    /// A check names a field its type does not have.
    UnknownPath {
        /// The path as written, type included.
        path: String,
        /// Where it was named, for example `step 3`.
        used_by: String,
        /// What is wrong with it.
        message: String,
        /// Fields with close names, closest first.
        suggestions: Vec<String>,
    },
    /// A value in a check does not fit the type it is compared with.
    BadValue {
        /// The value as written.
        value: String,
        /// Where it was written, for example `step 3`.
        used_by: String,
        /// Why it does not fit.
        message: String,
    },
    /// More than one type in [`Registry::types`] carries the attribute of
    /// one extension point, such as `#[reflect(AppSetup)]`, and a value
    /// written inside `App(..)` cannot tell which it is.
    SeveralAppTypes {
        /// The extension point.
        extension: Extension,
        /// Where the value is, for example `step 3` or `the setup`.
        used_by: String,
        /// The full type paths of every such type, in order.
        types: Vec<String>,
    },
    /// A value is written inside `App(..)`, but no type in
    /// [`Registry::types`] carries the extension point's attribute.
    NoAppType {
        /// The extension point.
        extension: Extension,
        /// Where the value is, for example `step 3` or `the setup`.
        used_by: String,
    },
    /// A value inside `App(..)` does not read as the app's type for its
    /// extension point: a misspelt variant, or a wrong argument.
    BadAppValue {
        /// The extension point.
        extension: Extension,
        /// Where it was written, for example `step 3` or `the view`.
        used_by: String,
        /// The value as written.
        value: String,
        /// The app's type, by its short type path.
        type_name: &'static str,
        /// Why it does not read.
        message: String,
        /// The type's variants with the types of their fields, as a file
        /// writes them, such as `StationNamed(String)`.
        variants: Vec<String>,
    },
    /// A check cannot run for another reason, such as a type that is
    /// registered but not as a resource.
    BadCheck {
        /// Where the check is, for example `step 3`.
        used_by: String,
        /// What is wrong.
        message: String,
    },
    /// The app's setup, view or a target cannot be used for another reason,
    /// such as a type that cannot be built from reflection.
    Invalid {
        /// Where it is, for example `step 3` or `the setup`.
        used_by: String,
        /// What is wrong.
        message: String,
    },
    /// A file could not be read.
    Io {
        /// The file.
        path: String,
        /// What went wrong.
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

/// The types a script is read against when it loads.
///
/// An app loads files with [`Scenario::from_ron_with`], and adds the types
/// to the app it plays them in with [`Registry::install`] when it does not
/// share one registry with it.
///
/// The registry carries the types, so a script is checked whole when it
/// loads, with no world at hand: a misspelt type, field or value, or an app
/// setup, view, target or check that does not read as the app's type, is a
/// [`LoadError`] rather than a failure part way through a run. The app's
/// types for its setups, views, targets and checks (see
/// [`crate::extension`]) are found there too; nothing is registered by name.
#[derive(Debug, Clone)]
pub struct Registry {
    /// The types checks may name, such as [`Check::Count`] and
    /// [`Check::Resource`], and the app's types that values inside `App(..)`
    /// are read as: a script is checked against them when it loads.
    ///
    /// Starts with every type that derives `Reflect`, as Bevy registers them
    /// by itself (an `App` does the same with Bevy's `reflect_auto_register`
    /// feature, which is on by default), so the app's types need no
    /// registration call. Add a type that is not found that way (a generic
    /// one, say) with [`Registry::register_type`], or share the app's own
    /// registry with [`Registry::with_types_of`].
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
    /// Registers `T`, and the types of its fields, for checks to name.
    pub fn register_type<T: GetTypeRegistration>(&mut self) {
        self.types.write().register::<T>();
    }

    /// Registers the state type `S`, `State<S>` and the type data that lets
    /// [`Check::InState`] read it, as `app.register_type_state::<S>()` does.
    pub fn register_state<S: States + FromReflect + GetTypeRegistration + Typed>(&mut self) {
        let mut types = self.types.write();
        types.register::<S>();
        types.register::<State<S>>();
        types.register_type_data::<S, ReflectState>();
    }

    /// The app's type for `extension` in [`Registry::types`], with its
    /// variants and the types of their fields, for offering values with a
    /// form; `None` when no type carries the extension point's attribute.
    ///
    /// # Errors
    ///
    /// [`SeveralAppTypes`] when more than one does.
    pub fn app_type(&self, extension: Extension) -> Result<Option<AppType>, SeveralAppTypes> {
        app_type(&self.types.read(), extension)
    }

    /// Uses the type registry of `world`'s app, shared rather than copied,
    /// so every type the app registers can be named by a script.
    #[must_use]
    pub fn with_types_of(mut self, world: &World) -> Self {
        if let Some(types) = world.get_resource::<AppTypeRegistry>() {
            self.types = types.0.clone();
        }
        self
    }

    /// Adds the registered types the app's type registry lacks to `world`,
    /// where the checks look for them. A type the app has already registered
    /// keeps its registration and gains the type data the checks need (such
    /// as [`ReflectState`]) that it lacks.
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

/// Copies into `theirs` the type data the checks read that `ours` has and
/// `theirs` lacks, keeping whatever `theirs` already holds.
///
/// The app may have registered a type itself (Bevy registers many on its
/// own) without, say, [`ReflectState`]; a check on it would then load and
/// still fail when it runs. Type data can only be copied by its concrete
/// type, so this names the kinds the checks look up.
fn fill_type_data(theirs: &mut TypeRegistration, ours: &TypeRegistration) {
    /// Copies one kind of type data, if `theirs` lacks it.
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

/// Reads every value inside `App(..)` in `script` (its setup, its view, the
/// targets of its steps and checks, and its checks) as the app's type in
/// `registry`, and checks that what each check names is there.
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

/// Refuses a scale factor that is not finite and above zero, saying why.
pub(crate) fn check_scale_factor(scale_factor: f32) -> Result<(), String> {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        Ok(())
    } else {
        Err(format!(
            "the scale factor is {scale_factor}; it must be finite and above zero"
        ))
    }
}

/// Reads the app's values in `steps`, and in the steps of the groups inside
/// them, and checks what each step names. `index` counts the steps that are
/// not groups, as the report does, so an error names the step the report
/// would; `path` is the groups around `steps`; `captured` collects the names
/// `Capture` steps give, in order, which a check may use only after one.
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

/// Refuses a check that compares with a value no `Capture` step has taken
/// among `captured`, naming the check's place `used_by`; `before` says before
/// what the capture must come.
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

/// Reads the app's values in `check`, and in every check inside it, as the
/// app's types, and checks that what each part names is there.
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

/// The load error for a value or check that cannot be used, naming where it
/// is; `in_check` says whether a check holds it.
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

/// The targets a step acts on, inside any `Timeout(..)` or `Force(..)`.
fn step_targets(step: &mut Step) -> Vec<&mut Target> {
    match step {
        Step::MoveTo(target)
        | Step::Glide { target, .. }
        | Step::Type {
            into: Some(target), ..
        } => vec![target],
        Step::Drag { from, to, .. } => vec![from, to],
        Step::Capture { value, .. } => value.target_mut().into_iter().collect(),
        Step::Timeout(_, inner) | Step::Force(inner) => step_targets(inner),
        _ => Vec::new(),
    }
}

/// Refuses a `Timeout(..)` or `Force(..)` around a step that does not wait
/// for a target, or around another of them.
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
            "`Timeout(..)` and `Force(..)` wrap one MoveTo, Glide, Drag, Click, Press or Type with an `into`, not {inner:?}; \
             a check waits with `Check(Within(frames, ..))`"
        ),
    })
}
