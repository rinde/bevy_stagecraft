//! Recording a scenario from a real session.
//!
//! Only the primary window is recorded, and nothing carrying [`crate::NotUnderTest`] is named as a target.

use core::{fmt, time::Duration};
use std::{fs, path::PathBuf};

use bevy::{
    app::{App, Plugin, PreUpdate},
    ecs::{
        entity::Entity,
        message::{Message, MessageCursor, Messages},
        query::With,
        resource::Resource,
        schedule::{IntoScheduleConfigs, SystemSet},
        system::Local,
        world::World,
    },
    input::{
        ButtonState, InputSystems,
        keyboard::{KeyCode, KeyboardInput},
        mouse::{MouseButton, MouseButtonInput, MouseScrollUnit, MouseWheel},
        touch::{TouchInput, TouchPhase},
    },
    math::Vec2,
    time::{Real, Time},
    window::{CursorMoved, PrimaryWindow, Window, WindowEvent},
};

mod hooks;
mod log;
mod target;

pub use hooks::{DEFAULT_CHECKPOINT_KEY, SessionHooks};
pub use log::{LoggedSteps, RecordedEvent, RecordedKind, StepOptions, read_log, steps_from_events};

use crate::{
    check::Check,
    check::suggest::{WorldSnapshot, suggest_checks},
    extension::{Setup, View, capture_setup, current_view},
    headless::FRAME_STEP,
    scenario::{Step, flatten},
    script::{FORMAT_VERSION, Script},
};

/// More is not taken, so a stuck key cannot fill the caption.
const MAX_NAME_CHARS: usize = 60;

/// Scripts turn the wheel in pixels.
pub const LINE_PIXELS: f32 = 24.0;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionSystems {
    /// Runs before `InputSystems` so the app never sees the key, and before [`SessionSystems::Record`].
    Checkpoint,
    /// Runs after the gallery has sorted the real input, so what it removed is not logged.
    Record,
}

#[derive(Resource, Debug, Clone)]
pub struct SessionConfig {
    pub dir: PathBuf,
    pub hooks: SessionHooks,
    pub options: StepOptions,
}

/// Without [`CursorOverlayPlugin`](crate::overlay::CursorOverlayPlugin) the name prompt is
/// invisible; set [`SessionHooks::checkpoint_names`] to `false` then.
#[derive(Debug, Clone)]
pub struct SessionPlugin {
    config: SessionConfig,
}

impl SessionPlugin {
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            config: SessionConfig {
                dir: dir.into(),
                hooks: SessionHooks::default(),
                options: StepOptions::default(),
            },
        }
    }

    #[must_use]
    pub fn with_hooks(mut self, hooks: SessionHooks) -> Self {
        self.config.hooks = hooks;
        self
    }

    #[must_use]
    pub const fn with_options(mut self, options: StepOptions) -> Self {
        self.config.options = options;
        self
    }
}

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone())
            .init_resource::<SessionRecorder>()
            .add_message::<CheckpointTaken>()
            .configure_sets(
                PreUpdate,
                SessionSystems::Checkpoint
                    .before(InputSystems)
                    .before(SessionSystems::Record),
            )
            .add_systems(
                PreUpdate,
                (
                    take_checkpoints.in_set(SessionSystems::Checkpoint),
                    record_session.in_set(SessionSystems::Record),
                ),
            );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionError(String);

impl SessionError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for SessionError {}

#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointTaken {
    pub number: usize,
    pub candidates: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    /// Recorded steps before it, not counting any checkpoint's checks.
    pub at: usize,
    pub name: Option<String>,
    pub candidates: Vec<Check>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavedSession {
    pub path: PathBuf,
    pub script: Script,
    pub candidates: Vec<Check>,
    pub checkpoints: Vec<Checkpoint>,
    /// The steps without any checkpoint's checks, which [`Self::save_checks`] puts back.
    recorded: Vec<Step>,
}

impl SavedSession {
    /// The checkpoints' checks stay as they are.
    /// # Errors
    ///
    /// [`SessionError`] when the script or file cannot be written.
    pub fn save_expect(&mut self, expect: Vec<Check>) -> Result<(), SessionError> {
        self.script.expect = expect;
        write_script(&self.path, &self.script)
    }

