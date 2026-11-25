//! Finding types, fields and values through Bevy's type registry, both at
//! load (type information only) and at run (with values).

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
    math::{Vec2, Vec3},
    reflect::{
        Access, GetPath, NamedField, ParsedPath, PartialReflect, Reflect, ReflectFromReflect,
        TypeInfo, TypePathTable, TypeRegistration, TypeRegistry, UnnamedField,
        enums::VariantInfo,
        serde::{TypedReflectDeserializer, TypedReflectSerializer},
    },
    state::reflect::ReflectState,
    window::{PrimaryWindow, Window},
};

use crate::headless::DEFAULT_SCALE_FACTOR;

use super::{
    CapturedValue, Problem,
    value::{Comparable, Comparison, Counted, Unit, Value},
};
use crate::targets::is_tooling;

const MAX_SUGGESTIONS: usize = 3;

const MIN_CLOSE_EDITS: usize = 2;

const CHARACTERS_PER_EDIT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    Counted,
    Resource,
    Component,
    State,
}

impl Role {
    pub(super) fn fits(self, registration: &TypeRegistration) -> bool {
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
        None if types.is_ambiguous(name) => {
            let mut paths: Vec<&str> = types
                .iter()
                .map(|registration| registration.type_info().type_path_table())
                .filter(|table| table.short_path() == name)
                .map(TypePathTable::path)
                .collect();
            paths.sort_unstable();
            let paths: Vec<String> = paths.iter().map(|path| format!("`{path}`")).collect();
            Err(Problem::Other(format!(
                "`{name}` names more than one registered type ({}); write its full type path",
                paths.join(", ")
            )))
        }
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

/// Closest first, ties by name: never hash order.
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

/// Dots inside generics (`State<Mode>`) belong to the type.
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

pub(super) fn check_comparison(
    types: &TypeRegistry,
    registration: &TypeRegistration,
    comparison: &Comparison,
) -> Result<(), Problem> {
    for value in comparison.values() {
        // A captured value is known only once the scenario has run.
        if value.captured_name().is_some() {
            continue;
        }
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

pub(super) fn check_count(comparison: &Comparison) -> Result<(), Problem> {
    for value in comparison.values() {
        if value.captured_name().is_some() {
            continue;
        }
        Counted::read(value).map_err(|message| Problem::BadValue {
            value: value.to_string(),
            message,
        })?;
    }
    Ok(())
}

pub(super) fn field_value<'a>(
    root: &'a dyn Reflect,
    path: &str,
    field: &str,
) -> Result<&'a dyn PartialReflect, String> {
    if field.is_empty() {
        return Ok(root.as_partial_reflect());
    }
    root.reflect_path(field)
        .map_err(|error| format!("`{path}` cannot be read now: {error}"))
}

pub(super) fn capture_field(
    world: &World,
    types: &TypeRegistry,
    root: &dyn Reflect,
    path: &str,
    field: &str,
    unit: Unit,
) -> Result<CapturedValue, String> {
    let value = field_value(root, path, field)?;
    let type_name = value
        .get_represented_type_info()
        .map_or("unknown", |info| info.type_path_table().short_path());
    let scaled = unit
        .is_logical()
        .then(|| to_logical(world, value))
        .transpose()?;
    let value = scaled.as_deref().unwrap_or(value);
    let ron = ron::to_string(&TypedReflectSerializer::new(value, types))
        .map_err(|error| format!("`{path}` cannot be written as a value: {error}"))?;
    Ok(CapturedValue::new(ron, type_name.to_owned()))
}

/// The type is kept, so the value still compares and writes as RON.
pub(super) fn to_logical(
    world: &World,
    value: &dyn PartialReflect,
) -> Result<Box<dyn PartialReflect>, String> {
    let inverse = 1.0 / primary_scale_factor(world);
    if let Some(number) = value.try_downcast_ref::<f32>() {
        return Ok(Box::new(*number * inverse));
    }
    if let Some(number) = value.try_downcast_ref::<f64>() {
        return Ok(Box::new(*number * f64::from(inverse)));
    }
    if let Some(vector) = value.try_downcast_ref::<Vec2>() {
        return Ok(Box::new(*vector * inverse));
    }
    if let Some(vector) = value.try_downcast_ref::<Vec3>() {
        return Ok(Box::new(*vector * inverse));
    }
    Err(format!(
        "`{}` is not a number in physical pixels, so it cannot be read as logical",
        value.reflect_type_path()
    ))
}

pub(super) fn check_logical(registration: &TypeRegistration) -> Result<(), Problem> {
    if is_logical_number(registration.type_id()) {
        return Ok(());
    }
    Err(Problem::Other(format!(
        "`{}` is not a number in physical pixels, so `unit: Logical` does not apply",
        registration.type_info().type_path_table().short_path()
    )))
}

fn is_logical_number(id: TypeId) -> bool {
    [
        TypeId::of::<f32>(),
        TypeId::of::<f64>(),
        TypeId::of::<Vec2>(),
        TypeId::of::<Vec3>(),
    ]
    .contains(&id)
}

/// Falls back to [`DEFAULT_SCALE_FACTOR`] when there is no window.
fn primary_scale_factor(world: &World) -> f32 {
    world
        .iter_entities()
        .find_map(|entity| {
            entity
                .contains::<PrimaryWindow>()
                .then(|| entity.get::<Window>())
                .flatten()
        })
        .map_or(DEFAULT_SCALE_FACTOR, |window| {
            window.resolution.scale_factor()
        })
}

fn order(a: &dyn PartialReflect, b: &dyn PartialReflect) -> Option<Ordering> {
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

pub(super) struct Reflected<'a> {
    pub(super) value: &'a dyn PartialReflect,
    pub(super) registration: &'a TypeRegistration,
    pub(super) types: &'a TypeRegistry,
}

impl fmt::Debug for Reflected<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reflected")
            .field("value", &self.value)
            .field("registration", &self.registration)
            .finish_non_exhaustive()
    }
}

