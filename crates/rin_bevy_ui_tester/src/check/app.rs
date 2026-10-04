//! Checks an app declares itself, as one reflected enum.
//!
//! An app writes its checks as the variants of one enum that derives
//! `Reflect` and carries `#[reflect(AppCheck)]`, and implements [`AppCheck`]
//! for it. Bevy's type registry finds the enum: there is no registration
//! call and no generic parameter. A script writes a value of it inside
//! `App(..)`, which is read against the enum when the script loads, so a
//! misspelt variant or a wrong argument is a load error, not a failure part
//! way through a run.

use alloc::sync::Arc;
use core::fmt;

use bevy::reflect::{
    FromType, GetTypeRegistration, PartialReflect, Reflect, ReflectFromReflect, TypeInfo, TypePath,
    TypeRegistration, TypeRegistry,
    enums::VariantInfo,
    reflect_trait,
    serde::{TypedReflectDeserializer, TypedReflectSerializer},
};
use ron::value::RawValue;
use serde::{Deserialize, Serialize};

use super::Problem;
use crate::{probe::Probe, scenario::CheckResult};

/// A check the app defines, on one reflected enum of its own.
///
/// Derive `Reflect`, add `#[reflect(AppCheck)]` (with [`ReflectAppCheck`] in
/// scope) and implement this trait; Bevy registers the type by itself. One
/// type per app: a second type with `#[reflect(AppCheck)]` is a load error
/// naming both.
///
/// Every field of a variant must be `Reflect` as well, since a script's value
/// is read and written through reflection. Primitives, `String` and
/// [`Target`](crate::Target) read best in a file.
///
/// ```
/// use bevy::prelude::*;
/// use rin_bevy_ui_tester::{AppCheck, Check, CheckResult, Probe, ReflectAppCheck};
///
/// #[derive(Resource)]
/// struct Score(u32);
///
/// #[derive(Reflect, Debug, Clone)]
/// #[reflect(AppCheck)]
/// enum GameCheck {
///     /// The score is exactly this.
///     ScoreIs(u32),
/// }
///
/// impl AppCheck for GameCheck {
///     fn run(&self, probe: &Probe) -> CheckResult {
///         match self {
///             Self::ScoreIs(wanted) => match probe.resource::<Score>() {
///                 Some(score) if score.0 == *wanted => Ok(()),
///                 Some(score) => Err(format!("the score is {}", score.0)),
///                 None => Err("there is no score".into()),
///             },
///         }
///     }
/// }
///
/// let check = Check::app(GameCheck::ScoreIs(3));
/// assert_eq!(check.to_string(), "ScoreIs(3)");
/// ```
#[reflect_trait]
pub trait AppCheck {
    /// Whether the check holds in the world `probe` reads.
    ///
    /// # Errors
    ///
    /// What is wrong, when it does not hold.
    fn run(&self, probe: &Probe<'_>) -> CheckResult;
}

// By hand: the struct `#[reflect_trait]` generates holds only function
// pointers, and derives nothing but `Clone`.
impl fmt::Debug for ReflectAppCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReflectAppCheck").finish_non_exhaustive()
    }
}

/// What a check that cannot be read as the app's type is told: how to
/// declare one.
const NO_APP_CHECK_TYPE: &str = "no type is an app check; derive `Reflect` on an enum, add \
    `#[reflect(AppCheck)]` and implement `AppCheck` for it";

/// The value inside a [`Check::App`](super::Check::App): one of the app's
/// checks.
///
/// Read from a file, it is kept as the RON text written there until the
/// script loads against a [`Registry`](crate::Registry), which reads it as
/// the app's type with [`TypedReflectDeserializer`]. Written to a file, it is
/// the RON text [`TypedReflectSerializer`] gives. Two values are equal when
/// their RON text is.
#[derive(Clone)]
pub struct AppCheckValue {
    /// The value as RON: as written in the file until it is typed, then as
    /// its type writes it; or why it could not be written.
    ron: Result<String, String>,
    /// The value itself, once its type is known.
    typed: Option<TypedApp>,
}

/// An app check value of a known type.
#[derive(Clone)]
struct TypedApp {
    /// The value, of the app's own type.
    value: Arc<dyn Reflect>,
    /// How to see it as an [`AppCheck`].
    check: ReflectAppCheck,
}

impl fmt::Debug for AppCheckValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppCheckValue")
            .field("ron", &self.ron)
            .field("typed", &self.typed.as_ref().map(|typed| &typed.value))
            .finish()
    }
}

// The RON text alone decides equality: an app has one app check type, so two
// values with the same text are the same value of it, typed or not yet typed.
impl PartialEq for AppCheckValue {
    fn eq(&self, other: &Self) -> bool {
        self.ron == other.ron
    }
}

