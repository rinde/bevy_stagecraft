//! What the gallery is doing, and what can be asked of it.

use alloc::collections::BTreeMap;
use std::path::PathBuf;

use bevy::ecs::resource::Resource;

use crate::{
    check::{Check, pick::Picked},
    session::{SavedSession, SessionError},
    trace::ScenarioTrace,
};

/// How fast the script plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Speed {
    /// A frame of script every fourth update.
    Quarter,
    /// A frame of script every other update.
    Half,
    /// A frame of script per update.
    Normal,
    /// Up to two frames of script per update.
    ///
    /// The second frame only runs when the script is in the middle of a move
    /// or a wait. Where a step needs the layout or picking of the frame
    /// before (a hover check, a target lookup, a check) it is held back to the
    /// next update, so a fast run never sees stale state.
    Double,
}

impl Speed {
    /// Every speed, slowest first.
    pub const ALL: [Self; 4] = [Self::Quarter, Self::Half, Self::Normal, Self::Double];

    /// How the speed reads on its button.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Quarter => "1/4x",
            Self::Half => "1/2x",
            Self::Normal => "1x",
            Self::Double => "2x",
        }
    }

    /// How many script frames to run in the update numbered `tick`.
    #[must_use]
    pub fn frames_at(self, tick: u32) -> u8 {
        match self {
            Self::Quarter => u8::from(tick.is_multiple_of(4)),
            Self::Half => u8::from(tick.is_multiple_of(2)),
            Self::Normal => 1,
            Self::Double => 2,
        }
    }
}

/// Which panel of the Scenarios window shows the selected entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum GalleryTab {
    /// Its description, its checks and its steps.
    #[default]
    Scenario,
    /// What the run did step by step: see [`crate::trace`].
    Trace,
}

impl GalleryTab {
    /// Every tab, in the order of the tab bar.
    pub const ALL: [Self; 2] = [Self::Scenario, Self::Trace];

    /// How the tab reads on its button.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Scenario => "Scenario",
            Self::Trace => "Trace",
        }
    }
}

/// Which of the two worlds around a step the Trace tab shows the UI of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum TraceWorld {
    /// As the step found it.
    Before,
    /// As the step left it.
    #[default]
    After,
}

impl TraceWorld {
    /// Both, in the order of their buttons.
    pub const ALL: [Self; 2] = [Self::Before, Self::After];

    /// How the world reads on its button.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Before => "Before",
            Self::After => "After",
        }
    }
}

/// Something to do to the gallery. Buttons, keys and code all ask the same way,
/// through [`GalleryState::request`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GalleryAction {
    /// Reset the app and play the entry with this index.
    Select(usize),
    /// Reset the app, play the selected entry up to the start of the step with
    /// this index (counting from 0) and pause there, with the app as it is at
    /// that point, for looking at it. Nothing is stored: runs are
    /// deterministic, so the same frames are simply played again, one script
    /// frame per update whatever the speed.
    JumpTo(usize),
    /// Play or pause; from the end of a run, play it again.
    TogglePlay,
    /// Pause and run one frame of the script.
    StepFrame,
    /// Pause and run to the end of the current step, or of the next one if
    /// the script is between steps.
    StepStep,
    /// Reset the app and play the selected entry from the start.
    Restart,
    /// Choose how fast the script plays.
    SetSpeed(Speed),
    /// Turn looping on or off.
    ToggleLoop,
    /// Pause the script and give the primary window to the real mouse and
    /// keyboard.
    TakeOver,
    /// Give the primary window back to the script and carry on playing.
    Resume,
    /// Write the selected entry as RON into the export directory.
    Export,
    /// Start recording a scenario from a session: pause the script, take the
    /// primary window over and log what the real mouse and keyboard do there
    /// (see [`crate::session`]).
    Record,
    /// Start recording a new scenario from an empty world: whatever is
    /// selected is dropped, the app's reset hook puts its state back, and
    /// nothing is set up before the session starts.
    NewEmpty,
    /// Start recording a new scenario from the app as it is now, scenario or
    /// no scenario: as [`Self::Record`], under the name of a new scenario.
    NewFromCurrent,
    /// Stop recording, write the scenario into the export directory and
    /// show the checks it suggests as a checklist.
    StopRecording,
    /// Tick or untick the check with this index among those suggested at the
    /// end, and write the recording again with the ticked ones as its
    /// `expect`.
    ToggleSuggestion(usize),
    /// Tick or untick a check suggested at a checkpoint, and write the
    /// recording again with the ticked ones as `Check` steps there.
    ToggleCheckpointSuggestion {
        /// The checkpoint, counting from 0.
        checkpoint: usize,
        /// The check among those it suggested.
        index: usize,
    },
    /// Start or stop pick mode, in which a click in the app window picks the
    /// node under the pointer instead of reaching the app. Only while
    /// recording or with a scenario selected, which it pauses.
    TogglePick,
    /// Use the picked node's target with this index for its checks.
    PickTarget(usize),
    /// Take the check with this index among those that hold for the picked
    /// node: a `Check` step at this point of the recording, or else an
    /// expectation of the selected scenario.
    AddPickedCheck(usize),
    /// Step a field of one of the app check forms up or down.
    StepFormField {
        /// The form, in the order of the app check type's variants.
        form: usize,
        /// The field, in order.
        field: usize,
        /// Up rather than down.
        up: bool,
    },
    /// Take the check app check form `n` makes, as [`Self::AddPickedCheck`]
    /// does.
    AddFormCheck(usize),
    /// Show this tab of the Scenarios window's panel.
    ShowTab(GalleryTab),
    /// Turn following on or off in the Trace tab: on, it shows the step being
    /// played (or, once the run is over, the first that failed); off, the step
    /// last chosen.
    ToggleTraceFollow,
    /// Show this step (counting from 0) in the Trace tab, and stop following.
    InspectStep(usize),
    /// Show the UI before or after the step in the Trace tab.
    ShowTraceWorld(TraceWorld),
}

