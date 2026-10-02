//! Scenarios as data: the serialisable [`Script`], the registries that give
//! its names meaning, and reading and writing RON.
//!
//! A scenario written in Rust and a `.ron` file are two spellings of the same
//! [`Script`]. Closures cannot be serialised, so a script refers to checks,
//! setups, views and app-defined targets by name, and a [`Registry`] maps
//! those names to code. Loading a script that names something the registry
//! lacks is an error, never a silent skip.

use alloc::{collections::BTreeMap, sync::Arc};
use core::fmt;

use bevy::ecs::world::World;
use ron::{Options, extensions::Extensions, ser::PrettyConfig};
use serde::{Deserialize, Serialize};

use crate::{
    probe::Probe,
    scenario::{CheckResult, HookFn, IntoCheckResult, NamedCheck, Scenario, Step, named},
    targets::{Target, TargetResolvers},
};

/// The script format this crate reads and writes. A file with another
/// version is refused rather than guessed at.
pub const FORMAT_VERSION: u32 = 1;

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
///     "(format_version: 1, name: \"demo\", steps: [Click(Left), Wait(2)])",
/// )
/// .unwrap();
/// assert_eq!(script.steps.len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// Must equal [`FORMAT_VERSION`].
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
    /// Names of checks that run once the steps have finished.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect: Vec<String>,
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
        #[derive(Deserialize)]
        struct Version {
            /// The file's format version.
            format_version: u32,
        }
        let options = ron_options();
        let version: Version = options
            .from_str(text)
            .map_err(|error| LoadError::Syntax(error.to_string()))?;
        if version.format_version != FORMAT_VERSION {
            return Err(LoadError::UnsupportedVersion {
                found: version.format_version,
                supported: FORMAT_VERSION,
            });
        }
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
fn ron_options() -> Options {
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
    /// position.
    Syntax(String),
    /// The file is of a format version this crate does not read.
    UnsupportedVersion {
        /// The version in the file.
        found: u32,
        /// The version this crate reads.
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
    /// The script has `expect_state` entries but no state probe is registered
    /// to read them with.
    NoStateProbe,
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
                "script format version {found} is not supported; this crate reads version {supported}"
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
                "the script has `expect_state` entries, but no state probe is registered"
            ),
            Self::Io { path, message } => write!(f, "cannot read {path}: {message}"),
        }
    }
}

impl core::error::Error for LoadError {}

/// Named checks over a [`Probe`]. Scripts refer to checks by these names.
#[derive(Clone, Default)]
pub struct CheckRegistry {
    /// Checks by name, ordered so nothing depends on hash order.
    checks: BTreeMap<String, NamedCheck>,
}

impl fmt::Debug for CheckRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.checks.keys()).finish()
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

    /// The check called `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&NamedCheck> {
        self.checks.get(name)
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

/// The name of the check made from one `expect_state` entry.
pub(crate) fn state_check_name(key: &str, value: &str) -> String {
    format!("state `{key}` is \"{value}\"")
}

/// The end-of-script check for one `expect_state` entry.
fn state_check(probe: &StateProbe, key: &str, value: &str) -> NamedCheck {
    let probe = probe.clone();
    let (key, value) = (key.to_owned(), value.to_owned());
    named(
        state_check_name(&key, &value),
        move |checked: &Probe<'_>| -> CheckResult {
            match probe.snapshot(checked.world()).get(&key) {
                Some(actual) if *actual == value => Ok(()),
                Some(actual) => Err(format!("state `{key}` is \"{actual}\", not \"{value}\"")),
                None => Err(format!("the app's state has no `{key}`")),
            }
        },
    )
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
    /// Reads the app's state, for [`Script::expect_state`].
    pub state: Option<StateProbe>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            checks: CheckRegistry::generic(),
            setups: HookRegistry::default(),
            views: HookRegistry::default(),
            targets: TargetResolvers::default(),
            state: None,
        }
    }
}

impl Registry {
    /// Sets the probe that reads the app's state, for
    /// [`Script::expect_state`].
    #[must_use]
    pub fn with_state(mut self, probe: StateProbe) -> Self {
        self.state = Some(probe);
        self
    }

    /// Makes the registered target resolvers available in `world`, where the
    /// player looks for them.
    pub fn install(&self, world: &mut World) {
        world
            .get_resource_or_insert_with(TargetResolvers::default)
            .extend_from(&self.targets);
    }
}

/// Resolves every name in `script` against `registry`.
pub(crate) fn build_scenario(script: Script, registry: &Registry) -> Result<Scenario, LoadError> {
    if script.format_version != FORMAT_VERSION {
        return Err(LoadError::UnsupportedVersion {
            found: script.format_version,
            supported: FORMAT_VERSION,
        });
    }
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
    let mut checks: Vec<NamedCheck> = Vec::new();
    for (index, step) in script.steps.iter().enumerate() {
        let used_by = || format!("step {index}");
        for target in step_targets(step) {
            if let Target::Custom { resolver, .. } = target
                && !registry.targets.contains(resolver)
            {
                return Err(LoadError::UnknownTarget {
                    resolver: resolver.clone(),
                    used_by: used_by(),
                });
            }
        }
        if let Step::Check(name) = step
            && !checks.iter().any(|check| &check.name == name)
        {
            let check = registry
                .checks
                .get(name)
                .ok_or_else(|| LoadError::UnknownCheck {
                    name: name.clone(),
                    used_by: used_by(),
                })?;
            checks.push(check.clone());
        }
    }
    let expects = script
        .expect
        .iter()
        .map(|name| {
            registry
                .checks
                .get(name)
                .cloned()
                .ok_or_else(|| LoadError::UnknownCheck {
                    name: name.clone(),
                    used_by: "the expect list".into(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut expects = expects;
    if !script.expect_state.is_empty() {
        let probe = registry.state.as_ref().ok_or(LoadError::NoStateProbe)?;
        expects.extend(
            script
                .expect_state
                .iter()
                .map(|(key, value)| state_check(probe, key, value)),
        );
    }
    Ok(Scenario::assemble(script, setup, view, checks, expects))
}

/// The targets a step acts on.
fn step_targets(step: &Step) -> Vec<&Target> {
    match step {
        Step::MoveTo(target) | Step::Glide { target, .. } => vec![target],
        Step::Drag { from, to, .. } => vec![from, to],
        _ => Vec::new(),
    }
}
