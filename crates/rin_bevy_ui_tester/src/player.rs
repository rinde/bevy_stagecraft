//! Plays a scenario through [`SyntheticInput`] and records what happened.

use alloc::collections::VecDeque;

use bevy::{
    app::{App, Last, Plugin, PreUpdate},
    ecs::{
        entity::Entity,
        resource::Resource,
        schedule::common_conditions::{not, resource_exists},
        schedule::{IntoScheduleConfigs, SystemSet},
        world::World,
    },
    input::{
        ButtonState,
        keyboard::KeyCode,
        mouse::{MouseButton, MouseScrollUnit},
    },
    math::Vec2,
    window::{PrimaryWindow, Window},
};

use crate::{
    check::{Check, suggest::NoiseFilter},
    headless::HeadlessPlugins,
    input::{SyntheticEvent, SyntheticInput, SyntheticInputSystems},
    probe::check_hover,
    scenario::{ORIGIN, Scenario, Step, flatten, with_group_path},
    targets::{Action, NotActionable, ResolvedTarget, Target, actionable, pressable, resolve},
    trace::{ScenarioTrace, TargetTrace, TraceCheck, TracePoint},
};

/// How a step or a check ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It ran and held.
    Pass,
    /// It ran and did not hold; the message says why.
    Fail(String),
    /// It never ran, because an earlier step failed.
    Skipped,
}

impl Outcome {
    /// Whether this is [`Outcome::Pass`].
    #[must_use]
    pub const fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
}

/// What happened to one step.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRecord {
    /// The step.
    pub step: Step,
    /// The names of the [`Step::Group`]s the step is in, outermost first;
    /// empty for a step at the top level.
    pub group: Vec<String>,
    /// How it ended.
    pub outcome: Outcome,
    /// The player frame the step began on, counted from 1.
    pub frame: u32,
}

/// Which kind of check a [`CheckRecord`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    /// Run by a [`Step::Check`] part-way through.
    Step,
    /// Run at the end of the script.
    Expect,
}

/// What happened to one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRecord {
    /// The check's name.
    pub name: String,
    /// When it runs.
    pub kind: CheckKind,
    /// How it ended.
    pub outcome: Outcome,
}

/// The outcome of the scenario being played, read by the gallery and the
/// recorder.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct ScenarioReport {
    /// The scenario's name.
    pub name: String,
    /// One record per step, in script order; steps not reached are
    /// [`Outcome::Skipped`].
    pub steps: Vec<StepRecord>,
    /// One record per check that ran or was skipped, in the order they ran.
    pub checks: Vec<CheckRecord>,
    /// The caption currently showing, if any.
    pub caption: Option<String>,
    /// How many frames the player has run.
    pub frames: u32,
    /// Whether the script and the expectations are done.
    pub finished: bool,
    /// Why the scenario's setup failed, if it did; no step plays then.
    pub setup_failure: Option<String>,
}

impl ScenarioReport {
    /// Whether the scenario finished with every step and check passing.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.finished
            && self.setup_failure.is_none()
            && self.steps.iter().all(|record| record.outcome.is_pass())
            && self.checks.iter().all(|record| record.outcome.is_pass())
    }

    /// One line per failure, naming the step or check.
    #[must_use]
    pub fn failures(&self) -> Vec<String> {
        let steps =
            self.steps
                .iter()
                .enumerate()
                .filter_map(|(index, record)| match &record.outcome {
                    Outcome::Fail(message) => Some(with_group_path(
                        &record.group,
                        &format!("step {index} ({:?}): {message}", record.step),
                    )),
                    Outcome::Pass | Outcome::Skipped => None,
                });
        let checks = self
            .checks
            .iter()
            .filter_map(|record| match &record.outcome {
                Outcome::Fail(message) => Some(format!("check `{}`: {message}", record.name)),
                Outcome::Pass | Outcome::Skipped => None,
            });
        self.setup_failure
            .iter()
            .map(|message| format!("setup: {message}"))
            .chain(steps)
            .chain(checks)
            .collect()
    }
}

/// System sets of the player plugin.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScenarioSystems {
    /// Advances the script by one frame, in `PreUpdate`. Runs before the
    /// synthetic input is drained, so what it queues reaches Bevy in the same
    /// frame.
    Play,
    /// Takes a [`TracePoint`] when the player is between steps, in `Last`,
    /// once the frame has been played. Does nothing unless
    /// [`ScenarioPlayer::enable_trace`] was called.
    Trace,
}