impl AppCheckValue {
    /// `value`, written as RON through its reflected form: the form a file
    /// holds and is read back with.
    ///
    /// A value reflection cannot write (a field whose type lacks
    /// serialisation) still runs, but exporting it is an error.
    #[must_use]
    pub fn of<T: AppCheck + Reflect + GetTypeRegistration + TypePath>(value: T) -> Self {
        let mut types = TypeRegistry::new();
        types.register::<T>();
        let ron = write(&types, value.as_partial_reflect())
            .map_err(|error| format!("cannot write {}: {error}", T::short_type_path()));
        Self {
            ron,
            typed: Some(TypedApp {
                value: Arc::new(value),
                check: <ReflectAppCheck as FromType<T>>::from_type(),
            }),
        }
    }

    /// The value written in `text`, which must be one RON value, not yet
    /// read as any type. Whitespace and comments around it are dropped.
    ///
    /// # Errors
    ///
    /// Why `text` is not one RON value.
    pub fn from_ron(text: &str) -> Result<Self, String> {
        let raw = RawValue::from_ron(text).map_err(|error| error.to_string())?;
        Ok(Self::raw(raw))
    }

    /// The value as `raw` holds it, untyped.
    fn raw(raw: &RawValue) -> Self {
        Self {
            ron: Ok(raw.trim().get_ron().to_owned()),
            typed: None,
        }
    }

    /// The RON text, or why the value could not be written.
    ///
    /// # Errors
    ///
    /// Why [`AppCheckValue::of`] could not write the value.
    pub fn ron(&self) -> Result<&str, &str> {
        self.ron.as_deref().map_err(String::as_str)
    }

    /// The value, once it is of the app's type: always for a value made in
    /// Rust, and for a value from a file once the script has loaded.
    #[must_use]
    pub fn value(&self) -> Option<&dyn Reflect> {
        self.typed.as_ref().map(|typed| &*typed.value)
    }

    /// Runs the check. The outer error means it cannot run at all.
    pub(super) fn run(&self, probe: Probe<'_>) -> Result<CheckResult, String> {
        let typed = self.typed.as_ref().ok_or_else(|| {
            format!(
                "the app check `{self}` was never read as the app's type; load the script with a `Registry`"
            )
        })?;
        let check = typed
            .check
            .get(&*typed.value)
            .ok_or_else(|| format!("`{self}` is not an `AppCheck`"))?;
        Ok(check.run(&probe))
    }

    /// Reads the value as the app's check type in `types`, unless it is of a
    /// type already.
    pub(super) fn type_against(&mut self, types: &TypeRegistry) -> Result<(), Problem> {
        if self.typed.is_some() {
            return Ok(());
        }
        let registration = registration(types)?;
        let text = self
            .ron
            .as_deref()
            .map_err(|reason| Problem::Other(reason.clone()))?;
        let type_name = registration.type_info().type_path_table().short_path();
        let bad = |message: String| Problem::BadAppCheck {
            written: text.to_owned(),
            type_name: type_name.to_owned(),
            message,
            variants: describe(registration)
                .variants
                .iter()
                .map(AppCheckVariant::form)
                .collect(),
        };
        let read = crate::script::ron_options()
            .from_str_seed(text, TypedReflectDeserializer::new(registration, types))
            .map_err(|error| bad(without_stack(&error.code.to_string()).to_owned()))?;
        let from = registration.data::<ReflectFromReflect>().ok_or_else(|| {
            Problem::Other(format!(
                "`{type_name}` cannot be built from reflection; keep `FromReflect`, which `#[derive(Reflect)]` gives"
            ))
        })?;
        // What the deserialiser accepts may still leave a field out.
        let value = from
            .from_reflect(read.as_ref())
            .ok_or_else(|| bad("a field is missing or does not fit".to_owned()))?;
        let check = registration
            .data::<ReflectAppCheck>()
            .cloned()
            .ok_or_else(|| Problem::Other(NO_APP_CHECK_TYPE.to_owned()))?;
        let ron = write(types, value.as_partial_reflect()).map_err(|error| {
            Problem::Other(format!("cannot write `{text}` back as RON: {error}"))
        })?;
        self.ron = Ok(ron);
        self.typed = Some(TypedApp {
            value: Arc::from(value),
            check,
        });
        Ok(())
    }
}

/// The RON text, or the reason it could not be written in angle brackets.
impl fmt::Display for AppCheckValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.ron {
            Ok(text) => f.write_str(text),
            Err(reason) => write!(f, "<{reason}>"),
        }
    }
}

/// Written as the bare RON value, so a file reads `App(StationNamed("A"))`.
impl Serialize for AppCheckValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let text = self.ron().map_err(serde::ser::Error::custom)?;
        RawValue::from_ron(text)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

/// Read as whatever RON value stands there, untouched; only RON can provide
/// one. It is typed when the script loads.
impl<'de> Deserialize<'de> for AppCheckValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        Ok(Self::raw(&raw))
    }
}

/// `message` without the ``(stack: `..`)`` Bevy's reflection adds in debug
/// builds, which names only the type the message names already.
fn without_stack(message: &str) -> &str {
    message
        .rfind(" (stack: ")
        .and_then(|at| message.get(..at))
        .unwrap_or(message)
}

/// `value` as compact RON, through reflection.
fn write(types: &TypeRegistry, value: &dyn PartialReflect) -> Result<String, ron::Error> {
    ron::to_string(&TypedReflectSerializer::new(value, types))
}