    /// A checkpoint with no entry in `at_checkpoints` gets no checks.
    /// # Errors
    ///
    /// [`SessionError`] when the script or file cannot be written.
    pub fn save_checks(
        &mut self,
        at_checkpoints: &[Vec<Check>],
        expect: Vec<Check>,
    ) -> Result<(), SessionError> {
        self.script.steps = with_checks(&self.recorded, &self.checkpoints, at_checkpoints);
        self.script.expect = expect;
        write_script(&self.path, &self.script)
    }
}

fn with_checks(steps: &[Step], checkpoints: &[Checkpoint], checks: &[Vec<Check>]) -> Vec<Step> {
    let mut out = Vec::with_capacity(steps.len() + checks.iter().map(Vec::len).sum::<usize>());
    let mut start = 0;
    for (n, checkpoint) in checkpoints.iter().enumerate() {
        // A checkpoint counted past the end goes last.
        let end = checkpoint.at.min(steps.len()).max(start);
        let mut segment: Vec<Step> = steps.get(start..end).unwrap_or(&[]).to_vec();
        segment.extend(
            checks
                .get(n)
                .into_iter()
                .flatten()
                .cloned()
                .map(Step::Check),
        );
        match &checkpoint.name {
            Some(name) if !segment.is_empty() => out.push(Step::group(name.clone(), segment)),
            _ => out.extend(segment),
        }
        start = end;
    }
    out.extend(steps.get(start..).unwrap_or(&[]).iter().cloned());
    out
}

fn write_script(path: &std::path::Path, script: &Script) -> Result<(), SessionError> {
    let ron = script
        .to_ron()
        .map_err(|error| SessionError::new(error.to_string()))?;
    fs::write(path, format!("{HEADER}{ron}\n"))
        .map_err(|error| SessionError::new(format!("cannot write {}: {error}", path.display())))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Request {
    Start(String),
    Stop,
}

#[derive(Debug)]
struct Active {
    name: String,
    stem: String,
    window_size: Option<(u32, u32)>,
    setup: Option<Setup>,
    view: Option<View>,
    before: WorldSnapshot,
    checkpoints: Vec<Vec<Check>>,
    names: Vec<Option<String>>,
    naming: Option<String>,
    /// Keys whose press the name took, so the app sees neither press nor release.
    swallowed: Vec<KeyCode>,
    elapsed: Duration,
    events: Vec<RecordedEvent>,
    cursor: Option<Vec2>,
    held_buttons: Vec<MouseButton>,
    held_keys: Vec<KeyCode>,
    /// The system's touch id, the finger number the log gives it, and where it last was.
    held_touches: Vec<(u64, u8, Vec2)>,
}

#[derive(Resource, Default)]
pub struct SessionRecorder {
    active: Option<Active>,
    request: Option<Request>,
    last: Option<Result<SavedSession, SessionError>>,
    cursors: Cursors,
}

impl fmt::Debug for SessionRecorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionRecorder")
            .field("recording", &self.active.is_some())
            .field("request", &self.request)
            .field("last", &self.last)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Cursors {
    moved: MessageCursor<CursorMoved>,
    buttons: MessageCursor<MouseButtonInput>,
    wheel: MessageCursor<MouseWheel>,
    keys: MessageCursor<KeyboardInput>,
    touches: MessageCursor<TouchInput>,
}

impl SessionRecorder {
    #[must_use]
    pub const fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    pub fn request_start(&mut self, name: impl Into<String>) {
        self.request = Some(Request::Start(name.into()));
    }

    pub fn request_stop(&mut self) {
        self.request = Some(Request::Stop);
    }

    #[must_use]
    pub fn naming(&self) -> Option<&str> {
        self.active.as_ref()?.naming.as_deref()
    }

    fn takes_keys(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.naming.is_some() || !active.swallowed.is_empty())
    }

    /// Returns whether the key stays taken; if not it goes back to the app.
    fn handle_key(
        &mut self,
        world: &mut World,
        config: &SessionConfig,
        delta: Duration,
        input: &KeyboardInput,
    ) -> bool {
        if input.key_code == config.hooks.checkpoint() {
            if input.state == ButtonState::Pressed && !input.repeat {
                match self.checkpoint(world, config, delta, None) {
                    Ok(_) if config.hooks.asks_names() => {
                        if let Some(active) = self.active.as_mut() {
                            active.naming = Some(String::new());
                        }
                    }
                    Ok(_) => {}
                    Err(error) => bevy::log::warn!("cannot take a checkpoint: {error}"),
                }
            }
            return true;
        }
        self.active
            .as_mut()
            .is_some_and(|active| active.take_key(input))
    }

    #[must_use]
    pub fn checkpoints_taken(&self) -> usize {
        self.active
            .as_ref()
            .map_or(0, |active| active.checkpoints.len())
    }

    #[must_use]
    pub const fn last(&self) -> Option<&Result<SavedSession, SessionError>> {
        self.last.as_ref()
    }
}

