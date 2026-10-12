//! Recording a scenario from a real session.
//!
//! Use the app by hand and save what was done as a scenario file, which then
//! runs in tests, the recorder and the gallery like any other.
//!
//! [`SessionPlugin`] watches the real mouse and keyboard of the primary
//! window. Between [`start`] and [`stop`] (or the gallery's **Record**
//! button) it logs input against a frame counter at the fixed 1/60 s, so the
//! script plays back the same whatever frame rate the session ran at. On stop
//! the log becomes readable steps (see [`steps_from_events`]):
//!
//! * runs of moves become `Glide`s along a simplified path;
//! * a press and release in place becomes a `Click`, a straight press, move
//!   and release a `Drag`;
//! * a click on a UI node with a label becomes `Target::text(..)`; anything
//!   else is offered to the app's [`AppTarget::at`](crate::AppTarget::at)
//!   and stays `screen(..)` if the app has no answer;
//! * idle time beyond a threshold is shortened to a short `Wait`.
//!
//! # What the app plugs in
//!
//! The crate knows nothing about the app, so it asks the app's own reflected
//! types (see [`crate::extension`]), found with no registration call:
//!
//! * [`AppSetup::capture`](crate::AppSetup::capture) runs on start and turns
//!   the state into one of the app's setups, writing whatever it needs into
//!   the directory (rails writes a save file); that is the script's `setup`;
//! * [`AppView::current`](crate::AppView::current) names the view the app is
//!   in, which is the script's `view`;
//! * [`AppTarget::at`](crate::AppTarget::at) turns a click position into one
//!   of the app's targets (rails turns ground clicks into lattice points).
//!
//! [`SessionHooks`] holds the rest:
//!
//! * [`SessionHooks::noise_filter`] says which types the world snapshots
//!   leave out;
//! * [`SessionHooks::checkpoint_key`] picks the checkpoint key (F8 unless
//!   set);
//! * [`SessionHooks::checkpoint_names`] turns the prompt for a checkpoint's
//!   name on or off (on unless set).
//!
//! # Suggested checks
//!
//! On start and on stop the recorder takes a [`WorldSnapshot`]: reflected
//! resources, component counts, the UI's texts and widget states, Bevy
//! states and the app's own facts (see [`crate::check::suggest`]). What changed
//! between the two becomes the script's `expect`, every check of it one that
//! holds at the end, in a fixed order; [`SavedSession::candidates`] keeps
//! them all and [`SavedSession::save_expect`] writes the file again with only
//! the ones the person wants. The gallery shows them as a checklist. An app
//! that keeps state outside the ECS publishes what matters of it as a
//! reflected resource, which the snapshot then reads.
//!
//! # Checkpoints
//!
//! Checks at the end cannot say what was true half-way through. Pressing the
//! checkpoint key (F8 unless [`SessionHooks::checkpoint_key`] sets another)
//! in the primary window while recording, or calling [`checkpoint`], takes a
//! snapshot there. It is compared with the snapshot of the checkpoint before,
//! or of the start for the first one, and what changed becomes `Check` steps
//! at that point in the script, each one a check that held when the
//! checkpoint was taken. The end's `expect` is then what changed since the
//! last checkpoint. [`SavedSession::checkpoints`] keeps every checkpoint's
//! candidates and [`SavedSession::save_checks`] writes the file again with the
//! ones the person wants; the gallery shows one checklist per checkpoint.
//!
//! The key never reaches the app while a session is recorded: its presses
//! and releases in the primary window are taken out of Bevy's keyboard
//! messages before `InputSystems` reads them, and are not logged as steps.
//! Each checkpoint also writes a [`CheckpointTaken`] message, which the
//! cursor overlay shows as a flash and a caption.
//!
//! # Naming a checkpoint
//!
//! Right after the key is pressed a prompt asks for a name, which the overlay
//! shows. Type it (the app sees none of those keys), then Enter keeps it and
//! Escape does without; a mouse click, the next checkpoint or the stop keeps
//! what was typed. A named checkpoint turns the steps recorded since the last
//! checkpoint, and its own `Check` steps, into one `Step::Group` called that
//! name, so a failure in a played back script reads `raise the bridge > step 3:
//! ...`. A checkpoint with no name leaves its steps where they are.
//! [`checkpoint_named`] takes a named one directly, and
//! [`SessionHooks::checkpoint_names`] turns the prompt off.
//!
//! # Checks chosen by hand
//!
//! [`add_check`] puts a check of the person's choosing into the script where
//! the session is now, as a `Check` step that stays whatever is ticked in the
//! checklists. The gallery's **Pick** mode does this with a check about a
//! node the person clicked (see [`crate::check::pick`]).
//!
//! # Limits
//!
//! Only the primary window is recorded, and nothing carrying
//! [`crate::NotUnderTest`] is ever named as a target. A click on a label plays
//! back on the middle of the labelled node, not on the exact pixel. Typed
//! text is not recorded, only key presses. Touch is not recorded.

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

