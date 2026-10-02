//! Plays a scenario through [`SyntheticInput`] and records what happened.

use alloc::collections::VecDeque;

use bevy::{
    app::{App, Plugin, PreUpdate},
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
    headless::HeadlessPlugins,
    input::{SyntheticEvent, SyntheticInput, SyntheticInputSystems},
    probe::{Probe, describe_entity, hovered_entities, is_within},
    scenario::{ORIGIN, Scenario, Step},
    targets::{ResolvedTarget, TargetResolvers, resolve},
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
}

impl ScenarioReport {
    /// Whether the scenario finished with every step and check passing.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.finished
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
                    Outcome::Fail(message) => {
                        Some(format!("step {index} ({:?}): {message}", record.step))
                    }
                    Outcome::Pass | Outcome::Skipped => None,
                });
        let checks = self
            .checks
            .iter()
            .filter_map(|record| match &record.outcome {
                Outcome::Fail(message) => Some(format!("check `{}`: {message}", record.name)),
                Outcome::Pass | Outcome::Skipped => None,
            });
        steps.chain(checks).collect()
    }
}

/// System sets of the player plugin, all in `PreUpdate`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScenarioSystems {
    /// Advances the script by one frame. Runs before the synthetic input is
    /// drained, so what it queues reaches Bevy in the same frame.
    Play,
}

/// Registers [`ScenarioPlayer`], [`ScenarioReport`], [`TargetResolvers`] and
/// the system that advances the script. Needs `SyntheticInputPlugin`, which
/// [`HeadlessPlugins`] includes.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScenarioPlayerPlugin;

impl Plugin for ScenarioPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScenarioPlayer>()
            .init_resource::<ScenarioReport>()
            .init_resource::<TargetResolvers>()
            .add_systems(
                PreUpdate,
                play.in_set(ScenarioSystems::Play)
                    .run_if(not(resource_exists::<ExternallyDriven>))
                    .before(SyntheticInputSystems::Drain),
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
    /// Run the named check.
    Check(usize, String),
    /// Change the caption.
    Caption(String),
}

/// A scenario being played.
#[derive(Debug)]
struct Running {
    /// The scenario.
    scenario: Scenario,
    /// Index of the next step to expand into operations.
    next_step: usize,
    /// Operations expanded from steps but not yet done.
    ops: VecDeque<Op>,
    /// Where the pointer is, as far as the script has moved it.
    cursor: Vec2,
    /// Whether a step failed and the rest is being skipped.
    aborted: bool,
}

/// Plays one scenario at a time, a frame per update.
#[derive(Resource, Debug, Default)]
pub struct ScenarioPlayer {
    /// Waiting to begin on the next frame.
    pending: Option<Scenario>,
    /// Being played.
    running: Option<Running>,
}

impl ScenarioPlayer {
    /// Starts playing `scenario` on the next frame, replacing the report. The
    /// app should have run `scenario.run_setup` and laid its UI out first.
    pub fn start(&mut self, scenario: Scenario) {
        self.running = None;
        self.pending = Some(scenario);
    }

