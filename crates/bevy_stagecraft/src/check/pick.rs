//! Checks and targets for one node, picked by pointing at it.
//!
//! The gallery's **Pick** mode (see `crate::gallery`) uses this; it reads the
//! world only, so picking never changes the app.
//!
//! * [`node_at`] finds the app's topmost UI node under a position, the way
//!   Bevy's UI picking would, without moving any pointer;
//! * [`subject_of`] goes from that node to the widget it belongs to, as the
//!   built-in checks do;
//! * [`target_alternatives`] lists the targets that find the widget again,
//!   best first: the one ranking of targets, which the session recorder
//!   uses too;
//! * [`checks_that_hold`] lists the built-in checks about it that hold now:
//!   its text, its states and its reflected component fields;
//! * [`app_check_forms`] fills a form for each variant of the app's check
//!   type (see [`AppCheck`](super::AppCheck)) from what the widget shows;
//! * [`Picked`] keeps all of that for one picked node.
//!
//! Every list is in a fixed order, never hash order.

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

/// How a position is rounded in a screen target: to a tenth of a pixel, as
/// recordings write them.
const SCREEN_PRECISION: f32 = 10.0;

/// The app's topmost UI node at `position` (logical pixels in the primary
/// window), or `None` where there is none.
///
/// It walks Bevy's `UiStack` from the front, as UI picking does: a node
/// counts when it is laid out, visible, hoverable (`Pickable`), not clipped
/// away at the position and not part of the tooling
/// ([`NotUnderTest`](crate::NotUnderTest)). Nothing is moved and no pointer
/// is touched.
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

/// Whether `entity` is a node of the app that `position` is on.
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

/// The widget `node` belongs to: the nearest widget at or above it (a
/// button, a checkbox, a slider, a node that tracks interaction), as the
/// built-in checks find it from a label; otherwise `node` itself.
#[must_use]
pub fn subject_of(world: &World, node: Entity) -> Entity {
    widget::widget(world, node)
}

/// The middle of `entity` in logical pixels, rounded as a recording rounds
/// positions.
fn centre_of(world: &World, entity: Entity) -> Option<Vec2> {
    let node = world.get::<ComputedNode>(entity)?;
    let transform = world.get::<UiGlobalTransform>(entity)?;
    let centre = transform.affine().translation * node.inverse_scale_factor;
    Some((centre * SCREEN_PRECISION).round() / SCREEN_PRECISION)
}

/// Every non-empty text at or below `entity`, depth first, each once.
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

/// The targets that name `subject`, best first, ending with what names the
/// position `at` (or the widget's middle when there is none).
///
/// This is the one place targets are ranked: pick mode offers the whole
/// list, and the session recorder writes the first for a click (see
/// [`crate::session`]). The order is the sturdiest first:
///
/// 1. `Component(..)` of each test id on the widget, by name: a reflected
///    marker component, that is a unit struct (`struct PauseButton;`) with
///    `#[reflect(Component)]`, that is not Bevy's or this crate's (the
///    default [`NoiseFilter`] decides) and that no other entity of the app
///    has;
/// 2. `Role(..)` of the widget's role (see [`WidgetRole`])
///    with its text as the label, or with no label when it shows none;
/// 3. `Text(..)` of each text at or below the widget, depth first;
/// 4. `Name(..)` of the widget's Bevy `Name`;
/// 5. what the app's [`AppTarget::at`](crate::AppTarget::at) makes of the
///    position;
/// 6. `Screen(..)` of the position, which always works but follows no layout
///    change.
///
/// Each of the first four is kept only if it names exactly one node and that
/// node is `subject` or lies in it, so a target that is ambiguous, or that
/// leads to another widget, is never offered. With no `subject` only the
/// last two are left.
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

/// `Component(..)` of every test id on `subject`, by name (see
/// [`target_alternatives`]), whether or not another entity has it too.
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

/// Whether the type `registration` describes can be a test id: a reflected
/// component that is a unit struct and that `filter` does not leave out.
fn is_test_id(registration: &TypeRegistration, filter: &NoiseFilter) -> bool {
    Role::Component.fits(registration)
        && !Role::Resource.fits(registration)
        && matches!(registration.type_info(), TypeInfo::Struct(info) if info.field_len() == 0)
        && !filter.ignores(registration.type_info().type_path_table())
}

/// `Role(..)` of `subject`'s role, labelled with its text, if it is a widget
/// with a role.
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

/// The built-in checks about `subject`, named through `target`, that hold in
/// `world` now, in a fixed order:
///
/// * its text: `Shows(..)` when the target is that very text, otherwise
///   `TextOf(target, Exact(..))`;
/// * its states: `Visible`, `Disabled` or `Not(Disabled)`, `Checked` or
///   `Not(Checked)` (for something that can be checked), `Selected` or
///   `Not(Selected)` (for something that can be selected), `Focused`,
///   `Hovered`;
/// * each field of each reflected component on the target's node and the
///   nodes above it up to the widget, as `Component(..)`, leaving out the
///   types `filter` ignores;
/// * last, `Tree(..)` with the snapshot of the node's tree, to snapshot the
///   node whole.
///
/// A target that is only a position names no node; then only `Shows(..)` of
/// the texts the widget shows is offered. Every check is kept only if it
/// would load from a file and holds.
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
        // Last: the snapshot of the node, for when the person wants the whole
        // of what is below it rather than one fact.
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

/// `Component(target, path, is: Eq(..))` for every field of every reflected
/// component the target's node or a node above it, up to `subject`, has;
/// for a type on several of them, the nearest, as the check finds it.
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

