//! Plays a scenario through [`SyntheticInput`] and records what happened.

use alloc::collections::{BTreeMap, VecDeque};

use bevy::{
    app::{App, First, Last, Plugin, PreUpdate},
    asset::{AssetServer, Assets, UntypedAssetId, UntypedHandle},
    ecs::{
        entity::Entity,
        error::ErrorHandler,
        query::With,
        resource::Resource,
        schedule::common_conditions::{not, resource_exists},
        schedule::{IntoScheduleConfigs, SystemSet},
        world::World,
    },
    input::{
        ButtonState,
        keyboard::KeyCode,
        mouse::{MouseButton, MouseScrollUnit},
        touch::TouchPhase,
    },
    math::{UVec2, Vec2},
    scene::{SceneListPatch, ScenePatch},
    window::{PrimaryWindow, Window},
};

mod catch;

use crate::{
    check::{
        CapturedValues, Check, Measure, ScreenshotRecord,
        screenshot::{self, Pictures, Readiness},
        suggest::NoiseFilter,
    },
    headless::HeadlessPlugins,
    input::{SyntheticEvent, SyntheticInput, SyntheticInputSystems, character_events},
    probe::check_hover,
    scenario::restore_rem_size,
    scenario::{ORIGIN, Scenario, Step, flatten, with_group_path},
    script::check_scale_factor,
    targets::{Action, NotActionable, ResolvedTarget, Target, actionable, pressable, resolve},
    trace::{ScenarioTrace, TargetTrace, TraceCaptured, TraceCheck, TracePoint},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail(String),
    /// A skipped check does not fail the scenario; a skipped step does.
    Skipped,
}

impl Outcome {
    #[must_use]
    pub const fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepRecord {
    pub step: Step,
    pub group: Vec<String>,
    pub outcome: Outcome,
    pub frame: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    Step,
    Expect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRecord {
    pub name: String,
    pub kind: CheckKind,
    pub outcome: Outcome,
}

#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct ScenarioReport {
    pub name: String,
    pub steps: Vec<StepRecord>,
    pub checks: Vec<CheckRecord>,
    pub caption: Option<String>,
    pub frames: u32,
    pub finished: bool,
    pub setup_failure: Option<String>,
    pub screenshots: Vec<ScreenshotRecord>,
}

impl ScenarioReport {
    /// A check skipped because it cannot run here counts as neither.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.finished
            && self.setup_failure.is_none()
            && self.steps.iter().all(|record| record.outcome.is_pass())
            && !self
                .checks
                .iter()
                .any(|record| matches!(record.outcome, Outcome::Fail(_)))
    }

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

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScenarioSystems {
    /// Runs before the synthetic input is drained, so what it queues reaches Bevy the same frame.
    Play,
    Trace,
}

/// Needs `SyntheticInputPlugin`, which [`HeadlessPlugins`] includes.
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
            )
            .add_systems(
                Last,
                fail_caught_errors
                    .after(ScenarioSystems::Trace)
                    .run_if(catching),
            )
            .add_systems(First, sync_error_handler);
    }
}

/// Present when something else, such as the gallery, advances the script.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub(crate) struct ExternallyDriven;

#[derive(Debug)]
enum Op {
    Frame(usize, Vec<SyntheticEvent>),
    CheckHover {
        step: usize,
        widget: Entity,
        label: String,
    },
    Act {
        step: usize,
        action: Step,
        timeout: u16,
        waited: u32,
    },
    Check {
        step: usize,
        check: Check,
        waited: u32,
    },
    Caption(String),
}

#[derive(Debug, Clone)]
struct Aim {
    widget: Entity,
    label: String,
}

