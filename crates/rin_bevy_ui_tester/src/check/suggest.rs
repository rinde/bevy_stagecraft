//! Checks suggested from what changed in the world while a session was
//! recorded.
//!
//! The person recording knows what should have changed; the world can say
//! what did. [`WorldSnapshot::take`] reads, through reflection:
//!
//! * the fields of every reflected resource;
//! * how many entities have each reflected component;
//! * the texts the app's UI nodes show, and the states of the widgets they
//!   label (disabled, checked, pressed, focused, visible);
//! * the current value of every reflected Bevy `State`;
//! * the app's own facts, from [`AppCheck::facts`](crate::AppCheck::facts).
//!
//! [`suggest_checks`] turns the difference between a snapshot from the start
//! of a recording and one from its end into built-in checks: a changed field
//! becomes `Resource(path: "Counter.0", is: Eq(1))`, a changed count
//! `Count(with: ["Ghost"], is: Eq(2))`, new text `Shows("Saved")` and text
//! that went away `Not(Shows("Saving"))`, a changed state `InState(..)`, a
//! widget that changed state `Disabled(Text("Save"))` or `Not(..)` of it, and
//! a fact that holds now and did not before `App(..)`. Every suggestion is run
//! against the world it came from and kept only if it holds there.
//!
//! The order is fixed: resources, counts, states, texts, widgets, then the
//! app's facts; within each, by type name, field order, text or label, never
//! hash order.
//!
//! A [`NoiseFilter`] leaves out what moves without meaning anything: by
//! default every type of Bevy's own crates (layout, transforms, picking,
//! rendering, input, time) and of this crate. The UI those crates draw still
//! counts, through its texts and widget states.

use alloc::collections::BTreeMap;

use bevy::{
    ecs::{entity::Entity, reflect::AppTypeRegistry, world::World},
    reflect::{
        PartialReflect, ReflectRef, TypePathTable, TypeRegistration, TypeRegistry,
        serde::TypedReflectSerializer,
    },
    ui::widget::Text,
};

use super::{
    Check, Comparison, Value,
    app::app_facts,
    reflect::{self, Role},
    widget::{self, WidgetState},
};
use crate::{
    extension::AppCheckValue,
    probe::Probe,
    scenario::Scenario,
    targets::{Target, is_tooling},
};

/// The types a [`NoiseFilter`] leaves out unless told otherwise: Bevy's own
/// crates and this one.
pub const DEFAULT_IGNORED: [&str; 2] = ["bevy_*", "rin_bevy_ui_tester::*"];

/// How deep a snapshot looks into a resource's fields before it takes what
/// is there as one value.
const MAX_FIELD_DEPTH: usize = 8;

/// The widget states a snapshot records, in the order suggestions name them.
const WIDGET_STATES: [WidgetState; 5] = [
    WidgetState::Disabled,
    WidgetState::Checked,
    WidgetState::Pressed,
    WidgetState::Focused,
    WidgetState::Visible,
];

/// Which reflected types a [`WorldSnapshot`] leaves out.
///
/// A pattern names a type by its short type path (`Transform`, `Time<Real>`),
/// its full type path, or its name without generics (`Time`); a pattern that
/// ends in `*` is a prefix of the full type path (`bevy_*`,
/// `my_game::debug::*`). A type is left out when an `ignore` pattern matches
/// it and no `keep` pattern does.
///
/// ```
/// use rin_bevy_ui_tester::NoiseFilter;
///
/// let filter = NoiseFilter::default().keep("Sprite").ignore("my_game::debug::*");
/// assert!(filter.ignores_path("bevy_transform::components::transform::Transform"));
/// assert!(!filter.ignores_path("bevy_sprite::sprite::Sprite"));
/// assert!(filter.ignores_path("my_game::debug::Overlay"));
/// assert!(!filter.ignores_path("my_game::Score"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoiseFilter {
    /// Patterns of types to leave out.
    ignore: Vec<String>,
    /// Patterns of types to keep even so.
    keep: Vec<String>,
}

impl Default for NoiseFilter {
    /// Leaves out [`DEFAULT_IGNORED`]: Bevy's layout, transform, picking and
    /// render internals, and the rest of Bevy's types with them.
    fn default() -> Self {
        Self {
            ignore: DEFAULT_IGNORED
                .iter()
                .map(|&pattern| pattern.to_owned())
                .collect(),
            keep: Vec::new(),
        }
    }
}

impl NoiseFilter {
    /// A filter that leaves nothing out.
    #[must_use]
    pub const fn nothing() -> Self {
        Self {
            ignore: Vec::new(),
            keep: Vec::new(),
        }
    }

