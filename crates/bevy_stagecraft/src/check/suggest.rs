//! Checks suggested from what changed in the world while a session was
//! recorded.
//!
//! [`WorldSnapshot::take`] reads resources, component counts, texts, widget
//! states, Bevy `State`s and the app's facts; [`suggest_checks`] turns the
//! difference of two snapshots into built-in checks, each kept only if it holds
//! in the world it came from.
//!
//! The order is fixed (resources, counts, states, texts, widgets, app facts;
//! within each by name, never hash order). A [`NoiseFilter`] leaves out what
//! moves without meaning: by default the types of Bevy's crates and this one.

use alloc::collections::BTreeMap;
use core::fmt::Write as _;

use bevy::{
    ecs::{entity::Entity, reflect::AppTypeRegistry, world::World},
    reflect::{
        PartialReflect, ReflectRef, TypePathTable, TypeRegistration, TypeRegistry,
        serde::TypedReflectSerializer,
    },
    text::EditableText,
    ui::widget::Text,
};

use super::{
    CapturedValues, Check, Comparison, Unit, Value,
    app::app_facts,
    reflect::{self, Role},
    widget::{self, WidgetState},
};
use crate::{
    extension::AppCheckValue,
    probe::Probe,
    targets::{Target, is_tooling},
};

pub const DEFAULT_IGNORED: [&str; 2] = ["bevy_*", "bevy_stagecraft::*"];

const MAX_FIELD_DEPTH: usize = 8;

const WIDGET_STATES: [WidgetState; 6] = [
    WidgetState::Disabled,
    WidgetState::Checked,
    WidgetState::Selected,
    WidgetState::Pressed,
    WidgetState::Focused,
    WidgetState::Visible,
];

/// Leaves types out of a suggestion.
///
/// A pattern names a type by its short type path (`Transform`, `Time<Real>`),
/// its full type path, or its name without generics; a trailing `*` makes it a
/// prefix of the full path (`bevy_*`). An `ignore` pattern leaves a type out
/// unless a `keep` pattern matches it too.
///
/// ```
/// use bevy_stagecraft::NoiseFilter;
///
/// let filter = NoiseFilter::default().keep("Sprite").ignore("my_game::debug::*");
/// assert!(filter.ignores_path("bevy_transform::components::transform::Transform"));
/// assert!(!filter.ignores_path("bevy_sprite::sprite::Sprite"));
/// assert!(filter.ignores_path("my_game::debug::Overlay"));
/// assert!(!filter.ignores_path("my_game::Score"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoiseFilter {
    ignore: Vec<String>,
    keep: Vec<String>,
}

impl Default for NoiseFilter {
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
    #[must_use]
    pub const fn nothing() -> Self {
        Self {
            ignore: Vec::new(),
            keep: Vec::new(),
        }
    }

    #[must_use]
    pub fn ignore(mut self, pattern: impl Into<String>) -> Self {
        self.ignore.push(pattern.into());
        self
    }

    #[must_use]
    pub fn keep(mut self, pattern: impl Into<String>) -> Self {
        self.keep.push(pattern.into());
        self
    }

    #[must_use]
    pub fn ignored(&self) -> &[String] {
        &self.ignore
    }

    #[must_use]
    pub fn kept(&self) -> &[String] {
        &self.keep
    }

    /// Returns whether the type is left out afterwards: not so where a broader
    /// `keep` pattern still matches it.
    pub fn hide(&mut self, type_path: &str) -> bool {
        self.keep.retain(|pattern| pattern != type_path);
        if !self.ignores_path(type_path) {
            self.ignore.push(type_path.to_owned());
        }
        self.ignores_path(type_path)
    }

    /// Adds a `keep` pattern only where a broader `ignore` (such as `bevy_*`)
    /// still matches.
    pub fn reveal(&mut self, type_path: &str) {
        self.ignore.retain(|pattern| pattern != type_path);
        if self.ignores_path(type_path) {
            self.keep.push(type_path.to_owned());
        }
    }

