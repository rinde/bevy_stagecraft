//! Scenarios as data: the serialisable [`Script`], the registries that give
//! its names meaning, and reading and writing RON.
//!
//! A scenario written in Rust and a `.ron` file are two spellings of the same
//! [`Script`]. Closures cannot be serialised, so a script refers to checks
//! defined in Rust, setups, views and app-defined targets by name, and a
//! [`Registry`] maps those names to code. Loading a script that names
//! something the registry lacks is an error, never a silent skip.

use alloc::{collections::BTreeMap, sync::Arc};
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

use crate::{
    check::{Check, Problem, did_you_mean},
    probe::Probe,
    scenario::{
        CheckResult, CustomCheck, HookFn, IntoCheckResult, NamedCheck, Scenario, Step, custom,
        named,
    },
    targets::{Target, TargetResolvers},
};

/// The script format this crate writes. A file of a version older than
/// [`OLDEST_FORMAT_VERSION`] or newer than this is refused rather than
/// guessed at.
///
/// Version 2 added checks spelled out as data ([`Check`]); version 3 added
/// the built-in checks that read the world through reflection (counts,
/// resources, components, states, widget states, text) and `All`, `Any` and
/// `Within`. Every file of an older version is also a file of a newer one.
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

/// Where a scenario's starting state comes from. The crate stores and passes
/// this along and never interprets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Setup {
    /// A function the app registered under this name (see
    /// [`Registry::setups`]).
    Named(String),
    /// A file next to the script, which the app knows how to load; rails uses
    /// its RON save files.
    File(String),
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
    /// Where the starting state comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<Setup>,
    /// The name of a registered view hook, which arranges how the world is
    /// shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// What the scenario does.
    pub steps: Vec<Step>,
    /// Checks that run once the steps have finished.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect: Vec<Check>,
    /// What the app's state must say once the steps have finished, as the
    /// app's own state probe (see [`StateProbe`]) names it: each key must
    /// have exactly this value. A recorded session writes what changed here,
    /// for the person to keep, edit or delete.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub expect_state: BTreeMap<String, String>,
}

impl Script {
    /// Parses RON text. Files may leave out the `Some(..)` around optional
    /// fields. Only the format version is checked here; names are resolved by
    /// [`Scenario::from_script`].
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
        options
            .from_str(text)
            .map_err(|error| LoadError::Syntax(error.to_string()))
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
    /// A check is named that nobody registered.
    UnknownCheck {
        /// The check's name.
        name: String,
        /// Where it was named, for example `step 3`.
        used_by: String,
    },
    /// A setup is named that nobody registered.
    UnknownSetup(String),
    /// A view is named that nobody registered.
    UnknownView(String),
    /// A custom target names a resolver that nobody registered.
    UnknownTarget {
        /// The resolver's name.
        resolver: String,
        /// Where it was named, for example `step 3`.
        used_by: String,
    },
    /// The script checks the app's state ([`Check::State`] or
    /// `expect_state`) but no state probe is registered to read it with.
    NoStateProbe,
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
    /// A check cannot run for another reason, such as a type that is
    /// registered but not as a resource.
    BadCheck {
        /// Where the check is, for example `step 3`.
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
            Self::UnknownCheck { name, used_by } => {
                write!(f, "{used_by} names check `{name}`, which is not registered")
            }
            Self::UnknownSetup(name) => {
                write!(
                    f,
                    "the script names setup `{name}`, which is not registered"
                )
            }
            Self::UnknownView(name) => {
                write!(f, "the script names view `{name}`, which is not registered")
            }
            Self::UnknownTarget { resolver, used_by } => write!(
                f,
                "{used_by} uses a custom target `{resolver}`, which has no registered resolver"
            ),
            Self::NoStateProbe => write!(
                f,
                "the script checks the app's state, but no state probe is registered"
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
            Self::BadCheck { used_by, message } => write!(f, "{used_by}: {message}"),
        }
    }
}

impl core::error::Error for LoadError {}

/// Checks over a [`Probe`] defined in Rust, by name: what [`Check::Named`]
/// and [`Check::Custom`] in a script refer to.
#[derive(Clone, Default)]
pub struct CheckRegistry {
    /// Checks by name, ordered so nothing depends on hash order.
    checks: BTreeMap<String, NamedCheck>,
    /// Checks that take an argument, by name.
    custom: BTreeMap<String, CustomCheck>,
}

