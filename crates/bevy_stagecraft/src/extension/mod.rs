//! The app's own setups, views, targets and checks, each one reflected enum.
//!
//! An app extends the crate at four points. For each it declares one enum
//! that derives `Reflect`, marks it with the matching attribute and
//! implements the matching trait:
//!
//! | Attribute                | Trait                       | What a value does                       |
//! | ------------------------ | --------------------------- | --------------------------------------- |
//! | `#[reflect(AppSetup)]`   | [`AppSetup`]                | builds the world a scenario starts from |
//! | `#[reflect(AppView)]`    | [`AppView`]                 | arranges how the world is shown         |
//! | `#[reflect(AppTarget)]`  | [`AppTarget`]               | finds where a step acts                 |
//! | `#[reflect(AppCheck)]`   | [`AppCheck`](crate::AppCheck) | asks something of the world           |
//!
//! Bevy's type registry finds each enum by itself: there is no registration
//! call, no name and no generic parameter. A script writes a value inside
//! `App(..)`:
//!
//! ```ron
//! setup: App(SavedWorld("two_stops_and_a_train.ron")),
//! view: App(TopDown),
//! steps: [
//!     MoveTo(App(Station("Alpha"))),
//!     Check(App(StationNamed("Alpha"))),
//! ],
//! ```
//!
//! The value is kept as the RON text written there (an [`AppValue`]) until
//! the script loads against a [`Registry`](crate::Registry), which reads it
//! as the app's type with Bevy's `TypedReflectDeserializer`. So a misspelt
//! variant or a wrong argument is a load error naming where it is and listing
//! the variants, not a failure part way through a run. Written back, a value
//! is the RON text `TypedReflectSerializer` gives.
//!
//! One type per extension point and app: a second type with the same
//! attribute is a load error naming both.

mod setup;
mod target;
mod view;

use alloc::{borrow::Cow, sync::Arc};
use core::fmt;

use std::path::Path;

use bevy::{
    ecs::{reflect::AppTypeRegistry, world::World},
    math::Vec2,
    reflect::{
        CreateTypeData, GetTypeRegistration, PartialReflect, Reflect, ReflectDeserialize,
        ReflectFromReflect, ReflectMut, ReflectSerialize, TypeData, TypeInfo, TypePath,
        TypeRegistration, TypeRegistry,
        enums::VariantInfo,
        serde::{TypedReflectDeserializer, TypedReflectSerializer},
    },
};
use ron::value::RawValue;
use serde::{Deserialize, Serialize};

pub use self::{
    setup::{AppSetup, ReflectAppSetup, Setup},
    target::{AppTarget, ReflectAppTarget},
    view::{AppView, ReflectAppView, View},
};
use crate::{
    check::{Problem, ReflectAppCheck},
    targets::Target,
};

/// One of the four places an app extends the crate with an enum of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Extension {
    /// The world a scenario starts from: [`AppSetup`].
    Setup,
    /// How the world is shown: [`AppView`].
    View,
    /// Where a step acts: [`AppTarget`].
    Target,
    /// What a check asks: [`AppCheck`](crate::AppCheck).
    Check,
}

impl Extension {
    /// The trait the app implements, which is also the attribute's name:
    /// `AppSetup` for `#[reflect(AppSetup)]`.
    #[must_use]
    pub const fn trait_name(self) -> &'static str {
        match self {
            Self::Setup => "AppSetup",
            Self::View => "AppView",
            Self::Target => "AppTarget",
            Self::Check => "AppCheck",
        }
    }

    /// What one value is called in a message, such as `app setup`.
    #[must_use]
    pub const fn noun(self) -> &'static str {
        match self {
            Self::Setup => "app setup",
            Self::View => "app view",
            Self::Target => "app target",
            Self::Check => "app check",
        }
    }
}

/// The extension point in words, such as `app setup`.
impl fmt::Display for Extension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.noun())
    }
}

/// A marker for one extension point, as the type parameter of [`AppValue`]:
/// [`SetupPoint`], [`ViewPoint`], [`TargetPoint`] or [`CheckPoint`].
pub trait ExtensionPoint: TypePath + Send + Sync + 'static {
    /// The type data the attribute registers, such as [`ReflectAppSetup`].
    type Data: TypeData + Clone;
    /// Which extension point this is.
    const EXTENSION: Extension;
}

/// The marker of [`AppSetup`] values.
#[derive(Debug, Clone, Copy, TypePath)]
pub struct SetupPoint;

impl ExtensionPoint for SetupPoint {
    type Data = ReflectAppSetup;
    const EXTENSION: Extension = Extension::Setup;
}

