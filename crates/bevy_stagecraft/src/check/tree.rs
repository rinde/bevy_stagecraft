//! UI tree snapshots: the UI below a node as indented lines of role, label
//! and states, and a partial match of an expected tree against it.
//!
//! A snapshot reads
//!
//! ```text
//! - panel:
//!   - button "Pause"
//!   - button "Lay" [pressed]
//!   - slider "Speed" [value=0.5]
//!   - text-input "Name" [focused] = "Alice"
//! ```
//!
//! Every line is a role (`panel`, `text`, or the [`WidgetRole`] in kebab case,
//! as `menu-item`), perhaps a quoted label, and perhaps states in brackets
//! (`disabled`, `checked`, `selected`, `pressed`, `focused`, `value=..`). A text input
//! also shows the text it holds after its states, as `= "Alice"` (quoted like a
//! label, and an expected line that leaves it out matches any value). A line with
//! lines below it ends in a colon.
//!
//! An expected tree is partial: it lists only what matters. A line matches an
//! actual one with the same role, the same label (when it gives one) and at
//! least its states; the lines below it match the lines below the actual one
//! as an ordered subsequence, so siblings may be left out but not reordered.
//! Roles come from the same components as [`Target::Role`](crate::Target),
//! not from AccessKit.

use alloc::collections::{BTreeMap, BTreeSet};
use core::fmt::Write as _;

use bevy::{
    camera::visibility::InheritedVisibility,
    ecs::{
        entity::Entity,
        hierarchy::Children,
        name::Name,
        world::{EntityRef, World},
    },
    input_focus::InputFocus,
    text::{EditableText, InlineBox, TextSpan},
    ui::{
        Display, InteractionDisabled, Node, Pressed,
        widget::{InlineImage, Text},
    },
    ui_widgets::SliderValue,
};

use super::{reflect::close_names, widget::label_of};
use crate::{probe::is_within, targets::WidgetRole, targets::is_tooling, targets::is_ui_node};

/// How far each level of the tree is indented.
const INDENT: usize = 2;

/// What a line of the tree is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A node that groups others and shows no text of its own.
    Panel,
    /// A node showing text.
    Text,
    /// An image set inline in text (`InlineImage`).
    Image,
    /// Room reserved inline in text for something else (`InlineBox`).
    InlineBox,
    /// A widget of this role.
    Widget(WidgetRole),
}

impl Kind {
    /// Every kind, in the order a hint lists them.
    const ALL: [Self; 14] = [
        Self::Panel,
        Self::Text,
        Self::Image,
        Self::InlineBox,
        Self::Widget(WidgetRole::Button),
        Self::Widget(WidgetRole::Checkbox),
        Self::Widget(WidgetRole::RadioButton),
        Self::Widget(WidgetRole::Slider),
        Self::Widget(WidgetRole::MenuButton),
        Self::Widget(WidgetRole::MenuItem),
        Self::Widget(WidgetRole::ListItem),
        Self::Widget(WidgetRole::Tab),
        Self::Widget(WidgetRole::TabList),
        Self::Widget(WidgetRole::TextInput),
    ];

    /// The name a tree writes: `panel`, `text`, `button`, `menu-item`.
    const fn name(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::Text => "text",
            Self::Image => "image",
            Self::InlineBox => "inline-box",
            Self::Widget(role) => match role {
                WidgetRole::Checkbox => "checkbox",
                WidgetRole::RadioButton => "radio",
                WidgetRole::Slider => "slider",
                WidgetRole::MenuButton => "menu-button",
                WidgetRole::MenuItem => "menu-item",
                WidgetRole::ListItem => "list-item",
                WidgetRole::Tab => "tab",
                WidgetRole::TabList => "tab-list",
                WidgetRole::TextInput => "text-input",
                WidgetRole::Button => "button",
            },
        }
    }

    /// The kind a tree calls `name`.
    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// A state a line can show, in the order a snapshot writes them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum State {
    /// `disabled`.
    Disabled,
    /// `checked`.
    Checked,
    /// `selected`.
    Selected,
    /// `pressed`.
    Pressed,
    /// `focused`.
    Focused,
    /// `value=0.5`, as the text after the equals sign.
    Value(String),
}