/// What the gallery is in the middle of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GalleryPhase {
    /// Nothing selected yet.
    Idle,
    /// The app has been reset and set up; this many updates of layout and
    /// picking to go before the script starts.
    Settling(u8),
    /// The script is being played.
    Running,
    /// The script is done. When looping, this many more updates of playing
    /// until it starts again.
    Finished(u16),
}

/// How a single-step request ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PendingStep {
    /// After one frame.
    Frame,
    /// At the next step boundary, counted from where the player was.
    Step {
        /// How many steps had begun when asked.
        from_started: usize,
        /// Whether the player was between steps when asked.
        from_boundary: bool,
    },
}

/// Whether a session is being recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recording {
    /// No.
    Off,
    /// Yes.
    On,
}

/// Whether pick mode is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picking {
    /// No: the app window's mouse goes where it would anyway.
    Off,
    /// Yes: a click in the app window picks the node under the pointer.
    On,
}

/// The checks the last recording suggested, as checklists: one per
/// checkpoint, then one for the end.
///
/// Each one is ticked or not. The recording's file holds the ticked ones of
/// a checkpoint as `Check` steps where it was taken, and the ticked ones of
/// the end as its `expect`. All are ticked to begin with, as in the file just
/// written. [`Self::entries`], [`Self::len`] and the like are about the end's
/// list; the `checkpoint_` methods about a checkpoint's.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestions {
    /// The recording, as last written.
    session: SavedSession,
    /// Whether each candidate of the end is ticked, in the candidates' order.
    ticked: Vec<bool>,
    /// Whether each candidate of each checkpoint is ticked.
    at_checkpoints: Vec<Vec<bool>>,
}

impl Suggestions {
    /// The checklists for `session`, every candidate ticked.
    pub(crate) fn new(session: SavedSession) -> Self {
        let ticked = vec![true; session.candidates.len()];
        let at_checkpoints = session
            .checkpoints
            .iter()
            .map(|checkpoint| vec![true; checkpoint.candidates.len()])
            .collect();
        Self {
            session,
            ticked,
            at_checkpoints,
        }
    }

    /// How many checkpoints the recording has, each with its own list.
    #[must_use]
    pub const fn checkpoints(&self) -> usize {
        self.at_checkpoints.len()
    }

    /// Each candidate of checkpoint `checkpoint` (counting from 0), in
    /// order, with whether it is ticked; nothing for a checkpoint there is
    /// not.
    pub fn checkpoint_entries(&self, checkpoint: usize) -> impl Iterator<Item = (&Check, bool)> {
        let candidates = self
            .session
            .checkpoints
            .get(checkpoint)
            .map_or(&[][..], |checkpoint| checkpoint.candidates.as_slice());
        let ticked = self
            .at_checkpoints
            .get(checkpoint)
            .map_or(&[][..], Vec::as_slice);
        candidates.iter().zip(ticked.iter().copied())
    }

