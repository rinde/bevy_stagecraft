//! Checks and targets for one node picked by pointing at it, for the gallery's
//! Pick mode. Every list is in a fixed order, never hash order.

use alloc::collections::BTreeSet;

use bevy::{
    camera::visibility::InheritedVisibility,
    ecs::{entity::Entity, hierarchy::ChildOf, name::Name, reflect::AppTypeRegistry, world::World},
    math::Vec2,
    picking::Pickable,
    reflect::{TypeInfo, TypePath, TypeRegistration, TypeRegistry, std_traits::ReflectDefault},
    ui::{
        CalculatedClip, Checkable, ComputedNode, Selectable, UiGlobalTransform, UiStack,
        widget::Text,
    },
};

use super::{
    CapturedValues, Check, Comparison, TextMatch, Unit, Value,
    reflect::{self, Role},
    suggest::{NoiseFilter, flatten, holds, name_of, ron_of, widget_check},
    widget::{self, WidgetState},
};
use crate::{
    extension::{AppCheckValue, AppField, AppVariant, Extension, app_target_at, app_type},
    targets::{Lookup, Target, WidgetRole, is_tooling, locate},
};

const SCREEN_PRECISION: f32 = 10.0;

/// The app's topmost UI node at `position`, found as UI picking does; the
/// tooling ([`NotUnderTest`](crate::NotUnderTest)) never counts.
#[must_use]
pub fn node_at(world: &World, position: Vec2) -> Option<Entity> {
    let stack = world.get_resource::<UiStack>()?;
    stack
        .uinodes
        .iter()
        .rev()
        .copied()
        .find(|entity| is_hit(world, *entity, position))
}

fn is_hit(world: &World, entity: Entity, position: Vec2) -> bool {
    let (Some(node), Some(transform)) = (
        world.get::<ComputedNode>(entity),
        world.get::<UiGlobalTransform>(entity),
    ) else {
        return false;
    };
    if node.size() == Vec2::ZERO
        || world
            .get::<InheritedVisibility>(entity)
            .is_some_and(|visibility| !visibility.get())
        || world
            .get::<Pickable>(entity)
            .is_some_and(|pickable| !pickable.is_hoverable)
    {
        return false;
    }
    // Layout is in physical pixels; positions are logical.
    let point = position / node.inverse_scale_factor;
    node.contains_point(*transform, point)
        && world
            .get::<CalculatedClip>(entity)
            .is_none_or(|clip| clip.contains_point(point))
        && !is_tooling(world, entity)
}

#[must_use]
pub fn subject_of(world: &World, node: Entity) -> Entity {
    widget::widget(world, node)
}

/// Rounded as a recording rounds positions.
fn centre_of(world: &World, entity: Entity) -> Option<Vec2> {
    let node = world.get::<ComputedNode>(entity)?;
    let transform = world.get::<UiGlobalTransform>(entity)?;
    let centre = transform.affine().translation * node.inverse_scale_factor;
    Some((centre * SCREEN_PRECISION).round() / SCREEN_PRECISION)
}

fn texts_within(world: &World, entity: Entity) -> Vec<String> {
    let mut texts = Vec::new();
    let mut stack = vec![entity];
    while let Some(current) = stack.pop() {
        if let Some(text) = world.get::<Text>(current)
            && !text.0.is_empty()
            && !texts.contains(&text.0)
        {
            texts.push(text.0.clone());
        }
        if let Some(children) = world.get::<bevy::ecs::hierarchy::Children>(current) {
            stack.extend(children.iter().rev());
        }
    }
    texts
}

