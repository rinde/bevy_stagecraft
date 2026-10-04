//! Finding types, fields and values through Bevy's type registry.
//!
//! A script names types by their short type path (`Editor`, `State<Mode>`)
//! and fields by a reflect path after it (`Editor.tool`). The same lookups
//! serve two moments: loading, where only type information is at hand and a
//! mistake is a load error, and running, where the world supplies the values.

use core::{any::TypeId, cmp::Ordering, fmt};

use bevy::{
    ecs::{
        archetype::ArchetypeEntity,
        component::ComponentId,
        entity::Entity,
        entity_disabling::DefaultQueryFilters,
        reflect::{AppTypeRegistry, ReflectComponent, ReflectResource},
        world::World,
    },
    reflect::{
        Access, NamedField, ParsedPath, PartialReflect, Reflect, ReflectFromReflect, TypeInfo,
        TypeRegistration, TypeRegistry, UnnamedField,
        enums::VariantInfo,
        serde::{TypedReflectDeserializer, TypedReflectSerializer},
    },
    state::reflect::ReflectState,
};

use super::{
    Problem,
    value::{Comparison, Counted, Measure, Value},
};
use crate::targets::is_tooling;

/// At most this many close names are suggested for a misspelt one.
const MAX_SUGGESTIONS: usize = 3;

/// A name this many edits or fewer from the one written counts as close,
/// however short the name.
const MIN_CLOSE_EDITS: usize = 2;

/// Beyond [`MIN_CLOSE_EDITS`], one edit per this many characters counts as
/// close, so longer names may be further off.
const CHARACTERS_PER_EDIT: usize = 3;

/// What a check needs a type for, which decides the type data it must have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    /// Any registered type: a component to count.
    Counted,
    /// A reflected resource (`#[reflect(Resource)]`).
    Resource,
    /// A reflected component (`#[reflect(Component)]`).
    Component,
    /// A reflected state (`register_type_state`).
    State,
}

impl Role {
    /// Whether `registration` can play this role.
    fn fits(self, registration: &TypeRegistration) -> bool {
        match self {
            Self::Counted => true,
            Self::Resource => {
                registration.data::<ReflectResource>().is_some()
                    && registration.data::<ReflectComponent>().is_some()
            }
            Self::Component => registration.data::<ReflectComponent>().is_some(),
            Self::State => registration.data::<ReflectState>().is_some(),
        }
    }

    /// Why a registered type cannot play this role, and how to fix it.
    fn mismatch(self, name: &str) -> String {
        match self {
            Self::Counted => format!("`{name}` cannot be counted"),
            Self::Resource => {
                format!("`{name}` is not a reflected resource; add `#[reflect(Resource)]`")
            }
            Self::Component => {
                format!("`{name}` is not a reflected component; add `#[reflect(Component)]`")
            }
            Self::State => format!(
                "`{name}` is not a reflected state; register it with `app.register_type_state::<{name}>()`"
            ),
        }
    }
}

/// The registered type called `name` (its short type path, or its full one)
/// that can play `role`.
pub(super) fn find_type<'r>(
    types: &'r TypeRegistry,
    name: &str,
    role: Role,
) -> Result<&'r TypeRegistration, Problem> {
    let found = types
        .get_with_short_type_path(name)
        .or_else(|| types.get_with_type_path(name));
    match found {
        Some(registration) if role.fits(registration) => Ok(registration),
        Some(_) => Err(Problem::Other(role.mismatch(name))),
        None if types.is_ambiguous(name) => Err(Problem::Other(format!(
            "`{name}` names more than one registered type; write its full type path"
        ))),
        None => Err(Problem::UnknownType {
            name: name.to_owned(),
            suggestions: close_names(
                name,
                types
                    .iter()
                    .filter(|registration| role.fits(registration))
                    .map(|registration| registration.type_info().type_path_table().short_path()),
            ),
        }),
    }
}