/// The longest name typed for a checkpoint, in characters; more is not taken,
/// so a stuck key cannot fill the caption.
const MAX_NAME_CHARS: usize = 60;

/// How many pixels one line of mouse wheel is worth in a script, which turns
/// the wheel in pixels.
pub const LINE_PIXELS: f32 = 24.0;

/// System sets of the session recorder, in `PreUpdate`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionSystems {
    /// Takes the checkpoint key out of the keyboard messages while recording
    /// and takes a checkpoint for each press. Before `InputSystems`, so the
    /// app never sees the key, and before [`SessionSystems::Record`].
    Checkpoint,
    /// Logs the real input of this update and carries out start and stop
    /// requests. After the gallery has sorted the real input, so what it
    /// removed is not logged.
    Record,
}

/// Where recordings go and what the app plugs in.
#[derive(Resource, Debug, Clone)]
pub struct SessionConfig {
    /// The directory the script and whatever the app's setup capture wrote
    /// go to.
    pub dir: PathBuf,
    /// What the app plugs in.
    pub hooks: SessionHooks,
    /// The thresholds that turn the log into steps.
    pub options: StepOptions,
}

/// Records sessions into `dir`; see the [module documentation](self).
///
/// The checkpoint key asks for a name by default, and that prompt is drawn
/// only by [`CursorOverlayPlugin`](crate::overlay::CursorOverlayPlugin). An
/// app that adds this plugin without the overlay sets
/// [`SessionHooks::checkpoint_names`] to `false`, or its keys are taken by a
/// prompt nobody can see.
#[derive(Debug, Clone)]
pub struct SessionPlugin {
    /// The configuration to insert.
    config: SessionConfig,
}

impl SessionPlugin {
    /// A recorder that writes into `dir` and has no hooks.
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

    /// Sets what the app plugs in.
    #[must_use]
    pub fn with_hooks(mut self, hooks: SessionHooks) -> Self {
        self.config.hooks = hooks;
        self
    }

    /// Sets the thresholds that turn the log into steps.
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

/// Starting or stopping a session failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionError(String);

impl SessionError {
    /// An error with this message.
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

/// A checkpoint taken while recording, as the overlay hears of it.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointTaken {
    /// Which checkpoint of the session it is, counting from 1.
    pub number: usize,
    /// How many checks the changes since the checkpoint before suggested.
    pub candidates: usize,
}

/// A checkpoint of a saved session: where it is and what it suggested.
#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    /// How many of the recorded steps, not counting any checkpoint's checks,
    /// come before it.
    pub at: usize,
    /// The name it was given, if any; the steps since the checkpoint before,
    /// and its checks, are a [`Step::Group`] of this name.
    pub name: Option<String>,
    /// The checks the world's changes since the checkpoint before (or the
    /// start) suggested, in order; each held when the checkpoint was taken.
    pub candidates: Vec<Check>,
}

/// A session that was written out.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedSession {
    /// The `.ron` file.
    pub path: PathBuf,
    /// What was written to it.
    pub script: Script,
    /// The checks the world's changes since the last checkpoint (or the
    /// start) suggested, in order; the file's `expect` starts as all of them.
    pub candidates: Vec<Check>,
    /// The checkpoints, in order; the file starts with all of each one's
    /// candidates as `Check` steps where it was taken.
    pub checkpoints: Vec<Checkpoint>,
    /// The steps without any checkpoint's checks, which
    /// [`Self::save_checks`] puts them back into.
    recorded: Vec<Step>,
}