/// The targets that name `subject`, sturdiest first. The first four kinds are
/// kept only if they name exactly one node.
#[must_use]
pub fn target_alternatives(
    world: &World,
    subject: Option<Entity>,
    at: Option<Vec2>,
) -> Vec<Target> {
    let mut targets: Vec<Target> = Vec::new();
    if let Some(subject) = subject {
        let leads_back = |target: &Target| {
            matches!(
                locate(world, target),
                Lookup::Found(node) if node == subject || widget::widget(world, node) == subject
            )
        };
        let candidates = marker_targets(world, subject)
            .into_iter()
            .chain(role_target(world, subject))
            .chain(texts_within(world, subject).into_iter().map(Target::text))
            .chain(
                world
                    .get::<Name>(subject)
                    .map(|name| Target::name(name.as_str())),
            );
        for candidate in candidates {
            if !targets.contains(&candidate) && leads_back(&candidate) {
                targets.push(candidate);
            }
        }
    }
    let position = at.or_else(|| subject.and_then(|subject| centre_of(world, subject)));
    if let Some(position) = position {
        if let Some(app) = app_target_at(world, position)
            && !targets.contains(&app)
        {
            targets.push(app);
        }
        targets.push(Target::screen(position));
    }
    targets
}

fn marker_targets(world: &World, subject: Entity) -> Vec<Target> {
    let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
        return Vec::new();
    };
    let types = registry.read();
    let filter = NoiseFilter::default();
    let mut names: Vec<String> = world.inspect_entity(subject).map_or_default(|infos| {
        infos
            .filter_map(|(_, info)| types.get(info.type_id()?))
            .filter(|registration| is_test_id(registration, &filter))
            .map(|registration| name_of(&types, registration))
            .collect()
    });
    names.sort_unstable();
    names.dedup();
    names.into_iter().map(Target::Component).collect()
}

fn is_test_id(registration: &TypeRegistration, filter: &NoiseFilter) -> bool {
    Role::Component.fits(registration)
        && !Role::Resource.fits(registration)
        && matches!(registration.type_info(), TypeInfo::Struct(info) if info.field_len() == 0)
        && !filter.ignores(registration.type_info().type_path_table())
}

fn role_target(world: &World, subject: Entity) -> Option<Target> {
    let entity = world.get_entity(subject).ok()?;
    let role = WidgetRole::of(&entity)?;
    let label = widget::label_of(world, subject);
    Some(if label.is_empty() {
        Target::of_role(role)
    } else {
        Target::role(role, label)
    })
}

/// A position-only target names no node, so only `Shows(..)` is offered. A check
/// is kept only if it would load from a file.
#[must_use]
pub fn checks_that_hold(
    world: &World,
    subject: Entity,
    target: &Target,
    filter: &NoiseFilter,
) -> Vec<Check> {
    let mut candidates = Vec::new();
    if matches!(widget::node(world, target), Lookup::Found(_)) {
        let text = widget::text_of(world, subject);
        if !text.is_empty() {
            candidates.push(if *target == Target::text(text.clone()) {
                Check::shows(text)
            } else {
                Check::text_of(target.clone(), TextMatch::exact(text))
            });
        }
        let is_widget = widget::is_widget(world, subject);
        let checkable = world.get::<Checkable>(subject).is_some();
        let selectable = world.get::<Selectable>(subject).is_some();
        candidates.push(widget_check(WidgetState::Visible, target.clone()));
        candidates.push(widget_check(WidgetState::Disabled, target.clone()));
        if is_widget {
            candidates.push(!widget_check(WidgetState::Disabled, target.clone()));
        }
        candidates.push(widget_check(WidgetState::Checked, target.clone()));
        if checkable {
            candidates.push(!widget_check(WidgetState::Checked, target.clone()));
        }
        candidates.push(widget_check(WidgetState::Selected, target.clone()));
        if selectable {
            candidates.push(!widget_check(WidgetState::Selected, target.clone()));
        }
        candidates.push(widget_check(WidgetState::Focused, target.clone()));
        candidates.push(Check::hovered(target.clone()));
        candidates.extend(component_checks(world, subject, target, filter));
        if let Ok(tree) = super::tree_snapshot(world, target) {
            candidates.push(Check::tree(target.clone(), tree));
        }
    } else {
        candidates.extend(texts_within(world, subject).into_iter().map(Check::shows));
    }
    let mut kept: Vec<Check> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !kept.contains(&candidate) && holds(world, &candidate) {
            kept.push(candidate);
        }
    }
    kept
}