impl fmt::Debug for CheckRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckRegistry")
            .field("checks", &self.checks.keys().collect::<Vec<_>>())
            .field("custom", &self.custom.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl CheckRegistry {
    /// Registers `check` under `name`, replacing any earlier one.
    pub fn register<R: IntoCheckResult>(
        &mut self,
        name: impl Into<String>,
        check: impl Fn(&Probe<'_>) -> R + Send + Sync + 'static,
    ) {
        let check = named(name, check);
        self.checks.insert(check.name.clone(), check);
    }

    /// Registers `check`, which takes an argument, under `name`, replacing
    /// any earlier one. A script runs it as
    /// `Custom(check: "name", argument: "...")`.
    pub fn register_custom<R: IntoCheckResult>(
        &mut self,
        name: impl Into<String>,
        check: impl Fn(&Probe<'_>, &str) -> R + Send + Sync + 'static,
    ) {
        let check = custom(name, check);
        self.custom.insert(check.name.clone(), check);
    }

    /// The check called `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&NamedCheck> {
        self.checks.get(name)
    }

    /// The check that takes an argument called `name`.
    #[must_use]
    pub fn get_custom(&self, name: &str) -> Option<&CustomCheck> {
        self.custom.get(name)
    }

    /// Whether a check is registered under `name`.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.checks.contains_key(name)
    }

    /// The registered names, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.checks.keys().map(String::as_str)
    }

    /// The checks this crate provides, which need no arguments:
    /// `pointer_over_ui` (some node is under the pointer) and
    /// `pointer_over_nothing` (none is).
    #[must_use]
    pub fn generic() -> Self {
        let mut registry = Self::default();
        registry.register("pointer_over_ui", |probe: &Probe<'_>| -> CheckResult {
            if probe.hovered().is_empty() {
                Err("nothing is under the pointer".into())
            } else {
                Ok(())
            }
        });
        registry.register("pointer_over_nothing", |probe: &Probe<'_>| -> CheckResult {
            if probe.hovered().is_empty() {
                Ok(())
            } else {
                Err("something is under the pointer".into())
            }
        });
        registry
    }
}

/// What the app's state looks like as plain text: named values.
///
/// The same probe serves a recording (what changed during the session) and a
/// loaded script (`expect_state`), so what is written is what is read.
///
/// ```
/// use rin_bevy_ui_tester::StateProbe;
///
/// let probe = StateProbe::new(|_world| [("score".to_owned(), "3".to_owned())].into());
/// let world = bevy::ecs::world::World::new();
/// assert_eq!(probe.snapshot(&world).get("score").map(String::as_str), Some("3"));
/// ```
#[derive(Clone)]
pub struct StateProbe(Arc<StateFn>);

/// What a state probe does.
type StateFn = dyn Fn(&World) -> BTreeMap<String, String> + Send + Sync;

impl fmt::Debug for StateProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StateProbe(..)")
    }
}

impl StateProbe {
    /// A probe that reads state with `read`. Keys name what is read, values
    /// are its current value as text; keep both stable and readable, as people
    /// edit them in the file.
    pub fn new(read: impl Fn(&World) -> BTreeMap<String, String> + Send + Sync + 'static) -> Self {
        Self(Arc::new(read))
    }

    /// The state of `world` right now.
    #[must_use]
    pub fn snapshot(&self, world: &World) -> BTreeMap<String, String> {
        (self.0)(world)
    }
}

/// Hooks over the world, by name: what [`Setup::Named`] and a script's view
/// refer to.
#[derive(Clone, Default)]
pub struct HookRegistry {
    /// Hooks by name.
    hooks: BTreeMap<String, Arc<HookFn>>,
}

impl fmt::Debug for HookRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.hooks.keys()).finish()
    }
}

impl HookRegistry {
    /// Registers `hook` under `name`, replacing any earlier one.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        hook: impl Fn(&mut World) + Send + Sync + 'static,
    ) {
        self.hooks.insert(name.into(), Arc::new(hook));
    }

    /// Whether a hook is registered under `name`.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.hooks.contains_key(name)
    }

    /// The hook called `name`.
    pub(crate) fn get(&self, name: &str) -> Option<Arc<HookFn>> {
        self.hooks.get(name).cloned()
    }
}

/// Everything a script's names are resolved against when it loads.
///
/// The default holds the [generic checks](CheckRegistry::generic). An app adds
/// its own checks, setups, views and target resolvers, then loads files with
/// [`Scenario::from_ron_with`] and installs the resolvers into the world it
/// plays in with [`Registry::install`].
#[derive(Debug, Clone)]
pub struct Registry {
    /// Named checks.
    pub checks: CheckRegistry,
    /// Named setups, for [`Setup::Named`].
    pub setups: HookRegistry,
    /// Named views.
    pub views: HookRegistry,
    /// Resolvers for [`Target::Custom`].
    pub targets: TargetResolvers,
    /// Reads the app's state, for [`Check::State`] and
    /// [`Script::expect_state`].
    pub state: Option<StateProbe>,
    /// The types checks may name, such as [`Check::Count`] and
    /// [`Check::Resource`]: a script is checked against them when it loads.
    /// Starts with the primitive types; add the app's with
    /// [`Registry::register_type`], or share the app's own registry with
    /// [`Registry::with_types_of`].
    pub types: TypeRegistryArc,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            checks: CheckRegistry::generic(),
            setups: HookRegistry::default(),
            views: HookRegistry::default(),
            targets: TargetResolvers::default(),
            state: None,
            types: TypeRegistryArc::default(),
        }
    }
}

