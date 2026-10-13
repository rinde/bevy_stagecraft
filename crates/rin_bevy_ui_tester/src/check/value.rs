//! Values a script writes, and how checks compare against them.
//!
//! A value is kept as the RON text the script holds. Only once the check
//! knows what it is compared with (a field of a resource, a count, a state)
//! is the text read, against that type, so `Eq(Lay)` means the variant `Lay`
//! of whatever enum the field holds.
//!
//! A value may also be a captured one, `Captured("views")`: what an earlier
//! `Capture` step measured. It is looked up when the check runs and then
//! compared as the literal it turned out to be. Anything written as
//! `Captured(..)` is read as a captured operand, so a literal of an app enum
//! with a variant called `Captured` cannot be written as a value.

use core::{cmp::Ordering, fmt};

use bevy::reflect::{
    GetTypeRegistration, Reflect, TypePath, TypeRegistry, serde::TypedReflectSerializer,
};
use ron::value::RawValue;
use serde::{Deserialize, Serialize};

use super::CapturedValues;

/// What a script spells a captured operand with: `Captured("views")`.
const CAPTURED: &str = "Captured";

/// A value as a script writes it: RON text, read against the type of what it
/// is compared with, or a value captured earlier in the scenario (see
/// [`Value::captured`]).
///
/// An offset (`plus`) is added to a whole number exactly, so a `u64` above
/// 2^53 keeps every digit, and to anything else as an `f64`.
///
/// ```
/// use rin_bevy_ui_tester::check::Value;
///
/// assert_eq!(Value::of(&3_u32).ron(), Ok("3"));
/// assert_eq!(Value::from_ron(" Lay ").unwrap().ron(), Ok("Lay"));
/// assert!(Value::from_ron("(").is_err());
/// assert_eq!(
///     Value::captured("views").plus(5.0).to_string(),
///     "Captured(name: \"views\", plus: 5)"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value(Repr);

/// What a [`Value`] is.
#[derive(Debug, Clone, PartialEq)]
enum Repr {
    /// RON text, or why it could not be written.
    Literal(Result<String, String>),
    /// A value a `Capture` step took earlier, with an optional offset.
    Captured {
        /// What the capture is called.
        name: String,
        /// Added to the captured number.
        plus: Option<f64>,
    },
    /// A captured value as it was when a check ran: never read from a file
    /// nor written to one.
    Resolved {
        /// The operand it came from, in words.
        from: String,
        /// The value, as RON.
        ron: String,
    },
}

/// Sound because the only `f64` a [`Repr`] holds is an offset that is finite
/// (`from_raw` and [`Value::plus`] refuse any other), so it equals itself.
impl Eq for Repr {}

impl Value {
    /// `value` as RON, written through its reflected form: the same form the
    /// check reads it back with.
    ///
    /// A value that reflection cannot write (a field whose type lacks
    /// serialisation) is kept as the reason, and the check that holds it
    /// cannot run.
    #[must_use]
    pub fn of<T: Reflect + GetTypeRegistration + TypePath>(value: &T) -> Self {
        let mut types = TypeRegistry::new();
        types.register::<T>();
        let serializer = TypedReflectSerializer::new(value.as_partial_reflect(), &types);
        Self(Repr::Literal(ron::to_string(&serializer).map_err(
            |error| format!("cannot write {}: {error}", T::short_type_path()),
        )))
    }

    /// The value written in `text`, which must be one RON value, or a
    /// captured one: `Captured("views")`, `Captured(name: "views", plus: 5)`.
    /// Whitespace and comments around it are dropped.
    ///
    /// # Errors
    ///
    /// Why `text` is not one RON value.
    pub fn from_ron(text: &str) -> Result<Self, String> {
        let raw = RawValue::from_ron(text).map_err(|error| error.to_string())?;
        Self::from_raw(raw.trim().get_ron())
    }