/// # Errors
///
/// [`SessionError`] when a session is on, the plugin is not added, or the setup capture, view
/// or directory fails; nothing is recorded then.
pub fn start(world: &mut World, name: &str) -> Result<(), SessionError> {
    with_recorder(world, |world, recorder, config| {
        recorder.begin(world, config, name)
    })
}

/// Takes a checkpoint now, as the checkpoint key does.
/// # Errors
///
/// [`SessionError`] when no session is on or the plugin is not added.
pub fn checkpoint(world: &mut World) -> Result<CheckpointTaken, SessionError> {
    with_recorder(world, |world, recorder, config| {
        recorder.checkpoint(world, config, FRAME_STEP, None)
    })
}

/// A blank name is the same as none, and no prompt opens.
/// # Errors
///
/// [`SessionError`] when no session is on or the plugin is not added.
pub fn checkpoint_named(
    world: &mut World,
    name: impl Into<String>,
) -> Result<CheckpointTaken, SessionError> {
    let name = name.into();
    with_recorder(world, |world, recorder, config| {
        recorder.checkpoint(world, config, FRAME_STEP, Some(name))
    })
}

/// It stays in the file whatever is ticked in the checklists.
/// # Errors
///
/// [`SessionError`] when no session is on or the plugin is not added.
pub fn add_check(world: &mut World, check: Check) -> Result<(), SessionError> {
    with_recorder(world, |_, recorder, _| {
        let active = recorder
            .active
            .as_mut()
            .ok_or_else(|| SessionError::new("no session is being recorded"))?;
        active.events.push(RecordedEvent {
            frame: frame_of(active.elapsed + FRAME_STEP),
            kind: RecordedKind::Check(check),
        });
        Ok(())
    })
}

/// # Errors
///
/// [`SessionError`] when no session is on or the file cannot be written.
pub fn stop(world: &mut World) -> Result<SavedSession, SessionError> {
    with_recorder(world, |world, recorder, config| {
        recorder.finish(world, config)
    })
}

fn with_recorder<T>(
    world: &mut World,
    body: impl FnOnce(&mut World, &mut SessionRecorder, &SessionConfig) -> Result<T, SessionError>,
) -> Result<T, SessionError> {
    let config = world
        .get_resource::<SessionConfig>()
        .cloned()
        .ok_or_else(|| SessionError::new("the session plugin is not added"))?;
    let mut recorder = world
        .remove_resource::<SessionRecorder>()
        .ok_or_else(|| SessionError::new("the session plugin is not added"))?;
    let result = body(world, &mut recorder, &config);
    world.insert_resource(recorder);
    result
}

fn frame_of(elapsed: Duration) -> u32 {
    let step = FRAME_STEP.as_nanos().max(1);
    u32::try_from((elapsed.as_nanos() + step / 2) / step).unwrap_or(u32::MAX)
}

/// Only unread messages go back after emptying the buffer; the older ones would be read twice.
fn take_out<M: Message + Clone>(
    world: &mut World,
    cursor: &mut MessageCursor<M>,
    is_key: impl Fn(&M) -> bool,
) -> Vec<M> {
    let Some(mut messages) = world.get_resource_mut::<Messages<M>>() else {
        return Vec::new();
    };
    let unread = cursor.len(&messages);
    let found = cursor.read(&messages).any(&is_key);
    if !found {
        return Vec::new();
    }
    let all: Vec<M> = messages.drain().collect();
    let older = all.len().saturating_sub(unread);
    let mut taken = Vec::new();
    for message in all.into_iter().skip(older) {
        if is_key(&message) {
            taken.push(message);
        } else {
            messages.write(message);
        }
    }
    *cursor = messages.get_cursor_current();
    taken
}