/// Registers [`ScenarioPlayer`], [`ScenarioReport`] and the system that advances the script. Needs `SyntheticInputPlugin`, which
/// [`HeadlessPlugins`] includes.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScenarioPlayerPlugin;

impl Plugin for ScenarioPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScenarioPlayer>()
            .init_resource::<ScenarioReport>()
            .add_systems(
                PreUpdate,
                play.in_set(ScenarioSystems::Play)
                    .run_if(not(resource_exists::<ExternallyDriven>))
                    .before(SyntheticInputSystems::Drain),
            )
            .add_systems(
                Last,
                trace_point.in_set(ScenarioSystems::Trace).run_if(tracing),
            );
    }
}

/// Present when something other than [`ScenarioPlayerPlugin`]'s own system
/// advances the script, such as the gallery, which paces it.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub(crate) struct ExternallyDriven;

/// One frame's worth of work, or a zero-frame action between frames.
#[derive(Debug)]
enum Op {
    /// Queue these events and let one frame pass.
    Frame(usize, Vec<SyntheticEvent>),
    /// Confirm the widget is under the pointer, as the last frame left it.
    CheckHover {
        /// Index of the step this belongs to.
        step: usize,
        /// The widget that should be hovered.
        widget: Entity,
        /// The step's name for the target, for the message.
        label: String,
    },
    /// Wait until the targets of the step can be acted on, then queue its
    /// frames; this has waited `waited` frames of at most `timeout`.
    Act {
        /// Index of the step this belongs to.
        step: usize,
        /// The step, without the `Timeout(..)` around it.
        action: Step,
        /// How many frames it may wait.
        timeout: u16,
        /// Frames waited so far.
        waited: u32,
    },
    /// Run the check, which has waited this many frames to hold.
    Check {
        /// Index of the step this belongs to.
        step: usize,
        /// The check.
        check: Check,
        /// Frames waited so far, for [`Check::Within`].
        waited: u32,
    },
    /// Change the caption.
    Caption(String),
}

/// The widget an earlier step sent the pointer to, which a press right
/// after it acts on.
#[derive(Debug, Clone)]
struct Aim {
    /// The widget.
    widget: Entity,
    /// How the script named it, for messages.
    label: String,
}

/// A scenario being played.
#[derive(Debug)]
struct Running {
    /// The scenario.
    scenario: Scenario,
    /// The steps to play: the script with its groups flattened away, so a
    /// grouped script plays frame for frame as the same steps ungrouped.
    plan: Vec<Step>,
    /// Index of the next step to expand into operations.
    next_step: usize,
    /// Operations expanded from steps but not yet done.
    ops: VecDeque<Op>,
    /// Where the pointer is, as far as the script has moved it.
    cursor: Vec2,
    /// The widget the last move sent the pointer to, until a press acts on
    /// it or the pointer goes elsewhere.
    aim: Option<Aim>,
    /// Whether a step failed and the rest is being skipped.
    aborted: bool,
    /// Whether the targets steps look up are written down.
    tracing: bool,
    /// What steps found for their targets, until the player moves it into
    /// its trace.
    resolved: Vec<(usize, TargetTrace)>,
    /// The expectations' outcomes so far, once the steps are done: `None`
    /// for one still waiting to hold.
    expected: Option<Vec<Option<Outcome>>>,
    /// Frames the expectations have waited.
    expect_waited: u32,
}

/// Plays one scenario at a time, a frame per update.
#[derive(Resource, Debug, Default)]
pub struct ScenarioPlayer {
    /// Waiting to begin on the next frame.
    pending: Option<Scenario>,
    /// Why the setup of the scenario waiting to begin failed, if it did.
    setup_failure: Option<String>,
    /// Being played.
    running: Option<Running>,
    /// What snapshots leave out, while a trace is being kept.
    tracing: Option<NoiseFilter>,
    /// What the last run (or the one going) did, between its steps.
    trace: Option<ScenarioTrace>,
    /// The step the run stops before, so a caller can look at the world as
    /// that step finds it.
    halt_before: Option<usize>,
    /// Whether the run has stopped at `halt_before`.
    halted: bool,
}

impl ScenarioPlayer {
    /// Starts playing `scenario` on the next frame, replacing the report. The
    /// app should have run `scenario.run_setup` and laid its UI out first.
    pub fn start(&mut self, scenario: Scenario) {
        self.running = None;
        self.pending = Some(scenario);
        self.setup_failure = None;
    }