    /// The value in `text`, which is already known to be one RON value.
    fn from_raw(text: &str) -> Result<Self, String> {
        /// `Captured("name")`.
        #[derive(Debug, Deserialize)]
        enum Short {
            /// The only form.
            Captured(String),
        }
        /// `Captured(name: "name", plus: 5)`.
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        enum Long {
            /// The only form.
            Captured {
                /// What the capture is called.
                name: String,
                /// Added to the captured number.
                #[serde(default)]
                plus: Option<f64>,
            },
        }

        let text = text.trim();
        let is_captured = text
            .strip_prefix(CAPTURED)
            .is_some_and(|rest| rest.trim_start().starts_with('('));
        if !is_captured {
            return Ok(Self(Repr::Literal(Ok(text.to_owned()))));
        }
        let options = crate::script::ron_options();
        let (name, plus) = match options.from_str::<Short>(text) {
            Ok(Short::Captured(name)) => (name, None),
            Err(_) => {
                let Long::Captured { name, plus } = options.from_str(text).map_err(
                    |error: ron::error::SpannedError| {
                        format!(
                            "{}; write `Captured(\"name\")` or `Captured(name: \"name\", plus: 5)`",
                            error.code
                        )
                    },
                )?;
                (name, plus)
            }
        };
        if plus.is_some_and(|plus| !plus.is_finite()) {
            return Err("`plus` must be a finite number".into());
        }
        Ok(Self(Repr::Captured { name, plus }))
    }

    /// The value a `Capture` step took under `name`, to compare with in place
    /// of a literal. See [`Value::plus`].
    #[must_use]
    pub fn captured(name: impl Into<String>) -> Self {
        Self(Repr::Captured {
            name: name.into(),
            plus: None,
        })
    }

    /// This captured value with `amount` added to it, for numbers only. On a
    /// value that is not a captured one it makes the value unusable, and the
    /// check that holds it cannot run.
    #[must_use]
    pub fn plus(self, amount: f64) -> Self {
        match self.0 {
            Repr::Captured { name, plus } => {
                let total = plus.unwrap_or(0.0) + amount;
                if total.is_finite() {
                    Self(Repr::Captured {
                        name,
                        plus: Some(total),
                    })
                } else {
                    Self(Repr::Literal(Err(format!(
                        "cannot add {amount} to Captured({name:?}): `plus` must be a finite number"
                    ))))
                }
            }
            other => Self(Repr::Literal(Err(format!(
                "cannot add {amount} to {}: only a captured value takes `plus`",
                Self(other)
            )))),
        }
    }

    /// The RON text, or why the value could not be written (or, for a
    /// captured value, that it has not been looked up yet).
    ///
    /// # Errors
    ///
    /// Why [`Value::of`] could not write the value.
    pub fn ron(&self) -> Result<&str, &str> {
        match &self.0 {
            Repr::Literal(text) => text.as_deref().map_err(String::as_str),
            Repr::Resolved { ron, .. } => Ok(ron),
            Repr::Captured { .. } => Err("a captured value is only known while a scenario runs"),
        }
    }

    /// The name of the capture this value refers to, if it does.
    #[must_use]
    pub fn captured_name(&self) -> Option<&str> {
        match &self.0 {
            Repr::Captured { name, .. } => Some(name),
            Repr::Literal(_) | Repr::Resolved { .. } => None,
        }
    }

    /// This value with a captured operand replaced by what was captured, so
    /// it can be read against a type; any other value as it is.
    ///
    /// # Errors
    ///
    /// Why the operand cannot be used: nothing was captured under its name,
    /// or `plus` does not apply to what was.
    pub(super) fn resolve(&self, captures: &CapturedValues) -> Result<Self, String> {
        let Repr::Captured { name, plus } = &self.0 else {
            return Ok(self.clone());
        };
        let Some(found) = captures.get(name) else {
            return Err(format!(
                "nothing was captured as \"{name}\" before this ({self})"
            ));
        };
        let ron = match plus {
            None => found.ron().to_owned(),
            Some(plus) => add(found.ron(), *plus).map_err(|reason| {
                format!(
                    "cannot add {plus} to {self} = {} (a `{}`): {reason}",
                    found.ron(),
                    found.type_name()
                )
            })?,
        };
        Ok(Self(Repr::Resolved {
            from: self.to_string(),
            ron,
        }))
    }
}

/// `ron`, a number, plus `plus`, as RON: a whole number stays whole, and is
/// added exactly (an `f64` holds integers only up to 2^53).
fn add(ron: &str, plus: f64) -> Result<String, &'static str> {
    let options = crate::script::ron_options();
    // Digits alone are an integer, whatever its type; a float is written with
    // a point, an exponent or a name.
    let whole = ron
        .trim()
        .trim_start_matches('-')
        .chars()
        .all(|digit| digit.is_ascii_digit() || digit == '_');
    if whole && plus.fract() == 0.0 {
        // `plus` is whole and finite, so printing it without decimals is exact.
        let exact = format!("{plus:.0}").parse::<i128>();
        // RON allows `1_000`; reflection never writes it, a literal might.
        let number = ron.trim().replace('_', "").parse::<i128>();
        if let (Ok(number), Ok(plus)) = (number, exact) {
            return number
                .checked_add(plus)
                .map(|sum| sum.to_string())
                .ok_or("the sum is not finite");
        }
    }
    let Ok(number) = options.from_str::<f64>(ron) else {
        return Err("it is not a number");
    };
    let sum = number + plus;
    if !sum.is_finite() {
        return Err("the sum is not finite");
    }
    if !whole {
        return Ok(format!("{sum:?}"));
    }
    if sum.fract() == 0.0 {
        Ok(format!("{sum:.0}"))
    } else {
        Err("a whole number cannot take a fractional `plus`")
    }
}

