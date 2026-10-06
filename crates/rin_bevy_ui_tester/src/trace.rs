//! What the world looked like around each step of a run, for finding out why
//! a scenario failed.
//!
//! A [`ScenarioTrace`] is switched on with [`ScenarioPlayer::enable_trace`]
//! (the recorder does so for every scenario). It holds a [`TracePoint`] each
//! time the player is between steps: a [`WorldSnapshot`] (the same one
//! suggestions are made from), the UI tree, and the frame it was taken on. The
//! point is taken in `Last`, after the frame has been played, which is also
//! when the recorder asks for its picture: a point and the picture of its
//! frame show the same moment.
//!
//! Steps that take no frame (checks, captions) share a point with the step
//! before them, so the world before and after such a step is the same one.
//! [`ScenarioTrace::before`] and [`ScenarioTrace::after`] find the points
//! around a step.
//!
//! [`ScenarioPlayer::enable_trace`]: crate::ScenarioPlayer::enable_trace

use alloc::collections::BTreeMap;
use core::fmt::Write as _;

use bevy::{
    ecs::{
        entity::Entity,
        hierarchy::{ChildOf, Children},
        name::Name,
        query::{With, Without},
        world::World,
    },
    ui::{Node, widget::Text},
};

use crate::{
    check::suggest::{NoiseFilter, WorldSnapshot, state_names},
    player::Outcome,
    targets::is_tooling,
};

/// How many levels of the UI tree a trace point writes down; deeper nodes are
/// left out, so a runaway hierarchy cannot make a trace huge.
const MAX_TREE_DEPTH: usize = 24;

/// One node of the UI tree, in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiLine {
    /// How far below a root the node is.
    pub depth: usize,
    /// The node's `Name`, or `Node` where it has none.
    pub label: String,
    /// The text the node shows itself, if any.
    pub text: Option<String>,
    /// The widget states it is in (`Disabled`, `Checked`, `Pressed`,
    /// `Focused`, `Visible`); empty for a node that is no widget.
    pub states: Vec<String>,
}

impl UiLine {
    /// The line as text: the label, the text in quotes, and the states.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut out = self.label.clone();
        if let Some(text) = &self.text {
            let _ = write!(out, " {text:?}");
        }
        if !self.states.is_empty() {
            let _ = write!(out, " [{}]", self.states.join(", "));
        }
        out
    }
}

/// The world between two steps.
#[derive(Debug, Clone, PartialEq)]
pub struct TracePoint {
    /// The player frame it was taken after: the same number the recorder
    /// names the picture of that frame by.
    pub frame: u32,
    /// How many steps had begun, so the next to run is the step with this
    /// index.
    pub started: usize,
    /// Whether the scenario, with its expectations, was over.
    pub finished: bool,
    /// The reflected state: resources, counts, texts, widgets, facts.
    pub snapshot: WorldSnapshot,
    /// The app's UI, as a tree, tooling left out.
    pub ui: Vec<UiLine>,
}

impl TracePoint {
    /// Reads `world` now.
    #[must_use]
    pub fn take(
        world: &World,
        filter: &NoiseFilter,
        frame: u32,
        started: usize,
        finished: bool,
    ) -> Self {
        Self {
            frame,
            started,
            finished,
            snapshot: WorldSnapshot::take(world, filter),
            ui: ui_tree(world),
        }
    }
}

/// What one target of a step turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetTrace {
    /// The target as the script wrote it.
    pub target: String,
    /// Where it was found (the last time the player looked), or why not.
    pub found: Result<String, String>,
}

/// A check that ran for a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceCheck {
    /// The check, in words.
    pub name: String,
    /// How it ended.
    pub outcome: Outcome,
}

/// What the player recorded about one step apart from the world around it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepTrace {
    /// The targets the step looked up, in the order it looked.
    pub targets: Vec<TargetTrace>,
    /// The checks the step ran.
    pub checks: Vec<TraceCheck>,
}

/// The trace of one run: see the [module documentation](self).
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioTrace {
    /// Which reflected types the snapshots leave out.
    filter: NoiseFilter,
    /// The points, in the order they were taken.
    points: Vec<TracePoint>,
    /// What was recorded per step, by index.
    steps: BTreeMap<usize, StepTrace>,
}

impl ScenarioTrace {
    /// An empty trace whose snapshots leave out what `filter` ignores.
    #[must_use]
    pub const fn new(filter: NoiseFilter) -> Self {
        Self {
            filter,
            points: Vec::new(),
            steps: BTreeMap::new(),
        }
    }

    /// The filter snapshots are taken with.
    #[must_use]
    pub const fn filter(&self) -> &NoiseFilter {
        &self.filter
    }

    /// The points, in the order they were taken.
    #[must_use]
    pub fn points(&self) -> &[TracePoint] {
        &self.points
    }

    /// The last point taken.
    #[must_use]
    pub fn last(&self) -> Option<&TracePoint> {
        self.points.last()
    }

    /// Adds a point.
    pub(crate) fn push(&mut self, point: TracePoint) {
        self.points.push(point);
    }