    /// Whether candidate `index` of checkpoint `checkpoint` is ticked.
    #[must_use]
    pub fn is_checkpoint_ticked(&self, checkpoint: usize, index: usize) -> bool {
        self.at_checkpoints
            .get(checkpoint)
            .and_then(|ticked| ticked.get(index))
            .copied()
            .unwrap_or(false)
    }

    /// The ticked candidates of checkpoint `checkpoint`, in order.
    #[must_use]
    pub fn checkpoint_ticked(&self, checkpoint: usize) -> Vec<Check> {
        self.checkpoint_entries(checkpoint)
            .filter(|(_, ticked)| *ticked)
            .map(|(check, _)| check.clone())
            .collect()
    }

    /// How many candidates are ticked, and how many there are, over every
    /// list.
    #[must_use]
    pub fn tally(&self) -> (usize, usize) {
        let lists = || {
            self.at_checkpoints
                .iter()
                .chain(core::iter::once(&self.ticked))
        };
        let ticked = lists().flatten().filter(|ticked| **ticked).count();
        let all = lists().map(Vec::len).sum();
        (ticked, all)
    }

    /// The recording, as last written.
    #[must_use]
    pub const fn session(&self) -> &SavedSession {
        &self.session
    }

    /// Each candidate, in order, with whether it is ticked.
    pub fn entries(&self) -> impl Iterator<Item = (&Check, bool)> {
        self.session
            .candidates
            .iter()
            .zip(self.ticked.iter().copied())
    }

    /// How many candidates there are.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.ticked.len()
    }

    /// Whether there are no candidates.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.ticked.is_empty()
    }

    /// Whether the candidate with this index is ticked.
    #[must_use]
    pub fn is_ticked(&self, index: usize) -> bool {
        self.ticked.get(index).copied().unwrap_or(false)
    }

    /// The ticked candidates, in order.
    #[must_use]
    pub fn ticked(&self) -> Vec<Check> {
        self.entries()
            .filter(|(_, ticked)| *ticked)
            .map(|(check, _)| check.clone())
            .collect()
    }

    /// Ticks or unticks the end's candidate with this index and writes the
    /// recording again. An index past the end changes nothing.
    pub(crate) fn toggle(&mut self, index: usize) -> Result<(), SessionError> {
        let Some(ticked) = self.ticked.get_mut(index) else {
            return Ok(());
        };
        *ticked = !*ticked;
        self.save()
    }

    /// Ticks or unticks candidate `index` of checkpoint `checkpoint` and
    /// writes the recording again. A checkpoint or index there is not
    /// changes nothing.
    pub(crate) fn toggle_at_checkpoint(
        &mut self,
        checkpoint: usize,
        index: usize,
    ) -> Result<(), SessionError> {
        let Some(ticked) = self
            .at_checkpoints
            .get_mut(checkpoint)
            .and_then(|ticked| ticked.get_mut(index))
        else {
            return Ok(());
        };
        *ticked = !*ticked;
        self.save()
    }

    /// Writes the recording with the ticked candidates of every list.
    fn save(&mut self) -> Result<(), SessionError> {
        let at_checkpoints: Vec<Vec<Check>> = (0..self.checkpoints())
            .map(|checkpoint| self.checkpoint_ticked(checkpoint))
            .collect();
        let expect = self.ticked();
        self.session.save_checks(&at_checkpoints, expect)
    }
}

/// How one scenario fared in a run of the whole catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioOutcome {
    /// The scenario's name.
    pub name: String,
    /// One line per failed step or check, or the setup's failure; empty when
    /// the scenario passed.
    pub failures: Vec<String>,
}

impl ScenarioOutcome {
    /// Whether the scenario passed.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// What every scenario of the catalogue did, in catalogue order.
///
/// After a run of the whole catalogue (see
/// [`crate::GalleryPlugin::with_run_all`]) the world keeps the complete report
/// as a resource.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct GalleryRunReport {
    /// One outcome per scenario that has finished.
    pub outcomes: Vec<ScenarioOutcome>,
}