#[derive(Debug)]
struct Running {
    scenario: Scenario,
    /// Groups are flattened away, so a grouped script plays frame for frame like an ungrouped one.
    plan: Vec<Step>,
    next_step: usize,
    ops: VecDeque<Op>,
    cursor: Vec2,
    touches: BTreeMap<u8, Vec2>,
    aim: Option<Aim>,
    aborted: bool,
    tracing: bool,
    resolved: Vec<(usize, TargetTrace)>,
    captures: CapturedValues,
    captured: Vec<(usize, TraceCaptured)>,
    /// `None` marks one still waiting to hold.
    expected: Option<Vec<Option<Outcome>>>,
    scenes_waited: u32,
    scenes_ready: bool,
    expect_waited: u32,
}

#[derive(Resource, Debug, Default)]
pub struct ScenarioPlayer {
    pending: Option<Scenario>,
    setup_failure: Option<String>,
    running: Option<Running>,
    tracing: Option<NoiseFilter>,
    trace: Option<ScenarioTrace>,
    halt_before: Option<usize>,
    halted: bool,
    previous_handler: Option<ErrorHandler>,
}

impl ScenarioPlayer {
    /// The app should have run `scenario.run_setup` and laid its UI out first.
    pub fn start(&mut self, scenario: Scenario) {
        self.running = None;
        self.pending = Some(scenario);
        self.setup_failure = None;
    }

    pub fn start_after_setup(&mut self, scenario: Scenario, setup: Result<bool, String>) {
        self.start(scenario);
        self.setup_failure = setup.err();
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.pending.is_some() || self.running.is_some()
    }

    /// Does nothing once the expectations have begun to run.
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

    fn sync_error_handler(&mut self, world: &mut World) {
        match (self.is_active(), self.previous_handler) {
            (true, None) => self.previous_handler = Some(catch::install(world)),
            (false, Some(previous)) => {
                catch::restore(world, previous);
                self.previous_handler = None;
            }
            (true, Some(_)) | (false, None) => {}
        }
    }

    pub fn stop(&mut self) {
        self.pending = None;
        self.running = None;
        self.setup_failure = None;
        self.halt_before = None;
        self.halted = false;
    }

    /// Needs [`ScenarioPlayerPlugin`], which takes the points.
    pub fn enable_trace(&mut self, filter: NoiseFilter) {
        self.tracing = Some(filter);
    }

    #[must_use]
    pub const fn trace(&self) -> Option<&ScenarioTrace> {
        self.trace.as_ref()
    }

    pub const fn take_trace(&mut self) -> Option<ScenarioTrace> {
        self.trace.take()
    }

    /// `None` lets the run on. Call before [`ScenarioPlayer::start`] or while the run is going.
    pub const fn pause_before(&mut self, index: Option<usize>) {
        self.halt_before = index;
        self.halted = false;
    }

    #[must_use]
    pub const fn halted(&self) -> bool {
        self.halted
    }

    #[must_use]
    pub fn steps_started(&self) -> usize {
        self.running.as_ref().map_or(0, |run| run.next_step)
    }