    /// Also leaves out the types `pattern` names.
    #[must_use]
    pub fn ignore(mut self, pattern: impl Into<String>) -> Self {
        self.ignore.push(pattern.into());
        self
    }

    /// Keeps the types `pattern` names, even where an ignore pattern names
    /// them too.
    #[must_use]
    pub fn keep(mut self, pattern: impl Into<String>) -> Self {
        self.keep.push(pattern.into());
        self
    }

    /// The patterns of types left out.
    #[must_use]
    pub fn ignored(&self) -> &[String] {
        &self.ignore
    }

    /// The patterns of types kept even so.
    #[must_use]
    pub fn kept(&self) -> &[String] {
        &self.keep
    }

    /// Whether the type with this full type path is left out. Patterns
    /// without `*` are compared with the full path and with its last
    /// segment, with and without generics.
    #[must_use]
    pub fn ignores_path(&self, type_path: &str) -> bool {
        let short = last_segment(type_path);
        let ident = short.split('<').next().unwrap_or(short);
        self.ignores_names(type_path, short, ident)
    }

    /// Whether the type `table` describes is left out.
    pub(super) fn ignores(&self, table: &TypePathTable) -> bool {
        let short = table.short_path();
        let ident = table.ident().unwrap_or(short);
        self.ignores_names(table.path(), short, ident)
    }

    /// Whether the type with these names is left out: its full type path,
    /// its short one, and its name without generics.
    fn ignores_names(&self, path: &str, short: &str, ident: &str) -> bool {
        let matches = |pattern: &String| {
            pattern.strip_suffix('*').map_or_else(
                || pattern == path || pattern == short || pattern == ident,
                |prefix| path.starts_with(prefix),
            )
        };
        self.ignore.iter().any(matches) && !self.keep.iter().any(matches)
    }
}

/// `b::C<d::E>` of `a::b::C<d::E>`: the path after the last `::` outside
/// generics.
fn last_segment(type_path: &str) -> &str {
    let mut depth = 0_usize;
    let mut start = 0;
    let mut previous = None;
    for (at, character) in type_path.char_indices() {
        match character {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ':' if depth == 0 && previous == Some(':') => start = at + 1,
            _ => {}
        }
        previous = Some(character);
    }
    type_path.get(start..).unwrap_or(type_path)
}

/// What the world looked like at one moment, as far as checks can tell:
/// see the [module documentation](self).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldSnapshot {
    /// Each reflected resource's fields, by the resource's name: the path of
    /// every field (`Counter.0`) with its value as RON, in declaration order.
    resources: BTreeMap<String, Vec<(String, String)>>,
    /// How many entities have each reflected component, by its name.
    counts: BTreeMap<String, usize>,
    /// How many of the app's UI nodes show each text.
    texts: BTreeMap<String, usize>,
    /// The states of each widget a unique text labels, by that text.
    widgets: BTreeMap<String, Vec<WidgetState>>,
    /// The value of each reflected state, as RON, by the state's name.
    states: BTreeMap<String, String>,
    /// The app's facts, in the order it lists them.
    facts: Vec<AppCheckValue>,
}

impl WorldSnapshot {
    /// Reads `world` now, leaving out the types `filter` ignores. Types are
    /// found in the world's `AppTypeRegistry`; without one, only texts and
    /// widgets are read.
    #[must_use]
    pub fn take(world: &World, filter: &NoiseFilter) -> Self {
        let mut snapshot = Self::default();
        snapshot.read_ui(world);
        let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
            return snapshot;
        };
        let types = registry.read();
        for registration in types.iter() {
            let table = registration.type_info().type_path_table();
            if filter.ignores(table) {
                continue;
            }
            snapshot.read_type(world, &types, registration);
        }
        snapshot.facts = app_facts(&types, Probe::new(world));
        drop(types);
        snapshot
    }

    /// Reads what the world holds of the type `registration` describes.
    fn read_type(&mut self, world: &World, types: &TypeRegistry, registration: &TypeRegistration) {
        let name = name_of(types, registration);
        if Role::Resource.fits(registration) {
            if let Some(resource) = reflect::resource(world, registration) {
                let mut fields = Vec::new();
                flatten(types, &name, resource.as_partial_reflect(), 0, &mut fields);
                self.resources.insert(name.clone(), fields);
            }
        } else if Role::Component.fits(registration)
            && world.components().get_id(registration.type_id()).is_some()
        {
            self.counts
                .insert(name.clone(), reflect::count(world, &[registration]));
        }
        if Role::State.fits(registration)
            && let Some(state) = reflect::state(world, registration)
            && let Some(ron) = ron_of(types, state.as_partial_reflect())
        {
            self.states.insert(name, ron);
        }
    }

    /// Reads the texts the app's UI shows and the states of the widgets
    /// they label.
    fn read_ui(&mut self, world: &World) {
        let mut labels: Vec<(String, Entity)> = world
            .try_query::<(Entity, &Text)>()
            .map(|query| {
                query
                    .iter_manual(world)
                    .filter(|(entity, _)| !is_tooling(world, *entity))
                    .map(|(entity, text)| (text.0.clone(), entity))
                    .collect()
            })
            .unwrap_or_default();
        labels.sort();
        for (text, _) in &labels {
            *self.texts.entry(text.clone()).or_default() += 1;
        }
        for (text, node) in labels {
            // A text shown twice names no widget: a target could not find it.
            if text.is_empty() || self.texts.get(&text) != Some(&1) {
                continue;
            }
            let widget = widget::widget(world, node);
            if !widget::is_widget(world, widget) {
                continue;
            }
            let states = WIDGET_STATES
                .into_iter()
                .filter(|state| widget::in_state(world, widget, *state, &text).is_ok())
                .collect();
            self.widgets.insert(text, states);
        }
    }
}