/// The marker of [`AppView`] values.
#[derive(Debug, Clone, Copy, TypePath)]
pub struct ViewPoint;

impl ExtensionPoint for ViewPoint {
    type Data = ReflectAppView;
    const EXTENSION: Extension = Extension::View;
}

/// The marker of [`AppTarget`] values.
#[derive(Debug, Clone, Copy, TypePath)]
pub struct TargetPoint;

impl ExtensionPoint for TargetPoint {
    type Data = ReflectAppTarget;
    const EXTENSION: Extension = Extension::Target;
}

/// The marker of [`AppCheck`](crate::AppCheck) values.
#[derive(Debug, Clone, Copy, TypePath)]
pub struct CheckPoint;

impl ExtensionPoint for CheckPoint {
    type Data = ReflectAppCheck;
    const EXTENSION: Extension = Extension::Check;
}

/// The value inside a [`Setup::App`]: one of the app's setups.
pub type AppSetupValue = AppValue<SetupPoint>;
/// The value inside a [`View::App`]: one of the app's views.
pub type AppViewValue = AppValue<ViewPoint>;
/// The value inside a [`Target::App`]: one of the app's targets.
pub type AppTargetValue = AppValue<TargetPoint>;
/// The value inside a [`Check::App`](crate::Check::App): one of the app's
/// checks.
pub type AppCheckValue = AppValue<CheckPoint>;

/// A value of the app's own type for one extension point `K`.
///
/// Read from a file, it is kept as the RON text written there until the
/// script loads against a [`Registry`](crate::Registry), which reads it as
/// the app's type with [`TypedReflectDeserializer`]. Written to a file, it is
/// the RON text [`TypedReflectSerializer`] gives. Two values are equal when
/// their RON text is.
///
/// It is `Reflect` (as an opaque value, read and written through its RON
/// text), so a variant of the app's check type can take a [`Target`] that
/// holds one.
#[derive(Reflect)]
#[reflect(opaque)]
#[reflect(Clone, Debug, Serialize, Deserialize)]
pub struct AppValue<K: ExtensionPoint> {
    /// The value as RON: as written in the file until it is typed, then as
    /// its type writes it; or why it could not be written.
    ron: Result<String, String>,
    /// The value itself, once its type is known.
    typed: Option<Typed<K>>,
}

/// A value of a known type.
struct Typed<K: ExtensionPoint> {
    /// The value, of the app's own type.
    value: Arc<dyn Reflect>,
    /// The type data that sees it as the extension point's trait.
    data: K::Data,
}

// By hand: a derive would ask `K: Clone`, which a marker need not be.
impl<K: ExtensionPoint> Clone for Typed<K> {
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
            data: self.data.clone(),
        }
    }
}

impl<K: ExtensionPoint> Clone for AppValue<K> {
    fn clone(&self) -> Self {
        Self {
            ron: self.ron.clone(),
            typed: self.typed.clone(),
        }
    }
}

/// The RON text, as a file writes it, so a target in a message reads
/// `App(Station("Alpha"))`.
impl<K: ExtensionPoint> fmt::Debug for AppValue<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

// The RON text alone decides equality: an app has one type per extension
// point, so two values with the same text are the same value of it, typed
// or not yet typed.
impl<K: ExtensionPoint> PartialEq for AppValue<K> {
    fn eq(&self, other: &Self) -> bool {
        self.ron == other.ron
    }
}