impl State {
    /// The state as it is written inside brackets.
    fn text(&self) -> String {
        match self {
            Self::Disabled => "disabled".into(),
            Self::Checked => "checked".into(),
            Self::Selected => "selected".into(),
            Self::Pressed => "pressed".into(),
            Self::Focused => "focused".into(),
            Self::Value(value) => format!("value={value}"),
        }
    }

    /// The state written as `text`, or what is wrong with it.
    fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "disabled" => Ok(Self::Disabled),
            "checked" => Ok(Self::Checked),
            "selected" => Ok(Self::Selected),
            "pressed" => Ok(Self::Pressed),
            "focused" => Ok(Self::Focused),
            other => match other.strip_prefix("value=") {
                Some(value) if !value.trim().is_empty() => Ok(Self::Value(value.trim().into())),
                _ => Err(format!(
                    "`{other}` is not a state; write disabled, checked, selected, pressed, focused \
                     or value=<number>"
                )),
            },
        }
    }

    /// Whether this expected state is the actual one `held`: values compare
    /// as numbers where both are, so `0.50` is `0.5`.
    fn is_held_by(&self, held: &Self) -> bool {
        match (self, held) {
            (Self::Value(wanted), Self::Value(actual)) => {
                match (wanted.parse::<f64>(), actual.parse::<f64>()) {
                    (Ok(wanted), Ok(actual)) => (wanted - actual).abs() < f64::from(f32::EPSILON),
                    _ => wanted == actual,
                }
            }
            _ => self == held,
        }
    }
}

/// Appends `text` to `out` in double quotes, escaping `"`, `\\` and newlines.
fn push_quoted(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        if matches!(character, '"' | '\\') {
            out.push('\\');
        }
        if character == '\n' {
            out.push_str("\\n");
        } else {
            out.push(character);
        }
    }
    out.push('"');
}

/// One line of a tree and the lines below it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    /// What it is.
    kind: Kind,
    /// The label: the text of a text node or widget, or a node's name.
    label: Option<String>,
    /// Its states.
    states: Vec<State>,
    /// The text a text input holds.
    value: Option<String>,
    /// The lines below it, in document order.
    children: Vec<Self>,
}

impl Line {
    /// The line without what lies below it: `button "Lay" [pressed]`, with
    /// the colon that says lines follow when `colon` is set.
    fn head(&self, colon: bool) -> String {
        let mut head = self.kind.name().to_owned();
        if let Some(label) = &self.label {
            head.push(' ');
            push_quoted(&mut head, label);
        }
        for state in &self.states {
            // Writing to a `String` cannot fail.
            let _ = write!(head, " [{}]", state.text());
        }
        if let Some(value) = &self.value {
            head.push_str(" = ");
            push_quoted(&mut head, value);
        }
        if colon {
            head.push(':');
        }
        head
    }

    /// Whether this expected line is satisfied by `actual` as far as its own
    /// head goes: role, label if it gives one, and every state it names.
    fn head_matches(&self, actual: &Self) -> bool {
        self.kind == actual.kind
            && self
                .label
                .as_ref()
                .is_none_or(|label| actual.label.as_ref() == Some(label))
            && self
                .states
                .iter()
                .all(|wanted| actual.states.iter().any(|held| wanted.is_held_by(held)))
            && self
                .value
                .as_ref()
                .is_none_or(|value| actual.value.as_ref() == Some(value))
    }

    /// Whether this expected line and everything below it is satisfied by
    /// `actual`: the lines below as an ordered subsequence, each taking the
    /// first actual line after the last one used that satisfies it.
    ///
    /// Uncached on purpose: one match asks about each pair of lines at most
    /// once, so remembering answers would only slow down the common case, a
    /// tree that matches. [`Matcher`] is for the diff.
    fn matches(&self, actual: &Self) -> bool {
        if !self.head_matches(actual) {
            return false;
        }
        let mut cursor = 0;
        for wanted in &self.children {
            let rest = actual.children.get(cursor..).unwrap_or_default();
            let Some(found) = rest.iter().position(|candidate| wanted.matches(candidate)) else {
                return false;
            };
            cursor += found + 1;
        }
        true
    }
}