impl Reflected<'_> {
    fn read(&self, value: &Value) -> Result<Box<dyn PartialReflect>, String> {
        read_value(self.types, self.registration, value).map_err(|problem| problem.message())
    }

    pub(super) fn describe(&self) -> String {
        describe(self.types, self.value)
    }
}

impl Comparable for Reflected<'_> {
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

pub(super) fn describe(types: &TypeRegistry, value: &dyn PartialReflect) -> String {
    ron::to_string(&TypedReflectSerializer::new(value, types))
        .unwrap_or_else(|_| format!("{value:?}"))
}

pub(super) fn app_types(world: &World) -> Result<&AppTypeRegistry, String> {
    world
        .get_resource::<AppTypeRegistry>()
        .ok_or_else(|| "the world has no type registry (`AppTypeRegistry`)".to_owned())
}

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

pub(super) fn component<'w>(
    world: &'w World,
    registration: &TypeRegistration,
    entity: Entity,
) -> Option<&'w dyn Reflect> {
    registration
        .data::<ReflectComponent>()?
        .reflect(world.get_entity(entity).ok()?)
}

pub(super) fn state<'w>(
    world: &'w World,
    registration: &TypeRegistration,
) -> Option<&'w dyn Reflect> {
    registration.data::<ReflectState>()?.reflect(world)
}

/// Leaves out disabled entities, as queries do, and the tooling's own.
pub(super) fn count(world: &World, types: &[&TypeRegistration]) -> usize {
    let mut ids: Vec<ComponentId> = Vec::with_capacity(types.len());
    for registration in types {
        let Some(id) = world.components().get_id(registration.type_id()) else {
            return 0;
        };
        ids.push(id);
    }
    let disabling: Vec<ComponentId> = world
        .get_resource::<DefaultQueryFilters>()
        .map_or_default(|filters| filters.disabling_ids().collect());
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
        assert_eq!(close_names("Editor", ["Camera", "Window"]).len(), 0);
    }

    #[test]
    fn a_path_splits_after_the_type_and_keeps_generics_whole() {
        assert_eq!(split_path("Editor.tool"), ("Editor", ".tool"));
        assert_eq!(split_path("Score"), ("Score", ""));
        assert_eq!(split_path("List[2]"), ("List", "[2]"));
        assert_eq!(split_path("Wrap<a.b>.0"), ("Wrap<a.b>", ".0"));
    }
}