impl<K: ExtensionPoint> AppValue<K> {
    /// `value`, written as RON through its reflected form: the form a file
    /// holds and is read back with.
    ///
    /// A value reflection cannot write (a field whose type lacks
    /// serialisation) still works, but exporting it is an error.
    #[must_use]
    pub fn of<T>(value: T) -> Self
    where
        T: Reflect + GetTypeRegistration + TypePath,
        K::Data: CreateTypeData<T>,
    {
        let mut types = TypeRegistry::new();
        types.register::<T>();
        let ron = write(&types, value.as_partial_reflect())
            .map_err(|error| format!("cannot write {}: {error}", T::short_type_path()));
        Self {
            ron,
            typed: Some(Typed {
                value: Arc::new(value),
                data: <K::Data as CreateTypeData<T>>::create_type_data(()),
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
    /// Why [`AppValue::of`] could not write the value.
    pub fn ron(&self) -> Result<&str, &str> {
        self.ron.as_deref().map_err(String::as_str)
    }

    /// The value, once it is of the app's type: always for a value made in
    /// Rust, and for a value from a file once the script has loaded.
    #[must_use]
    pub fn value(&self) -> Option<&dyn Reflect> {
        self.typed.as_ref().map(|typed| &*typed.value)
    }

    /// The value and the type data that sees it as the extension point's
    /// trait, once it is typed.
    pub(crate) fn typed_parts(&self) -> Option<(&dyn Reflect, &K::Data)> {
        self.typed
            .as_ref()
            .map(|typed| (&*typed.value, &typed.data))
    }

    /// A value of the app's type, already built, written as RON through
    /// `types`.
    pub(crate) fn from_typed(types: &TypeRegistry, value: Box<dyn Reflect>, data: K::Data) -> Self {
        let ron = write(types, value.as_partial_reflect())
            .map_err(|error| format!("cannot write {}: {error}", value.reflect_short_type_path()));
        Self {
            ron,
            typed: Some(Typed {
                value: Arc::from(value),
                data,
            }),
        }
    }

    /// This value, typed against the type registry of `world`'s app unless
    /// it is typed already. A value from a script that loaded is typed; one
    /// made with [`AppValue::from_ron`] is typed here, when it is used.
    pub(crate) fn typed_in(&self, world: &World) -> Result<Cow<'_, Self>, String> {
        if self.typed.is_some() {
            return Ok(Cow::Borrowed(self));
        }
        let registry = world.get_resource::<AppTypeRegistry>().ok_or_else(|| {
            format!(
                "the {} `{self}` cannot be read: the app has no type registry",
                K::EXTENSION
            )
        })?;
        let mut typed = self.clone();
        typed
            .type_against(&registry.read())
            .map_err(|problem| problem.message())?;
        Ok(Cow::Owned(typed))
    }

    /// Reads the value as the app's type for this extension point in
    /// `types`, unless it is of a type already, and every [`Target`] inside
    /// it as the app's target type.
    pub(crate) fn type_against(&mut self, types: &TypeRegistry) -> Result<(), Problem> {
        if self.typed.is_some() {
            return Ok(());
        }
        let extension = K::EXTENSION;
        let registration = registration::<K>(types)?;
        let text = self
            .ron
            .as_deref()
            .map_err(|reason| Problem::Other(reason.clone()))?;
        let type_name = registration.type_info().type_path_table().short_path();
        let bad = |message: String| Problem::BadAppValue {
            extension,
            written: text.to_owned(),
            type_name,
            message,
            variants: describe(registration)
                .variants
                .iter()
                .map(AppVariant::form)
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
        let mut value = from
            .from_reflect(read.as_ref())
            .ok_or_else(|| bad("a field is missing or does not fit".to_owned()))?;
        type_targets_within(value.as_partial_reflect_mut(), types)?;
        let data = registration
            .data::<K::Data>()
            .cloned()
            .ok_or(Problem::NoAppType(extension))?;
        let ron = write(types, value.as_partial_reflect()).map_err(|error| {
            Problem::Other(format!("cannot write `{text}` back as RON: {error}"))
        })?;
        self.ron = Ok(ron);
        self.typed = Some(Typed {
            value: Arc::from(value),
            data,
        });
        Ok(())
    }
}

/// The RON text, or the reason it could not be written in angle brackets.
impl<K: ExtensionPoint> fmt::Display for AppValue<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.ron {
            Ok(text) => f.write_str(text),
            Err(reason) => write!(f, "<{reason}>"),
        }
    }
}

/// Written as the bare RON value, so a file reads `App(Station("Alpha"))`.
impl<K: ExtensionPoint> Serialize for AppValue<K> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let text = self.ron().map_err(serde::ser::Error::custom)?;
        RawValue::from_ron(text)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

/// Read as whatever RON value stands there, untouched; only RON can provide
/// one. It is typed when the script loads.
impl<'de, K: ExtensionPoint> Deserialize<'de> for AppValue<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        Ok(Self::raw(&raw))
    }
}