    /// Starts `scenario` with the outcome of its setup
    /// ([`Scenario::run_setup`]): as [`ScenarioPlayer::start`] does when it
    /// built the world, and otherwise as a run that fails on the next frame
    /// with every step skipped and `message` as the report's
    /// [`ScenarioReport::setup_failure`].
    pub fn start_after_setup(&mut self, scenario: Scenario, setup: Result<bool, String>) {
        self.start(scenario);
        self.setup_failure = setup.err();
    }

    /// Whether a scenario is waiting to begin or still being played.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.pending.is_some() || self.running.is_some()
    }

    /// Adds `check` to the expectations of the scenario waiting to begin or
    /// being played, unless its expectations have begun to run; a finished
    /// run is left as it was.
    #[cfg(feature = "gallery")]
    pub(crate) fn add_expect(&mut self, check: Check) {
        if let Some(scenario) = self.pending.as_mut() {
            scenario.push_expect(check);
        } else if let Some(running) = self.running.as_mut()
            && running.expected.is_none()
        {
            running.scenario.push_expect(check);
        }
    }

    /// Drops the scenario being played or waiting to begin, if any.
    pub fn stop(&mut self) {
        self.pending = None;
        self.running = None;
        self.setup_failure = None;
        self.halt_before = None;
        self.halted = false;
    }

    /// Keeps a [`ScenarioTrace`] of the runs that begin from now on, its
    /// snapshots leaving out what `filter` ignores. Needs
    /// [`ScenarioPlayerPlugin`], which takes the points.
    pub fn enable_trace(&mut self, filter: NoiseFilter) {
        self.tracing = Some(filter);
    }

    /// The trace of the run being played or the last one, if tracing is on.
    #[must_use]
    pub const fn trace(&self) -> Option<&ScenarioTrace> {
        self.trace.as_ref()
    }

    /// Takes the trace out, leaving none.
    pub const fn take_trace(&mut self) -> Option<ScenarioTrace> {
        self.trace.take()
    }

    /// Makes the run stop, before step `index` begins, with
    /// [`ScenarioPlayer::halted`] set: the world is then as that step finds
    /// it, on the frame [`ScenarioTrace::before`] reports for it. Call
    /// before [`ScenarioPlayer::start`] (which leaves it alone) or while the
    /// run is going; `None` lets the run on, and a run already stopped
    /// continues on the next frame.
    pub const fn pause_before(&mut self, index: Option<usize>) {
        self.halt_before = index;
        self.halted = false;
    }

    /// Whether the run stopped where [`ScenarioPlayer::pause_before`] said.
    #[must_use]
    pub const fn halted(&self) -> bool {
        self.halted
    }

    /// How many steps have begun, which is also the index of the next one.
    #[must_use]
    pub fn steps_started(&self) -> usize {
        self.running.as_ref().map_or(0, |run| run.next_step)
    }

    /// Whether the player is between steps: no frame of the last one is left.
    /// What may remain are checks and captions, which take no frame. A player
    /// with no scenario counts as between steps; one whose step waits for a
    /// check to hold or for a target to become actionable does not.
    #[must_use]
    pub fn at_step_boundary(&self) -> bool {
        self.running.as_ref().is_none_or(|run| {
            run.ops.iter().all(|op| {
                !matches!(
                    op,
                    Op::Frame(..) | Op::Check { waited: 1.., .. } | Op::Act { waited: 1.., .. }
                )
            })
        })
    }
}

/// Whether a trace is being kept; the condition of the system that takes its
/// points.
fn tracing(player: Option<bevy::ecs::system::Res<ScenarioPlayer>>) -> bool {
    player.is_some_and(|player| player.tracing.is_some())
}

/// Takes a [`TracePoint`] after a frame that left the player between steps
/// with a later step due than the last point had, and one more when the
/// scenario ended.
fn trace_point(world: &mut World) {
    let Some(mut player) = world.remove_resource::<ScenarioPlayer>() else {
        return;
    };
    if !player.halted
        && let Some(filter) = player.tracing.clone()
        && let Some(report) = world.get_resource::<ScenarioReport>()
    {
        let (frame, finished) = (report.frames, report.finished);
        let started = if finished {
            report.steps.len()
        } else {
            player.steps_started()
        };
        let boundary = player.at_step_boundary();
        if let Some(trace) = player.trace.as_mut()
            && boundary
            && trace
                .last()
                .is_some_and(|last| started > last.started || (finished && !last.finished))
        {
            let point = TracePoint::take(world, &filter, frame, started, finished);
            trace.push(point);
        }
    }
    world.insert_resource(player);
}