/// Answers "does this expected line match that actual one" once per pair,
/// for [`render_diff`].
///
/// One call of [`Line::matches`] asks about each pair at most once, but the
/// diff asks again at every level it descends through, so a mismatch deep in
/// a tree costs its depth times the size of the tree (see
/// `a_deep_mismatch_matches_each_pair_once`). A result depends on the pair
/// alone, so remembering it keeps the semantics and makes the whole diff cost
/// about one match. It is not used for the first pass: a map lookup costs
/// more than comparing one head, so there it only slowed the common case, a
/// tree that matches.
///
/// Measured with `tree_matching_timings` (a mismatch, first pass and diff,
/// test profile; `spine N deep, W wide` is a chain of N levels each holding W
/// decoys before the next level):
///
/// | shape                         | before  | after   |
/// | ----------------------------- | ------- | ------- |
/// | spine 10 deep, 3 wide         | 5.9 us  | 7.2 us  |
/// | spine 100 deep, 5 wide        | 302 us  | 150 us  |
/// | spine 400 deep, 5 wide        | 4.5 ms  | 0.67 ms |
/// | spine 400 deep, 50 wide       | 28 ms   | 4.3 ms  |
/// | spine 1000 deep, 5 wide       | 16.5 ms | 1.3 ms  |
/// | flat, 1000 siblings           | 25 us   | 286 us  |
///
/// The diff drops from quadratic in the depth to linear, and a wide flat
/// level pays for filling the map without gaining anything; both are well
/// under a frame, and a tree that matches is as fast as before.
#[derive(Debug, Default)]
struct Matcher {
    /// The answer for a pair, keyed by the addresses of the two lines. Both
    /// trees outlive the matcher and are not moved while it runs, so an
    /// address names one line. Only looked up, never iterated.
    seen: BTreeMap<(usize, usize), bool>,
    /// How many pairs were worked out rather than remembered.
    #[cfg(test)]
    computed: usize,
    /// Remember nothing: the behaviour before the cache, for comparison.
    #[cfg(test)]
    uncached: bool,
}

impl Matcher {
    /// Whether `expected` and everything below it is satisfied by `actual`.
    fn matches(&mut self, expected: &Line, actual: &Line) -> bool {
        let key = (
            core::ptr::from_ref(expected).addr(),
            core::ptr::from_ref(actual).addr(),
        );
        #[cfg(test)]
        if self.uncached {
            self.computed += 1;
            return self.compute(expected, actual);
        }
        if let Some(known) = self.seen.get(&key) {
            return *known;
        }
        #[cfg(test)]
        {
            self.computed += 1;
        }
        let result = self.compute(expected, actual);
        self.seen.insert(key, result);
        result
    }

    /// The uncached answer of [`Self::matches`].
    fn compute(&mut self, expected: &Line, actual: &Line) -> bool {
        if !expected.head_matches(actual) {
            return false;
        }
        let mut cursor = 0;
        for wanted in &expected.children {
            let rest = actual.children.get(cursor..).unwrap_or_default();
            let Some(found) = rest
                .iter()
                .position(|candidate| self.matches(wanted, candidate))
            else {
                return false;
            };
            cursor += found + 1;
        }
        true
    }
}

/// The tree of the UI at and below `root`.
pub(super) fn snapshot(world: &World, root: Entity) -> String {
    let mut text = String::new();
    write_lines(&root_line(world, root), 0, &mut text);
    text.trim_end().to_owned()
}

/// Writes `line` and what is below it as the tree is written in a file.
fn write_lines(line: &Line, depth: usize, out: &mut String) {
    let _ = writeln!(
        out,
        "{}- {}",
        " ".repeat(depth * INDENT),
        line.head(!line.children.is_empty())
    );
    for child in &line.children {
        write_lines(child, depth + 1, out);
    }
}