/// The candidates closest to `wanted`, closest first, ties by name: never
/// hash order.
pub(super) fn close_names<'a>(
    wanted: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let wanted_lower = wanted.to_lowercase();
    let limit = (wanted.chars().count() / CHARACTERS_PER_EDIT).max(MIN_CLOSE_EDITS);
    let mut scored: Vec<(usize, &str)> = candidates
        .into_iter()
        .map(|candidate| {
            (
                edit_distance(&wanted_lower, &candidate.to_lowercase()),
                candidate,
            )
        })
        .filter(|(distance, _)| *distance <= limit)
        .collect();
    scored.sort_unstable();
    scored.dedup();
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, name)| name.to_owned())
        .collect()
}

/// How many single-character insertions, deletions and substitutions turn
/// `a` into `b`.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (row, from) in a.chars().enumerate() {
        let mut current = Vec::with_capacity(b.len() + 1);
        current.push(row + 1);
        for (column, to) in b.iter().enumerate() {
            let substitute = previous.get(column).copied().unwrap_or(0) + usize::from(from != *to);
            let delete = previous.get(column + 1).copied().unwrap_or(0) + 1;
            let insert = current.last().copied().unwrap_or(0) + 1;
            current.push(substitute.min(delete).min(insert));
        }
        previous = current;
    }
    previous.last().copied().unwrap_or(0)
}

/// Splits `Editor.tool` into the type, `Editor`, and the reflect path after
/// it, `.tool`. Dots inside generics (`State<Mode>`) belong to the type.
pub(super) fn split_path(full: &str) -> (&str, &str) {
    let mut depth = 0_usize;
    for (at, character) in full.char_indices() {
        match character {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            '.' | '[' | '#' if depth == 0 => {
                return full.split_at_checked(at).unwrap_or((full, ""));
            }
            _ => {}
        }
    }
    (full, "")
}

/// The type that `path` leads to from `root`, found from type information
/// alone, so it works before any value exists.
pub(super) fn field_type<'r>(
    types: &'r TypeRegistry,
    root: &'r TypeRegistration,
    full: &str,
    path: &str,
) -> Result<&'r TypeRegistration, Problem> {
    let unknown = |message: String, suggestions: Vec<String>| Problem::UnknownPath {
        path: full.to_owned(),
        message,
        suggestions,
    };
    if path.is_empty() {
        return Ok(root);
    }
    let parsed = ParsedPath::parse(path)
        .map_err(|error| unknown(format!("`{full}` is not a field path: {error}"), Vec::new()))?;
    let mut current = root;
    for step in &parsed.0 {
        let info = current.type_info();
        let name = info.type_path_table().short_path();
        let next = match (&step.access, info) {
            (Access::Field(field), TypeInfo::Struct(fields)) => {
                let Some(found) = fields.field(field) else {
                    return Err(unknown(
                        format!("`{name}` has no field `{field}`"),
                        close_names(field, fields.field_names().iter().copied()),
                    ));
                };
                Some(found.type_id())
            }
            (Access::FieldIndex(index), TypeInfo::Struct(fields)) => {
                fields.field_at(*index).map(NamedField::type_id)
            }
            (Access::TupleIndex(index), TypeInfo::TupleStruct(fields)) => {
                fields.field_at(*index).map(UnnamedField::type_id)
            }
            (Access::TupleIndex(index), TypeInfo::Tuple(fields)) => {
                fields.field_at(*index).map(UnnamedField::type_id)
            }
            (Access::ListIndex(_), TypeInfo::List(list)) => Some(list.item_ty().id()),
            (Access::ListIndex(_), TypeInfo::Array(array)) => Some(array.item_ty().id()),
            (access, TypeInfo::Enum(variants)) => variants
                .iter()
                .find_map(|variant| variant_field(variant, access)),
            _ => None,
        };
        let Some(next) = next else {
            return Err(unknown(
                format!("`{name}` has no `{}`", step.access),
                Vec::new(),
            ));
        };
        current = types.get(next).ok_or_else(|| {
            Problem::Other(format!(
                "the type of `{}` in `{name}` is not registered",
                step.access
            ))
        })?;
    }
    Ok(current)
}