    /// The Rust expression that builds the filter, for pasting into
    /// [`SessionHooks::noise_filter`](crate::SessionHooks::noise_filter).
    ///
    ///
    /// ```
    /// use bevy_stagecraft::NoiseFilter;
    ///
    /// let filter = NoiseFilter::default().keep("Time<Virtual>");
    /// assert_eq!(
    ///     filter.to_code(),
    ///     r#"NoiseFilter::default().keep("Time<Virtual>")"#
    /// );
    /// ```
    #[must_use]
    pub fn to_code(&self) -> String {
        let defaults = self.ignore.len() >= DEFAULT_IGNORED.len()
            && self.ignore.iter().zip(DEFAULT_IGNORED).all(|(a, b)| a == b);
        let (mut code, ignored) = if defaults {
            (
                "NoiseFilter::default()".to_owned(),
                self.ignore.get(DEFAULT_IGNORED.len()..).unwrap_or_default(),
            )
        } else {
            ("NoiseFilter::nothing()".to_owned(), self.ignore.as_slice())
        };
        for pattern in ignored {
            let _ = write!(code, ".ignore({pattern:?})");
        }
        for pattern in &self.keep {
            let _ = write!(code, ".keep({pattern:?})");
        }
        code
    }

    /// Patterns without `*` compare with the full path and its last segment,
    /// with and without generics.
    #[must_use]
    pub fn ignores_path(&self, type_path: &str) -> bool {
        let short = last_segment(type_path);
        let ident = short.split('<').next().unwrap_or(short);
        self.ignores_names(type_path, short, ident)
    }

    pub(super) fn ignores(&self, table: &TypePathTable) -> bool {
        let short = table.short_path();
        let ident = table.ident().unwrap_or(short);
        self.ignores_names(table.path(), short, ident)
    }

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

/// The path after the last `::` outside generics.
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldSnapshot {
    resources: BTreeMap<String, Vec<(String, String)>>,
    counts: BTreeMap<String, usize>,
    texts: BTreeMap<String, usize>,
    widgets: BTreeMap<String, Vec<WidgetState>>,
    states: BTreeMap<String, String>,
    facts: Vec<AppCheckValue>,
    type_paths: BTreeMap<String, String>,
}

impl WorldSnapshot {
    /// Without an `AppTypeRegistry`, only texts and widgets are read.
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

    #[cfg(test)]
    pub(crate) fn set_resource(&mut self, name: &str, type_path: &str, field: &str, ron: &str) {
        self.resources
            .insert(name.to_owned(), vec![(field.to_owned(), ron.to_owned())]);
        self.type_paths
            .insert(name.to_owned(), type_path.to_owned());
    }

    fn read_type(&mut self, world: &World, types: &TypeRegistry, registration: &TypeRegistration) {
        let name = name_of(types, registration);
        let type_path = registration.type_info().type_path_table().path();
        if Role::Resource.fits(registration) {
            if let Some(resource) = reflect::resource(world, registration) {
                let mut fields = Vec::new();
                flatten(types, &name, resource.as_partial_reflect(), 0, &mut fields);
                self.resources.insert(name.clone(), fields);
                self.type_paths.insert(name.clone(), type_path.to_owned());
            }
        } else if Role::Component.fits(registration)
            && world.components().get_id(registration.type_id()).is_some()
        {
            self.counts
                .insert(name.clone(), reflect::count(world, &[registration]));
            self.type_paths.insert(name.clone(), type_path.to_owned());
        }
        if Role::State.fits(registration)
            && let Some(state) = reflect::state(world, registration)
            && let Some(ron) = ron_of(types, state.as_partial_reflect())
        {
            self.type_paths.insert(name.clone(), type_path.to_owned());
            self.states.insert(name, ron);
        }
    }