/// The one registered type that is an app check.
fn registration(types: &TypeRegistry) -> Result<&TypeRegistration, Problem> {
    let mut found: Vec<&TypeRegistration> = types
        .iter()
        .filter(|registration| registration.data::<ReflectAppCheck>().is_some())
        .collect();
    // Never hash order.
    found.sort_by_key(|registration| registration.type_info().type_path());
    match found.as_slice() {
        [] => Err(Problem::Other(NO_APP_CHECK_TYPE.to_owned())),
        [one] => Ok(one),
        several => Err(Problem::SeveralAppCheckTypes(
            several
                .iter()
                .map(|registration| registration.type_info().type_path().to_owned())
                .collect(),
        )),
    }
}

/// The app's check type, its variants and their fields, as the gallery
/// needs them to offer app checks with a form: see [`app_check_type`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppCheckType {
    /// The full type path, such as `rails::checks::RailCheck`.
    pub type_path: String,
    /// The short type path, such as `RailCheck`.
    pub short_type_path: String,
    /// The variants, in declaration order. Empty when the type is not an
    /// enum.
    pub variants: Vec<AppCheckVariant>,
}

/// One variant of the app's check type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppCheckVariant {
    /// The variant's name, as a file writes it.
    pub name: String,
    /// Whether the fields have names (`Joined(from: String, to: String)`)
    /// rather than positions (`StationNamed(String)`).
    pub named_fields: bool,
    /// The fields, in order; none for a unit variant.
    pub fields: Vec<AppCheckField>,
}

impl AppCheckVariant {
    /// The variant with the short types of its fields, as a file would
    /// write it: `TrackRaisedOnBridge`, `StationNamed(String)`,
    /// `Joined(from: String, to: String)`.
    #[must_use]
    pub fn form(&self) -> String {
        if self.fields.is_empty() {
            return self.name.clone();
        }
        let fields: Vec<String> = self
            .fields
            .iter()
            .map(|field| match (&field.name, self.named_fields) {
                (Some(name), true) => format!("{name}: {}", field.short_type_path),
                _ => field.short_type_path.clone(),
            })
            .collect();
        format!("{}({})", self.name, fields.join(", "))
    }
}

/// One field of a variant of the app's check type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppCheckField {
    /// The field's name; `None` for a field known by its position.
    pub name: Option<String>,
    /// The field type's full type path, such as `alloc::string::String`.
    pub type_path: String,
    /// The field type's short type path, such as `String`.
    pub short_type_path: String,
}

/// More than one registered type is an app check; the paths name them all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeveralAppCheckTypes(pub Vec<String>);

impl fmt::Display for SeveralAppCheckTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", several_message(&self.0))
    }
}

impl core::error::Error for SeveralAppCheckTypes {}

/// Why several app check types are a mistake, naming them.
pub(super) fn several_message(types: &[String]) -> String {
    let names: Vec<String> = types.iter().map(|name| format!("`{name}`")).collect();
    format!(
        "an app has one app check type, but {} carry `#[reflect(AppCheck)]`",
        names.join(" and ")
    )
}

/// The app's check type in `types`, with its variants and their field types,
/// or `None` when no type is an app check.
///
/// # Errors
///
/// [`SeveralAppCheckTypes`] when more than one type is.
pub fn app_check_type(types: &TypeRegistry) -> Result<Option<AppCheckType>, SeveralAppCheckTypes> {
    match registration(types) {
        Ok(registration) => Ok(Some(describe(registration))),
        Err(Problem::SeveralAppCheckTypes(several)) => Err(SeveralAppCheckTypes(several)),
        Err(_) => Ok(None),
    }
}

/// The type `registration` describes, as an [`AppCheckType`].
fn describe(registration: &TypeRegistration) -> AppCheckType {
    let info = registration.type_info();
    let variants = match info {
        TypeInfo::Enum(variants) => variants.iter().map(describe_variant).collect(),
        _ => Vec::new(),
    };
    AppCheckType {
        type_path: info.type_path().to_owned(),
        short_type_path: info.type_path_table().short_path().to_owned(),
        variants,
    }
}

/// One variant, with its fields.
fn describe_variant(variant: &VariantInfo) -> AppCheckVariant {
    let (named_fields, fields) = match variant {
        VariantInfo::Struct(fields) => (
            true,
            fields
                .iter()
                .map(|field| AppCheckField {
                    name: Some(field.name().to_owned()),
                    type_path: field.type_path().to_owned(),
                    short_type_path: field.ty().short_path().to_owned(),
                })
                .collect(),
        ),
        VariantInfo::Tuple(fields) => (
            false,
            fields
                .iter()
                .map(|field| AppCheckField {
                    name: None,
                    type_path: field.type_path().to_owned(),
                    short_type_path: field.ty().short_path().to_owned(),
                })
                .collect(),
        ),
        VariantInfo::Unit(_) => (false, Vec::new()),
    };
    AppCheckVariant {
        name: variant.name().to_owned(),
        named_fields,
        fields,
    }
}