/// The type of the field `access` names in `variant`, if it has one.
fn variant_field(variant: &VariantInfo, access: &Access<'_>) -> Option<TypeId> {
    match (access, variant) {
        (Access::Field(field), VariantInfo::Struct(fields)) => {
            fields.field(field).map(NamedField::type_id)
        }
        (Access::FieldIndex(index), VariantInfo::Struct(fields)) => {
            fields.field_at(*index).map(NamedField::type_id)
        }
        (Access::TupleIndex(index), VariantInfo::Tuple(fields)) => {
            fields.field_at(*index).map(UnnamedField::type_id)
        }
        _ => None,
    }
}

/// Reads `value` as a value of the type `registration` describes, the way
/// Bevy reads scenes: with [`TypedReflectDeserializer`]. The result is the
/// concrete type where the type can be built from reflection.
pub(super) fn read_value(
    types: &TypeRegistry,
    registration: &TypeRegistration,
    value: &Value,
) -> Result<Box<dyn PartialReflect>, Problem> {
    let bad = |message: String| Problem::BadValue {
        value: value.to_string(),
        message,
    };
    let text = value.ron().map_err(|reason| bad(reason.to_owned()))?;
    let read = crate::script::ron_options()
        .from_str_seed(text, TypedReflectDeserializer::new(registration, types))
        .map_err(|error| {
            bad(format!(
                "expected a `{}`: {}",
                registration.type_info().type_path_table().short_path(),
                error.code
            ))
        })?;
    Ok(registration
        .data::<ReflectFromReflect>()
        .and_then(|from| from.from_reflect(read.as_ref()))
        .map_or(read, |concrete| concrete.into_partial_reflect()))
}

/// Checks, before anything runs, that `comparison`'s values fit the type
/// `registration` describes and, if it orders them, that they have an order.
pub(super) fn check_comparison(
    types: &TypeRegistry,
    registration: &TypeRegistration,
    comparison: &Comparison,
) -> Result<(), Problem> {
    for value in comparison.values() {
        let read = read_value(types, registration, value)?;
        if comparison.orders() && order(read.as_ref(), read.as_ref()).is_none() {
            return Err(Problem::Other(format!(
                "`{}` values have no order; compare them with `Eq` or `Ne`",
                registration.type_info().type_path_table().short_path()
            )));
        }
    }
    Ok(())
}

/// Checks a count's values before anything runs.
pub(super) fn check_count(comparison: &Comparison) -> Result<(), Problem> {
    for value in comparison.values() {
        Counted::read(value).map_err(|message| Problem::BadValue {
            value: value.to_string(),
            message,
        })?;
    }
    Ok(())
}

/// How two reflected values order, for the types that have one: numbers and
/// text.
fn order(a: &dyn PartialReflect, b: &dyn PartialReflect) -> Option<Ordering> {
    /// Tries each type in turn; the first both values are decides.
    macro_rules! order_as {
        ($($kind:ty),* $(,)?) => {
            $(
                if let (Some(a), Some(b)) = (a.try_downcast_ref::<$kind>(), b.try_downcast_ref::<$kind>()) {
                    return a.partial_cmp(b);
                }
            )*
        };
    }
    order_as!(
        u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64, String, char,
        bool
    );
    None
}

/// A reflected field, measured against values read as its own type.
pub(super) struct Reflected<'a> {
    /// The field.
    pub(super) value: &'a dyn PartialReflect,
    /// The field's type.
    pub(super) registration: &'a TypeRegistration,
    /// Where the field's type and its parts are registered.
    pub(super) types: &'a TypeRegistry,
}

// By hand: `TypeRegistry` is not `Debug`, and listing every type in it would
// say nothing about the field.
impl fmt::Debug for Reflected<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reflected")
            .field("value", &self.value)
            .field("registration", &self.registration)
            .finish_non_exhaustive()
    }
}

impl Reflected<'_> {
    /// Reads `value` as the field's type.
    fn read(&self, value: &Value) -> Result<Box<dyn PartialReflect>, String> {
        read_value(self.types, self.registration, value).map_err(|problem| problem.message())
    }

    /// The field's value as RON, for messages.
    pub(super) fn describe(&self) -> String {
        describe(self.types, self.value)
    }
}