/// The line for `root`, which is a line whatever it is: a root that is no
/// node, or is hidden, is still what was asked for.
fn root_line(world: &World, root: Entity) -> Line {
    lines_of(world, root, false, true).pop().unwrap_or(Line {
        kind: Kind::Panel,
        label: None,
        states: Vec::new(),
        value: None,
        children: Vec::new(),
    })
}

/// The lines `entity` stands for: its own, or for an entity that is no UI
/// node at all (a bare grouping entity) the lines of what is below it.
fn lines_of(world: &World, entity: Entity, below_widget: bool, is_root: bool) -> Vec<Line> {
    let Ok(this) = world.get_entity(entity) else {
        return Vec::new();
    };
    if !is_root && (is_tooling(world, entity) || is_hidden(&this)) {
        return Vec::new();
    }
    if let Some(line) = inline_line(&this) {
        return vec![line];
    }
    let is_text = this.contains::<Text>();
    let is_node = is_ui_node(world, entity);
    if !is_root && !is_node && !is_text {
        // A span is part of the text above it, so only what sits inline
        // below it shows; anything else groups.
        if this.contains::<TextSpan>() {
            return inline_lines(world, entity);
        }
        return children_of(world, entity, below_widget);
    }
    let role = WidgetRole::of(&this);
    let kind = match (role, is_text) {
        (Some(role), _) => Kind::Widget(role),
        (None, true) => Kind::Text,
        (None, false) => Kind::Panel,
    };
    if below_widget && kind == Kind::Text {
        // The widget's label already has this text, but not what sits
        // inline in it.
        return inline_lines(world, entity);
    }
    let states = if matches!(kind, Kind::Widget(_)) {
        states_of(world, &this)
    } else {
        Vec::new()
    };
    let label = match kind {
        Kind::Text | Kind::Widget(_) => {
            Some(label_of(world, entity)).filter(|text| !text.is_empty())
        }
        Kind::Panel | Kind::Image | Kind::InlineBox => None,
    }
    .or_else(|| this.get::<Name>().map(|name| name.as_str().to_owned()));
    let value = (kind == Kind::Widget(WidgetRole::TextInput))
        .then(|| {
            this.get::<EditableText>()
                .map(|text| text.value().to_string())
        })
        .flatten();
    let children = if kind == Kind::Text {
        inline_lines(world, entity)
    } else {
        children_of(
            world,
            entity,
            below_widget || matches!(kind, Kind::Widget(_)),
        )
    };
    if !is_root && kind == Kind::Panel && label.is_none() && children.is_empty() {
        return Vec::new();
    }
    vec![Line {
        kind,
        label,
        states,
        value,
        children,
    }]
}

/// The line for an inline image or box, or `None` for anything else. Nothing
/// inside it is read: Bevy lays out no text below an inline box.
fn inline_line(entity: &EntityRef<'_>) -> Option<Line> {
    let kind = if entity.contains::<InlineImage>() {
        Kind::Image
    } else if entity.contains::<InlineBox>() {
        Kind::InlineBox
    } else {
        return None;
    };
    Some(Line {
        kind,
        label: None,
        states: Vec::new(),
        value: None,
        children: Vec::new(),
    })
}

/// The lines of the images and boxes set inline in the text of `entity`, in
/// order, found through its spans.
fn inline_lines(world: &World, entity: Entity) -> Vec<Line> {
    let Some(children) = world.get::<Children>(entity) else {
        return Vec::new();
    };
    children
        .iter()
        .flat_map(|child| {
            let Ok(this) = world.get_entity(*child) else {
                return Vec::new();
            };
            if is_tooling(world, *child) || is_hidden(&this) {
                Vec::new()
            } else if let Some(line) = inline_line(&this) {
                vec![line]
            } else if this.contains::<TextSpan>() {
                inline_lines(world, *child)
            } else {
                Vec::new()
            }
        })
        .collect()
}