/// The player system: advances the script one frame.
fn play(world: &mut World) {
    advance_world(world, true);
}

/// Advances the script by one frame, if one is playing.
///
/// With `settled` false the call does only what needs no fresh picking or
/// layout: it moves on only when the next operation is a frame of input. A
/// caller that runs several frames in one update uses this for all but the
/// first, so a hover check or a target lookup never sees stale state.
pub(crate) fn advance_world(world: &mut World, settled: bool) {
    let Some(mut player) = world.remove_resource::<ScenarioPlayer>() else {
        return;
    };
    if player.is_active() {
        let mut report = world
            .remove_resource::<ScenarioReport>()
            .unwrap_or_default();
        player.advance(world, &mut report, settled);
        world.insert_resource(report);
    }
    world.insert_resource(player);
}

/// Eases `t` in `0..=1` so movement starts and ends gently.
fn ease(t: f32) -> f32 {
    t * t * 2.0f32.mul_add(-t, 3.0)
}

/// Marks a step failed in the report.
fn fail_step(report: &mut ScenarioReport, index: usize, message: String) {
    if let Some(record) = report.steps.get_mut(index) {
        record.outcome = Outcome::Fail(message);
    }
}

/// Records a check's outcome.
fn record_check(report: &mut ScenarioReport, name: &str, kind: CheckKind, outcome: Outcome) {
    report.checks.push(CheckRecord {
        name: name.into(),
        kind,
        outcome,
    });
}

impl ScenarioPlayer {
    /// Starts `scenario`: a fresh report with every step not yet reached,
    /// or, when its setup failed, a finished report saying so.
    fn begin(&mut self, world: &mut World, report: &mut ScenarioReport, scenario: Scenario) {
        self.halted = false;
        self.trace = self.tracing.clone().map(|filter| {
            let first = TracePoint::take(world, &filter, 0, 0, false);
            let mut trace = ScenarioTrace::new(filter);
            trace.push(first);
            trace
        });
        let flat = flatten(scenario.script());
        let plan: Vec<Step> = flat.iter().map(|flat| flat.step.clone()).collect();
        *report = ScenarioReport {
            name: scenario.name().into(),
            steps: flat
                .iter()
                .map(|flat| StepRecord {
                    step: flat.step.clone(),
                    group: flat.group.iter().map(|name| (*name).to_owned()).collect(),
                    outcome: Outcome::Skipped,
                    frame: 0,
                })
                .collect(),
            ..ScenarioReport::default()
        };
        if let Some(message) = self.setup_failure.take() {
            report.setup_failure = Some(message);
            report.frames = 1;
            report.finished = true;
            return;
        }
        // What the script last did beats where a real window thinks the
        // cursor is, so a gallery restart does not depend on the mouse.
        let cursor = world
            .get_resource::<SyntheticInput>()
            .and_then(SyntheticInput::cursor)
            .or_else(|| {
                world
                    .query_filtered::<&Window, bevy::ecs::query::With<PrimaryWindow>>()
                    .iter(world)
                    .next()
                    .and_then(Window::cursor_position)
            })
            .unwrap_or(ORIGIN);
        self.running = Some(Running {
            scenario,
            plan,
            next_step: 0,
            ops: VecDeque::new(),
            cursor,
            aim: None,
            aborted: false,
            tracing: self.tracing.is_some(),
            resolved: Vec::new(),
            expected: None,
            expect_waited: 0,
        });
    }

    /// Does the work of one frame.
    fn advance(&mut self, world: &mut World, report: &mut ScenarioReport, settled: bool) {
        self.advance_frame(world, report, settled);
        if let Some(run) = self.running.as_mut() {
            move_targets(&mut self.trace, run);
        }
    }