impl Measure for Reflected<'_> {
    fn equals(&self, value: &Value) -> Result<bool, String> {
        let expected = self.read(value)?;
        self.value
            .reflect_partial_eq(expected.as_ref())
            .ok_or_else(|| {
                format!(
                    "`{}` values cannot be compared",
                    self.registration.type_info().type_path_table().short_path()
                )
            })
    }

    fn order(&self, value: &Value) -> Result<Ordering, String> {
        let expected = self.read(value)?;
        order(self.value, expected.as_ref()).ok_or_else(|| {
            format!(
                "`{}` values have no order",
                self.registration.type_info().type_path_table().short_path()
            )
        })
    }
}

/// `value` as RON, or as Rust's debug form when it cannot be written.
pub(super) fn describe(types: &TypeRegistry, value: &dyn PartialReflect) -> String {
    ron::to_string(&TypedReflectSerializer::new(value, types))
        .unwrap_or_else(|_| format!("{value:?}"))
}

/// The world's type registry, which every reflected check reads through.
pub(super) fn app_types(world: &World) -> Result<&AppTypeRegistry, String> {
    world
        .get_resource::<AppTypeRegistry>()
        .ok_or_else(|| "the world has no type registry (`AppTypeRegistry`)".to_owned())
}

/// The resource `registration` describes, if the world has one.
pub(super) fn resource<'w>(
    world: &'w World,
    registration: &TypeRegistration,
) -> Option<&'w dyn Reflect> {
    // Resources are components of their own entity.
    let reflect = registration.data::<ReflectComponent>()?;
    let id = world.components().get_id(registration.type_id())?;
    let entity = world.resource_entities().get(id)?;
    reflect.reflect(world.get_entity(entity).ok()?)
}

/// The component `registration` describes on `entity`, if it has one.
pub(super) fn component<'w>(
    world: &'w World,
    registration: &TypeRegistration,
    entity: Entity,
) -> Option<&'w dyn Reflect> {
    registration
        .data::<ReflectComponent>()?
        .reflect(world.get_entity(entity).ok()?)
}

/// The current value of the state `registration` describes, if the app is in
/// one.
pub(super) fn state<'w>(
    world: &'w World,
    registration: &TypeRegistration,
) -> Option<&'w dyn Reflect> {
    registration.data::<ReflectState>()?.reflect(world)
}

/// How many entities of the app have every component in `types`, leaving out
/// disabled entities, as queries do, and the tooling's own.
pub(super) fn count(world: &World, types: &[&TypeRegistration]) -> usize {
    let mut ids: Vec<ComponentId> = Vec::with_capacity(types.len());
    for registration in types {
        // A type never used as a component is on no entity.
        let Some(id) = world.components().get_id(registration.type_id()) else {
            return 0;
        };
        ids.push(id);
    }
    let disabling: Vec<ComponentId> = world
        .get_resource::<DefaultQueryFilters>()
        .map(|filters| filters.disabling_ids().collect())
        .unwrap_or_default();
    world
        .archetypes()
        .iter()
        .filter(|archetype| {
            ids.iter().all(|id| archetype.contains(*id))
                && !disabling.iter().any(|id| archetype.contains(*id))
        })
        .flat_map(|archetype| archetype.entities().iter().map(ArchetypeEntity::id))
        .filter(|entity| !is_tooling(world, *entity))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_names_come_closest_first_and_leave_out_distant_ones() {
        assert_eq!(
            close_names("Gost", ["Ghost", "Host", "Score", "Ghosts"]),
            ["Ghost", "Host", "Ghosts"]
        );
        assert!(close_names("Editor", ["Camera", "Window"]).is_empty());
    }

    #[test]
    fn a_path_splits_after_the_type_and_keeps_generics_whole() {
        assert_eq!(split_path("Editor.tool"), ("Editor", ".tool"));
        assert_eq!(split_path("Score"), ("Score", ""));
        assert_eq!(split_path("List[2]"), ("List", "[2]"));
        assert_eq!(split_path("Wrap<a.b>.0"), ("Wrap<a.b>", ".0"));
    }
}