impl SavedSession {
    /// Writes the file again with `expect` as the script's expect list, such
    /// as the candidates the person ticked. The checkpoints' checks stay as
    /// they are.
    ///
    /// # Errors
    ///
    /// [`SessionError`] when the script cannot be written as RON or the file
    /// cannot be written.
    pub fn save_expect(&mut self, expect: Vec<Check>) -> Result<(), SessionError> {
        self.script.expect = expect;
        write_script(&self.path, &self.script)
    }

    /// Writes the file again with `at_checkpoints[n]` as the `Check` steps of
    /// checkpoint `n` and `expect` as the expect list, such as the candidates
    /// the person ticked. A checkpoint `at_checkpoints` has no entry for gets
    /// no checks.
    ///
    /// # Errors
    ///
    /// [`SessionError`] when the script cannot be written as RON or the file
    /// cannot be written.
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

/// `steps` with each checkpoint's checks put in as `Check` steps: those of
/// checkpoint `n` after the first `checkpoints[n].at` steps, with the
/// candidates of checkpoint `n` replaced by `checks[n]`.
///
/// A named checkpoint wraps what led to it in a [`Step::Group`]: the steps
/// since the checkpoint before it, and its own checks.
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

/// Writes `script` to `path`, after the [`HEADER`].
fn write_script(path: &std::path::Path, script: &Script) -> Result<(), SessionError> {
    let ron = script
        .to_ron()
        .map_err(|error| SessionError::new(error.to_string()))?;
    fs::write(path, format!("{HEADER}{ron}\n"))
        .map_err(|error| SessionError::new(format!("cannot write {}: {error}", path.display())))
}

/// A request to start or stop, made without access to the whole world.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Request {
    /// Start a session called this.
    Start(String),
    /// Stop and save.
    Stop,
}

/// A session in progress.
#[derive(Debug)]
struct Active {
    /// The scenario's name.
    name: String,
    /// File name without extension, shared with the app's setup capture.
    stem: String,
    /// Window size at the start, in logical pixels.
    window_size: Option<(u32, u32)>,
    /// The app's setup, as captured at the start.
    setup: Option<Setup>,
    /// The app's view at the start.
    view: Option<View>,
    /// The world at the last checkpoint, or at the start before the first.
    before: WorldSnapshot,
    /// What each checkpoint so far suggested, in order.
    checkpoints: Vec<Vec<Check>>,
    /// The name of each checkpoint so far, in order; `None` for one never
    /// named.
    names: Vec<Option<String>>,
    /// The name being typed for the last checkpoint, while it is asked for.
    naming: Option<String>,
    /// Keys the name took the press of and whose release is still to come,
    /// so the app sees neither.
    swallowed: Vec<KeyCode>,
    /// Real time spent recording.
    elapsed: Duration,
    /// The log so far.
    events: Vec<RecordedEvent>,
    /// Where the cursor is in the primary window.
    cursor: Option<Vec2>,
    /// Buttons held down, so a stop never leaves one pressed in the log.
    held_buttons: Vec<MouseButton>,
    /// Keys held down.
    held_keys: Vec<KeyCode>,
}

/// The session recorder: whether a session is on, and the last result.
///
/// Start and stop with [`SessionRecorder::request_start`] and
/// [`SessionRecorder::request_stop`] (carried out at the next update), or
/// straight away with [`start`] and [`stop`].
#[derive(Resource, Default)]
pub struct SessionRecorder {
    /// The session in progress.
    active: Option<Active>,
    /// What was asked and not yet done.
    request: Option<Request>,
    /// What the last stop or failed start came to.
    last: Option<Result<SavedSession, SessionError>>,
    /// How far each kind of message has been read.
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

/// Where this recorder has read up to in each kind of message.
#[derive(Default)]
struct Cursors {
    /// Cursor movement.
    moved: MessageCursor<CursorMoved>,
    /// Mouse buttons.
    buttons: MessageCursor<MouseButtonInput>,
    /// The wheel.
    wheel: MessageCursor<MouseWheel>,
    /// Keys.
    keys: MessageCursor<KeyboardInput>,
}

impl SessionRecorder {
    /// Whether a session is being recorded.
    #[must_use]
    pub const fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    /// Asks for a session called `name` to start at the next update.
    pub fn request_start(&mut self, name: impl Into<String>) {
        self.request = Some(Request::Start(name.into()));
    }

