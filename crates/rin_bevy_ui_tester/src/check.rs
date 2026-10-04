//! Checks as data: what a [`Step::Check`](crate::Step::Check) or an
//! expectation asks of the world.
//!
//! The common questions (is this text shown, is this under the pointer, what
//! does the app's state say) are spelled out in the script itself, so a `.ron`
//! file reads on its own. Anything app-specific is a check defined in Rust and
//! referred to by name, with an argument if it takes one.

use core::{fmt, ops::Not};

use bevy::ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::{
    probe::{Probe, check_hover},
    scenario::{CheckResult, Scenario},
    targets::{Target, resolve},
};

/// What a check asks of the world.
///
/// In a script, a check defined in Rust is written as just its name in
/// quotes; the others are spelled out:
///
/// ```ron
/// Check("saved"),
/// Check(Shows("Score: 3")),
/// Check(Hovered(Text("OK"))),
/// Check(State(key: "score", value: "3")),
/// Check(Not(Shows("Error"))),
/// Check(Custom(check: "disabled", argument: "OK")),
/// ```
///
/// In Rust, `!check` wraps a check in [`Check::Not`], and a `&str` or
/// `String` turns into [`Check::Named`].
///
/// ```
/// use rin_bevy_ui_tester::{Check, Target};
///
/// let check = !Check::hovered(Target::text("OK"));
/// assert_eq!(check.to_string(), "not: Text(\"OK\") is under the pointer");
/// assert_eq!(Check::from("saved"), Check::named("saved"));
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Check {
    /// A check defined in Rust: with [`Scenario::check`] or, for a loaded
    /// script, registered in [`crate::CheckRegistry`]. Written as just the
    /// name.
    Named(String),
    /// A check defined in Rust that takes an argument, like
    /// [`Target::Custom`]: with [`Scenario::custom_check`] or
    /// [`crate::CheckRegistry::register_custom`].
    Custom {
        /// The name the check is defined under.
        check: String,
        /// What to check, in a form the check understands.
        argument: String,
    },
    /// Some UI node of the app shows exactly this text.
    Shows(String),
    /// The target's widget, or something below it, is under the pointer. A
    /// target that is only a position names no widget, and the check fails.
    Hovered(Target),
    /// The app's state probe (see [`crate::StateProbe`]) reports `key` with
    /// exactly this value.
    State {
        /// What the probe calls the value.
        key: String,
        /// The value it must have, as text.
        value: String,
    },
    /// The check inside does not hold. A check that cannot run at all, such
    /// as a name nobody defined, still fails.
    Not(Box<Self>),
}

/// [`Check`] with every variant written out by name: serde's derived form,
/// which [`Written`] adds the bare name to. A mirror rather than a derive on
/// [`Check`] itself, so the derived functions stay private.
#[derive(Serialize, Deserialize)]
#[serde(remote = "Check")]
enum Spelled {
    /// See [`Check::Named`].
    Named(String),
    /// See [`Check::Custom`].
    Custom {
        /// See [`Check::Custom`].
        check: String,
        /// See [`Check::Custom`].
        argument: String,
    },
    /// See [`Check::Shows`].
    Shows(String),
    /// See [`Check::Hovered`].
    Hovered(Target),
    /// See [`Check::State`].
    State {
        /// See [`Check::State`].
        key: String,
        /// See [`Check::State`].
        value: String,
    },
    /// See [`Check::Not`].
    Not(Box<Check>),
}

/// How a check is written in a script: just a name in quotes, or spelled out.
#[derive(Deserialize)]
#[serde(
    untagged,
    expecting = "a check: a name in quotes, or Shows(..), Hovered(..), State(..), Not(..), Custom(..) or Named(..)"
)]
enum Written {
    /// A check defined in Rust, by name.
    Name(String),
    /// Any check, spelled out.
    Spelled(#[serde(with = "Spelled")] Check),
}

/// A named check is written as just its name, which is also how version 1
/// scripts wrote every check.
impl Serialize for Check {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Named(name) => serializer.serialize_str(name),
            spelled => Spelled::serialize(spelled, serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Check {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Written::deserialize(deserializer)? {
            Written::Name(name) => Self::Named(name),
            Written::Spelled(check) => check,
        })
    }
}

impl Check {
    /// The check defined in Rust under `name`.
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self::Named(name.into())
    }