    /// Whether a scenario is waiting to begin or still being played.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.pending.is_some() || self.running.is_some()
    }

    /// Drops the scenario being played or waiting to begin, if any.
    pub fn stop(&mut self) {
        self.pending = None;
        self.running = None;
    }

    /// How many steps have begun, which is also the index of the next one.
    #[must_use]
    pub fn steps_started(&self) -> usize {
        self.running.as_ref().map_or(0, |run| run.next_step)
    }

    /// Whether the player is between steps: no frame of the last one is left.
    /// What may remain are checks and captions, which take no frame. A player
    /// with no scenario counts as between steps.
    #[must_use]
    pub fn at_step_boundary(&self) -> bool {
        self.running
            .as_ref()
            .is_none_or(|run| run.ops.iter().all(|op| !matches!(op, Op::Frame(..))))
    }
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
    /// Does the work of one frame.
    fn advance(&mut self, world: &mut World, report: &mut ScenarioReport, settled: bool) {
        if let Some(scenario) = self.pending.take() {
            *report = ScenarioReport {
                name: scenario.name().into(),
                steps: scenario
                    .script()
                    .iter()
                    .map(|step| StepRecord {
                        step: step.clone(),
                        outcome: Outcome::Skipped,
                        frame: 0,
                    })
                    .collect(),
                ..ScenarioReport::default()
            };
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
                next_step: 0,
                ops: VecDeque::new(),
                cursor,
                aborted: false,
            });
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
                if run.next_step < run.scenario.script().len() {
                    let index = run.next_step;
                    run.next_step += 1;
                    run.expand(world, report, index);
                    continue;
                }
                run.finish(world, report);
                self.running = None;
                return;
            };
            match op {
                Op::Frame(step, events) => {
                    run.apply(world, report, step, events);
                    return;
                }
                Op::CheckHover {
                    step,
                    widget,
                    label,
                } => {
                    if let Err(message) = check_hover(world, widget, &label) {
                        run.abort(report, step, message);
                    }
                }
                Op::Check(step, name) => {
                    let Some(check) = run
                        .scenario
                        .checks()
                        .iter()
                        .find(|check| check.name == name)
                    else {
                        let message = format!("the scenario defines no check named `{name}`");
                        record_check(
                            report,
                            &name,
                            CheckKind::Step,
                            Outcome::Fail(message.clone()),
                        );
                        fail_step(report, step, message);
                        continue;
                    };
                    match check.run(&Probe::new(world)) {
                        Ok(()) => record_check(report, &name, CheckKind::Step, Outcome::Pass),
                        Err(message) => {
                            record_check(
                                report,
                                &name,
                                CheckKind::Step,
                                Outcome::Fail(message.clone()),
                            );
                            fail_step(report, step, message);
                        }
                    }
                }
                Op::Caption(text) => report.caption = Some(text),
            }
        }
    }
}