    fn read_ui(&mut self, world: &World) {
        let mut labels: Vec<(String, Entity)> = world
            .try_query::<(Entity, &Text)>()
            .map_or_default(|query| {
                query
                    .iter_manual(world)
                    .filter(|(entity, _)| !is_tooling(world, *entity))
                    .map(|(entity, text)| (text.0.clone(), entity))
                    .collect()
            });
        labels.sort();
        // A text input's value counts as a text only, for `Shows` and its `Not`.
        let values: Vec<String> =
            world
                .try_query::<(Entity, &EditableText)>()
                .map_or_default(|query| {
                    query
                        .iter_manual(world)
                        .filter(|(entity, _)| !is_tooling(world, *entity))
                        .map(|(_, field)| field.value().to_string())
                        .filter(|value| !value.is_empty())
                        .collect()
                });
        for value in values {
            *self.texts.entry(value).or_default() += 1;
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotChange {
    pub key: String,
    pub before: Option<String>,
    pub after: Option<String>,
    /// `None` for texts, widgets and app facts, which belong to no type.
    pub type_path: Option<String>,
}

impl WorldSnapshot {
    #[must_use]
    pub fn entries(&self) -> BTreeMap<String, String> {
        self.lines()
            .into_iter()
            .map(|(key, (value, _))| (key, value))
            .collect()
    }

    fn lines(&self) -> BTreeMap<String, (String, Option<String>)> {
        let mut out = BTreeMap::new();
        let type_path = |name: &String| self.type_paths.get(name).cloned();
        for (name, fields) in &self.resources {
            for (path, ron) in fields {
                out.insert(format!("resource {path}"), (ron.clone(), type_path(name)));
            }
        }
        for (name, count) in &self.counts {
            out.insert(
                format!("count {name}"),
                (count.to_string(), type_path(name)),
            );
        }
        for (name, ron) in &self.states {
            out.insert(format!("state {name}"), (ron.clone(), type_path(name)));
        }
        for (text, count) in &self.texts {
            out.insert(
                format!("text {text:?}"),
                (format!("shown {count}\u{d7}"), None),
            );
        }
        for (label, states) in &self.widgets {
            let states: Vec<String> = states.iter().map(|state| format!("{state:?}")).collect();
            out.insert(format!("widget {label:?}"), (states.join(", "), None));
        }
        for (index, fact) in self.facts.iter().enumerate() {
            out.insert(format!("fact {index:03}"), (fact.to_string(), None));
        }
        out
    }

    #[must_use]
    pub fn changes_from(&self, before: &Self) -> Vec<SnapshotChange> {
        let was = before.lines();
        let now = self.lines();
        let mut keys: Vec<&String> = was.keys().chain(now.keys()).collect();
        keys.sort();
        keys.dedup();
        keys.into_iter()
            .filter_map(|key| {
                let (old, new) = (was.get(key), now.get(key));
                (old.map(|(value, _)| value) != new.map(|(value, _)| value)).then(|| {
                    SnapshotChange {
                        key: key.clone(),
                        before: old.map(|(value, _)| value.clone()),
                        after: new.map(|(value, _)| value.clone()),
                        type_path: new.or(old).and_then(|(_, path)| path.clone()),
                    }
                })
            })
            .collect()
    }
}

pub(crate) fn state_names(world: &World, widget: Entity) -> Vec<String> {
    if !widget::is_widget(world, widget) {
        return Vec::new();
    }
    WIDGET_STATES
        .into_iter()
        .filter(|state| widget::in_state(world, widget, *state, "").is_ok())
        .map(|state| format!("{state:?}"))
        .collect()
}

/// The short type path, or the full one where it names more than one type.
pub(super) fn name_of(types: &TypeRegistry, registration: &TypeRegistration) -> String {
    let table = registration.type_info().type_path_table();
    if types.is_ambiguous(table.short_path()) {
        table.path().to_owned()
    } else {
        table.short_path().to_owned()
    }
}

pub(super) fn ron_of(types: &TypeRegistry, value: &dyn PartialReflect) -> Option<String> {
    ron::to_string(&TypedReflectSerializer::new(value, types)).ok()
}

/// Opens structs and tuples; anything else is one value.
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

/// Each suggestion holds in `world`, the world `after` was taken of.
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
                    unit: Unit::Physical,
                    is: Comparison::Eq(value),
                });
            }
        }
    }
}

fn count_changes(before: &WorldSnapshot, after: &WorldSnapshot, out: &mut Vec<Check>) {
    for (name, count) in &after.counts {
        if before.counts.get(name).copied().unwrap_or(0) != *count {
            out.push(Check::count_of([name.clone()]).eq(*count));
        }
    }
}

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