fn take_checkpoints(
    world: &mut World,
    mut keys: Local<MessageCursor<KeyboardInput>>,
    mut window_events: Local<MessageCursor<WindowEvent>>,
) {
    let recording = world
        .get_resource::<SessionRecorder>()
        .is_some_and(SessionRecorder::is_recording);
    let config = recording
        .then(|| world.get_resource::<SessionConfig>().cloned())
        .flatten();
    let window = primary_window(world).map(|(window, _)| window);
    let (Some(config), Some(window), true) = (config, window, recording) else {
        // Keep the cursors current so a later session does not take old messages for new ones.
        drop(take_out(world, &mut keys, |_| false));
        drop(take_out(world, &mut window_events, |_| false));
        return;
    };
    let key = config.hooks.checkpoint();
    // Decide before reading, so keys written with the checkpoint key in one update still reach the app.
    let typing = world
        .get_resource::<SessionRecorder>()
        .is_some_and(SessionRecorder::takes_keys);
    let taken = take_out(world, &mut keys, |input| {
        input.window == window && (input.key_code == key || typing)
    });
    let mut consumed = Vec::new();
    if !taken.is_empty() {
        let delta = world
            .get_resource::<Time<Real>>()
            .map_or(FRAME_STEP, Time::delta);
        let Some(mut recorder) = world.remove_resource::<SessionRecorder>() else {
            return;
        };
        let mut passed = Vec::new();
        for input in taken {
            if recorder.handle_key(world, &config, delta, &input) {
                consumed.push(input);
            } else {
                passed.push(input);
            }
        }
        world.insert_resource(recorder);
        if let Some(mut messages) = world.get_resource_mut::<Messages<KeyboardInput>>() {
            for input in passed {
                messages.write(input);
            }
            *keys = messages.get_cursor_current();
        }
    }
    // Apps can read the keyboard through the window events too.
    drop(take_out(world, &mut window_events, |event| {
        matches!(event, WindowEvent::KeyboardInput(input)
            if input.window == window && consumed.contains(input))
    }));
}

fn record_session(world: &mut World) {
    let Some(config) = world.get_resource::<SessionConfig>().cloned() else {
        return;
    };
    let Some(mut recorder) = world.remove_resource::<SessionRecorder>() else {
        return;
    };
    recorder.log(world);
    match recorder.request.take() {
        Some(Request::Start(name)) => {
            let result = recorder.begin(world, &config, &name);
            if let Err(error) = result {
                bevy::log::warn!("cannot start recording a session: {error}");
                recorder.last = Some(Err(error));
            }
        }
        Some(Request::Stop) => {
            let result = recorder.finish(world, &config);
            if let Err(error) = &result {
                bevy::log::warn!("cannot save the session: {error}");
            }
            recorder.last = Some(result);
        }
        None => {}
    }
    world.insert_resource(recorder);
}

/// Rounds to a tenth of a pixel so a file does not fill with digits.
fn tidy(position: Vec2) -> Vec2 {
    (position * 10.0).round() / 10.0
}

fn primary_window(world: &mut World) -> Option<(Entity, Option<Vec2>)> {
    let entity = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .iter(world)
        .next()?;
    let cursor = world
        .get::<Window>(entity)
        .and_then(Window::cursor_position);
    Some((entity, cursor))
}

fn unread<M: Message + Clone>(world: &World, cursor: &mut MessageCursor<M>) -> Vec<M> {
    world
        .get_resource::<Messages<M>>()
        .map_or_default(|messages| cursor.read(messages).cloned().collect())
}