/// The lines of everything directly below `entity`, in order.
fn children_of(world: &World, entity: Entity, below_widget: bool) -> Vec<Line> {
    world
        .get::<Children>(entity)
        .map(|children| {
            children
                .iter()
                .flat_map(|child| lines_of(world, *child, below_widget, false))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether the node is not on screen: not displayed, or hidden in the
/// hierarchy.
fn is_hidden(entity: &EntityRef<'_>) -> bool {
    entity
        .get::<Node>()
        .is_some_and(|node| node.display == Display::None)
        || entity
            .get::<InheritedVisibility>()
            .is_some_and(|visibility| !visibility.get())
}

/// The states of the widget `entity`, in the order they are written.
fn states_of(world: &World, entity: &EntityRef<'_>) -> Vec<State> {
    let mut states = BTreeSet::new();
    if entity.contains::<InteractionDisabled>() {
        states.insert(State::Disabled);
    }
    if entity.contains::<bevy::ui::Checked>() {
        states.insert(State::Checked);
    }
    if entity.contains::<bevy::ui::Selected>() {
        states.insert(State::Selected);
    }
    if entity.contains::<Pressed>() {
        states.insert(State::Pressed);
    }
    if world
        .get_resource::<InputFocus>()
        .and_then(InputFocus::get)
        .is_some_and(|focus| is_within(world, focus, entity.id()))
    {
        states.insert(State::Focused);
    }
    if let Some(value) = entity.get::<SliderValue>() {
        states.insert(State::Value(value.0.to_string()));
    }
    states.into_iter().collect()
}

/// Reads an expected tree: the lines `- role "label" [state]`, indented to
/// show what is below what, with one root.
pub(super) fn parse(text: &str) -> Result<Tree, String> {
    let mut stack: Vec<(usize, Line)> = Vec::new();
    let mut root: Option<Line> = None;
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        if raw.trim().is_empty() {
            continue;
        }
        let at = |message: String| format!("line {number}: {message}");
        if raw.contains('\t') {
            return Err(at("indent with spaces, not tabs".into()));
        }
        let indent = raw.len() - raw.trim_start().len();
        let line = parse_line(raw.trim()).map_err(at)?;
        let mut popped: Option<usize> = None;
        while stack.last().is_some_and(|(top, _)| *top >= indent) {
            let Some((top, line)) = stack.pop() else {
                break;
            };
            popped = Some(top);
            match stack.last_mut() {
                Some((_, parent)) => parent.children.push(line),
                None => root = Some(line),
            }
        }
        if popped.is_some_and(|top| top != indent) {
            return Err(at(
                "the indentation does not line up with the lines above".into()
            ));
        }
        if stack.is_empty() && root.is_some() {
            return Err(at(
                "a tree has one root: indent this line below another, or write a separate check"
                    .into(),
            ));
        }
        stack.push((indent, line));
    }
    while let Some((_, line)) = stack.pop() {
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(line),
            None => root = Some(line),
        }
    }
    root.map(Tree)
        .ok_or_else(|| "the tree is empty: write at least `- panel`".into())
}

/// A tree read from text, ready to be matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Tree(Line);

impl Tree {
    /// Matches the tree against the snapshot of the UI at and below `root`:
    /// nothing when it matches, the diff otherwise.
    pub(super) fn compare(&self, world: &World, root: Entity) -> Result<(), String> {
        let actual = root_line(world, root);
        if self.0.matches(&actual) {
            return Ok(());
        }
        let mut diff = String::new();
        render_diff(&mut Matcher::default(), &self.0, &actual, 0, &mut diff);
        Err(diff.trim_end().to_owned())
    }
}

/// Reads one `- role "label" [state] [state]:` line, already trimmed.
fn parse_line(text: &str) -> Result<Line, String> {
    let rest = text
        .strip_prefix('-')
        .ok_or("a line starts with `- `, as in `- button \"Pause\"`")?
        .trim_start();
    let rest = rest.strip_suffix(':').map_or(rest, str::trim_end);
    let (name, mut rest) = rest
        .split_once(char::is_whitespace)
        .map_or((rest, ""), |(name, rest)| (name, rest.trim_start()));
    let kind = Kind::from_name(name).ok_or_else(|| {
        let known: Vec<&str> = Kind::ALL.into_iter().map(Kind::name).collect();
        let close = close_names(name, known.iter().copied());
        let hint = if close.is_empty() {
            String::new()
        } else {
            format!("; did you mean {}?", close.join(" or "))
        };
        format!("`{name}` is not a role{hint} (roles: {})", known.join(", "))
    })?;
    let mut label = None;
    if let Some(after) = rest.strip_prefix('"') {
        let (text, after) = read_quoted(after)?;
        label = Some(text);
        rest = after.trim_start();
    }
    let mut states = Vec::new();
    while let Some(after) = rest.strip_prefix('[') {
        let (inside, after) = after
            .split_once(']')
            .ok_or("a state in brackets is missing its `]`")?;
        for state in inside.split(',') {
            states.push(State::parse(state)?);
        }
        rest = after.trim_start();
    }
    let mut value = None;
    if let Some(after) = rest.strip_prefix('=') {
        let after = after
            .trim_start()
            .strip_prefix('"')
            .ok_or("a value is quoted: `= \"text\"`")?;
        let (text, after) = read_quoted(after)?;
        value = Some(text);
        rest = after.trim_start();
    }
    if !rest.is_empty() {
        return Err(format!(
            "`{rest}` is not understood after the role; write a quoted label, then states in \
             brackets, then `= \"value\"`"
        ));
    }
    Ok(Line {
        kind,
        label,
        states,
        value,
        children: Vec::new(),
    })
}

/// Reads the rest of a quoted label, after its opening quote: the label with
/// escapes undone and what follows the closing quote.
fn read_quoted(text: &str) -> Result<(String, &str), String> {
    let mut label = String::new();
    let mut characters = text.char_indices();
    while let Some((at, character)) = characters.next() {
        match character {
            '"' => return Ok((label, text.get(at + 1..).unwrap_or_default())),
            '\\' => match characters.next() {
                Some((_, 'n')) => label.push('\n'),
                Some((_, escaped @ ('"' | '\\'))) => label.push(escaped),
                _ => return Err("a label escapes only `\\\"`, `\\\\` and `\\n`".into()),
            },
            other => label.push(other),
        }
    }
    Err("a label is missing its closing quote".into())
}

/// Writes the diff of an expected line against the actual one it was matched
/// with: context for what holds, `-` for what was expected and `+` for what
/// is there.
fn render_diff(
    matcher: &mut Matcher,
    expected: &Line,
    actual: &Line,
    depth: usize,
    out: &mut String,
) {
    if !expected.head_matches(actual) {
        write_expected(expected, depth, '-', out);
        let _ = writeln!(
            out,
            "+ {}- {}",
            " ".repeat(depth * INDENT),
            actual.head(!actual.children.is_empty())
        );
        return;
    }
    let _ = writeln!(
        out,
        "  {}- {}",
        " ".repeat(depth * INDENT),
        expected.head(!expected.children.is_empty())
    );
    let mut cursor = 0;
    for wanted in &expected.children {
        let rest = actual.children.get(cursor..).unwrap_or_default();
        if let Some(found) = rest
            .iter()
            .position(|candidate| matcher.matches(wanted, candidate))
        {
            write_expected(wanted, depth + 1, ' ', out);
            cursor += found + 1;
        } else if let Some(found) = rest
            .iter()
            .position(|candidate| candidate.kind == wanted.kind)
        {
            if let Some(candidate) = rest.get(found) {
                render_diff(matcher, wanted, candidate, depth + 1, out);
            }
            cursor += found + 1;
        } else {
            write_expected(wanted, depth + 1, '-', out);
        }
    }
}

/// Writes `line` and what is below it, every line marked with `marker`.
fn write_expected(line: &Line, depth: usize, marker: char, out: &mut String) {
    let _ = writeln!(
        out,
        "{marker} {}- {}",
        " ".repeat(depth * INDENT),
        line.head(!line.children.is_empty())
    );
    for child in &line.children {
        write_expected(child, depth + 1, marker, out);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may panic")]

    use super::*;

    #[test]
    fn a_tree_is_read_with_its_labels_states_and_depth() {
        let tree = parse(
            "- panel:\n  - button \"Lay\" [pressed, disabled]\n  - slider \"Speed\" [value=0.5]\n",
        )
        .unwrap();
        let Tree(root) = tree;
        assert_eq!(root.kind, Kind::Panel);
        assert_eq!(root.children.len(), 2);
        let button = root.children.first().unwrap();
        assert_eq!(button.label.as_deref(), Some("Lay"));
        assert_eq!(button.states, [State::Pressed, State::Disabled]);
    }

    #[test]
    fn a_label_with_quotes_and_backslashes_survives_writing_and_reading() {
        let line = Line {
            kind: Kind::Text,
            label: Some("say \"hi\"\\\nnow".into()),
            states: Vec::new(),
            value: None,
            children: Vec::new(),
        };
        let written = line.head(false);
        assert_eq!(parse_line(&format!("- {written}")).unwrap(), line);
    }

    #[test]
    fn a_tree_that_cannot_be_read_says_which_line_and_why() {
        let cases = [
            ("- panel\n- panel", "line 2: a tree has one root"),
            ("- buton \"x\"", "did you mean button"),
            (
                "- panel:\n   - button\n  - button",
                "line 3: the indentation",
            ),
            ("- panel:\n\t- button", "line 2: indent with spaces"),
            ("- button \"x\" [flying]", "`flying` is not a state"),
            ("- button \"x", "missing its closing quote"),
            ("button", "starts with `- `"),
            ("", "the tree is empty"),
        ];
        for (text, expected) in cases {
            let message = parse(text).unwrap_err();
            assert!(message.contains(expected), "{text:?}: {message}");
        }
    }

    #[test]
    fn values_compare_as_numbers() {
        assert!(State::Value("0.50".into()).is_held_by(&State::Value("0.5".into())));
        assert!(!State::Value("0.6".into()).is_held_by(&State::Value("0.5".into())));
    }

    /// A line with a label and children.
    fn node(label: &str, children: Vec<Line>) -> Line {
        Line {
            kind: Kind::Panel,
            label: Some(label.into()),
            states: Vec::new(),
            value: None,
            children,
        }
    }

    /// A spine `depth` deep. Every level holds `width` text decoys, of another
    /// kind than the expected panels, and then the next level; the bottom is
    /// the panel "a".
    fn spine(depth: usize, width: usize) -> Line {
        (0..depth).fold(node("a", Vec::new()), |below, _| {
            let mut children: Vec<Line> = (0..width)
                .map(|_| Line {
                    kind: Kind::Text,
                    ..node("p", Vec::new())
                })
                .collect();
            children.push(below);
            node("p", children)
        })
    }

    /// The expected chain of `depth` panels down to "b", which `spine` lacks.
    fn wanted_chain(depth: usize) -> Line {
        (0..depth).fold(node("b", Vec::new()), |below, _| node("p", vec![below]))
    }

    #[test]
    fn matching_keeps_the_ordered_partial_semantics() {
        let actual = node(
            "r",
            vec![
                node("a", Vec::new()),
                node("b", vec![node("x", Vec::new())]),
                node("c", Vec::new()),
            ],
        );
        let wants = |labels: &[&str]| {
            node(
                "r",
                labels.iter().map(|label| node(label, Vec::new())).collect(),
            )
        };
        assert!(wants(&["a", "c"]).matches(&actual));
        assert!(wants(&["b"]).matches(&actual));
        assert!(!wants(&["c", "a"]).matches(&actual), "order must hold");
        assert!(!wants(&["a", "a"]).matches(&actual), "a line is used once");
        assert!(!wants(&["z"]).matches(&actual));
    }

    #[test]
    fn a_cached_answer_for_a_pair_is_the_uncached_one() {
        for (depth, width) in [(1, 0), (3, 2), (5, 3)] {
            let actual = spine(depth, width);
            for expected in [wanted_chain(depth), wanted_chain(depth - 1)] {
                let mut cached = Matcher::default();
                let mut plain = Matcher {
                    uncached: true,
                    ..Matcher::default()
                };
                assert_eq!(
                    cached.matches(&expected, &actual),
                    plain.matches(&expected, &actual)
                );
                let (mut with, mut without) = (String::new(), String::new());
                render_diff(&mut cached, &expected, &actual, 0, &mut with);
                render_diff(&mut plain, &expected, &actual, 0, &mut without);
                assert_eq!(with, without);
            }
        }
    }

    #[test]
    fn the_diff_of_a_deep_mismatch_points_at_the_first_of_the_right_kind() {
        let mut diff = String::new();
        render_diff(
            &mut Matcher::default(),
            &wanted_chain(2),
            &spine(2, 1),
            0,
            &mut diff,
        );
        assert!(diff.contains("-     - panel \"b\""), "{diff}");
        assert!(diff.contains("+     - panel \"a\""), "{diff}");
        assert!(!diff.contains("text"), "{diff}");
    }

    #[test]
    fn a_deep_mismatch_matches_each_pair_once() {
        let (depth, width) = (200, 5);
        let (expected, actual) = (wanted_chain(depth), spine(depth, width));
        let mut cached = Matcher::default();
        render_diff(&mut cached, &expected, &actual, 0, &mut String::new());
        let mut plain = Matcher {
            uncached: true,
            ..Matcher::default()
        };
        render_diff(&mut plain, &expected, &actual, 0, &mut String::new());
        // One pair per level and decoy at most, where asking again at every
        // level cost depth times more.
        assert!(
            cached.computed <= depth * (width + 2),
            "{} pairs worked out",
            cached.computed
        );
        assert!(
            plain.computed > 10 * cached.computed,
            "{} against {}",
            plain.computed,
            cached.computed
        );
    }

    /// Run by hand for the numbers in the pull request that added `Matcher`:
    /// `cargo test -p bevy_stagecraft tree_matching_timings -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    #[expect(clippy::print_stdout, reason = "the measurement is the output")]
    fn tree_matching_timings() {
        use std::time::Instant;
        /// Runs averaged per shape.
        const REPEATS: u32 = 20;
        // The whole of `Tree::compare` on a mismatch: the first pass, then the diff.
        let time = |expected: &Line, actual: &Line, uncached: bool| {
            let start = Instant::now();
            let mut pairs = 0;
            for _ in 0..REPEATS {
                let mut matcher = Matcher {
                    uncached,
                    ..Matcher::default()
                };
                expected.matches(actual);
                render_diff(&mut matcher, expected, actual, 0, &mut String::new());
                pairs = matcher.computed;
            }
            (start.elapsed() / REPEATS, pairs)
        };
        let flat = |count: usize| node("r", (0..count).map(|_| node("p", Vec::new())).collect());
        let shapes = [
            (
                "flat, 1000 siblings, one wanted",
                node("r", vec![node("z", Vec::new())]),
                flat(1000),
            ),
            ("spine 10 deep, 3 wide", wanted_chain(10), spine(10, 3)),
            ("spine 100 deep, 5 wide", wanted_chain(100), spine(100, 5)),
            ("spine 400 deep, 5 wide", wanted_chain(400), spine(400, 5)),
            ("spine 400 deep, 50 wide", wanted_chain(400), spine(400, 50)),
            (
                "spine 1000 deep, 5 wide",
                wanted_chain(1000),
                spine(1000, 5),
            ),
        ];
        for (name, expected, actual) in &shapes {
            let (before, before_pairs) = time(expected, actual, true);
            let (after, after_pairs) = time(expected, actual, false);
            std::println!(
                "{name}: before {before:?} ({before_pairs} pairs in the diff), after {after:?} ({after_pairs})"
            );
        }
    }
}