/// A new widget gets each state but visible, which its text already says.
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

pub(super) const fn widget_check(state: WidgetState, target: Target) -> Check {
    match state {
        WidgetState::Disabled => Check::Disabled(target),
        WidgetState::Checked => Check::Checked(target),
        WidgetState::Selected => Check::Selected(target),
        WidgetState::Pressed => Check::Pressed(target),
        WidgetState::Focused => Check::Focused(target),
        WidgetState::Visible => Check::Visible(target),
        WidgetState::OnTop => Check::OnTop(target),
        WidgetState::Exists => Check::Exists(target),
    }
}

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
    // Release the lock first: running the check takes it again.
    loads
        && matches!(
            check.poll(world, &CapturedValues::default(), 0),
            Some(crate::Outcome::Pass)
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

    const GAME_SCORE: &str = "game::Score";
    const BEVY_TIME: &str = "bevy_time::time::Time<bevy_time::virt::Virtual>";

    #[test]
    fn hiding_a_type_adds_an_ignore_pattern_for_its_full_path() {
        let mut filter = NoiseFilter::default();
        assert!(filter.hide(GAME_SCORE));
        assert!(filter.ignores_path(GAME_SCORE));
        assert_eq!(
            filter.ignored().last().map(String::as_str),
            Some(GAME_SCORE)
        );
    }

    #[test]
    fn hiding_a_type_that_is_already_left_out_adds_nothing() {
        let mut filter = NoiseFilter::default();
        assert!(filter.hide(BEVY_TIME));
        assert_eq!(filter, NoiseFilter::default());
    }

    #[test]
    fn hiding_a_type_drops_the_keep_patterns_that_name_it_exactly() {
        let mut filter = NoiseFilter::default().keep(BEVY_TIME);
        assert!(!filter.ignores_path(BEVY_TIME));
        assert!(filter.hide(BEVY_TIME));
        assert_eq!(filter.kept().len(), 0);
        assert_eq!(filter, NoiseFilter::default());
    }

    #[test]
    fn hiding_a_type_a_broader_keep_pattern_matches_says_it_is_still_shown() {
        let mut filter = NoiseFilter::default().keep("game::*");
        assert!(!filter.hide(GAME_SCORE));
        assert!(!filter.ignores_path(GAME_SCORE));
        assert_eq!(
            filter.kept(),
            ["game::*"],
            "the app's pattern is left alone"
        );
    }

    #[test]
    fn revealing_a_type_drops_the_ignore_patterns_that_name_it_exactly() {
        let mut filter = NoiseFilter::nothing()
            .ignore(GAME_SCORE)
            .ignore("game::Lives");
        filter.reveal(GAME_SCORE);
        assert!(!filter.ignores_path(GAME_SCORE));
        assert_eq!(filter.ignored(), ["game::Lives"]);
        assert_eq!(filter.kept().len(), 0);
    }

    #[test]
    fn revealing_a_type_a_broader_ignore_pattern_matches_adds_a_keep_pattern() {
        let mut filter = NoiseFilter::default();
        filter.reveal(BEVY_TIME);
        assert!(!filter.ignores_path(BEVY_TIME));
        assert!(filter.ignores_path("bevy_time::time::Time<bevy_time::real::Real>"));
        assert_eq!(filter.kept(), [BEVY_TIME]);
        assert_eq!(filter.ignored(), DEFAULT_IGNORED);
    }

    #[test]
    fn revealing_a_type_nothing_leaves_out_changes_nothing() {
        let mut filter = NoiseFilter::default();
        filter.reveal(GAME_SCORE);
        assert_eq!(filter, NoiseFilter::default());
    }

    #[test]
    fn the_default_filter_is_written_as_default() {
        assert_eq!(NoiseFilter::default().to_code(), "NoiseFilter::default()");
    }

    #[test]
    fn the_default_filter_with_more_patterns_is_written_as_default_and_the_rest() {
        let filter = NoiseFilter::default()
            .ignore("game::debug::*")
            .keep("Time<Virtual>")
            .keep("Sprite");
        assert_eq!(
            filter.to_code(),
            r#"NoiseFilter::default().ignore("game::debug::*").keep("Time<Virtual>").keep("Sprite")"#
        );
    }

    #[test]
    fn a_filter_that_does_not_start_with_the_defaults_is_written_from_nothing() {
        assert_eq!(NoiseFilter::nothing().to_code(), "NoiseFilter::nothing()");
        let filter = NoiseFilter::nothing().ignore("bevy_*").keep("Sprite");
        assert_eq!(
            filter.to_code(),
            r#"NoiseFilter::nothing().ignore("bevy_*").keep("Sprite")"#
        );
        let reordered = NoiseFilter::nothing()
            .ignore("bevy_stagecraft::*")
            .ignore("bevy_*");
        assert_eq!(
            reordered.to_code(),
            r#"NoiseFilter::nothing().ignore("bevy_stagecraft::*").ignore("bevy_*")"#
        );
    }

    #[test]
    fn code_escapes_quotes_and_backslashes_as_a_rust_string_literal() {
        let filter = NoiseFilter::nothing().ignore("a\"b\\c");
        assert_eq!(
            filter.to_code(),
            r#"NoiseFilter::nothing().ignore("a\"b\\c")"#
        );
    }

    fn snapshot(score: &str, time: &str, ghosts: usize, text: Option<&str>) -> WorldSnapshot {
        let mut snapshot = WorldSnapshot::default();
        snapshot
            .resources
            .insert("Score".into(), vec![("Score.0".into(), score.into())]);
        snapshot.resources.insert(
            "Time<Virtual>".into(),
            vec![("Time<Virtual>.elapsed".into(), time.into())],
        );
        snapshot.counts.insert("Ghost".into(), ghosts);
        snapshot.states.insert("Mode".into(), "Play".into());
        for (name, path) in [
            ("Score", GAME_SCORE),
            ("Time<Virtual>", BEVY_TIME),
            ("Ghost", "game::Ghost"),
            ("Mode", "bevy_state::state::State<game::Mode>"),
        ] {
            snapshot.type_paths.insert(name.into(), path.into());
        }
        if let Some(text) = text {
            snapshot.texts.insert(text.into(), 1);
            snapshot
                .widgets
                .insert(text.into(), vec![WidgetState::Visible]);
        }
        snapshot
    }

    #[test]
    fn a_change_names_the_full_type_path_of_the_type_it_is_about() {
        let before = snapshot("1", "0.0", 1, None);
        let after = snapshot("2", "0.1", 0, Some("Saved"));
        let changes = after.changes_from(&before);
        let paths: Vec<(&str, Option<&str>)> = changes
            .iter()
            .map(|change| (change.key.as_str(), change.type_path.as_deref()))
            .collect();
        assert_eq!(
            paths,
            [
                ("count Ghost", Some("game::Ghost")),
                ("resource Score.0", Some(GAME_SCORE)),
                ("resource Time<Virtual>.elapsed", Some(BEVY_TIME)),
                ("text \"Saved\"", None),
                ("widget \"Saved\"", None),
            ],
            "the order is that of the keys, as before"
        );
    }

    #[test]
    fn a_change_to_a_line_that_went_away_still_names_its_type() {
        let before = snapshot("1", "0.0", 1, None);
        let mut after = snapshot("1", "0.0", 1, None);
        after.counts.clear();
        after.type_paths.remove("Ghost");
        let changes = after.changes_from(&before);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes
                .first()
                .and_then(|change| change.type_path.as_deref()),
            Some("game::Ghost")
        );
    }

    #[test]
    fn a_change_to_a_text_names_no_type() {
        let before = WorldSnapshot::default();
        let mut after = WorldSnapshot::default();
        after.texts.insert("Hello".into(), 1);
        let changes = after.changes_from(&before);
        assert_eq!(changes.len(), 1);
        assert!(changes.iter().all(|change| change.type_path.is_none()));
    }

    #[test]
    fn the_last_segment_of_a_path_keeps_its_generics_whole() {
        assert_eq!(last_segment("a::b::C<d::E>"), "C<d::E>");
        assert_eq!(last_segment("Plain"), "Plain");
    }
}