impl Active {
    /// A finger that came down before the session began, or that touches down twice, is ignored.
    fn log_touch(&mut self, world: &World, frame: u32, touch: &TouchInput) {
        let position = tidy(touch.position);
        let held = self
            .held_touches
            .iter()
            .position(|(id, ..)| *id == touch.id);
        let finger = match (touch.phase, held) {
            (TouchPhase::Started, None) => {
                self.close_naming();
                let Some(finger) = (0..=u8::MAX).find(|candidate| {
                    self.held_touches
                        .iter()
                        .all(|(_, used, _)| used != candidate)
                }) else {
                    return;
                };
                self.held_touches.push((touch.id, finger, position));
                finger
            }
            (TouchPhase::Moved, Some(at)) => {
                let Some((_, finger, last)) = self.held_touches.get_mut(at) else {
                    return;
                };
                *last = position;
                let finger = *finger;
                // Several moves in one frame are one move, to where it ended.
                if let Some(RecordedEvent {
                    frame: last_frame,
                    kind:
                        RecordedKind::Touch {
                            finger: last_finger,
                            phase: TouchPhase::Moved,
                            position: last_position,
                            target,
                        },
                }) = self.events.last_mut()
                    && *last_frame == frame
                    && *last_finger == finger
                {
                    *last_position = position;
                    *target = target::classify(world, position);
                    return;
                }
                finger
            }
            (TouchPhase::Ended | TouchPhase::Canceled, Some(at)) => {
                let (_, finger, _) = self.held_touches.remove(at);
                finger
            }
            _ => return,
        };
        self.events.push(RecordedEvent {
            frame,
            kind: RecordedKind::Touch {
                finger,
                phase: touch.phase,
                position,
                target: target::classify(world, position),
            },
        });
    }
}

impl SessionRecorder {
    fn log(&mut self, world: &mut World) {
        let moves = unread(world, &mut self.cursors.moved);
        let buttons = unread(world, &mut self.cursors.buttons);
        let wheel = unread(world, &mut self.cursors.wheel);
        let keys = unread(world, &mut self.cursors.keys);
        let touches = unread(world, &mut self.cursors.touches);
        let delta = world
            .get_resource::<Time<Real>>()
            .map_or(FRAME_STEP, Time::delta);
        let Some((window, _)) = primary_window(world) else {
            return;
        };
        let Some(active) = self.active.as_mut() else {
            return;
        };
        active.elapsed += delta;
        let frame = frame_of(active.elapsed);
        let world: &World = world;

        // Kinds are not ordered against each other, so the cursor moves before anything is pressed.
        if let Some(position) = moves
            .iter()
            .filter(|moved| moved.window == window)
            .map(|moved| tidy(moved.position))
            .next_back()
        {
            active.cursor = Some(position);
            active.events.push(RecordedEvent {
                frame,
                kind: RecordedKind::Moved(position),
            });
        }
        for press in buttons.iter().filter(|press| press.window == window) {
            if press.state == ButtonState::Pressed {
                active.close_naming();
            }
            let Some(position) = active.cursor else {
                continue;
            };
            match press.state {
                ButtonState::Pressed => active.held_buttons.push(press.button),
                ButtonState::Released => active.held_buttons.retain(|held| *held != press.button),
            }
            active.events.push(RecordedEvent {
                frame,
                kind: RecordedKind::Button {
                    button: press.button,
                    state: press.state,
                    position,
                    target: target::classify(world, position),
                },
            });
        }
        for turn in wheel.iter().filter(|turn| turn.window == window) {
            let Some(position) = active.cursor else {
                continue;
            };
            let scale = match turn.unit {
                MouseScrollUnit::Line => LINE_PIXELS,
                MouseScrollUnit::Pixel => 1.0,
            };
            active.events.push(RecordedEvent {
                frame,
                kind: RecordedKind::Wheel {
                    x: turn.x * scale,
                    y: turn.y * scale,
                    position,
                },
            });
        }
        for touch in touches.iter().filter(|touch| touch.window == window) {
            active.log_touch(world, frame, touch);
        }
        // The operating system repeats a held key; a script holds it once.
        for key in keys
            .iter()
            .filter(|key| key.window == window && !key.repeat)
        {
            match key.state {
                ButtonState::Pressed => active.held_keys.push(key.key_code),
                ButtonState::Released => active.held_keys.retain(|held| *held != key.key_code),
            }
            active.events.push(RecordedEvent {
                frame,
                kind: RecordedKind::Key {
                    key: key.key_code,
                    state: key.state,
                    text: key.text.as_ref().map(ToString::to_string),
                },
            });
        }
    }