    /// A player with no scenario counts as between steps; a step waiting on a check or target does not.
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

fn tracing(player: Option<bevy::ecs::system::Res<ScenarioPlayer>>) -> bool {
    player.is_some_and(|player| player.tracing.is_some())
}

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

fn catching(player: Option<bevy::ecs::system::Res<ScenarioPlayer>>) -> bool {
    player.is_some_and(|player| player.previous_handler.is_some())
}

fn sync_error_handler(world: &mut World) {
    let Some(mut player) = world.remove_resource::<ScenarioPlayer>() else {
        return;
    };
    player.sync_error_handler(world);
    world.insert_resource(player);
}

fn fail_caught_errors(world: &mut World) {
    let caught = catch::take();
    if caught.is_empty() {
        return;
    }
    let Some(mut player) = world.remove_resource::<ScenarioPlayer>() else {
        return;
    };
    if let Some(mut report) = world.get_resource_mut::<ScenarioReport>() {
        let message = caught.join("; ");
        let index = report
            .steps
            .iter()
            .rposition(|record| record.outcome != Outcome::Skipped);
        if let Some(index) = index {
            let message = match report.steps.get(index).map(|record| &record.outcome) {
                Some(Outcome::Fail(earlier)) => format!("{earlier}; {message}"),
                _ => message,
            };
            if let Some(run) = player.running.as_mut() {
                run.abort(&mut report, index, message);
            } else {
                fail_step(&mut report, index, message);
            }
        } else {
            report.setup_failure = Some(message);
            report.finished = true;
            player.running = None;
        }
    }
    world.insert_resource(player);
}

fn play(world: &mut World) {
    advance_world(world, true);
}

/// With `settled` false it moves on only when the next op is a frame of input, so a hover
/// check or target lookup never sees stale state.
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

fn ease(t: f32) -> f32 {
    t * t * 2.0f32.mul_add(-t, 3.0)
}

fn fail_step(report: &mut ScenarioReport, index: usize, message: String) {
    if let Some(record) = report.steps.get_mut(index) {
        record.outcome = Outcome::Fail(message);
    }
}

fn record_check(report: &mut ScenarioReport, name: &str, kind: CheckKind, outcome: Outcome) {
    report.checks.push(CheckRecord {
        name: name.into(),
        kind,
        outcome,
    });
}

impl ScenarioPlayer {
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
        // What the script last did beats the real cursor, so a gallery restart does not depend on the mouse.
        let cursor = world
            .get_resource::<SyntheticInput>()
            .and_then(SyntheticInput::cursor)
            .or_else(|| {
                world
                    .query_filtered::<&Window, With<PrimaryWindow>>()
                    .iter(world)
                    .next()
                    .and_then(Window::cursor_position)
            })
            .unwrap_or(ORIGIN);
        if let Some(mut pictures) = world.get_resource_mut::<Pictures>() {
            pictures.reset();
        }
        self.running = Some(Running {
            scenario,
            plan,
            next_step: 0,
            ops: VecDeque::new(),
            cursor,
            touches: BTreeMap::new(),
            aim: None,
            aborted: false,
            tracing: self.tracing.is_some(),
            resolved: Vec::new(),
            captures: CapturedValues::default(),
            captured: Vec::new(),
            expected: None,
            scenes_waited: 0,
            scenes_ready: false,
            expect_waited: 0,
        });
    }

