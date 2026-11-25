//! What the world looked like around each step of a run.
//!
//! Points are taken in `Last`, when the recorder takes its picture; steps that take no frame share a point.

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
    check::suggest::{NoiseFilter, SnapshotChange, WorldSnapshot, state_names},
    overlay::{TargetDescriptions, describe_step},
    player::{Outcome, StepRecord},
    scenario::{Step, with_group_path},
    targets::{is_tooling, is_ui_node},
};

/// Deeper nodes are dropped so a runaway hierarchy cannot make a trace huge.
const MAX_TREE_DEPTH: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiLine {
    pub depth: usize,
    pub label: String,
    pub text: Option<String>,
    pub states: Vec<String>,
}

impl UiLine {
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

#[derive(Debug, Clone, PartialEq)]
pub struct TracePoint {
    /// The player frame it was taken after, which names the recorder's picture of it.
    pub frame: u32,
    /// Steps begun, so the next to run has this index.
    pub started: usize,
    pub finished: bool,
    pub snapshot: WorldSnapshot,
    pub ui: Vec<UiLine>,
}

impl TracePoint {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetTrace {
    pub target: String,
    pub found: Result<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceCheck {
    pub name: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceCaptured {
    pub name: String,
    pub measured: String,
    pub value: String,
    pub type_name: String,
}

impl TraceCaptured {
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "{:?} = {} ({}), {}",
            self.name, self.value, self.type_name, self.measured
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepTrace {
    pub targets: Vec<TargetTrace>,
    pub checks: Vec<TraceCheck>,
    pub captured: Vec<TraceCaptured>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioTrace {
    filter: NoiseFilter,
    points: Vec<TracePoint>,
    steps: BTreeMap<usize, StepTrace>,
}

impl ScenarioTrace {
    #[must_use]
    pub const fn new(filter: NoiseFilter) -> Self {
        Self {
            filter,
            points: Vec::new(),
            steps: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn filter(&self) -> &NoiseFilter {
        &self.filter
    }

    #[must_use]
    pub fn points(&self) -> &[TracePoint] {
        &self.points
    }

    #[must_use]
    pub fn last(&self) -> Option<&TracePoint> {
        self.points.last()
    }

    pub(crate) fn push(&mut self, point: TracePoint) {
        self.points.push(point);
    }

    #[must_use]
    pub fn step(&self, index: usize) -> Option<&StepTrace> {
        self.steps.get(&index)
    }

    #[must_use]
    pub fn before(&self, index: usize) -> Option<&TracePoint> {
        self.points
            .iter()
            .rev()
            .find(|point| point.started <= index)
    }

    /// The first point at which a later step was due, or the end; `None` while the step runs.
    #[must_use]
    pub fn after(&self, index: usize) -> Option<&TracePoint> {
        self.points
            .iter()
            .find(|point| point.started > index || point.finished)
    }

    /// Replaces any earlier answer, so a step that waits keeps its latest.
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

    pub(crate) fn add_check(&mut self, index: usize, check: TraceCheck) {
        self.steps.entry(index).or_default().checks.push(check);
    }

    pub(crate) fn add_captured(&mut self, index: usize, captured: TraceCaptured) {
        self.steps.entry(index).or_default().captured.push(captured);
    }
}

/// The one wording of a step for the recorder's trace page and the gallery's Trace tab.
#[must_use]
pub fn step_title(
    index: usize,
    steps: &[StepRecord],
    descriptions: &TargetDescriptions,
) -> Option<String> {
    let record = steps.get(index)?;
    let before: Vec<Step> = steps
        .iter()
        .take(index)
        .map(|earlier| earlier.step.clone())
        .collect();
    Some(with_group_path(
        &record.group,
        &format!(
            "{}. {}",
            index + 1,
            describe_step(&record.step, &before, descriptions)
        ),
    ))
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepView<'a> {
    pub index: usize,
    pub title: String,
    pub outcome: Outcome,
    pub targets: &'a [TargetTrace],
    pub checks: &'a [TraceCheck],
    pub captured: &'a [TraceCaptured],
    pub before: Option<&'a TracePoint>,
    pub after: Option<&'a TracePoint>,
    /// Changes the filter given to [`StepView::new`] does not ignore.
    pub changes: Vec<SnapshotChange>,
    /// Changes that filter ignores.
    pub hidden: Vec<SnapshotChange>,
}

impl<'a> StepView<'a> {
    /// A trace already filtered by `filter` loses nothing more; the gallery's holds everything and filters here.
    #[must_use]
    pub fn new(
        index: usize,
        steps: &[StepRecord],
        trace: &'a ScenarioTrace,
        descriptions: &TargetDescriptions,
        filter: &NoiseFilter,
    ) -> Option<Self> {
        let record = steps.get(index)?;
        let recorded = trace.step(index);
        let (before, after) = (trace.before(index), trace.after(index));
        let all = match (before, after) {
            (Some(before), Some(after)) => after.snapshot.changes_from(&before.snapshot),
            _ => Vec::new(),
        };
        let (hidden, changes): (Vec<_>, Vec<_>) = all.into_iter().partition(|change| {
            change
                .type_path
                .as_deref()
                .is_some_and(|path| filter.ignores_path(path))
        });
        Some(Self {
            index,
            title: step_title(index, steps, descriptions)?,
            outcome: record.outcome.clone(),
            targets: recorded.map_or(&[], |step| step.targets.as_slice()),
            checks: recorded.map_or(&[], |step| step.checks.as_slice()),
            captured: recorded.map_or(&[], |step| step.captured.as_slice()),
            before,
            after,
            changes,
            hidden,
        })
    }

    #[must_use]
    pub const fn ran(&self) -> bool {
        !matches!(self.outcome, Outcome::Skipped)
    }

    #[must_use]
    pub fn failure(&self) -> Option<&str> {
        match &self.outcome {
            Outcome::Fail(message) => Some(message),
            Outcome::Pass | Outcome::Skipped => None,
        }
    }
}

impl TargetTrace {
    #[must_use]
    pub const fn is_found(&self) -> bool {
        self.found.is_ok()
    }

    #[must_use]
    pub fn answer(&self) -> &str {
        match &self.found {
            Ok(text) | Err(text) => text,
        }
    }
}

impl TraceCheck {
    #[must_use]
    pub fn failure(&self) -> &str {
        match &self.outcome {
            Outcome::Fail(message) => message,
            Outcome::Pass | Outcome::Skipped => "",
        }
    }
}

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

fn walk(world: &World, entity: Entity, depth: usize, lines: &mut Vec<UiLine>) {
    if depth > MAX_TREE_DEPTH || is_tooling(world, entity) || !is_ui_node(world, entity) {
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
    #![allow(clippy::expect_used, reason = "tests may panic")]

    use super::*;

    fn point(frame: u32, started: usize, finished: bool) -> TracePoint {
        TracePoint {
            frame,
            started,
            finished,
            snapshot: WorldSnapshot::default(),
            ui: Vec::new(),
        }
    }

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

    fn records() -> Vec<StepRecord> {
        let record = |step: Step, outcome: Outcome| StepRecord {
            step,
            group: Vec::new(),
            outcome,
            frame: 1,
        };
        vec![
            record(Step::wait(3), Outcome::Pass),
            record(Step::wait(4), Outcome::Fail("no such button".into())),
            record(Step::wait(5), Outcome::Skipped),
        ]
    }

    #[test]
    fn a_step_in_a_group_is_titled_with_the_group_path_in_front() {
        let mut records = records();
        if let Some(record) = records.get_mut(1) {
            record.group = vec!["raise the bridge".into(), "lever".into()];
        }
        assert_eq!(
            step_title(1, &records, &TargetDescriptions::default()),
            Some("raise the bridge > lever > 2. Wait 4 frames".to_owned())
        );
    }

    #[test]
    fn a_step_is_titled_with_its_number_from_one_and_its_words() {
        let titles: Vec<_> = (0..4)
            .map(|index| step_title(index, &records(), &TargetDescriptions::default()))
            .collect();
        assert_eq!(
            titles,
            [
                Some("1. Wait 3 frames".to_owned()),
                Some("2. Wait 4 frames".to_owned()),
                Some("3. Wait 5 frames".to_owned()),
                None,
            ]
        );
    }

    #[test]
    fn a_step_view_brings_together_the_outcome_what_was_looked_up_and_the_worlds() {
        let mut trace = trace();
        trace.add_target(
            1,
            TargetTrace {
                target: "Text(\"Save\")".into(),
                found: Err("no such text".into()),
            },
        );
        let steps = records();
        let view = step_view(1, &steps, &trace).expect("the report has the step");
        assert_eq!(view.title, "2. Wait 4 frames");
        assert_eq!(view.failure(), Some("no such button"));
        assert!(view.ran());
        assert_eq!(view.targets.len(), 1);
        assert!(!view.targets.iter().any(TargetTrace::is_found));
        assert_eq!(view.before.map(|point| point.frame), Some(3));
        assert_eq!(view.after.map(|point| point.frame), Some(5));
    }

    #[test]
    fn a_step_that_never_ran_has_no_failure_and_says_so() {
        let trace = trace();
        let steps = records();
        let view = step_view(2, &steps, &trace).expect("the report has the step");
        assert!(!view.ran());
        assert_eq!(view.failure(), None);
        assert!(view.targets.is_empty() && view.checks.is_empty());
    }

    #[test]
    fn a_step_the_report_does_not_have_has_no_view() {
        let trace = trace();
        let steps = records();
        assert!(step_view(3, &steps, &trace).is_none());
    }

    fn point_with(started: usize, finished: bool, score: &str, time: &str) -> TracePoint {
        let mut snapshot = WorldSnapshot::default();
        snapshot.set_resource("Score", "game::Score", "Score.0", score);
        snapshot.set_resource(
            "Time<Virtual>",
            "bevy_time::time::Time<bevy_time::virt::Virtual>",
            "Time<Virtual>.elapsed",
            time,
        );
        TracePoint {
            snapshot,
            ..point(0, started, finished)
        }
    }

    #[test]
    fn a_step_view_lists_the_changes_the_filter_lets_through_and_keeps_the_rest_apart() {
        let mut trace = ScenarioTrace::new(NoiseFilter::nothing());
        trace.push(point_with(0, false, "1", "0.0"));
        trace.push(point_with(3, true, "2", "0.1"));
        let steps = records();
        let describe = |view: &StepView<'_>| -> (Vec<String>, Vec<String>) {
            let keys = |changes: &[SnapshotChange]| {
                changes.iter().map(|change| change.key.clone()).collect()
            };
            (keys(&view.changes), keys(&view.hidden))
        };
        let view = step_view(0, &steps, &trace).expect("the report has the step");
        assert_eq!(
            describe(&view),
            (
                vec!["resource Score.0".to_owned()],
                vec!["resource Time<Virtual>.elapsed".to_owned()]
            ),
            "the default filter leaves out Bevy's time"
        );
        let everything = StepView::new(
            0,
            &steps,
            &trace,
            &TargetDescriptions::default(),
            &NoiseFilter::nothing(),
        )
        .expect("the report has the step");
        assert_eq!(everything.changes.len(), 2);
        assert_eq!(everything.hidden.len(), 0);
    }

    fn step_view<'a>(
        index: usize,
        steps: &[StepRecord],
        trace: &'a ScenarioTrace,
    ) -> Option<StepView<'a>> {
        StepView::new(
            index,
            steps,
            trace,
            &TargetDescriptions::default(),
            &NoiseFilter::default(),
        )
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