/// Written as the bare RON value, so a file reads `Eq(Lay)`, not a string; a
/// captured one as `Captured("views")`, or with an offset as
/// `Captured(name: "views", plus: 5.0)`, which is RON too.
impl Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let text = match &self.0 {
            Repr::Captured { name, plus } => {
                let quoted = ron::to_string(name).map_err(serde::ser::Error::custom)?;
                plus.map_or_else(
                    || format!("{CAPTURED}({quoted})"),
                    |plus| format!("{CAPTURED}(name: {quoted}, plus: {plus:?})"),
                )
            }
            Repr::Resolved { .. } => {
                return Err(serde::ser::Error::custom(
                    "a captured value that was looked up is not part of a script",
                ));
            }
            Repr::Literal(_) => self.ron().map_err(serde::ser::Error::custom)?.to_owned(),
        };
        RawValue::from_ron(&text)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

/// Read as whatever RON value stands there, or a captured one; only RON can
/// provide one.
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        Self::from_raw(raw.trim().get_ron()).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Literal(Ok(text)) => f.write_str(text),
            Repr::Literal(Err(reason)) => write!(f, "<{reason}>"),
            Repr::Captured { name, plus: None } => write!(f, "{CAPTURED}({name:?})"),
            Repr::Captured {
                name,
                plus: Some(plus),
            } => write!(f, "{CAPTURED}(name: {name:?}, plus: {plus})"),
            Repr::Resolved { from, ron } => write!(f, "{from} = {ron}"),
        }
    }
}

/// How a measured value must compare with the values written in the script.
///
/// A value is a literal, or one a `Capture` step took earlier:
///
/// ```ron
/// Count(with: ["Ghost"], is: Eq(1)),
/// Resource(path: "Score.0", is: Between(1, 3)),
/// Count(with: ["Ghost"], is: Eq(Captured("ghosts"))),
/// Resource(path: "Score.0", is: Ge(Captured(name: "score", plus: 1))),
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Comparison {
    /// Equal to the value.
    Eq(Value),
    /// Not equal to the value.
    Ne(Value),
    /// Less than the value.
    Lt(Value),
    /// Less than or equal to the value.
    Le(Value),
    /// Greater than the value.
    Gt(Value),
    /// Greater than or equal to the value.
    Ge(Value),
    /// From the first value to the second, both included.
    Between(Value, Value),
}

impl Comparison {
    /// The values the comparison holds, in order.
    pub(super) fn values(&self) -> Vec<&Value> {
        match self {
            Self::Eq(value)
            | Self::Ne(value)
            | Self::Lt(value)
            | Self::Le(value)
            | Self::Gt(value)
            | Self::Ge(value) => vec![value],
            Self::Between(low, high) => vec![low, high],
        }
    }

    /// Whether the comparison needs an order, not just equality.
    pub(super) const fn orders(&self) -> bool {
        !matches!(self, Self::Eq(_) | Self::Ne(_))
    }

    /// Whether any value in the comparison is a captured one, as written or
    /// already looked up.
    pub(super) fn is_captured(&self) -> bool {
        self.values()
            .iter()
            .any(|value| matches!(value.0, Repr::Captured { .. } | Repr::Resolved { .. }))
    }

    /// The names of the captures the comparison refers to, in order.
    pub(super) fn captured_names(&self) -> Vec<&str> {
        self.values()
            .into_iter()
            .filter_map(Value::captured_name)
            .collect()
    }

    /// The comparison with every captured value replaced by what was
    /// captured, ready to [`test`](Self::test).
    ///
    /// # Errors
    ///
    /// Why a captured value cannot be used.
    pub(super) fn resolve(&self, captures: &CapturedValues) -> Result<Self, String> {
        let look = |value: &Value| value.resolve(captures);
        Ok(match self {
            Self::Eq(value) => Self::Eq(look(value)?),
            Self::Ne(value) => Self::Ne(look(value)?),
            Self::Lt(value) => Self::Lt(look(value)?),
            Self::Le(value) => Self::Le(look(value)?),
            Self::Gt(value) => Self::Gt(look(value)?),
            Self::Ge(value) => Self::Ge(look(value)?),
            Self::Between(low, high) => Self::Between(look(low)?, look(high)?),
        })
    }

