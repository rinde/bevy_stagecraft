//! Values a script writes, and how checks compare against them. A value stays
//! RON text until compared, so `Eq(Lay)` reads as the field's own enum.

use core::{cmp::Ordering, fmt};

use bevy::reflect::{
    GetTypeRegistration, Reflect, TypePath, TypeRegistry, serde::TypedReflectSerializer,
};
use ron::value::RawValue;
use serde::{Deserialize, Serialize};

use super::CapturedValues;

const CAPTURED: &str = "Captured";

/// Whether a number is a physical or logical pixel value. Bevy's UI mixes both
/// (`ComputedNode.size` is physical, `ScrollPosition` logical).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Unit {
    #[default]
    Physical,
    Logical,
}

impl Unit {
    #[must_use]
    pub const fn is_logical(self) -> bool {
        matches!(self, Self::Logical)
    }

    #[must_use]
    pub const fn is_physical(&self) -> bool {
        matches!(self, Self::Physical)
    }
}

/// `plus` adds exactly to a whole number, so a `u64` above 2^53 keeps every
/// digit.
///
/// ```
/// use bevy_stagecraft::check::Value;
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

#[derive(Debug, Clone, PartialEq)]
enum Repr {
    Literal(Result<String, String>),
    Captured { name: String, plus: Option<f64> },
    Resolved { from: String, ron: String },
}

/// Sound because the only `f64` is a finite offset (`from_raw` and
/// [`Value::plus`] refuse any other).
impl Eq for Repr {}

impl Value {
    /// A value reflection cannot write is kept as its reason, and the check
    /// holding it cannot run.
    #[must_use]
    pub fn of<T: Reflect + GetTypeRegistration + TypePath>(value: &T) -> Self {
        let mut types = TypeRegistry::new();
        types.register::<T>();
        let serializer = TypedReflectSerializer::new(value.as_partial_reflect(), &types);
        Self(Repr::Literal(ron::to_string(&serializer).map_err(
            |error| format!("cannot write {}: {error}", T::short_type_path()),
        )))
    }

    /// Whitespace and comments around the value are dropped.
    ///
    /// # Errors
    ///
    /// Why `text` is not one RON value.
    pub fn from_ron(text: &str) -> Result<Self, String> {
        let raw = RawValue::from_ron(text).map_err(|error| error.to_string())?;
        Self::from_raw(raw.trim().get_ron())
    }

    fn from_raw(text: &str) -> Result<Self, String> {
        #[derive(Debug, Deserialize)]
        enum Short {
            Captured(String),
        }
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        enum Long {
            Captured {
                name: String,
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

    #[must_use]
    pub fn captured(name: impl Into<String>) -> Self {
        Self(Repr::Captured {
            name: name.into(),
            plus: None,
        })
    }

    /// Numbers only: on a value that is not captured it makes the value
    /// unusable.
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

    /// # Errors
    ///
    /// The value is captured, or its text is not RON.
    pub fn ron(&self) -> Result<&str, &str> {
        match &self.0 {
            Repr::Literal(text) => text.as_deref().map_err(String::as_str),
            Repr::Resolved { ron, .. } => Ok(ron),
            Repr::Captured { .. } => Err("a captured value is only known while a scenario runs"),
        }
    }

    #[must_use]
    pub fn captured_name(&self) -> Option<&str> {
        match &self.0 {
            Repr::Captured { name, .. } => Some(name),
            Repr::Literal(_) | Repr::Resolved { .. } => None,
        }
    }

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

/// A whole number stays whole and adds exactly (an `f64` holds integers only
/// up to 2^53).
fn add(ron: &str, plus: f64) -> Result<String, &'static str> {
    let options = crate::script::ron_options();
    // Digits alone are an integer; a float has a point, an exponent or a name.
    let whole = ron
        .trim()
        .trim_start_matches('-')
        .chars()
        .all(|digit| digit.is_ascii_digit() || digit == '_');
    if whole && plus.fract() == 0.0 {
        let exact = format!("{plus:.0}").parse::<i128>();
        // RON allows `1_000`; reflection never writes it.
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

/// How a measured value must compare with the values in the script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Comparison {
    Eq(Value),
    Ne(Value),
    Lt(Value),
    Le(Value),
    Gt(Value),
    Ge(Value),
    Between(Value, Value),
}

impl Comparison {
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

    pub(super) const fn orders(&self) -> bool {
        !matches!(self, Self::Eq(_) | Self::Ne(_))
    }

    pub(super) fn is_captured(&self) -> bool {
        self.values()
            .iter()
            .any(|value| matches!(value.0, Repr::Captured { .. } | Repr::Resolved { .. }))
    }

    pub(super) fn captured_names(&self) -> Vec<&str> {
        self.values()
            .into_iter()
            .filter_map(Value::captured_name)
            .collect()
    }

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

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is {}", self.expected())
    }
}

pub(super) trait Comparable {
    fn equals(&self, value: &Value) -> Result<bool, String>;

    fn order(&self, value: &Value) -> Result<Ordering, String>;
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Counted(pub(super) usize);

impl Counted {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TextMatch {
    Exact(String),
    Contains(String),
    StartsWith(String),
}

impl TextMatch {
    #[must_use]
    pub fn exact(text: impl Into<String>) -> Self {
        Self::Exact(text.into())
    }

    #[must_use]
    pub fn contains(text: impl Into<String>) -> Self {
        Self::Contains(text.into())
    }

    #[must_use]
    pub fn starts_with(text: impl Into<String>) -> Self {
        Self::StartsWith(text.into())
    }

    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        match self {
            Self::Exact(wanted) => text == wanted,
            Self::Contains(wanted) => text.contains(wanted.as_str()),
            Self::StartsWith(wanted) => text.starts_with(wanted.as_str()),
        }
    }
}

impl fmt::Display for TextMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(text) => write!(f, "is \"{text}\""),
            Self::Contains(text) => write!(f, "contains \"{text}\""),
            Self::StartsWith(text) => write!(f, "starts with \"{text}\""),
        }
    }
}