    /// Asks for the session to stop and be saved at the next update.
    pub fn request_stop(&mut self) {
        self.request = Some(Request::Stop);
    }

    /// The name typed so far for the last checkpoint, while the prompt for it
    /// is open (see [`SessionHooks::checkpoint_names`]).
    #[must_use]
    pub fn naming(&self) -> Option<&str> {
        self.active.as_ref()?.naming.as_deref()
    }

    /// Whether every key of the primary window is taken for now: a name is
    /// being typed, or the release of a key it took is still to come.
    fn takes_keys(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.naming.is_some() || !active.swallowed.is_empty())
    }

    /// Acts on one key message that was taken out of the app's reach: the
    /// checkpoint key takes a checkpoint and opens the prompt for its name,
    /// and while the prompt is open the keys edit the name. Returns whether
    /// the key stays taken; if not it goes back to the app.
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

    /// How many checkpoints the session in progress has taken, or 0 when none
    /// is on.
    #[must_use]
    pub fn checkpoints_taken(&self) -> usize {
        self.active
            .as_ref()
            .map_or(0, |active| active.checkpoints.len())
    }

    /// What the last stop, or a start that failed, came to.
    #[must_use]
    pub const fn last(&self) -> Option<&Result<SavedSession, SessionError>> {
        self.last.as_ref()
    }
}

/// Starts recording a session called `name`, straight away: the app's setup
/// capture ([`AppSetup::capture`](crate::AppSetup::capture)) runs now and the
/// state it captures is where playback will begin.
///
/// # Errors
///
/// [`SessionError`] when a session is already on, when the plugin is not
/// added, or when the app's setup capture, its view or the directory fails;
/// nothing is recorded then.
pub fn start(world: &mut World, name: &str) -> Result<(), SessionError> {
    with_recorder(world, |world, recorder, config| {
        recorder.begin(world, config, name)
    })
}

/// Takes a checkpoint now, as the checkpoint key does.
///
/// The world is compared with the last checkpoint (or the start) and what
/// changed becomes `Check` steps at this point of the script, before any
/// input of the next update.
///
/// # Errors
///
/// [`SessionError`] when no session is on or the plugin is not added.
pub fn checkpoint(world: &mut World) -> Result<CheckpointTaken, SessionError> {
    with_recorder(world, |world, recorder, config| {
        recorder.checkpoint(world, config, FRAME_STEP, None)
    })
}

/// Takes a checkpoint now, as [`checkpoint`] does, and names it.
///
/// The steps recorded since the last checkpoint, and the `Check` steps of
/// this one, become a `Group` called `name` in the saved script. A blank name
/// is the same as none. No prompt opens, whatever
/// [`SessionHooks::checkpoint_names`] says.
///
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

/// Puts `check` into the recording as a `Check` step at this point.
///
/// It goes before any input of the next update, as choosing a check in the
/// gallery's pick mode does, and stays in the file whatever is ticked in the
/// checklists.
///
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

/// Stops the session and writes it out.
///
/// # Errors
///
/// [`SessionError`] when no session is on or the file cannot be written.
pub fn stop(world: &mut World) -> Result<SavedSession, SessionError> {
    with_recorder(world, |world, recorder, config| {
        recorder.finish(world, config)
    })
}

/// Runs `body` with the recorder and the configuration taken out of `world`.
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

/// The frame of 1/60 s that `elapsed` falls in, rounded to the nearest.
fn frame_of(elapsed: Duration) -> u32 {
    let step = FRAME_STEP.as_nanos().max(1);
    u32::try_from((elapsed.as_nanos() + step / 2) / step).unwrap_or(u32::MAX)
}