    /// Whether `measured` compares as asked.
    ///
    /// # Errors
    ///
    /// Why the comparison cannot be made at all: a value that does not fit,
    /// or values that have no order.
    pub(super) fn test(&self, measured: &dyn Comparable) -> Result<bool, String> {
        Ok(match self {
            Self::Eq(value) => measured.equals(value)?,
            Self::Ne(value) => !measured.equals(value)?,
            Self::Lt(value) => measured.order(value)? == Ordering::Less,
            Self::Le(value) => measured.order(value)? != Ordering::Greater,
            Self::Gt(value) => measured.order(value)? == Ordering::Greater,
            Self::Ge(value) => measured.order(value)? != Ordering::Less,
            Self::Between(low, high) => {
                measured.order(low)? != Ordering::Less && measured.order(high)? != Ordering::Greater
            }
        })
    }

    /// What the comparison expects, in words: `1`, `less than 3`.
    #[must_use]
    pub fn expected(&self) -> String {
        match self {
            Self::Eq(value) => value.to_string(),
            Self::Ne(value) => format!("not {value}"),
            Self::Lt(value) => format!("less than {value}"),
            Self::Le(value) => format!("at most {value}"),
            Self::Gt(value) => format!("more than {value}"),
            Self::Ge(value) => format!("at least {value}"),
            Self::Between(low, high) => format!("between {low} and {high}"),
        }
    }
}

/// `is 1`, `is less than 3`.
impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is {}", self.expected())
    }
}

/// Something a [`Comparison`] can be made against: a count, or a reflected
/// field.
pub(super) trait Comparable {
    /// Whether the measured value equals `value`.
    ///
    /// # Errors
    ///
    /// Why `value` cannot be compared with it.
    fn equals(&self, value: &Value) -> Result<bool, String>;

    /// How the measured value orders against `value`.
    ///
    /// # Errors
    ///
    /// Why the two have no order.
    fn order(&self, value: &Value) -> Result<Ordering, String>;
}

/// A count of entities, compared with whole numbers.
#[derive(Debug, Clone, Copy)]
pub(super) struct Counted(pub(super) usize);

impl Counted {
    /// Reads `value` as a count.
    pub(super) fn read(value: &Value) -> Result<usize, String> {
        let text = value.ron()?;
        crate::script::ron_options()
            .from_str::<usize>(text)
            .map_err(|_| format!("`{value}` is not a count"))
    }
}

impl Comparable for Counted {
    fn equals(&self, value: &Value) -> Result<bool, String> {
        Ok(self.0 == Self::read(value)?)
    }

    fn order(&self, value: &Value) -> Result<Ordering, String> {
        Ok(self.0.cmp(&Self::read(value)?))
    }
}

/// How the text of a target must read.
///
/// ```ron
/// TextOf(Text("Score"), StartsWith("Score: ")),
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TextMatch {
    /// Exactly this text.
    Exact(String),
    /// This text somewhere in it.
    Contains(String),
    /// Starting with this text.
    StartsWith(String),
}

impl TextMatch {
    /// Exactly `text`.
    #[must_use]
    pub fn exact(text: impl Into<String>) -> Self {
        Self::Exact(text.into())
    }

    /// `text` somewhere in it.
    #[must_use]
    pub fn contains(text: impl Into<String>) -> Self {
        Self::Contains(text.into())
    }

    /// Starting with `text`.
    #[must_use]
    pub fn starts_with(text: impl Into<String>) -> Self {
        Self::StartsWith(text.into())
    }

    /// Whether `text` matches.
    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        match self {
            Self::Exact(wanted) => text == wanted,
            Self::Contains(wanted) => text.contains(wanted.as_str()),
            Self::StartsWith(wanted) => text.starts_with(wanted.as_str()),
        }
    }
}

/// `is "OK"`, `contains "OK"`, `starts with "OK"`.
impl fmt::Display for TextMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(text) => write!(f, "is \"{text}\""),
            Self::Contains(text) => write!(f, "contains \"{text}\""),
            Self::StartsWith(text) => write!(f, "starts with \"{text}\""),
        }
    }
}