    fn begin(
        &mut self,
        world: &mut World,
        config: &SessionConfig,
        name: &str,
    ) -> Result<(), SessionError> {
        if self.active.is_some() {
            return Err(SessionError::new("a session is already being recorded"));
        }
        fs::create_dir_all(&config.dir).map_err(|error| {
            SessionError::new(format!("cannot create {}: {error}", config.dir.display()))
        })?;
        let stem = free_stem(&config.dir, name)
            .map_err(|error| SessionError::new(format!("no file name could be chosen: {error}")))?;
        let setup = capture_setup(world, &config.dir, &stem).map_err(|error| {
            SessionError::new(format!("the app's setup could not be captured: {error}"))
        })?;
        let view = current_view(world)
            .map_err(|error| SessionError::new(format!("the app's view is unknown: {error}")))?;
        let before = config.hooks.snapshot(world);
        let (_, cursor) = primary_window(world).unzip();
        let window_size = world
            .query_filtered::<&Window, With<PrimaryWindow>>()
            .iter(world)
            .next()
            .map(|window| {
                let size = window.resolution.size().round().as_uvec2();
                (size.x, size.y)
            });
        // Whatever arrived before now is not part of the session.
        drop(unread(world, &mut self.cursors.moved));
        drop(unread(world, &mut self.cursors.buttons));
        drop(unread(world, &mut self.cursors.wheel));
        drop(unread(world, &mut self.cursors.keys));
        drop(unread(world, &mut self.cursors.touches));
        self.active = Some(Active {
            name: name.to_owned(),
            stem,
            window_size,
            setup,
            view,
            before,
            checkpoints: Vec::new(),
            names: Vec::new(),
            naming: None,
            swallowed: Vec::new(),
            elapsed: Duration::ZERO,
            events: Vec::new(),
            cursor: cursor.flatten(),
            held_buttons: Vec::new(),
            held_keys: Vec::new(),
            held_touches: Vec::new(),
        });
        Ok(())
    }

    /// Logged `ahead` frames after the last one, before whatever input comes in that frame.
    fn checkpoint(
        &mut self,
        world: &mut World,
        config: &SessionConfig,
        ahead: Duration,
        name: Option<String>,
    ) -> Result<CheckpointTaken, SessionError> {
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| SessionError::new("no session is being recorded"))?;
        active.close_naming();
        let now = config.hooks.snapshot(world);
        let candidates = suggest_checks(&active.before, &now, world);
        active.before = now;
        active.events.push(RecordedEvent {
            frame: frame_of(active.elapsed + ahead),
            kind: RecordedKind::Checkpoint,
        });
        let taken = CheckpointTaken {
            number: active.checkpoints.len() + 1,
            candidates: candidates.len(),
        };
        active.checkpoints.push(candidates);
        active
            .names
            .push(name.filter(|name| !name.trim().is_empty()));
        if world.write_message(taken).is_none() {
            bevy::log::warn!("checkpoint {} taken, but nothing can show it", taken.number);
        }
        Ok(taken)
    }

    fn finish(
        &mut self,
        world: &World,
        config: &SessionConfig,
    ) -> Result<SavedSession, SessionError> {
        let mut active = self
            .active
            .take()
            .ok_or_else(|| SessionError::new("no session is being recorded"))?;
        let end = frame_of(active.elapsed);
        // Let go of what is still down so the script never ends with one stuck.
        let position = active.cursor.unwrap_or(Vec2::ZERO);
        for button in core::mem::take(&mut active.held_buttons) {
            active.events.push(RecordedEvent {
                frame: end,
                kind: RecordedKind::Button {
                    button,
                    state: ButtonState::Released,
                    position,
                    target: crate::targets::Target::screen(position),
                },
            });
        }
        for key in core::mem::take(&mut active.held_keys) {
            active.events.push(RecordedEvent {
                frame: end,
                kind: RecordedKind::Key {
                    key,
                    state: ButtonState::Released,
                    text: None,
                },
            });
        }
        for (_, finger, position) in core::mem::take(&mut active.held_touches) {
            active.events.push(RecordedEvent {
                frame: end,
                kind: RecordedKind::Touch {
                    finger,
                    phase: TouchPhase::Ended,
                    position,
                    target: crate::targets::Target::screen(position),
                },
            });
        }
        active.close_naming();
        let logged = read_log(&active.events, end, &config.options);
        let after = config.hooks.snapshot(world);
        let candidates = suggest_checks(&active.before, &after, world);
        let checkpoints: Vec<Checkpoint> = logged
            .checkpoints
            .iter()
            .zip(active.checkpoints)
            .zip(active.names)
            .map(|((at, candidates), name)| Checkpoint {
                at: *at,
                name,
                candidates,
            })
            .collect();
        let kept: Vec<Vec<Check>> = checkpoints
            .iter()
            .map(|checkpoint| checkpoint.candidates.clone())
            .collect();
        let script = Script {
            format_version: FORMAT_VERSION,
            name: active.name,
            window_size: active.window_size,
            action_timeout: None,
            // The display's own scale factor is not the scenario's to fix.
            scale_factor: None,
            setup: active.setup,
            view: active.view,
            // A freshly recorded script runs in every variant.
            variants: Vec::new(),
            tags: Vec::new(),
            skip: None,
            fixme: None,
            should_fail: None,
            steps: with_checks(&logged.steps, &checkpoints, &kept),
            expect: candidates.clone(),
        };
        let path = config.dir.join(format!("{}.ron", active.stem));
        write_script(&path, &script)?;
        let saved = SavedSession {
            path,
            script,
            candidates,
            checkpoints,
            recorded: logged.steps,
        };
        self.last = Some(Ok(saved.clone()));
        Ok(saved)
    }
}