impl Running {
    /// Stops the script at a failed step; what is left is skipped.
    fn abort(&mut self, report: &mut ScenarioReport, step: usize, message: String) {
        fail_step(report, step, message);
        self.ops.clear();
        self.next_step = self.scenario.script().len();
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
        let Some(step) = self.scenario.script().get(index).cloned() else {
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
        match step {
            Step::MoveTo(target) => {
                let Some(to) = self.resolve(world, report, index, &target) else {
                    return;
                };
                self.move_ops(index, core::iter::once(to.position), to, &target);
            }
            Step::Glide { target, frames } => {
                let Some(to) = self.resolve(world, report, index, &target) else {
                    return;
                };
                self.glide_ops(index, to, frames, &target);
            }
            Step::Drag { from, to, frames } => {
                let Some(start) = self.resolve(world, report, index, &from) else {
                    return;
                };
                let Some(end) = self.resolve(world, report, index, &to) else {
                    return;
                };
                self.move_ops(index, core::iter::once(start.position), start, &from);
                self.ops.push_back(Op::Frame(
                    index,
                    vec![SyntheticEvent::Button(
                        MouseButton::Left,
                        ButtonState::Pressed,
                    )],
                ));
                // The glide starts where the drag does, whatever the cursor
                // was doing before.
                self.cursor = start.position;
                self.glide_ops(index, end, frames, &to);
                self.ops.push_back(Op::Frame(
                    index,
                    vec![SyntheticEvent::Button(
                        MouseButton::Left,
                        ButtonState::Released,
                    )],
                ));
            }
            Step::Press(button) => self.frame(
                index,
                vec![SyntheticEvent::Button(button, ButtonState::Pressed)],
            ),
            Step::Release(button) => self.frame(
                index,
                vec![SyntheticEvent::Button(button, ButtonState::Released)],
            ),
            Step::Click(button) => self.frame(
                index,
                vec![
                    SyntheticEvent::Button(button, ButtonState::Pressed),
                    SyntheticEvent::Button(button, ButtonState::Released),
                ],
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
            Step::Key { .. } | Step::KeyUp { .. } | Step::Tap { .. } => {}
            Step::Check(name) => self.ops.push_back(Op::Check(index, name)),
            Step::Caption(text) => self.ops.push_back(Op::Caption(text)),
        }
    }

    /// Queues one frame of `events`.
    fn frame(&mut self, step: usize, events: Vec<SyntheticEvent>) {
        self.ops.push_back(Op::Frame(step, events));
    }

    /// Works a target out, or fails the step and returns `None`.
    fn resolve(
        &mut self,
        world: &World,
        report: &mut ScenarioReport,
        step: usize,
        target: &crate::targets::Target,
    ) -> Option<ResolvedTarget> {
        match resolve(world, target) {
            Ok(resolved) => Some(resolved),
            Err(message) => {
                self.abort(report, step, message);
                None
            }
        }
    }

    /// Queues one frame per position, then the hover check for the target.
    fn move_ops(
        &mut self,
        step: usize,
        positions: impl IntoIterator<Item = Vec2>,
        to: ResolvedTarget,
        target: &crate::targets::Target,
    ) {
        for position in positions {
            self.frame(step, vec![SyntheticEvent::MoveCursor(position)]);
        }
        if let Some(widget) = to.widget {
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
        target: &crate::targets::Target,
    ) {
        let frames = frames.max(1);
        let from = self.cursor;
        let positions: Vec<Vec2> = (1..=frames)
            .map(|frame| from.lerp(to.position, ease(f32::from(frame) / f32::from(frames))))
            .collect();
        self.move_ops(step, positions, to, target);
    }

    /// Runs the end-of-script expectations and closes the report.
    fn finish(&self, world: &World, report: &mut ScenarioReport) {
        for expect in self.scenario.expects() {
            let outcome = if self.aborted {
                Outcome::Skipped
            } else {
                match expect.run(&Probe::new(world)) {
                    Ok(()) => Outcome::Pass,
                    Err(message) => Outcome::Fail(message),
                }
            };
            record_check(report, &expect.name, CheckKind::Expect, outcome);
        }
        report.finished = true;
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

/// Fails unless `widget`, or something below it, is under the pointer.
fn check_hover(world: &World, widget: Entity, label: &str) -> Result<(), String> {
    let hovered = hovered_entities(world);
    if hovered
        .iter()
        .any(|entity| is_within(world, *entity, widget))
    {
        return Ok(());
    }
    let wanted = describe_entity(world, widget);
    Err(hovered.first().map_or_else(
        || format!("{label} ({wanted}) is not under the pointer: nothing is"),
        |covering| {
            format!(
                "{label} ({wanted}) is not under the pointer: it is covered by {}",
                describe_entity(world, *covering)
            )
        },
    ))
}

/// Frames of layout and picking a fresh UI needs before the script starts.
const SETTLE_FRAMES: usize = 2;

impl Scenario {
    /// Plays the scenario in `app`, which needs [`HeadlessPlugins`] (or the
    /// equivalent) and, if [`ScenarioPlayerPlugin`] is not there yet, gets it
    /// even after the app is finished.
    /// Runs the setup hook and settles the UI first, then returns the report.
    pub fn run_in(&self, app: &mut App) -> ScenarioReport {
        if !app.world().contains_resource::<ScenarioPlayer>() {
            // `add_plugins` refuses once the app is finished, but the app is
            // allowed to gain systems and resources, so build it by hand.
            Plugin::build(&ScenarioPlayerPlugin, app);
        }
        self.run_setup(app.world_mut());
        for _ in 0..SETTLE_FRAMES {
            app.update();
        }
        if let Some(mut player) = app.world_mut().get_resource_mut::<ScenarioPlayer>() {
            player.start(self.clone());
        }
        while app
            .world()
            .get_resource::<ScenarioPlayer>()
            .is_some_and(ScenarioPlayer::is_active)
        {
            app.update();
        }
        app.world()
            .get_resource::<ScenarioReport>()
            .cloned()
            .unwrap_or_default()
    }

    /// Plays the scenario in a fresh [`HeadlessPlugins`] app and returns the
    /// report.
    #[must_use]
    pub fn run_headless(&self) -> ScenarioReport {
        self.run_in(&mut self.headless_app())
    }

    /// A finished [`HeadlessPlugins`] app with the scenario's window size, or
    /// the default one. Use it to add what the scenario needs (for example
    /// [`crate::script::Registry::install`]) before [`Scenario::run_in`].
    pub fn headless_app(&self) -> App {
        let mut plugins = HeadlessPlugins::default();
        if let Some((width, height)) = self.window() {
            plugins = plugins.with_window_size(width, height);
        }
        let mut app = App::new();
        app.add_plugins(plugins);
        app.finish();
        app.cleanup();
        app
    }
}