/// The name a check writes for the type `registration` describes: its short
/// type path, or the full one where the short one names more than one type.
pub(super) fn name_of(types: &TypeRegistry, registration: &TypeRegistration) -> String {
    let table = registration.type_info().type_path_table();
    if types.is_ambiguous(table.short_path()) {
        table.path().to_owned()
    } else {
        table.short_path().to_owned()
    }
}

/// `value` as compact RON through reflection, the form a check's value is
/// read back from; `None` when reflection cannot write it.
pub(super) fn ron_of(types: &TypeRegistry, value: &dyn PartialReflect) -> Option<String> {
    ron::to_string(&TypedReflectSerializer::new(value, types)).ok()
}

/// Adds every field of `value`, which sits at `path`, to `out`: structs and
/// tuples are opened, anything else (numbers, text, enums, lists, maps) is
/// one value.
pub(super) fn flatten(
    types: &TypeRegistry,
    path: &str,
    value: &dyn PartialReflect,
    depth: usize,
    out: &mut Vec<(String, String)>,
) {
    let fields: Vec<(String, &dyn PartialReflect)> = match value.reflect_ref() {
        ReflectRef::Struct(fields) if depth < MAX_FIELD_DEPTH => (0..fields.field_len())
            .filter_map(|index| {
                Some((
                    format!("{path}.{}", fields.name_at(index)?),
                    fields.field_at(index)?,
                ))
            })
            .collect(),
        ReflectRef::TupleStruct(fields) if depth < MAX_FIELD_DEPTH => (0..fields.field_len())
            .filter_map(|index| Some((format!("{path}.{index}"), fields.field(index)?)))
            .collect(),
        ReflectRef::Tuple(fields) if depth < MAX_FIELD_DEPTH => (0..fields.field_len())
            .filter_map(|index| Some((format!("{path}.{index}"), fields.field(index)?)))
            .collect(),
        _ => {
            if let Some(ron) = ron_of(types, value) {
                out.push((path.to_owned(), ron));
            }
            return;
        }
    };
    for (field_path, field) in fields {
        flatten(types, &field_path, field, depth + 1, out);
    }
}

/// The checks that say what changed from `before` to `after`, in a fixed
/// order, each of which holds in `world`: see the [module
/// documentation](self). `world` is the world `after` was taken of.
#[must_use]
pub fn suggest_checks(before: &WorldSnapshot, after: &WorldSnapshot, world: &World) -> Vec<Check> {
    let mut candidates = Vec::new();
    resource_changes(before, after, &mut candidates);
    count_changes(before, after, &mut candidates);
    state_changes(before, after, &mut candidates);
    text_changes(before, after, &mut candidates);
    widget_changes(before, after, &mut candidates);
    fact_changes(before, after, &mut candidates);
    let mut kept: Vec<Check> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !kept.contains(&candidate) && holds(world, &candidate) {
            kept.push(candidate);
        }
    }
    kept
}

/// `Resource(path, is: Eq(value))` for every field whose value changed.
fn resource_changes(before: &WorldSnapshot, after: &WorldSnapshot, out: &mut Vec<Check>) {
    for (name, fields) in &after.resources {
        let old = before.resources.get(name);
        for (path, ron) in fields {
            let was = old.and_then(|fields| {
                fields
                    .iter()
                    .find(|(old_path, _)| old_path == path)
                    .map(|(_, old)| old)
            });
            if was != Some(ron)
                && let Ok(value) = Value::from_ron(ron)
            {
                out.push(Check::Resource {
                    path: path.clone(),
                    is: Comparison::Eq(value),
                });
            }
        }
    }
}