    /// Does the work of one frame, leaving what the steps found for their
    /// targets in the run.
    fn advance_frame(&mut self, world: &mut World, report: &mut ScenarioReport, settled: bool) {
        if self.halted {
            return;
        }
        if let Some(scenario) = self.pending.take() {
            self.begin(world, report, scenario);
        }
        let Some(run) = self.running.as_mut() else {
            return;
        };
        if !settled && !matches!(run.ops.front(), Some(Op::Frame(..))) {
            return;
        }
        report.frames += 1;
        loop {
            let Some(op) = run.ops.pop_front() else {
                if self.halt_before.is_some_and(|index| run.next_step >= index) {
                    // This frame did not happen: the world is as the last one
                    // left it, and the next call takes up from here.
                    report.frames -= 1;
                    self.halted = true;
                    return;
                }
                if run.next_step < run.plan.len() {
                    let index = run.next_step;
                    run.next_step += 1;
                    run.expand(world, report, index);
                    continue;
                }
                if run.finish(world, report) {
                    move_targets(&mut self.trace, run);
                    self.running = None;
                }
                // Either the end, or a frame for expectations still waiting.
                return;
            };
            match op {
                Op::Frame(step, events) => {
                    run.apply(world, report, step, events);
                    return;
                }
                Op::Act {
                    step,
                    action,
                    timeout,
                    waited,
                } => match run.act(world, step, &action, false) {
                    Ok(()) => {}
                    Err(NotActionable::Yet(_)) if waited < u32::from(timeout) => {
                        // Not yet: look again after one more frame.
                        run.ops.push_front(Op::Act {
                            step,
                            action,
                            timeout,
                            waited: waited + 1,
                        });
                        return;
                    }
                    Err(NotActionable::Yet(reason)) => {
                        run.abort(
                            report,
                            step,
                            format!("{reason} (still after {timeout} frames)"),
                        );
                    }
                    Err(NotActionable::Never(reason)) => run.abort(report, step, reason),
                },
                Op::CheckHover {
                    step,
                    widget,
                    label,
                } => {
                    if let Err(message) = check_hover(world, widget, &label) {
                        run.abort(report, step, message);
                    }
                }
                Op::Check {
                    step,
                    check,
                    waited,
                } => {
                    let name = check.to_string();
                    let Some(result) = check.poll(world, waited) else {
                        // Not yet: look again after one more frame.
                        run.ops.push_front(Op::Check {
                            step,
                            check,
                            waited: waited + 1,
                        });
                        return;
                    };
                    record_step_check(&mut self.trace, report, step, &name, result);
                }
                Op::Caption(text) => report.caption = Some(text),
            }
        }
    }
}

/// Records the end of a step's check in the report and the trace, and fails
/// the step if it did not hold.
fn record_step_check(
    trace: &mut Option<ScenarioTrace>,
    report: &mut ScenarioReport,
    step: usize,
    name: &str,
    result: Result<(), String>,
) {
    let outcome = result
        .clone()
        .map_or_else(Outcome::Fail, |()| Outcome::Pass);
    if let Some(trace) = trace.as_mut() {
        trace.add_check(
            step,
            TraceCheck {
                name: name.to_owned(),
                outcome: outcome.clone(),
            },
        );
    }
    record_check(report, name, CheckKind::Step, outcome);
    if let Err(message) = result {
        fail_step(report, step, message);
    }
}

/// Moves what the steps of `run` found for their targets into `trace`.
fn move_targets(trace: &mut Option<ScenarioTrace>, run: &mut Running) {
    let Some(trace) = trace.as_mut() else {
        return;
    };
    for (step, found) in run.resolved.drain(..) {
        trace.add_target(step, found);
    }
}

impl Running {
    /// [`reach`], writing down what it found when a trace is kept.
    fn reach_traced(
        &mut self,
        world: &World,
        step: usize,
        target: &Target,
        action: Action,
        forced: bool,
    ) -> Result<ResolvedTarget, NotActionable> {
        let result = reach(world, target, action, forced);
        if self.tracing {
            let found = match &result {
                Ok(to) => Ok(format!(
                    "at ({:.0}, {:.0}){}",
                    to.position.x,
                    to.position.y,
                    if to.widget.is_some() {
                        ", a widget"
                    } else {
                        ""
                    }
                )),
                Err(NotActionable::Yet(message) | NotActionable::Never(message)) => {
                    Err(message.clone())
                }
            };
            self.resolved.push((
                step,
                TargetTrace {
                    target: format!("{target:?}"),
                    found,
                },
            ));
        }
        result
    }

    /// Stops the script at a failed step; what is left is skipped.
    fn abort(&mut self, report: &mut ScenarioReport, step: usize, message: String) {
        fail_step(report, step, message);
        self.ops.clear();
        self.next_step = self.plan.len();
        self.aborted = true;
    }