/// Reads every [`Target`] at or inside `value` (a field of an app check, say)
/// as the app's target type, so a misspelt app target there is a load error
/// too.
fn type_targets_within(
    value: &mut dyn PartialReflect,
    types: &TypeRegistry,
) -> Result<(), Problem> {
    if let Some(target) = value.try_downcast_mut::<Target>() {
        return target.type_against(types);
    }
    match value.reflect_mut() {
        ReflectMut::Struct(fields) => {
            for index in 0..fields.field_len() {
                if let Some(field) = fields.field_at_mut(index) {
                    type_targets_within(field, types)?;
                }
            }
        }
        ReflectMut::TupleStruct(fields) => {
            for index in 0..fields.field_len() {
                if let Some(field) = fields.field_mut(index) {
                    type_targets_within(field, types)?;
                }
            }
        }
        ReflectMut::Tuple(fields) => {
            for index in 0..fields.field_len() {
                if let Some(field) = fields.field_mut(index) {
                    type_targets_within(field, types)?;
                }
            }
        }
        ReflectMut::Enum(variant) => {
            for index in 0..variant.field_len() {
                if let Some(field) = variant.field_at_mut(index) {
                    type_targets_within(field, types)?;
                }
            }
        }
        ReflectMut::List(items) => {
            for index in 0..items.len() {
                if let Some(item) = items.get_mut(index) {
                    type_targets_within(item, types)?;
                }
            }
        }
        ReflectMut::Array(items) => {
            for index in 0..items.len() {
                if let Some(item) = items.get_mut(index) {
                    type_targets_within(item, types)?;
                }
            }
        }
        // Map keys cannot change, and an app has no reason to key by target.
        _ => {}
    }
    Ok(())
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

/// The one registered type for the extension point `K`.
pub(crate) fn registration<K: ExtensionPoint>(
    types: &TypeRegistry,
) -> Result<&TypeRegistration, Problem> {
    let mut found: Vec<&TypeRegistration> = types
        .iter()
        .filter(|registration| registration.data::<K::Data>().is_some())
        .collect();
    // Never hash order.
    found.sort_by_key(|registration| registration.type_info().type_path());
    match found.as_slice() {
        [] => Err(Problem::NoAppType(K::EXTENSION)),
        [one] => Ok(one),
        several => Err(Problem::SeveralAppTypes {
            extension: K::EXTENSION,
            types: several
                .iter()
                .map(|registration| registration.type_info().type_path().to_owned())
                .collect(),
        }),
    }
}

/// The type data of the one registered type for the extension point `K` in
/// `world`'s app, if there is one.
///
/// # Errors
///
/// When more than one type is registered for it, naming them.
pub(crate) fn data_in<K: ExtensionPoint>(world: &World) -> Result<Option<K::Data>, String> {
    let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
        return Ok(None);
    };
    let types = registry.read();
    match registration::<K>(&types) {
        Ok(registration) => Ok(registration.data::<K::Data>().cloned()),
        Err(Problem::NoAppType(_)) => Ok(None),
        Err(problem) => Err(problem.message()),
    }
}

/// What a file with a custom target by resolver name, as format versions 1
/// to 3 could hold, is told.
pub(crate) const NO_CUSTOM_TARGET: &str = "custom targets are no longer looked up by \
    resolver name: declare the app's targets as one enum with `#[derive(Reflect)]` and \
    `#[reflect(AppTarget)]`, and write one as `App(..)`, such as `MoveTo(App(Station(\"Alpha\")))`";

/// The setup the app's [`AppSetup::capture`] makes of `world` now, for a
/// recording that writes into `dir` under `stem`; `None` when no type is the
/// app's setup type or the app captures none.
pub(crate) fn capture_setup(
    world: &mut World,
    dir: &Path,
    stem: &str,
) -> Result<Option<Setup>, String> {
    let Some(data) = data_in::<SetupPoint>(world)? else {
        return Ok(None);
    };
    let Some(captured) = data.capture(world, dir, stem)? else {
        return Ok(None);
    };
    let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
        return Ok(None);
    };
    let value = AppValue::from_typed(&registry.read(), captured, data);
    Ok(Some(Setup::App(value)))
}

/// The view the app's [`AppView::current`] names for `world` now; `None`
/// when no type is the app's view type or the app names none.
pub(crate) fn current_view(world: &World) -> Result<Option<View>, String> {
    let Some(data) = data_in::<ViewPoint>(world)? else {
        return Ok(None);
    };
    let (Some(current), Some(registry)) =
        (data.current(world), world.get_resource::<AppTypeRegistry>())
    else {
        return Ok(None);
    };
    let value = AppValue::from_typed(&registry.read(), current, data);
    Ok(Some(View::App(value)))
}

/// What the app's [`AppTarget::at`] makes of `position` in `world`; `None`
/// when no type, or more than one, is the app's target type, or the app has
/// no target there.
pub(crate) fn app_target_at(world: &World, position: Vec2) -> Option<Target> {
    let data = data_in::<TargetPoint>(world).ok()??;
    let found = data.at(world, position)?;
    let registry = world.get_resource::<AppTypeRegistry>()?;
    let value = AppValue::from_typed(&registry.read(), found, data);
    Some(Target::App(value))
}