/// `Count(with: [T], is: Eq(n))` for every component whose count changed.
fn count_changes(before: &WorldSnapshot, after: &WorldSnapshot, out: &mut Vec<Check>) {
    for (name, count) in &after.counts {
        if before.counts.get(name).copied().unwrap_or(0) != *count {
            out.push(Check::count_of([name.clone()]).eq(*count));
        }
    }
}

/// `InState(state, value)` for every state whose value changed.
fn state_changes(before: &WorldSnapshot, after: &WorldSnapshot, out: &mut Vec<Check>) {
    for (name, ron) in &after.states {
        if before.states.get(name) != Some(ron)
            && let Ok(value) = Value::from_ron(ron)
        {
            out.push(Check::InState {
                state: name.clone(),
                value,
            });
        }
    }
}

/// `Shows(text)` for text shown now and not before, then `Not(Shows(text))`
/// for text shown before and not now.
fn text_changes(before: &WorldSnapshot, after: &WorldSnapshot, out: &mut Vec<Check>) {
    for text in after.texts.keys() {
        if !before.texts.contains_key(text) {
            out.push(Check::shows(text.clone()));
        }
    }
    for text in before.texts.keys() {
        if !after.texts.contains_key(text) {
            out.push(!Check::shows(text.clone()));
        }
    }
}

/// For a widget there before and after, each state it went into or out of;
/// for a widget new since, each state it is in other than being visible,
/// which its text already says.
fn widget_changes(before: &WorldSnapshot, after: &WorldSnapshot, out: &mut Vec<Check>) {
    for (label, states) in &after.widgets {
        let old = before.widgets.get(label);
        for state in WIDGET_STATES {
            let now = states.contains(&state);
            let was = old.map_or(state == WidgetState::Visible, |old| old.contains(&state));
            if now != was {
                let check = widget_check(state, Target::text(label.clone()));
                out.push(if now { check } else { !check });
            }
        }
    }
}

/// The check that the target's widget is in `state`.
pub(super) const fn widget_check(state: WidgetState, target: Target) -> Check {
    match state {
        WidgetState::Disabled => Check::Disabled(target),
        WidgetState::Checked => Check::Checked(target),
        WidgetState::Pressed => Check::Pressed(target),
        WidgetState::Focused => Check::Focused(target),
        WidgetState::Visible => Check::Visible(target),
        WidgetState::Exists => Check::Exists(target),
    }
}

/// `App(fact)` for every fact listed now and not before, then
/// `Not(App(fact))` for every fact listed before and not now.
fn fact_changes(before: &WorldSnapshot, after: &WorldSnapshot, out: &mut Vec<Check>) {
    for fact in &after.facts {
        if !before.facts.contains(fact) {
            out.push(Check::App(fact.clone()));
        }
    }
    for fact in &before.facts {
        if !after.facts.contains(fact) {
            out.push(!Check::App(fact.clone()));
        }
    }
}

/// Whether `check` would load from a file and holds in `world` now.
pub(super) fn holds(world: &World, check: &Check) -> bool {
    let loads = world
        .get_resource::<AppTypeRegistry>()
        .is_none_or(|registry| {
            let types = registry.read();
            check.parts().iter().all(|part| {
                part.problem(&types).is_ok()
                    && !matches!(part, Check::App(value) if value.ron().is_err())
            })
        });
    // The lock is let go first: running the check takes it again.
    loads
        && matches!(
            check.poll(world, &Scenario::new("suggest"), 0),
            Some(Ok(()))
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_filter_leaves_out_layout_transform_picking_and_render_internals() {
        let filter = NoiseFilter::default();
        for path in [
            "bevy_transform::components::transform::Transform",
            "bevy_transform::components::global_transform::GlobalTransform",
            "bevy_ui::ui_node::ComputedNode",
            "bevy_picking::hover::HoverMap",
            "bevy_render::view::visibility::ViewVisibility",
            "bevy_time::time::Time<bevy_time::real::Real>",
        ] {
            assert!(filter.ignores_path(path), "{path}");
        }
        assert!(!filter.ignores_path("counter::Counter"));
    }

    #[test]
    fn a_pattern_names_a_type_by_its_short_path_or_its_name_without_generics() {
        let filter = NoiseFilter::nothing().ignore("Score").ignore("Time");
        assert!(filter.ignores_path("game::Score"));
        assert!(filter.ignores_path("bevy_time::time::Time<bevy_time::real::Real>"));
        assert!(!filter.ignores_path("game::Scores"));
    }

    #[test]
    fn the_last_segment_of_a_path_keeps_its_generics_whole() {
        assert_eq!(last_segment("a::b::C<d::E>"), "C<d::E>");
        assert_eq!(last_segment("Plain"), "Plain");
    }
}