    /// Queues `events` and tracks where they leave the pointer.
    fn apply(
        &mut self,
        world: &mut World,
        report: &mut ScenarioReport,
        step: usize,
        events: Vec<SyntheticEvent>,
    ) {
        let Some(mut input) = world.get_resource_mut::<SyntheticInput>() else {
            self.abort(
                report,
                step,
                "there is no SyntheticInput; add SyntheticInputPlugin".into(),
            );
            return;
        };
        for event in events {
            if let SyntheticEvent::MoveCursor(position) = event {
                self.cursor = position;
            }
            input.push(event);
        }
    }

    /// Turns step `index` into operations.
    fn expand(&mut self, world: &World, report: &mut ScenarioReport, index: usize) {
        let Some(step) = self.plan.get(index).cloned() else {
            return;
        };
        if let Some(record) = report.steps.get_mut(index) {
            record.outcome = Outcome::Pass;
            record.frame = report.frames;
        }
        if let Some(events) = key_events(&step) {
            self.frame(index, events);
            return;
        }
        let (step, timeout, forced) = unwrapped(step, self.scenario.action_timeout_frames());
        match step {
            Step::MoveTo(_)
            | Step::Glide { .. }
            | Step::Drag { .. }
            | Step::Click(_)
            | Step::Press(_)
                if !forced =>
            {
                self.ops.push_back(Op::Act {
                    step: index,
                    action: step,
                    timeout,
                    waited: 0,
                });
            }
            Step::MoveTo(_)
            | Step::Glide { .. }
            | Step::Drag { .. }
            | Step::Click(_)
            | Step::Press(_) => {
                if let Err(NotActionable::Yet(message) | NotActionable::Never(message)) =
                    self.act(world, index, &step, true)
                {
                    self.abort(report, index, message);
                }
            }
            Step::Release(button) => self.frame(
                index,
                vec![SyntheticEvent::Button(button, ButtonState::Released)],
            ),
            Step::Wheel { x, y } => self.frame(
                index,
                vec![SyntheticEvent::Wheel {
                    unit: MouseScrollUnit::Pixel,
                    x,
                    y,
                }],
            ),
            Step::Wait(frames) => {
                for _ in 0..frames {
                    self.frame(index, Vec::new());
                }
            }
            // Keys were handled above; `unwrapped` took the wrappers off, and
            // the groups were flattened away when the run began.
            Step::Key { .. }
            | Step::KeyUp { .. }
            | Step::Tap { .. }
            | Step::Timeout(..)
            | Step::Force(_)
            | Step::Group { .. } => {}
            Step::Check(check) => self.ops.push_back(Op::Check {
                step: index,
                check,
                waited: 0,
            }),
            Step::Caption(text) => self.ops.push_back(Op::Caption(text)),
        }
    }

    /// Queues the frames of `step` (a move, glide, drag, click or press) if
    /// its targets can be acted on now, or says why not. A `forced` step
    /// skips the checks, and the hover check after it.
    fn act(
        &mut self,
        world: &World,
        index: usize,
        step: &Step,
        forced: bool,
    ) -> Result<(), NotActionable> {
        match step {
            Step::MoveTo(target) => {
                let to = self.reach_traced(world, index, target, Action::Point, forced)?;
                self.move_ops(index, core::iter::once(to.position), to, target, !forced);
                self.aim_at(to, target, forced);
            }
            Step::Glide { target, frames } => {
                let to = self.reach_traced(world, index, target, Action::Point, forced)?;
                self.glide_ops(index, to, *frames, target, !forced);
                self.aim_at(to, target, forced);
            }
            Step::Drag { from, to, frames } => {
                // Both ends are checked on the same frame, before anything
                // moves: the drag presses at its start.
                let start = self.reach_traced(world, index, from, Action::Press, forced)?;
                let end = self.reach_traced(world, index, to, Action::Point, forced)?;
                self.move_ops(
                    index,
                    core::iter::once(start.position),
                    start,
                    from,
                    !forced,
                );
                self.frame(
                    index,
                    vec![SyntheticEvent::Button(
                        MouseButton::Left,
                        ButtonState::Pressed,
                    )],
                );
                // The glide starts where the drag does, whatever the cursor
                // was doing before.
                self.cursor = start.position;
                self.glide_ops(index, end, *frames, to, !forced);
                self.frame(
                    index,
                    vec![SyntheticEvent::Button(
                        MouseButton::Left,
                        ButtonState::Released,
                    )],
                );
                self.aim = None;
            }
            Step::Click(button) | Step::Press(button) => {
                if !forced && let Some(aim) = &self.aim {
                    pressable(world, aim.widget, self.cursor, &aim.label)?;
                }
                self.aim = None;
                let mut events = vec![SyntheticEvent::Button(*button, ButtonState::Pressed)];
                if matches!(step, Step::Click(_)) {
                    events.push(SyntheticEvent::Button(*button, ButtonState::Released));
                }
                self.frame(index, events);
            }
            _ => {}
        }
        Ok(())
    }