    fn advance(&mut self, world: &mut World, report: &mut ScenarioReport, settled: bool) {
        self.advance_frame(world, report, settled);
        if let Some(run) = self.running.as_mut() {
            move_targets(&mut self.trace, run);
        }
    }

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
        match run.await_scenes(world, report) {
            SceneWait::Ready => {}
            SceneWait::Waiting => return,
            SceneWait::TimedOut => {
                self.running = None;
                return;
            }
        }
        loop {
            let Some(op) = run.ops.pop_front() else {
                if self.halt_before.is_some_and(|index| run.next_step >= index) {
                    // This frame did not happen; the next call takes up from here.
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
                    if run.play_check(world, report, &mut self.trace, step, check, waited) {
                        return;
                    }
                }
                Op::Caption(text) => report.caption = Some(text),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SceneWait {
    Ready,
    Waiting,
    TimedOut,
}

impl Running {
    fn await_scenes(&mut self, world: &World, report: &mut ScenarioReport) -> SceneWait {
        if self.scenes_ready {
            return SceneWait::Ready;
        }
        let unready = unready_scenes(world);
        if unready.is_empty() {
            self.scenes_ready = true;
            SceneWait::Ready
        } else if self.scenes_waited < u32::from(self.scenario.action_timeout_frames()) {
            self.scenes_waited += 1;
            SceneWait::Waiting
        } else {
            report.setup_failure = Some(format!(
                "scenes not ready after {} frames: {}",
                self.scenes_waited,
                unready.join(", ")
            ));
            report.finished = true;
            SceneWait::TimedOut
        }
    }
}

/// A scene is applied in the frame it resolves, so a resolved scene is a spawned one.
fn unready_scenes(world: &World) -> Vec<String> {
    let Some(server) = world.get_resource::<AssetServer>() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if let Some(patches) = world.get_resource::<Assets<ScenePatch>>() {
        for (id, patch) in patches.iter() {
            if patch.resolved.is_none() {
                lines.push(waiting_for(
                    server,
                    "scene",
                    id.untyped(),
                    &patch.dependencies,
                ));
            }
        }
    }
    if let Some(lists) = world.get_resource::<Assets<SceneListPatch>>() {
        for (id, list) in lists.iter() {
            if list.resolved.is_none() {
                lines.push(waiting_for(
                    server,
                    "scene list",
                    id.untyped(),
                    &list.dependencies,
                ));
            }
        }
    }
    lines.sort();
    lines
}

fn asset_name(server: &AssetServer, id: UntypedAssetId) -> String {
    server
        .get_path(id)
        .map_or_else(|| id.to_string(), |path| path.to_string())
}

fn waiting_for(
    server: &AssetServer,
    kind: &str,
    id: UntypedAssetId,
    dependencies: &[UntypedHandle],
) -> String {
    let mut missing: Vec<String> = dependencies
        .iter()
        .filter(|handle| !server.is_loaded_with_dependencies(handle.id()))
        .map(|handle| asset_name(server, handle.id()))
        .collect();
    missing.sort();
    if missing.is_empty() {
        format!("{kind} {} (not resolved)", asset_name(server, id))
    } else {
        format!(
            "{kind} {} waits for {}",
            asset_name(server, id),
            missing.join(", ")
        )
    }
}

impl Running {
    /// Returns whether the frame is over, which it is while the check waits for its picture or a `Within`.
    fn play_check(
        &mut self,
        world: &mut World,
        report: &mut ScenarioReport,
        trace: &mut Option<ScenarioTrace>,
        step: usize,
        check: Check,
        waited: u32,
    ) -> bool {
        let name = check.to_string();
        let result = match self.poll_check(world, report, &check, waited) {
            Polled::Waiting => {
                // A picture on its way is not waiting for the check to hold.
                self.ops.push_front(Op::Check {
                    step,
                    check,
                    waited,
                });
                return true;
            }
            Polled::Done(None) => {
                self.ops.push_front(Op::Check {
                    step,
                    check,
                    waited: waited + 1,
                });
                return true;
            }
            Polled::Done(Some(result)) => result,
        };
        record_step_check(trace, report, step, &name, result);
        false
    }
}

fn record_step_check(
    trace: &mut Option<ScenarioTrace>,
    report: &mut ScenarioReport,
    step: usize,
    name: &str,
    outcome: Outcome,
) {
    if let Some(trace) = trace.as_mut() {
        trace.add_check(
            step,
            TraceCheck {
                name: name.to_owned(),
                outcome: outcome.clone(),
            },
        );
    }
    record_check(report, name, CheckKind::Step, outcome.clone());
    if let Outcome::Fail(message) = outcome {
        fail_step(report, step, message);
    }
}

fn move_targets(trace: &mut Option<ScenarioTrace>, run: &mut Running) {
    let Some(trace) = trace.as_mut() else {
        return;
    };
    for (step, found) in run.resolved.drain(..) {
        trace.add_target(step, found);
    }
    for (step, captured) in run.captured.drain(..) {
        trace.add_captured(step, captured);
    }
}

impl Running {
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

    fn abort(&mut self, report: &mut ScenarioReport, step: usize, message: String) {
        fail_step(report, step, message);
        self.ops.clear();
        self.next_step = self.plan.len();
        self.aborted = true;
    }

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
            match event {
                SyntheticEvent::MoveCursor(position) => self.cursor = position,
                SyntheticEvent::Touch {
                    finger,
                    phase: TouchPhase::Started | TouchPhase::Moved,
                    position,
                } => {
                    if let Ok(finger) = u8::try_from(finger) {
                        self.touches.insert(finger, position);
                    }
                }
                SyntheticEvent::Touch { finger, .. } => {
                    if let Ok(finger) = u8::try_from(finger) {
                        self.touches.remove(&finger);
                    }
                }
                _ => {}
            }
            input.push(event);
        }
    }

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
            | Step::TouchDown { .. }
            | Step::TouchMove { .. }
            | Step::TouchTap(_)
            | Step::TouchDrag { .. }
            | Step::Click(_)
            | Step::Press(_)
            | Step::Type { into: Some(_), .. }
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
            | Step::TouchDown { .. }
            | Step::TouchMove { .. }
            | Step::TouchTap(_)
            | Step::TouchDrag { .. }
            | Step::Click(_)
            | Step::Press(_)
            | Step::Type { into: Some(_), .. } => {
                if let Err(NotActionable::Yet(message) | NotActionable::Never(message)) =
                    self.act(world, index, &step, true)
                {
                    self.abort(report, index, message);
                }
            }
            Step::TouchUp { finger } => self.lift(report, index, finger, TouchPhase::Ended),
            Step::TouchCancel { finger } => self.lift(report, index, finger, TouchPhase::Canceled),
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
            Step::MoveBurst(positions) => {
                // Positions name no widget, so a press right after checks none.
                self.aim = None;
                let events = positions
                    .iter()
                    .map(|position| SyntheticEvent::MoveCursor(*position))
                    .collect();
                self.frame(index, events);
            }
            Step::Type { text, into: None } => self.type_frames(index, &text),
            Step::Wait(frames) => {
                for _ in 0..frames {
                    self.frame(index, Vec::new());
                }
            }
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
            Step::Capture { name, value } => self.capture(world, report, index, name, &value),
            Step::Caption(text) => self.ops.push_back(Op::Caption(text)),
        }
    }

    fn capture(
        &mut self,
        world: &World,
        report: &mut ScenarioReport,
        index: usize,
        name: String,
        value: &Measure,
    ) {
        match value.read(world) {
            Ok(read) => {
                if self.tracing {
                    self.captured.push((
                        index,
                        TraceCaptured {
                            name: name.clone(),
                            measured: value.to_string(),
                            value: read.ron().to_owned(),
                            type_name: read.type_name().to_owned(),
                        },
                    ));
                }
                self.captures.insert(name, read);
            }
            Err(message) => self.abort(
                report,
                index,
                format!("cannot capture {name:?} ({value}): {message}"),
            ),
        }
    }

    /// A `forced` step skips the checks and the hover check after it.
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
                // Check both ends on the same frame, before anything moves.
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
            Step::TouchDown { .. }
            | Step::TouchMove { .. }
            | Step::TouchTap(_)
            | Step::TouchDrag { .. } => self.act_touch(world, index, step, forced)?,
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
            Step::Type {
                text,
                into: Some(target),
            } => {
                // Click on the frame after the move so a text field is focused before the first character.
                let to = self.reach_traced(world, index, target, Action::Press, forced)?;
                self.move_ops(index, core::iter::once(to.position), to, target, !forced);
                self.aim = None;
                self.frame(
                    index,
                    vec![
                        SyntheticEvent::Button(MouseButton::Left, ButtonState::Pressed),
                        SyntheticEvent::Button(MouseButton::Left, ButtonState::Released),
                    ],
                );
                self.type_frames(index, text);
            }
            _ => {}
        }
        Ok(())
    }

    /// The finger is checked first so a script mistake fails at once.
    fn act_touch(
        &mut self,
        world: &World,
        index: usize,
        step: &Step,
        forced: bool,
    ) -> Result<(), NotActionable> {
        match step {
            Step::TouchDown { at, finger } => {
                self.finger_ready(*finger, false)?;
                let to = self.reach_traced(world, index, at, Action::Press, forced)?;
                self.frame(
                    index,
                    vec![touch_event(*finger, TouchPhase::Started, to.position)],
                );
            }
            Step::TouchMove { to, finger } => {
                self.finger_ready(*finger, true)?;
                let to = self.reach_traced(world, index, to, Action::Point, forced)?;
                self.frame(
                    index,
                    vec![touch_event(*finger, TouchPhase::Moved, to.position)],
                );
            }
            Step::TouchTap(target) => {
                self.finger_ready(0, false)?;
                let at = self.reach_traced(world, index, target, Action::Press, forced)?;
                // Lift a frame later: Bevy clicks only an entity the pointer hovered the frame before.
                self.frame(
                    index,
                    vec![touch_event(0, TouchPhase::Started, at.position)],
                );
                self.frame(index, vec![touch_event(0, TouchPhase::Ended, at.position)]);
            }
            Step::TouchDrag { from, to, frames } => {
                self.finger_ready(0, false)?;
                let start = self.reach_traced(world, index, from, Action::Press, forced)?;
                let end = self.reach_traced(world, index, to, Action::Point, forced)?;
                self.frame(
                    index,
                    vec![touch_event(0, TouchPhase::Started, start.position)],
                );
                let frames = (*frames).max(1);
                for frame in 1..=frames {
                    let position = start
                        .position
                        .lerp(end.position, ease(f32::from(frame) / f32::from(frames)));
                    self.frame(index, vec![touch_event(0, TouchPhase::Moved, position)]);
                }
                self.frame(index, vec![touch_event(0, TouchPhase::Ended, end.position)]);
            }
            _ => {}
        }
        Ok(())
    }

    fn finger_ready(&self, finger: u8, must_be_down: bool) -> Result<(), NotActionable> {
        match (self.touches.contains_key(&finger), must_be_down) {
            (true, false) => Err(NotActionable::Never(format!(
                "finger {finger} is already down"
            ))),
            (false, true) => Err(NotActionable::Never(finger_not_down(finger))),
            _ => Ok(()),
        }
    }

    fn lift(&mut self, report: &mut ScenarioReport, index: usize, finger: u8, phase: TouchPhase) {
        match self.touches.get(&finger).copied() {
            Some(position) => self.frame(index, vec![touch_event(finger, phase, position)]),
            None => self.abort(report, index, finger_not_down(finger)),
        }
    }

    fn type_frames(&mut self, step: usize, text: &str) {
        for character in text.chars() {
            self.frame(step, character_events(character));
        }
    }

    fn aim_at(&mut self, to: ResolvedTarget, target: &Target, forced: bool) {
        self.aim = to.widget.filter(|_| !forced).map(|widget| Aim {
            widget,
            label: format!("{target:?}"),
        });
    }

    fn frame(&mut self, step: usize, events: Vec<SyntheticEvent>) {
        self.ops.push_back(Op::Frame(step, events));
    }

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

    /// Returns whether it finished; otherwise a frame passes and it runs again.
    fn finish(&mut self, world: &mut World, report: &mut ScenarioReport) -> bool {
        let expects = self.scenario.expects();
        let waited = self.expect_waited;
        let mut outcomes = self
            .expected
            .take()
            .unwrap_or_else(|| vec![None; expects.len()]);
        for (expect, outcome) in expects.iter().zip(outcomes.iter_mut()) {
            if outcome.is_some() {
                continue;
            }
            *outcome = if self.aborted {
                Some(Outcome::Skipped)
            } else {
                match self.poll_check(world, report, expect, waited) {
                    Polled::Waiting => {
                        // A picture is on its way: the other expectations keep their place.
                        self.expected = Some(outcomes);
                        return false;
                    }
                    Polled::Done(outcome) => outcome,
                }
            };
        }
        if outcomes.iter().any(Option::is_none) {
            self.expect_waited += 1;
            self.expected = Some(outcomes);
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

enum Polled {
    /// Poll again after a frame, without counting it as waiting for the check to hold.
    Waiting,
    Done(Option<Outcome>),
}

impl Running {
    fn poll_check(
        &self,
        world: &mut World,
        report: &mut ScenarioReport,
        check: &Check,
        waited: u32,
    ) -> Polled {
        let readiness = screenshot::prepare(
            world,
            check,
            self.scenario.base_dir(),
            self.scenario.name(),
            &mut report.screenshots,
        );
        if readiness == Readiness::Waiting {
            return Polled::Waiting;
        }
        let outcome = check.poll(world, &self.captures, waited);
        screenshot::forget(world);
        Polled::Done(outcome)
    }
}

/// Returns the frames it may wait (innermost `Timeout(..)`, else `timeout`) and whether it is forced.
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

fn finger_not_down(finger: u8) -> String {
    format!("finger {finger} is not down")
}

fn touch_event(finger: u8, phase: TouchPhase, position: Vec2) -> SyntheticEvent {
    SyntheticEvent::Touch {
        finger: u64::from(finger),
        phase,
        position,
    }
}

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

const SETTLE_FRAMES: usize = 2;

impl Scenario {
    /// Gets [`ScenarioPlayerPlugin`] if the app lacks it, even after the app is finished.
    /// The window size and scale factor the scenario sets stay on the primary window.
    pub fn run_in(&self, app: &mut App) -> ScenarioReport {
        self.play_in(app, None).0
    }

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

    #[must_use]
    pub fn run_headless_traced(&self) -> (ScenarioReport, ScenarioTrace) {
        self.run_in_traced(&mut self.headless_app(), NoiseFilter::default())
    }

    fn play_in(
        &self,
        app: &mut App,
        filter: Option<NoiseFilter>,
    ) -> (ScenarioReport, Option<ScenarioTrace>) {
        if !app.world().contains_resource::<ScenarioPlayer>() {
            // `add_plugins` refuses once the app is finished; build by hand.
            Plugin::build(&ScenarioPlayerPlugin, app);
        }
        self.fit_primary_window(app.world_mut());
        let rem_before = self.fit_rem_size(app.world_mut());
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
        // A panic in the first update should be caught too.
        sync_error_handler(app.world_mut());
        while app
            .world()
            .get_resource::<ScenarioPlayer>()
            .is_some_and(ScenarioPlayer::is_active)
        {
            app.update();
        }
        sync_error_handler(app.world_mut());
        let trace = app
            .world_mut()
            .get_resource_mut::<ScenarioPlayer>()
            .and_then(|mut player| player.take_trace());
        let report = app
            .world()
            .get_resource::<ScenarioReport>()
            .cloned()
            .unwrap_or_default();
        // The app was handed in, so restore its text size.
        restore_rem_size(app.world_mut(), rem_before);
        (report, trace)
    }

    /// Applies the scale factor first because the logical size is derived from the physical one.
    fn fit_primary_window(&self, world: &mut World) {
        let scale = self
            .scale()
            .filter(|scale| check_scale_factor(*scale).is_ok());
        let size = self
            .window()
            .map(|(width, height)| UVec2::new(width, height).as_vec2());
        if scale.is_none() && size.is_none() {
            return;
        }
        for mut window in world
            .query_filtered::<&mut Window, With<PrimaryWindow>>()
            .iter_mut(world)
        {
            let resolution = &mut window.resolution;
            let (width, height) = size.map_or_else(
                || (resolution.width(), resolution.height()),
                |size| (size.x, size.y),
            );
            if let Some(scale) = scale
                && resolution.scale_factor_override() != Some(scale)
            {
                resolution.set_scale_factor_override(Some(scale));
            }
            if (resolution.width(), resolution.height()) != (width, height) {
                resolution.set(width, height);
            }
        }
    }

    #[must_use]
    pub fn run_headless(&self) -> ScenarioReport {
        self.run_in(&mut self.headless_app())
    }

    /// Add what the scenario needs (for example [`crate::script::Registry::install`]) before [`Scenario::run_in`].
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