    /// The check defined in Rust under `check`, asked about `argument`.
    #[must_use]
    pub fn custom(check: impl Into<String>, argument: impl Into<String>) -> Self {
        Self::Custom {
            check: check.into(),
            argument: argument.into(),
        }
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

    /// The app's state reports `key` as `value`.
    #[must_use]
    pub fn state(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self::State {
            key: key.into(),
            value: value.into(),
        }
    }

    /// This check and every check inside it, outermost first.
    pub(crate) fn parts(&self) -> Vec<&Self> {
        let mut parts = vec![self];
        let mut current = self;
        while let Self::Not(inner) = current {
            parts.push(inner);
            current = inner;
        }
        parts
    }

    /// Runs the check against `world`. Checks defined in Rust, and the state
    /// probe, come from `scenario`.
    ///
    /// # Errors
    ///
    /// Why the check did not hold, or why it could not run.
    pub(crate) fn run(&self, world: &World, scenario: &Scenario) -> CheckResult {
        self.evaluate(world, scenario).and_then(|held| held)
    }

    /// Runs the check. The outer error means it could not run at all, which
    /// [`Check::Not`] passes on rather than inverting.
    fn evaluate(&self, world: &World, scenario: &Scenario) -> Result<CheckResult, String> {
        let probe = Probe::new(world);
        match self {
            Self::Named(name) => scenario
                .checks()
                .iter()
                .find(|check| check.name == *name)
                .map(|check| check.run(&probe))
                .ok_or_else(|| format!("the scenario defines no check named `{name}`")),
            Self::Custom { check, argument } => scenario
                .custom_checks()
                .iter()
                .find(|defined| defined.name == *check)
                .map(|defined| defined.run(&probe, argument))
                .ok_or_else(|| format!("the scenario defines no custom check named `{check}`")),
            Self::Shows(text) => Ok(if probe.has_text(text) {
                Ok(())
            } else {
                Err(format!("no UI node shows \"{text}\""))
            }),
            Self::Hovered(target) => {
                let resolved = match resolve(world, target) {
                    Ok(resolved) => resolved,
                    Err(message) => return Ok(Err(message)),
                };
                let widget = resolved.widget.ok_or_else(|| {
                    format!("{target:?} is a position, not a widget, so it cannot be hovered")
                })?;
                Ok(check_hover(world, widget, &format!("{target:?}")))
            }
            Self::State { key, value } => {
                let state = scenario
                    .state_probe()
                    .ok_or_else(|| format!("there is no state probe to read `{key}` with"))?
                    .snapshot(world);
                let actual = state
                    .get(key)
                    .ok_or_else(|| format!("the app's state has no `{key}`"))?;
                Ok(if actual == value {
                    Ok(())
                } else {
                    Err(format!("state `{key}` is \"{actual}\", not \"{value}\""))
                })
            }
            Self::Not(inner) => Ok(match inner.evaluate(world, scenario)? {
                Ok(()) => Err(format!("{inner} holds, but should not")),
                Err(_) => Ok(()),
            }),
        }
    }
}

/// The check in words, which is also its name in reports.
impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => f.write_str(name),
            Self::Custom { check, argument } => write!(f, "{check} \"{argument}\""),
            Self::Shows(text) => write!(f, "\"{text}\" is shown"),
            Self::Hovered(target) => write!(f, "{target:?} is under the pointer"),
            Self::State { key, value } => write!(f, "state `{key}` is \"{value}\""),
            Self::Not(inner) => write!(f, "not: {inner}"),
        }
    }
}

impl Not for Check {
    type Output = Self;

    fn not(self) -> Self {
        Self::Not(Box::new(self))
    }
}

impl From<&str> for Check {
    fn from(name: &str) -> Self {
        Self::named(name)
    }
}

impl From<String> for Check {
    fn from(name: String) -> Self {
        Self::Named(name)
    }
}