/// Messages of kind `M` written since `cursor` last read, taking those
/// `is_key` picks out of `messages` and returning them.
///
/// Messages are taken out by emptying the buffer and writing the rest back.
/// Only those not yet read go back: the older ones every reader has already
/// had, and written again they would be read twice.
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

/// The system: while recording, takes the checkpoint key out of the primary
/// window's keyboard messages and takes a checkpoint for each press; while a
/// checkpoint's name is being typed, takes the typed keys too.
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
        // Reading on keeps the cursors current, so a session that starts later
        // does not take old messages for new ones.
        drop(take_out(world, &mut keys, |_| false));
        drop(take_out(world, &mut window_events, |_| false));
        return;
    };
    let key = config.hooks.checkpoint();
    // Whether keys are being taken is decided before the first message is
    // read, so keys written with the checkpoint key in one update still reach
    // the app.
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

/// The system: logs this update's real input and carries out requests.
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

/// A position as the log keeps it, to a tenth of a pixel, so a file does not
/// fill up with digits nobody needs.
fn tidy(position: Vec2) -> Vec2 {
    (position * 10.0).round() / 10.0
}

/// The primary window and its cursor, if it has both.
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

/// Takes what `cursor` has not read yet out of the world's messages.
fn unread<M: Message + Clone>(world: &World, cursor: &mut MessageCursor<M>) -> Vec<M> {
    world
        .get_resource::<Messages<M>>()
        .map(|messages| cursor.read(messages).cloned().collect())
        .unwrap_or_default()
}

impl SessionRecorder {
    /// Logs this update's real input, or skips it when not recording.
    fn log(&mut self, world: &mut World) {
        let moves = unread(world, &mut self.cursors.moved);
        let buttons = unread(world, &mut self.cursors.buttons);
        let wheel = unread(world, &mut self.cursors.wheel);
        let keys = unread(world, &mut self.cursors.keys);
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

        // Messages of different kinds are not ordered against each other, so
        // within an update the cursor has moved before anything is pressed.
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
                // Going back to the mouse is done with the name.
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

    /// Starts a session: captures the setup, the view and the state, notes
    /// the window size, and begins logging from the next update.
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
        let stem = free_stem(&config.dir, name);
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
        });
        Ok(())
    }

    /// Takes a checkpoint, logged in the frame `ahead` after the last one
    /// logged: before whatever input comes in that frame.
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
        // The name asked for the checkpoint before is final once another is taken.
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

    /// Ends the session and writes the script.
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
        // A button or key still down when the person stops is let go, so the
        // script never ends with one stuck.
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
        // A name still being typed when the person stops is kept.
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

/// What a recorded file starts with. RON allows comments, and this tells the
/// person what they may change.
const HEADER: &str = "\
// Recorded from a session. Edit freely.
// `steps` are what was done: glides, clicks and drags, with long idle time
// shortened. `Check` steps among them were suggested at a checkpoint by what
// changed since the one before; `expect` holds the checks suggested by what
// changed since the last checkpoint (or the start). Keep the ones you want
// checked and delete the rest. A `Group` is a run of steps a checkpoint was
// named for; its name leads the report of every step in it.
";

/// A file-name-safe version of `name` that no file in `dir` has yet.
fn free_stem(dir: &std::path::Path, name: &str) -> String {
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
    let mut candidate = stem.clone();
    let mut number = 1_u32;
    while dir.join(format!("{candidate}.ron")).exists() {
        number += 1;
        candidate = format!("{stem}-{number}");
    }
    candidate
}

impl Active {
    /// Acts on one taken key message while a name is typed or a key it took is
    /// held: edits the name, ends the prompt on Enter (keeping the name) or
    /// Escape (keeping none). Returns whether the app must not see the key.
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

    /// Ends the asking for a name, keeping what was typed (a name of only
    /// blanks is none).
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

/// Shared by the gallery: a path shown to the person.
impl SavedSession {
    /// How the save reads in a status line.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "Saved {} ({} steps)",
            self.path.display(),
            flatten(&self.script.steps).len()
        )
    }
}