impl GalleryRunReport {
    /// Whether every scenario passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.outcomes.iter().all(ScenarioOutcome::passed)
    }

    /// The scenarios that failed, in catalogue order.
    pub fn failed(&self) -> impl Iterator<Item = &ScenarioOutcome> {
        self.outcomes.iter().filter(|outcome| !outcome.passed())
    }

    /// The report in words: the tally, then every scenario with `PASS` or
    /// `FAIL`, and under each failure what went wrong.
    #[must_use]
    pub fn summary(&self) -> String {
        let failed = self.failed().count();
        let mut lines = vec![format!(
            "{} of {} scenarios passed",
            self.outcomes.len() - failed,
            self.outcomes.len()
        )];
        for outcome in &self.outcomes {
            let mark = if outcome.passed() { "PASS" } else { "FAIL" };
            lines.push(format!("{mark} {}", outcome.name));
            lines.extend(
                outcome
                    .failures
                    .iter()
                    .map(|failure| format!("       {failure}")),
            );
        }
        lines.join("\n")
    }
}

/// The gallery's controls and what it remembers: selection, mode, speed and
/// each entry's last result. Ask for things with [`Self::request`].
#[derive(Resource, Debug, Clone)]
pub struct GalleryState {
    /// The entry being played.
    pub(crate) selected: Option<usize>,
    /// Where in a run the gallery is.
    pub(crate) phase: GalleryPhase,
    /// Whether the script advances; pausing clears it.
    pub(crate) playing: bool,
    /// Whether the real mouse and keyboard have the primary window.
    pub(crate) taken_over: bool,
    /// How fast the script plays.
    pub(crate) speed: Speed,
    /// Whether a finished run starts again.
    pub(crate) looping: bool,
    /// How many updates a loop waits between runs.
    pub(crate) loop_pause: u16,
    /// Counts updates, for the speeds slower than one.
    pub(crate) tick: u32,
    /// A single-step request not yet carried out.
    pub(crate) step: Option<PendingStep>,
    /// The step a jump is fast-forwarding to, until the player halts before
    /// it or the script ends.
    pub(crate) jump: Option<usize>,
    /// Requests not yet carried out, oldest first.
    pub(crate) pending: Vec<GalleryAction>,
    /// Whether each entry passed the last time it finished.
    pub(crate) results: BTreeMap<usize, bool>,
    /// Where exports go.
    pub(crate) export_dir: PathBuf,
    /// The last thing worth telling the person, such as where an export went.
    pub(crate) status: String,
    /// Whether a session is being recorded, as of the last request round.
    pub(crate) recording: Recording,
    /// The checks the last recording suggested, until something else is
    /// selected or recorded.
    pub(crate) suggestions: Option<Suggestions>,
    /// Whether a click in the app window picks a node.
    pub(crate) picking: Picking,
    /// The node picked last, until pick mode ends.
    pub(crate) picked: Option<Picked>,
    /// Counts changes to the scenarios themselves, such as an expectation
    /// added by picking, so what shows them is redrawn.
    pub(crate) revision: u32,
    /// How the selected entry's setup went, for the player once the UI has
    /// settled.
    pub(crate) setup: Result<bool, String>,
    /// Set while the whole catalogue is being played once, front to back: what
    /// has finished so far. The next entry is the one after the last of them.
    pub(crate) run_all: Option<GalleryRunReport>,
    /// The Scenarios window's panel being shown.
    pub(crate) tab: GalleryTab,
    /// The trace of the selected entry's last complete run, taken when it
    /// ended: runs are deterministic, so it is also the trace of a run that
    /// is on its way (a jump) and is kept across one.
    pub(crate) trace: Option<ScenarioTrace>,
    /// The step the Trace tab was pinned to; `None` while it follows the
    /// script.
    pub(crate) trace_step: Option<usize>,
    /// Which world around the step the Trace tab shows.
    pub(crate) trace_world: TraceWorld,
}

impl GalleryState {
    /// A state with nothing selected.
    #[must_use]
    pub(crate) const fn new(export_dir: PathBuf, loop_pause: u16) -> Self {
        Self {
            selected: None,
            phase: GalleryPhase::Idle,
            playing: false,
            taken_over: false,
            speed: Speed::Normal,
            looping: true,
            loop_pause,
            tick: 0,
            step: None,
            jump: None,
            pending: Vec::new(),
            results: BTreeMap::new(),
            export_dir,
            status: String::new(),
            recording: Recording::Off,
            suggestions: None,
            picking: Picking::Off,
            picked: None,
            revision: 0,
            setup: Ok(false),
            run_all: None,
            tab: GalleryTab::Scenario,
            trace: None,
            trace_step: None,
            trace_world: TraceWorld::After,
        }
    }