    /// What was recorded for step `index`, if it began.
    #[must_use]
    pub fn step(&self, index: usize) -> Option<&StepTrace> {
        self.steps.get(&index)
    }

    /// The world as step `index` found it: the last point taken before the
    /// step began.
    #[must_use]
    pub fn before(&self, index: usize) -> Option<&TracePoint> {
        self.points
            .iter()
            .rev()
            .find(|point| point.started <= index)
    }

    /// The world once step `index` was done: the first point at which a later
    /// step was due, or the end. `None` while the step is still going.
    #[must_use]
    pub fn after(&self, index: usize) -> Option<&TracePoint> {
        self.points
            .iter()
            .find(|point| point.started > index || point.finished)
    }

    /// Remembers what step `index` found for `target`, replacing what it found
    /// the last time it looked, so a step that waits keeps its latest answer.
    pub(crate) fn add_target(&mut self, index: usize, trace: TargetTrace) {
        let step = self.steps.entry(index).or_default();
        match step
            .targets
            .iter_mut()
            .find(|known| known.target == trace.target)
        {
            Some(known) => *known = trace,
            None => step.targets.push(trace),
        }
    }

    /// Remembers a check step `index` ran.
    pub(crate) fn add_check(&mut self, index: usize, check: TraceCheck) {
        self.steps.entry(index).or_default().checks.push(check);
    }
}

/// The app's UI nodes as a flat list in document order, tooling left out.
fn ui_tree(world: &World) -> Vec<UiLine> {
    let Some(mut roots) = world
        .try_query_filtered::<Entity, (With<Node>, Without<ChildOf>)>()
        .map(|query| query.iter_manual(world).collect::<Vec<_>>())
    else {
        return Vec::new();
    };
    roots.sort();
    let mut lines = Vec::new();
    for root in roots {
        walk(world, root, 0, &mut lines);
    }
    lines
}

/// Adds `entity` and, below it, its UI children to `lines`.
fn walk(world: &World, entity: Entity, depth: usize, lines: &mut Vec<UiLine>) {
    if depth > MAX_TREE_DEPTH || is_tooling(world, entity) || world.get::<Node>(entity).is_none() {
        return;
    }
    lines.push(UiLine {
        depth,
        label: world
            .get::<Name>(entity)
            .map_or_else(|| "Node".into(), |name| name.as_str().into()),
        text: world.get::<Text>(entity).map(|text| text.0.clone()),
        states: state_names(world, entity),
    });
    if let Some(children) = world.get::<Children>(entity) {
        for child in children {
            walk(world, *child, depth + 1, lines);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A point at `frame` with `started` steps begun.
    fn point(frame: u32, started: usize, finished: bool) -> TracePoint {
        TracePoint {
            frame,
            started,
            finished,
            snapshot: WorldSnapshot::default(),
            ui: Vec::new(),
        }
    }

    /// Points for a three-step script whose middle step takes no frame.
    fn trace() -> ScenarioTrace {
        let mut trace = ScenarioTrace::new(NoiseFilter::default());
        trace.push(point(0, 0, false));
        trace.push(point(3, 1, false));
        trace.push(point(5, 3, false));
        trace.push(point(5, 3, true));
        trace
    }

    #[test]
    fn the_world_before_a_step_is_the_last_point_before_it_began() {
        let trace = trace();
        assert_eq!(trace.before(0).map(|p| p.frame), Some(0));
        assert_eq!(trace.before(1).map(|p| p.frame), Some(3));
        assert_eq!(
            trace.before(2).map(|p| p.frame),
            Some(3),
            "a step with no frame before it shares the point of the step before"
        );
    }

    #[test]
    fn the_world_after_a_step_is_the_first_point_at_which_a_later_step_was_due() {
        let trace = trace();
        assert_eq!(trace.after(0).map(|p| p.frame), Some(3));
        assert_eq!(
            trace.after(1).map(|p| p.frame),
            Some(5),
            "a zero-frame step ends where the next one is due"
        );
        assert_eq!(trace.after(2).map(|p| p.frame), Some(5));
    }

    #[test]
    fn a_step_still_going_has_no_world_after_it() {
        let mut trace = ScenarioTrace::new(NoiseFilter::default());
        trace.push(point(0, 0, false));
        assert!(trace.after(0).is_none());
    }

    #[test]
    fn looking_a_target_up_again_replaces_the_answer() {
        let mut trace = ScenarioTrace::new(NoiseFilter::default());
        let target = |found: Result<String, String>| TargetTrace {
            target: "Text(\"Save\")".into(),
            found,
        };
        trace.add_target(2, target(Err("not there yet".into())));
        trace.add_target(2, target(Ok("at (10, 20)".into())));
        let step = trace.step(2).map(|step| step.targets.clone());
        assert_eq!(step, Some(vec![target(Ok("at (10, 20)".into()))]));
    }

    #[test]
    fn a_ui_line_reads_as_label_text_and_states() {
        let line = UiLine {
            depth: 1,
            label: "Node".into(),
            text: Some("Save".into()),
            states: vec!["Visible".into()],
        };
        assert_eq!(line.describe(), "Node \"Save\" [Visible]");
    }
}