    /// Remembers the widget a move sent the pointer to, for a press right
    /// after it; a forced move, or one to a position, leaves none.
    fn aim_at(&mut self, to: ResolvedTarget, target: &Target, forced: bool) {
        self.aim = to.widget.filter(|_| !forced).map(|widget| Aim {
            widget,
            label: format!("{target:?}"),
        });
    }

    /// Queues one frame of `events`.
    fn frame(&mut self, step: usize, events: Vec<SyntheticEvent>) {
        self.ops.push_back(Op::Frame(step, events));
    }

    /// Queues one frame per position, then, when `confirm` says so, the
    /// hover check for the target.
    fn move_ops(
        &mut self,
        step: usize,
        positions: impl IntoIterator<Item = Vec2>,
        to: ResolvedTarget,
        target: &Target,
        confirm: bool,
    ) {
        for position in positions {
            self.frame(step, vec![SyntheticEvent::MoveCursor(position)]);
        }
        if let Some(widget) = to.widget.filter(|_| confirm) {
            self.ops.push_back(Op::CheckHover {
                step,
                widget,
                label: format!("{target:?}"),
            });
        }
    }

    /// Queues an eased move from the cursor to `to` over `frames` frames.
    fn glide_ops(
        &mut self,
        step: usize,
        to: ResolvedTarget,
        frames: u16,
        target: &Target,
        confirm: bool,
    ) {
        let frames = frames.max(1);
        let from = self.cursor;
        let positions: Vec<Vec2> = (1..=frames)
            .map(|frame| from.lerp(to.position, ease(f32::from(frame) / f32::from(frames))))
            .collect();
        self.move_ops(step, positions, to, target, confirm);
    }

    /// Runs the end-of-script expectations and, once none waits any more,
    /// records them in order and closes the report. Returns whether it did;
    /// otherwise a frame passes and it runs again.
    fn finish(&mut self, world: &World, report: &mut ScenarioReport) -> bool {
        let expects = self.scenario.expects();
        let waited = self.expect_waited;
        let outcomes = self
            .expected
            .get_or_insert_with(|| vec![None; expects.len()]);
        for (expect, outcome) in expects.iter().zip(outcomes.iter_mut()) {
            if outcome.is_some() {
                continue;
            }
            *outcome = if self.aborted {
                Some(Outcome::Skipped)
            } else {
                expect.poll(world, waited).map(|result| match result {
                    Ok(()) => Outcome::Pass,
                    Err(message) => Outcome::Fail(message),
                })
            };
        }
        if outcomes.iter().any(Option::is_none) {
            self.expect_waited += 1;
            return false;
        }
        for (expect, outcome) in expects.iter().zip(outcomes.iter()) {
            if let Some(outcome) = outcome {
                record_check(
                    report,
                    &expect.to_string(),
                    CheckKind::Expect,
                    outcome.clone(),
                );
            }
        }
        report.finished = true;
        true
    }
}

/// `step` without the `Timeout(..)` and `Force(..)` around it, with the
/// frames it may wait for its target (the innermost `Timeout(..)`, else
/// `timeout`) and whether it is forced.
fn unwrapped(step: Step, timeout: u16) -> (Step, u16, bool) {
    match step {
        Step::Timeout(frames, inner) => unwrapped(*inner, frames),
        Step::Force(inner) => {
            let (step, timeout, _) = unwrapped(*inner, timeout);
            (step, timeout, true)
        }
        other => (other, timeout, false),
    }
}

/// Works `target` out for `action`: checked for actionability, or, when
/// `forced`, as it is now with no checks, a miss failing at once.
fn reach(
    world: &World,
    target: &Target,
    action: Action,
    forced: bool,
) -> Result<ResolvedTarget, NotActionable> {
    if forced {
        resolve(world, target)
            .map(|resolved| ResolvedTarget::at(resolved.position))
            .map_err(NotActionable::Never)
    } else {
        actionable(world, target, action)
    }
}