/// For a type on several nodes, the nearest, as the check finds it.
fn component_checks(
    world: &World,
    subject: Entity,
    target: &Target,
    filter: &NoiseFilter,
) -> Vec<Check> {
    let Lookup::Found(start) = widget::node(world, target) else {
        return Vec::new();
    };
    let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
        return Vec::new();
    };
    let types = registry.read();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut checks = Vec::new();
    for entity in chain_up_to(world, start, subject) {
        let mut found: Vec<(String, &TypeRegistration)> =
            world.inspect_entity(entity).map_or_default(|infos| {
                infos
                    .filter_map(|(_, info)| types.get(info.type_id()?))
                    .filter(|registration| {
                        Role::Component.fits(registration)
                            && !Role::Resource.fits(registration)
                            && !filter.ignores(registration.type_info().type_path_table())
                    })
                    .map(|registration| (name_of(&types, registration), registration))
                    .collect()
            });
        found.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (name, registration) in found {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(component) = reflect::component(world, registration, entity) else {
                continue;
            };
            let mut fields = Vec::new();
            flatten(
                &types,
                &name,
                component.as_partial_reflect(),
                0,
                &mut fields,
            );
            checks.extend(fields.into_iter().filter_map(|(path, ron)| {
                Some(Check::Component {
                    target: target.clone(),
                    path,
                    unit: Unit::Physical,
                    is: Comparison::Eq(Value::from_ron(&ron).ok()?),
                })
            }));
        }
    }
    checks
}