    /// Makes the gallery play every entry once, front to back, with no looping,
    /// and exit when the last has finished.
    pub(crate) fn begin_run_all(&mut self) {
        self.looping = false;
        self.run_all = Some(GalleryRunReport::default());
    }

    /// Whether the whole catalogue is being played automatically, in which
    /// case the controls do nothing.
    #[must_use]
    pub const fn is_running_all(&self) -> bool {
        self.run_all.is_some()
    }

    /// Asks for something; it happens before the script next advances.
    pub fn request(&mut self, action: GalleryAction) {
        self.pending.push(action);
    }

    /// The selected entry's index.
    #[must_use]
    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Where in a run the gallery is.
    #[must_use]
    pub const fn phase(&self) -> GalleryPhase {
        self.phase
    }

    /// The step a jump to a step is still on its way to, if one is.
    #[must_use]
    pub const fn jumping_to(&self) -> Option<usize> {
        self.jump
    }

    /// Whether the script advances (not paused, not taken over).
    #[must_use]
    pub const fn is_playing(&self) -> bool {
        self.playing && !self.taken_over
    }

    /// Whether the real mouse and keyboard have the primary window.
    #[must_use]
    pub const fn is_taken_over(&self) -> bool {
        self.taken_over
    }

    /// Whether the script has run to its end and is not being played again.
    #[must_use]
    pub const fn is_finished(&self) -> bool {
        matches!(self.phase, GalleryPhase::Finished(_))
    }

    /// How fast the script plays.
    #[must_use]
    pub const fn speed(&self) -> Speed {
        self.speed
    }

    /// Whether a finished run starts again.
    #[must_use]
    pub const fn is_looping(&self) -> bool {
        self.looping
    }

    /// Whether entry `index` passed the last time it finished; `None` if it
    /// has not finished yet.
    #[must_use]
    pub fn result(&self, index: usize) -> Option<bool> {
        self.results.get(&index).copied()
    }

    /// The last message for the person, such as where an export went.
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Whether a session is being recorded.
    #[must_use]
    pub const fn is_recording(&self) -> bool {
        matches!(self.recording, Recording::On)
    }

    /// The checks the last recording suggested, shown as a checklist until
    /// something else is selected or recorded.
    #[must_use]
    pub const fn suggestions(&self) -> Option<&Suggestions> {
        self.suggestions.as_ref()
    }

    /// Whether pick mode is on: a click in the app window picks the node
    /// under the pointer, and the app gets none of the mouse.
    #[must_use]
    pub const fn is_picking(&self) -> bool {
        matches!(self.picking, Picking::On)
    }

    /// The node picked last, with its targets and the checks on offer, until
    /// pick mode ends.
    #[must_use]
    pub const fn picked(&self) -> Option<&Picked> {
        self.picked.as_ref()
    }

    /// The panel of the Scenarios window being shown.
    #[must_use]
    pub const fn tab(&self) -> GalleryTab {
        self.tab
    }

    /// The trace of the selected entry's last complete run, if the gallery has
    /// seen one end.
    #[must_use]
    pub const fn trace(&self) -> Option<&ScenarioTrace> {
        self.trace.as_ref()
    }

    /// Whether the Trace tab follows the script.
    #[must_use]
    pub const fn is_following_trace(&self) -> bool {
        self.trace_step.is_none()
    }

    /// The step the Trace tab was pinned to: the one it shows while it is not
    /// following.
    #[must_use]
    pub const fn trace_step(&self) -> Option<usize> {
        self.trace_step
    }

    /// Which world around the step the Trace tab shows.
    #[must_use]
    pub const fn trace_world(&self) -> TraceWorld {
        self.trace_world
    }

    /// Whether the script, not the real mouse and keyboard, has the primary
    /// window: something is selected and it has not been taken over.
    #[must_use]
    pub const fn input_is_scripted(&self) -> bool {
        self.selected.is_some() && !self.taken_over
    }
}