/// The events of a keyboard step, or `None` for any other step.
fn key_events(step: &Step) -> Option<Vec<SyntheticEvent>> {
    let press = |code: KeyCode| SyntheticEvent::Key(code, ButtonState::Pressed);
    let release = |code: KeyCode| SyntheticEvent::Key(code, ButtonState::Released);
    match step {
        Step::Key { key, modifiers } => Some(
            modifiers
                .keys()
                .into_iter()
                .chain(core::iter::once(*key))
                .map(press)
                .collect(),
        ),
        Step::KeyUp { key, modifiers } => Some(
            core::iter::once(*key)
                .chain(modifiers.keys().into_iter().rev())
                .map(release)
                .collect(),
        ),
        Step::Tap { key, modifiers } => {
            let held = modifiers.keys();
            let mut events: Vec<_> = held.iter().copied().map(press).collect();
            events.push(press(*key));
            events.push(release(*key));
            events.extend(held.iter().rev().copied().map(release));
            Some(events)
        }
        _ => None,
    }
}

/// Frames of layout and picking a fresh UI needs before the script starts.
const SETTLE_FRAMES: usize = 2;

impl Scenario {
    /// Plays the scenario in `app`, which needs [`HeadlessPlugins`] (or the
    /// equivalent) and, if [`ScenarioPlayerPlugin`] is not there yet, gets it
    /// even after the app is finished.
    /// Builds the world with the scenario's setup and settles the UI first,
    /// then returns the report; a setup that fails fails the report, with
    /// every step skipped.
    pub fn run_in(&self, app: &mut App) -> ScenarioReport {
        self.play_in(app, None).0
    }

    /// Plays the scenario in `app` as [`Scenario::run_in`] does, keeping a
    /// [`ScenarioTrace`] whose snapshots leave out what `filter` ignores.
    pub fn run_in_traced(
        &self,
        app: &mut App,
        filter: NoiseFilter,
    ) -> (ScenarioReport, ScenarioTrace) {
        let (report, trace) = self.play_in(app, Some(filter));
        (
            report,
            trace.unwrap_or_else(|| ScenarioTrace::new(NoiseFilter::nothing())),
        )
    }

    /// Plays the scenario in a fresh [`HeadlessPlugins`] app and returns the
    /// report and its trace, with the default [`NoiseFilter`].
    #[must_use]
    pub fn run_headless_traced(&self) -> (ScenarioReport, ScenarioTrace) {
        self.run_in_traced(&mut self.headless_app(), NoiseFilter::default())
    }

    /// Plays the scenario, with a trace when `filter` is given.
    fn play_in(
        &self,
        app: &mut App,
        filter: Option<NoiseFilter>,
    ) -> (ScenarioReport, Option<ScenarioTrace>) {
        if !app.world().contains_resource::<ScenarioPlayer>() {
            // `add_plugins` refuses once the app is finished, but the app is
            // allowed to gain systems and resources, so build it by hand.
            Plugin::build(&ScenarioPlayerPlugin, app);
        }
        let setup = self.run_setup(app.world_mut());
        for _ in 0..SETTLE_FRAMES {
            app.update();
        }
        if let Some(mut player) = app.world_mut().get_resource_mut::<ScenarioPlayer>() {
            if let Some(filter) = filter {
                player.enable_trace(filter);
            }
            player.start_after_setup(self.clone(), setup);
        }
        while app
            .world()
            .get_resource::<ScenarioPlayer>()
            .is_some_and(ScenarioPlayer::is_active)
        {
            app.update();
        }
        let trace = app
            .world_mut()
            .get_resource_mut::<ScenarioPlayer>()
            .and_then(|mut player| player.take_trace());
        let report = app
            .world()
            .get_resource::<ScenarioReport>()
            .cloned()
            .unwrap_or_default();
        (report, trace)
    }

    /// Plays the scenario in a fresh [`HeadlessPlugins`] app and returns the
    /// report.
    #[must_use]
    pub fn run_headless(&self) -> ScenarioReport {
        self.run_in(&mut self.headless_app())
    }

    /// A finished [`HeadlessPlugins`] app with the scenario's window size and
    /// scale factor, or the default ones. Use it to add what the scenario needs (for example
    /// [`crate::script::Registry::install`]) before [`Scenario::run_in`].
    pub fn headless_app(&self) -> App {
        let mut plugins = HeadlessPlugins::default();
        if let Some((width, height)) = self.window() {
            plugins = plugins.with_window_size(width, height);
        }
        if let Some(scale_factor) = self.scale() {
            plugins = plugins.with_scale_factor(scale_factor);
        }
        let mut app = App::new();
        app.add_plugins(plugins);
        app.finish();
        app.cleanup();
        app
    }
}