fn chain_up_to(world: &World, start: Entity, top: Entity) -> Vec<Entity> {
    let mut chain = vec![start];
    let mut current = start;
    while current != top {
        let Some(parent) = world.get::<ChildOf>(current) else {
            return vec![start];
        };
        current = parent.parent();
        chain.push(current);
    }
    chain
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum FieldValue {
    Integer { value: i128, signed: bool },
    Float(f64),
    Bool(bool),
    Target(Target),
    Text(String),
    Ron(String),
    Unfillable(String),
}

impl FieldValue {
    #[must_use]
    pub fn ron(&self) -> Option<String> {
        match self {
            Self::Integer { value, .. } => Some(value.to_string()),
            Self::Float(value) => ron::to_string(value).ok(),
            Self::Bool(value) => Some(value.to_string()),
            Self::Target(target) => ron::to_string(target).ok(),
            Self::Text(text) => ron::to_string(text).ok(),
            Self::Ron(ron) => Some(ron.clone()),
            Self::Unfillable(_) => None,
        }
    }

    #[must_use]
    pub const fn can_step(&self) -> bool {
        matches!(
            self,
            Self::Integer { .. } | Self::Float(_) | Self::Bool(_) | Self::Target(_)
        )
    }

    /// Numbers step by one (unsigned stop at zero), bools flip, targets cycle
    /// through `targets`; anything else stays.
    pub fn step(&mut self, up: bool, targets: &[Target]) {
        match self {
            Self::Integer { value, signed } => {
                *value = if up {
                    value.saturating_add(1)
                } else {
                    value.saturating_sub(1)
                };
                if !*signed {
                    *value = (*value).max(0);
                }
            }
            Self::Float(value) => *value += if up { 1.0 } else { -1.0 },
            Self::Bool(value) => *value = !*value,
            Self::Target(target) => {
                let count = targets.len();
                if count == 0 {
                    return;
                }
                let at = targets.iter().position(|other| other == target);
                let next = match (at, up) {
                    (Some(at), true) => (at + 1) % count,
                    (Some(at), false) => (at + count - 1) % count,
                    (None, _) => 0,
                };
                if let Some(next) = targets.get(next) {
                    *target = next.clone();
                }
            }
            Self::Text(_) | Self::Ron(_) | Self::Unfillable(_) => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppCheckForm {
    pub variant: AppVariant,
    pub values: Vec<FieldValue>,
}

impl AppCheckForm {
    #[must_use]
    pub fn ron(&self) -> Option<String> {
        if self.values.is_empty() {
            return Some(self.variant.name.clone());
        }
        let mut parts = Vec::with_capacity(self.values.len());
        for (field, value) in self.variant.fields.iter().zip(&self.values) {
            let ron = value.ron()?;
            parts.push(match (&field.name, self.variant.named_fields) {
                (Some(name), true) => format!("{name}: {ron}"),
                _ => ron,
            });
        }
        Some(format!("{}({})", self.variant.name, parts.join(", ")))
    }

    /// # Errors
    ///
    /// The check text cannot be read as a [`Check`].
    pub fn check(&self, types: &TypeRegistry) -> Result<Check, String> {
        let ron = self.ron().ok_or_else(|| {
            let missing: Vec<&str> = self
                .values
                .iter()
                .filter_map(|value| match value {
                    FieldValue::Unfillable(type_path) => Some(type_path.as_str()),
                    _ => None,
                })
                .collect();
            format!("cannot fill a field of type {}", missing.join(", "))
        })?;
        let mut check = Check::App(AppCheckValue::from_ron(&ron)?);
        check
            .type_app_values(types)
            .map_err(|problem| problem.message())?;
        Ok(check)
    }
}

/// Empty unless the app has exactly one check type.
#[must_use]
pub fn app_check_forms(
    world: &World,
    subject: Entity,
    target: Option<&Target>,
) -> Vec<AppCheckForm> {
    let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
        return Vec::new();
    };
    let types = registry.read();
    let Ok(Some(check_type)) = app_type(&types, Extension::Check) else {
        return Vec::new();
    };
    let text = widget::text_of(world, subject);
    check_type
        .variants
        .into_iter()
        .map(|variant| {
            let values = variant
                .fields
                .iter()
                .map(|field| fill(&types, field, &text, target))
                .collect();
            AppCheckForm { variant, values }
        })
        .collect()
}

fn fill(types: &TypeRegistry, field: &AppField, text: &str, target: Option<&Target>) -> FieldValue {
    let short = field.short_type_path.as_str();
    if field.type_path == Target::type_path() {
        return target.map_or_else(
            || FieldValue::Unfillable(field.type_path.clone()),
            |target| FieldValue::Target(target.clone()),
        );
    }
    match short {
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" => FieldValue::Integer {
            value: first_number(text, false).unwrap_or(0),
            signed: false,
        },
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" => FieldValue::Integer {
            value: first_number(text, true).unwrap_or(0),
            signed: true,
        },
        "f32" | "f64" => FieldValue::Float(
            first_number(text, true)
                .and_then(|value| i32::try_from(value).ok())
                .map_or(0.0, f64::from),
        ),
        "bool" => FieldValue::Bool(false),
        "String" => FieldValue::Text(text.to_owned()),
        _ if short.starts_with("Option<") => FieldValue::Ron("None".to_owned()),
        _ => default_of(types, &field.type_path).map_or_else(
            || FieldValue::Unfillable(field.type_path.clone()),
            FieldValue::Ron,
        ),
    }
}

fn default_of(types: &TypeRegistry, type_path: &str) -> Option<String> {
    let registration = types.get_with_type_path(type_path)?;
    let value = registration.data::<ReflectDefault>()?.default();
    ron_of(types, value.as_partial_reflect())
}

fn first_number(text: &str, signed: bool) -> Option<i128> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let digits: String = text
        .get(start..)?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let negative = signed
        && start > 0
        && text
            .get(..start)
            .is_some_and(|before| before.ends_with('-'));
    let value: i128 = digits.parse().ok()?;
    Some(if negative { -value } else { value })
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickedForm {
    pub form: AppCheckForm,
    pub check: Result<Check, String>,
    pub holds: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Picked {
    node: Entity,
    subject: Entity,
    targets: Vec<Target>,
    chosen: usize,
    checks: Vec<Check>,
    forms: Vec<PickedForm>,
    filter: NoiseFilter,
    added: Vec<Check>,
}

impl Picked {
    #[must_use]
    pub fn new(world: &World, node: Entity, filter: &NoiseFilter, at: Option<Vec2>) -> Self {
        let subject = subject_of(world, node);
        let targets = target_alternatives(world, Some(subject), at);
        let mut picked = Self {
            node,
            subject,
            targets,
            chosen: 0,
            checks: Vec::new(),
            forms: Vec::new(),
            filter: filter.clone(),
            added: Vec::new(),
        };
        picked.reset_forms(world);
        picked
    }

    #[must_use]
    pub const fn node(&self) -> Entity {
        self.node
    }

    #[must_use]
    pub const fn subject(&self) -> Entity {
        self.subject
    }

    #[must_use]
    pub fn targets(&self) -> &[Target] {
        &self.targets
    }

    #[must_use]
    pub const fn chosen(&self) -> usize {
        self.chosen
    }

    #[must_use]
    pub fn target(&self) -> Option<&Target> {
        self.targets.get(self.chosen)
    }

    #[must_use]
    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    #[must_use]
    pub fn forms(&self) -> &[PickedForm] {
        &self.forms
    }

    #[must_use]
    pub fn is_added(&self, check: &Check) -> bool {
        self.added.contains(check)
    }

    pub fn mark_added(&mut self, check: Check) {
        if !self.added.contains(&check) {
            self.added.push(check);
        }
    }

    /// An index past the end changes nothing.
    pub fn choose_target(&mut self, world: &World, index: usize) {
        if index < self.targets.len() {
            self.chosen = index;
            self.reset_forms(world);
        }
    }

    pub fn step_field(&mut self, world: &World, form: usize, field: usize, up: bool) {
        let targets = self.targets.clone();
        if let Some(value) = self
            .forms
            .get_mut(form)
            .and_then(|picked| picked.form.values.get_mut(field))
        {
            value.step(up, &targets);
        }
        self.refresh(world);
    }

    fn reset_forms(&mut self, world: &World) {
        self.forms = app_check_forms(world, self.subject, self.target())
            .into_iter()
            .map(|form| PickedForm {
                form,
                check: Err(String::new()),
                holds: false,
            })
            .collect();
        self.refresh(world);
    }

    pub fn refresh(&mut self, world: &World) {
        self.checks = self
            .target()
            .map_or_default(|target| checks_that_hold(world, self.subject, target, &self.filter));
        let built: Vec<Result<Check, String>> = world
            .get_resource::<AppTypeRegistry>()
            .map_or_default(|registry| {
                let types = registry.read();
                self.forms
                    .iter()
                    .map(|picked| picked.form.check(&types))
                    .collect()
            });
        // Release the lock first: running a check takes it again.
        for (picked, check) in self.forms.iter_mut().zip(built) {
            picked.holds = check.as_ref().is_ok_and(|check| {
                matches!(
                    check.poll(world, &CapturedValues::default(), 0),
                    Some(crate::Outcome::Pass)
                )
            });
            picked.check = check;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_number_in_a_text_is_read_with_its_sign_only_when_signed() {
        assert_eq!(first_number("Score: 3", false), Some(3));
        assert_eq!(first_number("Lives -2 left", true), Some(-2));
        assert_eq!(first_number("Lives -2 left", false), Some(2));
        assert_eq!(first_number("none", true), None);
    }

    #[test]
    fn stepping_an_unsigned_number_stops_at_zero_and_a_target_cycles() {
        let mut number = FieldValue::Integer {
            value: 0,
            signed: false,
        };
        number.step(false, &[]);
        assert_eq!(
            number,
            FieldValue::Integer {
                value: 0,
                signed: false
            }
        );
        let targets = [Target::text("A"), Target::text("B")];
        let mut target = FieldValue::Target(Target::text("B"));
        target.step(true, &targets);
        assert_eq!(target, FieldValue::Target(Target::text("A")));
    }
}
