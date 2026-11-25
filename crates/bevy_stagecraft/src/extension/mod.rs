//! The app's own setups, views, targets and checks: one reflected enum each, marked with
//! `#[reflect(AppSetup)]`, `AppView`, `AppTarget` or `AppCheck`.
//!
//! Bevy's type registry finds each enum: no registration call, no name. A second type for
//! one extension point is a load error naming both.
//!
//! A script writes a value inside `App(..)`. It is kept as RON text (an [`AppValue`]) until
//! the script loads against a [`Registry`](crate::Registry), so a misspelled variant is a
//! load error, not a failure part way through a run.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Extension {
    Setup,
    View,
    Target,
    Check,
}

impl Extension {
    #[must_use]
    pub const fn trait_name(self) -> &'static str {
        match self {
            Self::Setup => "AppSetup",
            Self::View => "AppView",
            Self::Target => "AppTarget",
            Self::Check => "AppCheck",
        }
    }

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

impl fmt::Display for Extension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.noun())
    }
}

pub trait ExtensionPoint: TypePath + Send + Sync + 'static {
    type Data: TypeData + Clone;
    const EXTENSION: Extension;
}

#[derive(Debug, Clone, Copy, TypePath)]
pub struct SetupPoint;

impl ExtensionPoint for SetupPoint {
    type Data = ReflectAppSetup;
    const EXTENSION: Extension = Extension::Setup;
}

#[derive(Debug, Clone, Copy, TypePath)]
pub struct ViewPoint;

impl ExtensionPoint for ViewPoint {
    type Data = ReflectAppView;
    const EXTENSION: Extension = Extension::View;
}

#[derive(Debug, Clone, Copy, TypePath)]
pub struct TargetPoint;

impl ExtensionPoint for TargetPoint {
    type Data = ReflectAppTarget;
    const EXTENSION: Extension = Extension::Target;
}

#[derive(Debug, Clone, Copy, TypePath)]
pub struct CheckPoint;

impl ExtensionPoint for CheckPoint {
    type Data = ReflectAppCheck;
    const EXTENSION: Extension = Extension::Check;
}

pub type AppSetupValue = AppValue<SetupPoint>;
pub type AppViewValue = AppValue<ViewPoint>;
pub type AppTargetValue = AppValue<TargetPoint>;
pub type AppCheckValue = AppValue<CheckPoint>;

/// A value of the app's own type for one extension point `K`, kept as RON text until the
/// script loads.
#[derive(Reflect)]
#[reflect(opaque)]
#[reflect(Clone, Debug, Serialize, Deserialize)]
pub struct AppValue<K: ExtensionPoint> {
    ron: Result<String, String>,
    typed: Option<Typed<K>>,
}

struct Typed<K: ExtensionPoint> {
    value: Arc<dyn Reflect>,
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

impl<K: ExtensionPoint> fmt::Debug for AppValue<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

// The RON text alone decides equality: an app has one type per extension point.
impl<K: ExtensionPoint> PartialEq for AppValue<K> {
    fn eq(&self, other: &Self) -> bool {
        self.ron == other.ron
    }
}

impl<K: ExtensionPoint> AppValue<K> {
    /// A value reflection cannot write (a field whose type lacks serialization) still works,
    /// but exporting it is an error.
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

    /// `text` must be one RON value; whitespace and comments around it are dropped.
    ///
    /// # Errors
    ///
    /// Why `text` is not one RON value.
    pub fn from_ron(text: &str) -> Result<Self, String> {
        let raw = RawValue::from_ron(text).map_err(|error| error.to_string())?;
        Ok(Self::raw(raw))
    }

    fn raw(raw: &RawValue) -> Self {
        Self {
            ron: Ok(raw.trim().get_ron().to_owned()),
            typed: None,
        }
    }

    /// # Errors
    ///
    /// Why [`AppValue::of`] could not write the value.
    pub fn ron(&self) -> Result<&str, &str> {
        self.ron.as_deref().map_err(String::as_str)
    }

    #[must_use]
    pub fn value(&self) -> Option<&dyn Reflect> {
        self.typed.as_ref().map(|typed| &*typed.value)
    }

    pub(crate) fn typed_parts(&self) -> Option<(&dyn Reflect, &K::Data)> {
        self.typed
            .as_ref()
            .map(|typed| (&*typed.value, &typed.data))
    }

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
        // What the deserializer accepts may still leave a field out.
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

impl<K: ExtensionPoint> fmt::Display for AppValue<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.ron {
            Ok(text) => f.write_str(text),
            Err(reason) => write!(f, "<{reason}>"),
        }
    }
}

impl<K: ExtensionPoint> Serialize for AppValue<K> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let text = self.ron().map_err(serde::ser::Error::custom)?;
        RawValue::from_ron(text)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de, K: ExtensionPoint> Deserialize<'de> for AppValue<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        Ok(Self::raw(&raw))
    }
}

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

/// Drops the ``(stack: `..`)`` Bevy's reflection adds in debug builds.
fn without_stack(message: &str) -> &str {
    message
        .rfind(" (stack: ")
        .and_then(|at| message.get(..at))
        .unwrap_or(message)
}

fn write(types: &TypeRegistry, value: &dyn PartialReflect) -> Result<String, ron::Error> {
    ron::to_string(&TypedReflectSerializer::new(value, types))
}

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

pub(crate) const NO_CUSTOM_TARGET: &str = "custom targets are no longer looked up by \
    resolver name: declare the app's targets as one enum with `#[derive(Reflect)]` and \
    `#[reflect(AppTarget)]`, and write one as `App(..)`, such as `MoveTo(App(Station(\"Alpha\")))`";

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

pub(crate) fn app_target_at(world: &World, position: Vec2) -> Option<Target> {
    let data = data_in::<TargetPoint>(world).ok()??;
    let found = data.at(world, position)?;
    let registry = world.get_resource::<AppTypeRegistry>()?;
    let value = AppValue::from_typed(&registry.read(), found, data);
    Some(Target::App(value))
}

pub(crate) fn no_app_type(extension: Extension) -> String {
    let name = extension.trait_name();
    format!(
        "no type is an {extension}; derive `Reflect` on an enum, add `#[reflect({name})]` and \
         implement `{name}` for it"
    )
}

pub(crate) fn several_message(extension: Extension, types: &[String]) -> String {
    let names: Vec<String> = types.iter().map(|name| format!("`{name}`")).collect();
    format!(
        "an app has one {extension} type, but {} carry `#[reflect({})]`",
        names.join(" and "),
        extension.trait_name()
    )
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppType {
    pub type_path: String,
    pub short_type_path: String,
    pub variants: Vec<AppVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppVariant {
    pub name: String,
    pub named_fields: bool,
    pub fields: Vec<AppField>,
}

impl AppVariant {
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppField {
    pub name: Option<String>,
    pub type_path: String,
    pub short_type_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeveralAppTypes {
    pub extension: Extension,
    pub types: Vec<String>,
}

impl fmt::Display for SeveralAppTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&several_message(self.extension, &self.types))
    }
}

impl core::error::Error for SeveralAppTypes {}

/// # Errors
///
/// [`SeveralAppTypes`] when more than one type does.
pub fn app_type(
    types: &TypeRegistry,
    extension: Extension,
) -> Result<Option<AppType>, SeveralAppTypes> {
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
