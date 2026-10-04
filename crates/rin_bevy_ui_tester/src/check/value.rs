//! Values a script writes, and how checks compare against them.
//!
//! A value is kept as the RON text the script holds. Only once the check
//! knows what it is compared with (a field of a resource, a count, a state)
//! is the text read, against that type, so `Eq(Lay)` means the variant `Lay`
//! of whatever enum the field holds.

use core::{cmp::Ordering, fmt};

use bevy::reflect::{
    GetTypeRegistration, Reflect, TypePath, TypeRegistry, serde::TypedReflectSerializer,
};
use ron::value::RawValue;
use serde::{Deserialize, Serialize};

/// A value as a script writes it: RON text, read against the type of what it
/// is compared with.
///
/// ```
/// use rin_bevy_ui_tester::check::Value;
///
/// assert_eq!(Value::of(&3_u32).ron(), Ok("3"));
/// assert_eq!(Value::from_ron(" Lay ").unwrap().ron(), Ok("Lay"));
/// assert!(Value::from_ron("(").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value(Result<String, String>);

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
        Self(
            ron::to_string(&serializer)
                .map_err(|error| format!("cannot write {}: {error}", T::short_type_path())),
        )
    }

    /// The value written in `text`, which must be one RON value. Whitespace
    /// and comments around it are dropped.
    ///
    /// # Errors
    ///
    /// Why `text` is not one RON value.
    pub fn from_ron(text: &str) -> Result<Self, String> {
        let raw = RawValue::from_ron(text).map_err(|error| error.to_string())?;
        Ok(Self(Ok(raw.trim().get_ron().to_owned())))
    }

    /// The RON text, or why the value could not be written.
    ///
    /// # Errors
    ///
    /// Why [`Value::of`] could not write the value.
    pub fn ron(&self) -> Result<&str, &str> {
        self.0.as_deref().map_err(String::as_str)
    }
}

/// Written as the bare RON value, so a file reads `Eq(Lay)`, not a string.
impl Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let text = self.ron().map_err(serde::ser::Error::custom)?;
        RawValue::from_ron(text)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

/// Read as whatever RON value stands there; only RON can provide one.
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        Ok(Self(Ok(raw.trim().get_ron().to_owned())))
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Ok(text) => f.write_str(text),
            Err(reason) => write!(f, "<{reason}>"),
        }
    }
}

/// How a measured value must compare with the values written in the script.
///
/// ```ron
/// Count(with: ["Ghost"], is: Eq(1)),
/// Resource(path: "Score.0", is: Between(1, 3)),
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

    /// Whether `measured` compares as asked.
    ///
    /// # Errors
    ///
    /// Why the comparison cannot be made at all: a value that does not fit,
    /// or values that have no order.
    pub(super) fn test(&self, measured: &dyn Measure) -> Result<bool, String> {
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
pub(super) trait Measure {
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
            .map_err(|_| format!("`{text}` is not a count"))
    }
}

impl Measure for Counted {
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