impl Registry {
    /// Sets the probe that reads the app's state, for [`Check::State`] and
    /// [`Script::expect_state`].
    #[must_use]
    pub fn with_state(mut self, probe: StateProbe) -> Self {
        self.state = Some(probe);
        self
    }

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

    /// Uses the type registry of `world`'s app, shared rather than copied,
    /// so every type the app registers can be named by a script.
    #[must_use]
    pub fn with_types_of(mut self, world: &World) -> Self {
        if let Some(types) = world.get_resource::<AppTypeRegistry>() {
            self.types = types.0.clone();
        }
        self
    }

    /// Makes the registered target resolvers available in `world`, where the
    /// player looks for them, and adds the registered types the app's type
    /// registry lacks, where the checks look for them. A type the app has
    /// already registered keeps its registration and gains the type data the
    /// checks need (such as [`ReflectState`]) that it lacks.
    pub fn install(&self, world: &mut World) {
        world
            .get_resource_or_insert_with(TargetResolvers::default)
            .extend_from(&self.targets);
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

/// Resolves every name in `script` against `registry`.
pub(crate) fn build_scenario(script: Script, registry: &Registry) -> Result<Scenario, LoadError> {
    check_version(script.format_version)?;
    let setup = match &script.setup {
        Some(Setup::Named(name)) => Some(
            registry
                .setups
                .get(name)
                .ok_or_else(|| LoadError::UnknownSetup(name.clone()))?,
        ),
        Some(Setup::File(_)) | None => None,
    };
    let view = match &script.view {
        Some(name) => Some(
            registry
                .views
                .get(name)
                .ok_or_else(|| LoadError::UnknownView(name.clone()))?,
        ),
        None => None,
    };
    let types = registry.types.read();
    let mut resolved = Resolved::default();
    for (index, step) in script.steps.iter().enumerate() {
        let used_by = format!("step {index}");
        for target in step_targets(step) {
            require_resolver(registry, target, &used_by)?;
        }
        if let Step::Check(check) = step {
            resolved.add(registry, &types, check, &used_by)?;
        }
    }
    for check in &script.expect {
        resolved.add(registry, &types, check, "the expect list")?;
    }
    drop(types);
    if !script.expect_state.is_empty() {
        resolved.reads_state = true;
    }
    let state = match (&registry.state, resolved.reads_state) {
        (None, true) => return Err(LoadError::NoStateProbe),
        (state, _) => state.clone(),
    };
    Ok(Scenario::assemble(
        script,
        setup,
        view,
        resolved.checks,
        resolved.custom,
        state,
    ))
}

/// What a script's checks refer to, found in the registry.
#[derive(Debug, Default)]
struct Resolved {
    /// The named checks, one per name.
    checks: Vec<NamedCheck>,
    /// The checks that take an argument, one per name.
    custom: Vec<CustomCheck>,
    /// Whether some check reads the app's state.
    reads_state: bool,
}

impl Resolved {
    /// Finds what `check`, and every check inside it, refers to.
    fn add(
        &mut self,
        registry: &Registry,
        types: &TypeRegistry,
        check: &Check,
        used_by: &str,
    ) -> Result<(), LoadError> {
        let unknown = |name: &str| LoadError::UnknownCheck {
            name: name.to_owned(),
            used_by: used_by.to_owned(),
        };
        for part in check.parts() {
            match part {
                Check::Named(name) => {
                    if !self.checks.iter().any(|known| known.name == *name) {
                        let found = registry.checks.get(name).ok_or_else(|| unknown(name))?;
                        self.checks.push(found.clone());
                    }
                }
                Check::Custom { check: name, .. } => {
                    if !self.custom.iter().any(|known| known.name == *name) {
                        let found = registry
                            .checks
                            .get_custom(name)
                            .ok_or_else(|| unknown(name))?;
                        self.custom.push(found.clone());
                    }
                }
                Check::State { .. } => self.reads_state = true,
                _ => {}
            }
            if let Some(target) = part.target() {
                require_resolver(registry, target, used_by)?;
            }
            part.problem(types)
                .map_err(|problem| load_error(problem, used_by))?;
        }
        Ok(())
    }
}

/// The load error for a check that cannot run, naming where it is.
fn load_error(problem: Problem, used_by: &str) -> LoadError {
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
        Problem::Other(message) => LoadError::BadCheck { used_by, message },
    }
}

/// Refuses a custom target whose resolver nobody registered.
fn require_resolver(registry: &Registry, target: &Target, used_by: &str) -> Result<(), LoadError> {
    match target {
        Target::Custom { resolver, .. } if !registry.targets.contains(resolver) => {
            Err(LoadError::UnknownTarget {
                resolver: resolver.clone(),
                used_by: used_by.to_owned(),
            })
        }
        _ => Ok(()),
    }
}

/// The targets a step acts on.
fn step_targets(step: &Step) -> Vec<&Target> {
    match step {
        Step::MoveTo(target) | Step::Glide { target, .. } => vec![target],
        Step::Drag { from, to, .. } => vec![from, to],
        _ => Vec::new(),
    }
}