/// What a value that cannot be read as the app's type is told when no type
/// is registered for the extension point: how to declare one.
pub(crate) fn no_app_type(extension: Extension) -> String {
    let name = extension.trait_name();
    format!(
        "no type is an {extension}; derive `Reflect` on an enum, add `#[reflect({name})]` and \
         implement `{name}` for it"
    )
}

/// Why several types for one extension point are a mistake, naming them.
pub(crate) fn several_message(extension: Extension, types: &[String]) -> String {
    let names: Vec<String> = types.iter().map(|name| format!("`{name}`")).collect();
    format!(
        "an app has one {extension} type, but {} carry `#[reflect({})]`",
        names.join(" and "),
        extension.trait_name()
    )
}

/// Why a value does not read as the app's type, with the forms it can take.
pub(crate) fn bad_app_value(
    written: &str,
    type_name: &str,
    message: &str,
    variants: &[String],
) -> String {
    let forms: Vec<String> = variants.iter().map(|form| format!("`{form}`")).collect();
    format!(
        "`{written}` is not a `{type_name}`: {message}; `{type_name}` is one of {}",
        forms.join(", ")
    )
}

/// The app's type for one extension point, its variants and their fields,
/// as the gallery needs them to offer values with a form: see [`app_type`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppType {
    /// The full type path, such as `rails::checks::RailCheck`.
    pub type_path: String,
    /// The short type path, such as `RailCheck`.
    pub short_type_path: String,
    /// The variants, in declaration order. Empty when the type is not an
    /// enum.
    pub variants: Vec<AppVariant>,
}

/// One variant of an app's type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppVariant {
    /// The variant's name, as a file writes it.
    pub name: String,
    /// Whether the fields have names (`Joined(from: String, to: String)`)
    /// rather than positions (`StationNamed(String)`).
    pub named_fields: bool,
    /// The fields, in order; none for a unit variant.
    pub fields: Vec<AppField>,
}

impl AppVariant {
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

/// One field of a variant of an app's type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppField {
    /// The field's name; `None` for a field known by its position.
    pub name: Option<String>,
    /// The field type's full type path, such as `alloc::string::String`.
    pub type_path: String,
    /// The field type's short type path, such as `String`.
    pub short_type_path: String,
}

/// More than one registered type is the app's type for one extension point;
/// the paths name them all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeveralAppTypes {
    /// The extension point.
    pub extension: Extension,
    /// The full type paths, in order.
    pub types: Vec<String>,
}

impl fmt::Display for SeveralAppTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&several_message(self.extension, &self.types))
    }
}

impl core::error::Error for SeveralAppTypes {}

/// The app's type for `extension` in `types`, with its variants and their
/// field types, or `None` when no type carries the attribute.
///
/// # Errors
///
/// [`SeveralAppTypes`] when more than one type does.
pub fn app_type(
    types: &TypeRegistry,
    extension: Extension,
) -> Result<Option<AppType>, SeveralAppTypes> {
    /// The same for one marker.
    fn of<K: ExtensionPoint>(types: &TypeRegistry) -> Result<Option<AppType>, SeveralAppTypes> {
        match registration::<K>(types) {
            Ok(registration) => Ok(Some(describe(registration))),
            Err(Problem::SeveralAppTypes { extension, types }) => {
                Err(SeveralAppTypes { extension, types })
            }
            Err(_) => Ok(None),
        }
    }
    match extension {
        Extension::Setup => of::<SetupPoint>(types),
        Extension::View => of::<ViewPoint>(types),
        Extension::Target => of::<TargetPoint>(types),
        Extension::Check => of::<CheckPoint>(types),
    }
}

/// The type `registration` describes, as an [`AppType`].
fn describe(registration: &TypeRegistration) -> AppType {
    let info = registration.type_info();
    let variants = match info {
        TypeInfo::Enum(variants) => variants.iter().map(describe_variant).collect(),
        _ => Vec::new(),
    };
    AppType {
        type_path: info.type_path().to_owned(),
        short_type_path: info.type_path_table().short_path().to_owned(),
        variants,
    }
}

/// One variant, with its fields.
fn describe_variant(variant: &VariantInfo) -> AppVariant {
    let (named_fields, fields) = match variant {
        VariantInfo::Struct(fields) => (
            true,
            fields
                .iter()
                .map(|field| AppField {
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
                .map(|field| AppField {
                    name: None,
                    type_path: field.type_path().to_owned(),
                    short_type_path: field.ty().short_path().to_owned(),
                })
                .collect(),
        ),
        VariantInfo::Unit(_) => (false, Vec::new()),
    };
    AppVariant {
        name: variant.name().to_owned(),
        named_fields,
        fields,
    }
}