/// `start` and the entities above it up to and including `top`, nearest
/// first; only `start` when `top` is not above it.
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

/// The value in one field of an [`AppCheckForm`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum FieldValue {
    /// A whole number; `signed` says whether it may go below zero.
    Integer {
        /// The number.
        value: i128,
        /// Whether the field's type is signed.
        signed: bool,
    },
    /// A floating-point number.
    Float(f64),
    /// Yes or no.
    Bool(bool),
    /// A [`Target`], one of the picked node's.
    Target(Target),
    /// Text.
    Text(String),
    /// A value the form cannot change, as RON: `None`, or the type's default.
    Ron(String),
    /// A type the form cannot fill: no default and nothing to take it from.
    /// Its type path.
    Unfillable(String),
}

impl FieldValue {
    /// The value as RON, as it goes into the check; `None` when the field
    /// cannot be filled.
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

    /// Whether [`Self::step`] changes the value.
    #[must_use]
    pub const fn can_step(&self) -> bool {
        matches!(
            self,
            Self::Integer { .. } | Self::Float(_) | Self::Bool(_) | Self::Target(_)
        )
    }

    /// Moves the value one step: a number up or down by one (not below zero
    /// for an unsigned type), yes to no and back, a target to the next or the
    /// previous of `targets`. Anything else stays.
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

/// One variant of the app's check type, with a value in each field.
#[derive(Debug, Clone, PartialEq)]
pub struct AppCheckForm {
    /// The variant and its fields' types.
    pub variant: AppVariant,
    /// A value for each field, in order.
    pub values: Vec<FieldValue>,
}

impl AppCheckForm {
    /// The variant with its values, as a file writes it inside `App(..)`:
    /// `Activated(3)`, `Between(at_least: 1, at_most: 2)`; `None` while a
    /// field cannot be filled.
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

    /// The check the form makes, read as the app's check type in `types`.
    ///
    /// # Errors
    ///
    /// Why the form does not make a check of that type.
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

/// A form for each variant of the app's check type, in declaration order.
///
/// Each field is filled from what `subject` shows: a [`Target`] field with
/// `target`, text with the widget's text, a number with the first number in
/// that text (or zero), `bool` with `false`, an `Option` with `None`, and
/// any other type with its reflected default where it has one. Nothing when
/// the app has no check type, or more than one.
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

/// The first value of a form's field.
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

/// The reflected default of the type at `type_path`, as RON.
fn default_of(types: &TypeRegistry, type_path: &str) -> Option<String> {
    let registration = types.get_with_type_path(type_path)?;
    let value = registration.data::<ReflectDefault>()?.default();
    ron_of(types, value.as_partial_reflect())
}

/// The first whole number written in `text`, with a minus sign when
/// `signed`: 3 in `Score: 3`.
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

/// An app check form, with the check it makes and whether that holds.
#[derive(Debug, Clone, PartialEq)]
pub struct PickedForm {
    /// The form.
    pub form: AppCheckForm,
    /// The check the form makes, or why it makes none.
    pub check: Result<Check, String>,
    /// Whether the check holds now.
    pub holds: bool,
}

/// One picked node, with what can be checked about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Picked {
    /// The node under the pointer.
    node: Entity,
    /// The widget it belongs to, which the targets name.
    subject: Entity,
    /// The targets that name it, best first.
    targets: Vec<Target>,
    /// Which of them the checks use.
    chosen: usize,
    /// The built-in checks that hold, through the chosen target.
    checks: Vec<Check>,
    /// The app's check forms.
    forms: Vec<PickedForm>,
    /// What component checks leave out.
    filter: NoiseFilter,
    /// The checks already taken, so they read as such.
    added: Vec<Check>,
}

impl Picked {
    /// Picks `node`, clicked at `at` (its widget's middle when `None`): works
    /// out its widget, the targets that name it (see
    /// [`target_alternatives`]), the checks that hold and the app's check
    /// forms.
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

    /// The node that was under the pointer.
    #[must_use]
    pub const fn node(&self) -> Entity {
        self.node
    }

    /// The widget the node belongs to.
    #[must_use]
    pub const fn subject(&self) -> Entity {
        self.subject
    }

    /// The targets that name the widget, best first.
    #[must_use]
    pub fn targets(&self) -> &[Target] {
        &self.targets
    }

    /// The index of the target the checks use.
    #[must_use]
    pub const fn chosen(&self) -> usize {
        self.chosen
    }

    /// The target the checks use.
    #[must_use]
    pub fn target(&self) -> Option<&Target> {
        self.targets.get(self.chosen)
    }

    /// The built-in checks that hold, through the chosen target.
    #[must_use]
    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    /// The app's check forms.
    #[must_use]
    pub fn forms(&self) -> &[PickedForm] {
        &self.forms
    }

    /// Whether `check` was taken already.
    #[must_use]
    pub fn is_added(&self, check: &Check) -> bool {
        self.added.contains(check)
    }

    /// Notes that `check` was taken.
    pub fn mark_added(&mut self, check: Check) {
        if !self.added.contains(&check) {
            self.added.push(check);
        }
    }

    /// Uses target `index` for the checks and refills the forms with it. An
    /// index past the end changes nothing.
    pub fn choose_target(&mut self, world: &World, index: usize) {
        if index < self.targets.len() {
            self.chosen = index;
            self.reset_forms(world);
        }
    }

    /// Steps field `field` of form `form` up or down (see
    /// [`FieldValue::step`]) and works its check out again.
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

    /// Fills the forms afresh and works everything out again.
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

    /// Works out again which checks hold, in the world as it is now.
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
        // The lock is let go first: running a check takes it again.
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