/// RON allows comments; this tells the person what they may change.
const HEADER: &str = "\
// Recorded from a session. Edit freely.
// `steps` are what was done: glides, clicks and drags, with long idle time
// shortened. `Check` steps among them were suggested at a checkpoint by what
// changed since the one before; `expect` holds the checks suggested by what
// changed since the last checkpoint (or the start). Keep the ones you want
// checked and delete the rest. A `Group` is a run of steps a checkpoint was
// named for; its name leads the report of every step in it.
";

/// No file in `dir` starts with `"{stem}."`, so an orphaned saved world or capture file is safe too.
fn free_stem(dir: &std::path::Path, name: &str) -> Result<String, crate::save::SaveError> {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let stem = if stem.is_empty() {
        "session".to_owned()
    } else {
        stem
    };
    crate::save::free_session_stem(dir, &stem)
}

impl Active {
    /// Returns whether the app must not see the key.
    fn take_key(&mut self, input: &KeyboardInput) -> bool {
        let key = input.key_code;
        let swallowed = self.swallowed.iter().position(|taken| *taken == key);
        if input.state == ButtonState::Released {
            return swallowed.is_some_and(|index| {
                self.swallowed.remove(index);
                true
            });
        }
        if self.naming.is_none() {
            return swallowed.is_some();
        }
        if swallowed.is_none() {
            self.swallowed.push(key);
        }
        match key {
            KeyCode::Enter | KeyCode::NumpadEnter => self.close_naming(),
            KeyCode::Escape => self.naming = None,
            KeyCode::Backspace => {
                if let Some(typed) = self.naming.as_mut() {
                    typed.pop();
                }
            }
            _ => {
                if let (Some(typed), Some(text)) = (self.naming.as_mut(), input.text.as_ref()) {
                    for letter in text.chars().filter(|letter| !letter.is_control()) {
                        if typed.chars().count() < MAX_NAME_CHARS {
                            typed.push(letter);
                        }
                    }
                }
            }
        }
        true
    }

    /// A name of only blanks is none.
    fn close_naming(&mut self) {
        let Some(typed) = self.naming.take() else {
            return;
        };
        let typed = typed.trim();
        if !typed.is_empty()
            && let Some(slot) = self.names.last_mut()
        {
            *slot = Some(typed.to_owned());
        }
    }
}

impl SavedSession {
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "Saved {} ({} steps)",
            self.path.display(),
            flatten(&self.script.steps).len()
        )
    }
}
