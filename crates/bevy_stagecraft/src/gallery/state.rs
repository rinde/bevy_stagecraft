//! What the gallery is doing, and what can be asked of it.
//!
//! Everything that changes [`GalleryState`] goes through
//! [`GalleryState::request`] with a [`GalleryAction`], which buttons, keys and
//! code all use. It is carried out before the script next advances.
//!
//! **Save as setup** ([`GalleryAction::SaveAsSetup`]) is refused while a script
//! plays ([`GalleryState::can_save_as_setup`]).
//!
//! Results are kept per entry and variant. A run of everything plays every
//! entry in every variant and reports an entry whose scenario does not name the
//! variant as `SKIP`, not as a failure. Export and the pick checks use the
//! catalog's scenario as written.

use alloc::collections::BTreeMap;
use std::path::PathBuf;

use bevy::{ecs::resource::Resource, text::RemSize};

use crate::{
    check::{Check, pick::Picked, suggest::NoiseFilter},
    session::{SavedSession, SessionError},
    trace::ScenarioTrace,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Speed {
    Quarter,
    Half,
    Normal,
    /// Up to two frames per update. The second runs only mid-move or mid-wait;
    /// a step that needs the previous frame's layout or picking waits.
    Double,
}

impl Speed {
    pub const ALL: [Self; 4] = [Self::Quarter, Self::Half, Self::Normal, Self::Double];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Quarter => "1/4x",
            Self::Half => "1/2x",
            Self::Normal => "1x",
            Self::Double => "2x",
        }
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum GalleryTab {
    #[default]
    Scenario,
    Trace,
}

impl GalleryTab {
    pub const ALL: [Self; 2] = [Self::Scenario, Self::Trace];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Scenario => "Scenario",
            Self::Trace => "Trace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum TraceWorld {
    Before,
    #[default]
    After,
}

impl TraceWorld {
    pub const ALL: [Self; 2] = [Self::Before, Self::After];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Before => "Before",
            Self::After => "After",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NoisePreset {
    /// The filter the app set ([`SessionHooks::noise_filter`]).
    ///
    /// [`SessionHooks::noise_filter`]: crate::SessionHooks::noise_filter
    App,
    /// [`NoiseFilter::nothing`]: includes the gallery's own changes.
    Everything,
}

impl NoisePreset {
    pub const ALL: [Self; 2] = [Self::App, Self::Everything];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::App => "App default",
            Self::Everything => "Everything",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiddenList {
    Closed,
    Open,
}

impl HiddenList {
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Closed => Self::Open,
            Self::Open => Self::Closed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GalleryAction {
    Select(usize),
    /// Plays again in the new variant when an entry is selected.
    SelectVariant(usize),
    /// `None` shows all. A run of everything skips the filtered-out entries; an
    /// index with no tag shows everything.
    SelectTag(Option<usize>),
    /// Plays to the start of step `n` (from 0) and pauses, one frame per update
    /// at any speed. Nothing is stored: runs are deterministic.
    JumpTo(usize),
    TogglePlay,
    StepFrame,
    /// Runs to the end of the next step when between steps.
    StepStep,
    Restart,
    SetSpeed(Speed),
    ToggleLoop,
    TakeOver,
    Resume,
    Export,
    /// Refused with a status line while a script plays: the saved moment would
    /// be arbitrary ([`GalleryState::is_script_playing`]).
    SaveAsSetup,
    Record,
    NewEmpty,
    NewFromCurrent,
    StopRecording,
    /// Index among the checks suggested at the end.
    ToggleSuggestion(usize),
    ToggleCheckpointSuggestion {
        checkpoint: usize,
        index: usize,
    },
    /// Only while recording or with a scenario selected, which it pauses.
    TogglePick,
    PickTarget(usize),
    AddPickedCheck(usize),
    StepFormField {
        form: usize,
        field: usize,
        up: bool,
    },
    AddFormCheck(usize),
    ShowTab(GalleryTab),
    ToggleTraceFollow,
    InspectStep(usize),
    ShowTraceWorld(TraceWorld),
    /// Index among the change lines listed, visible first, then hidden when the
    /// list is open. Accepted at any time: only the display changes.
    ToggleType(usize),
    /// Lasts for the gallery session. Accepted at any time.
    UseNoise(NoisePreset),
    ToggleHiddenChanges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GalleryPhase {
    Idle,
    /// Updates of layout and picking left before the script starts.
    Settling(u8),
    Running,
    /// Updates left, when looping, before it starts again.
    Finished(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PendingStep {
    Frame,
    Step {
        from_started: usize,
        from_boundary: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recording {
    Off,
    On,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picking {
    Off,
    On,
}

/// The checks the last recording suggested, all ticked to begin with.
///
/// One checklist per checkpoint, then one for the end. [`Self::entries`],
/// [`Self::len`] and the like are about the end's list; the `checkpoint_`
/// methods about a checkpoint's.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestions {
    session: SavedSession,
    ticked: Vec<bool>,
    at_checkpoints: Vec<Vec<bool>>,
}

impl Suggestions {
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

    #[must_use]
    pub const fn checkpoints(&self) -> usize {
        self.at_checkpoints.len()
    }

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

    #[must_use]
    pub fn is_checkpoint_ticked(&self, checkpoint: usize, index: usize) -> bool {
        self.at_checkpoints
            .get(checkpoint)
            .and_then(|ticked| ticked.get(index))
            .copied()
            .unwrap_or(false)
    }

    #[must_use]
    pub fn checkpoint_ticked(&self, checkpoint: usize) -> Vec<Check> {
        self.checkpoint_entries(checkpoint)
            .filter(|(_, ticked)| *ticked)
            .map(|(check, _)| check.clone())
            .collect()
    }

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

    #[must_use]
    pub const fn session(&self) -> &SavedSession {
        &self.session
    }

    pub fn entries(&self) -> impl Iterator<Item = (&Check, bool)> {
        self.session
            .candidates
            .iter()
            .zip(self.ticked.iter().copied())
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.ticked.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.ticked.is_empty()
    }

    #[must_use]
    pub fn is_ticked(&self, index: usize) -> bool {
        self.ticked.get(index).copied().unwrap_or(false)
    }

    #[must_use]
    pub fn ticked(&self) -> Vec<Check> {
        self.entries()
            .filter(|(_, ticked)| *ticked)
            .map(|(check, _)| check.clone())
            .collect()
    }

    /// An index past the end changes nothing.
    pub(crate) fn toggle(&mut self, index: usize) -> Result<(), SessionError> {
        let Some(ticked) = self.ticked.get_mut(index) else {
            return Ok(());
        };
        *ticked = !*ticked;
        self.save()
    }

    /// A checkpoint or index past the end changes nothing.
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

    fn save(&mut self) -> Result<(), SessionError> {
        let at_checkpoints: Vec<Vec<Check>> = (0..self.checkpoints())
            .map(|checkpoint| self.checkpoint_ticked(checkpoint))
            .collect();
        let expect = self.ticked();
        self.session.save_checks(&at_checkpoints, expect)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioResult {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioOutcome {
    pub name: String,
    pub failures: Vec<String>,
    /// Skipped because the scenario does not run in this variant.
    pub skipped: bool,
}

impl ScenarioOutcome {
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.failures.is_empty() && !self.skipped
    }

    #[must_use]
    pub const fn skipped(&self) -> bool {
        self.skipped
    }

    #[must_use]
    pub const fn failed(&self) -> bool {
        !self.passed() && !self.skipped
    }
}

#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct GalleryRunReport {
    pub outcomes: Vec<ScenarioOutcome>,
}

impl GalleryRunReport {
    /// A skipped scenario is nothing to fail the run over.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.outcomes.iter().all(|outcome| !outcome.failed())
    }

    pub fn failed(&self) -> impl Iterator<Item = &ScenarioOutcome> {
        self.outcomes.iter().filter(|outcome| outcome.failed())
    }

    #[must_use]
    pub fn summary(&self) -> String {
        let failed = self.failed().count();
        let skipped = self.outcomes.iter().filter(|o| o.skipped()).count();
        let mut lines = vec![format!(
            "{} of {} scenarios passed{}",
            self.outcomes.len() - failed - skipped,
            self.outcomes.len(),
            if skipped == 0 {
                String::new()
            } else {
                format!(", {skipped} skipped")
            }
        )];
        for outcome in &self.outcomes {
            let mark = if outcome.skipped() {
                "SKIP"
            } else if outcome.passed() {
                "PASS"
            } else {
                "FAIL"
            };
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

/// The gallery's controls and what it remembers. Ask with [`Self::request`].
#[derive(Resource, Debug, Clone)]
pub struct GalleryState {
    pub(crate) selected: Option<usize>,
    pub(crate) phase: GalleryPhase,
    pub(crate) playing: bool,
    pub(crate) taken_over: bool,
    pub(crate) speed: Speed,
    pub(crate) looping: bool,
    pub(crate) loop_pause: u16,
    pub(crate) tick: u32,
    pub(crate) step: Option<PendingStep>,
    pub(crate) jump: Option<usize>,
    pub(crate) pending: Vec<GalleryAction>,
    pub(crate) variant: usize,
    pub(crate) tag: Option<usize>,
    /// Restored where a variant or scenario sets nothing.
    pub(crate) original_window: Option<(f32, f32, Option<f32>)>,
    pub(crate) original_rem: Option<RemSize>,
    /// Keyed by entry index and variant index.
    pub(crate) results: BTreeMap<(usize, usize), ScenarioResult>,
    pub(crate) export_dir: PathBuf,
    pub(crate) status: String,
    pub(crate) recording: Recording,
    pub(crate) suggestions: Option<Suggestions>,
    pub(crate) picking: Picking,
    pub(crate) picked: Option<Picked>,
    pub(crate) revision: u32,
    pub(crate) setup: Result<bool, String>,
    /// Set while the whole catalog plays; the next entry follows the last finished.
    pub(crate) run_all: Option<GalleryRunReport>,
    pub(crate) tab: GalleryTab,
    /// Also the trace of a run on its way (a jump), since runs are deterministic.
    pub(crate) trace: Option<ScenarioTrace>,
    pub(crate) trace_step: Option<usize>,
    pub(crate) trace_world: TraceWorld,
    /// The trace holds everything; this is applied when the tab is drawn.
    pub(crate) noise: NoiseFilter,
    pub(crate) base_noise: NoiseFilter,
    /// What [`GalleryAction::ToggleType`] indexes; rewritten whenever the tab
    /// is drawn.
    pub(crate) noise_types: Vec<String>,
    pub(crate) hidden_list: HiddenList,
}

impl GalleryState {
    #[must_use]
    pub(crate) fn new(export_dir: PathBuf, loop_pause: u16, noise: NoiseFilter) -> Self {
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
            variant: 0,
            tag: None,
            original_window: None,
            original_rem: None,
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
            noise: noise.clone(),
            base_noise: noise,
            noise_types: Vec::new(),
            hidden_list: HiddenList::Closed,
        }
    }

    pub(crate) fn begin_run_all(&mut self) {
        self.looping = false;
        self.run_all = Some(GalleryRunReport::default());
    }

    #[must_use]
    pub const fn is_running_all(&self) -> bool {
        self.run_all.is_some()
    }

    pub fn request(&mut self, action: GalleryAction) {
        self.pending.push(action);
    }

    /// Keeps the selection and results following the entries they belong to.
    pub(crate) fn insert_entry(&mut self, index: usize) {
        if let Some(selected) = self.selected.as_mut()
            && *selected >= index
        {
            *selected = selected.saturating_add(1);
        }
        self.results = core::mem::take(&mut self.results)
            .into_iter()
            .map(|((entry, variant), result)| {
                let entry = if entry >= index {
                    entry.saturating_add(1)
                } else {
                    entry
                };
                ((entry, variant), result)
            })
            .collect();
    }

    pub(crate) fn forget_results(&mut self, index: usize) {
        self.results.retain(|(entry, _), _| *entry != index);
    }

    #[must_use]
    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    #[must_use]
    pub const fn phase(&self) -> GalleryPhase {
        self.phase
    }

    #[must_use]
    pub const fn jumping_to(&self) -> Option<usize> {
        self.jump
    }

    #[must_use]
    pub const fn is_playing(&self) -> bool {
        self.playing && !self.taken_over
    }

    /// Paused, stepping, taken over, finished and recording are not running.
    #[must_use]
    pub const fn is_script_playing(&self) -> bool {
        self.selected.is_some()
            && !self.taken_over
            && (self.playing || self.jump.is_some())
            && !matches!(self.phase, GalleryPhase::Finished(_))
    }

    #[must_use]
    pub const fn can_save_as_setup(&self) -> bool {
        !self.is_script_playing()
    }

    #[must_use]
    pub const fn is_taken_over(&self) -> bool {
        self.taken_over
    }

    #[must_use]
    pub const fn is_finished(&self) -> bool {
        matches!(self.phase, GalleryPhase::Finished(_))
    }

    #[must_use]
    pub const fn speed(&self) -> Speed {
        self.speed
    }

    #[must_use]
    pub const fn is_looping(&self) -> bool {
        self.looping
    }

    #[must_use]
    pub const fn variant(&self) -> usize {
        self.variant
    }

    #[must_use]
    pub const fn tag(&self) -> Option<usize> {
        self.tag
    }

    #[must_use]
    pub fn result(&self, index: usize) -> Option<ScenarioResult> {
        self.result_in(index, self.variant)
    }

    #[must_use]
    pub fn result_in(&self, index: usize, variant: usize) -> Option<ScenarioResult> {
        self.results.get(&(index, variant)).copied()
    }

    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    #[must_use]
    pub const fn is_recording(&self) -> bool {
        matches!(self.recording, Recording::On)
    }

    #[must_use]
    pub const fn suggestions(&self) -> Option<&Suggestions> {
        self.suggestions.as_ref()
    }

    #[must_use]
    pub const fn is_picking(&self) -> bool {
        matches!(self.picking, Picking::On)
    }

    #[must_use]
    pub const fn picked(&self) -> Option<&Picked> {
        self.picked.as_ref()
    }

    #[must_use]
    pub const fn tab(&self) -> GalleryTab {
        self.tab
    }

    #[must_use]
    pub const fn trace(&self) -> Option<&ScenarioTrace> {
        self.trace.as_ref()
    }

    #[must_use]
    pub const fn is_following_trace(&self) -> bool {
        self.trace_step.is_none()
    }

    #[must_use]
    pub const fn trace_step(&self) -> Option<usize> {
        self.trace_step
    }

    #[must_use]
    pub const fn trace_world(&self) -> TraceWorld {
        self.trace_world
    }

    /// Only the Trace tab uses it; recordings and pick mode keep the app's.
    #[must_use]
    pub const fn noise(&self) -> &NoiseFilter {
        &self.noise
    }

    #[must_use]
    pub fn noise_preset(&self) -> Option<NoisePreset> {
        if self.noise == self.base_noise {
            Some(NoisePreset::App)
        } else if self.noise == NoiseFilter::nothing() {
            Some(NoisePreset::Everything)
        } else {
            None
        }
    }

    #[must_use]
    pub fn noise_edits(&self) -> (usize, usize) {
        let added = |now: &[String], base: &[String]| {
            now.iter().filter(|pattern| !base.contains(pattern)).count()
        };
        (
            added(self.noise.ignored(), self.base_noise.ignored()),
            added(self.noise.kept(), self.base_noise.kept()),
        )
    }

    #[must_use]
    pub const fn shows_hidden_changes(&self) -> bool {
        matches!(self.hidden_list, HiddenList::Open)
    }

    pub(crate) fn toggle_noise_type(&mut self, index: usize) {
        let Some(type_path) = self.noise_types.get(index).cloned() else {
            return;
        };
        let name = short_type_name(&type_path);
        if self.noise.ignores_path(&type_path) {
            self.noise.reveal(&type_path);
            self.status = format!("Showing the changes of {name}");
        } else {
            let before = self.noise.clone();
            if self.noise.hide(&type_path) {
                self.status = format!("Hiding the changes of {name}");
            } else {
                // The pattern would do nothing, so it is not kept either.
                self.noise = before;
                "Kept by a keep pattern of the app".clone_into(&mut self.status);
            }
        }
        self.revision = self.revision.wrapping_add(1);
    }

    pub(crate) fn use_noise(&mut self, preset: NoisePreset) {
        self.noise = match preset {
            NoisePreset::App => self.base_noise.clone(),
            NoisePreset::Everything => NoiseFilter::nothing(),
        };
        self.revision = self.revision.wrapping_add(1);
    }

    #[must_use]
    pub const fn input_is_scripted(&self) -> bool {
        self.selected.is_some() && !self.taken_over
    }
}

/// Shortens `Time<bevy_time::virt::Virtual>` to `Time<Virtual>`.
fn short_type_name(type_path: &str) -> String {
    const DELIMITERS: [char; 6] = ['<', '>', ',', ' ', '(', ')'];
    let mut name = String::with_capacity(type_path.len());
    let mut rest = type_path;
    while !rest.is_empty() {
        let end = rest.find(DELIMITERS).unwrap_or(rest.len());
        let (path, after) = rest.split_at(end);
        name.push_str(path.rsplit("::").next().unwrap_or(path));
        let delimiter_len = after.chars().next().map_or(0, char::len_utf8);
        let (delimiter, after) = after.split_at(delimiter_len);
        name.push_str(delimiter);
        rest = after;
    }
    name
}

#[cfg(test)]
mod tests {
    use super::short_type_name;

    #[test]
    fn a_plain_type_path_is_shortened_to_its_last_segment() {
        assert_eq!(short_type_name("my_game::score::Score"), "Score");
    }

    #[test]
    fn a_generic_type_path_keeps_its_arguments_shortened() {
        assert_eq!(
            short_type_name("bevy_time::time::Time<bevy_time::virt::Virtual>"),
            "Time<Virtual>"
        );
        assert_eq!(
            short_type_name("a::Pair<b::Left, (c::Right, d::Other)>"),
            "Pair<Left, (Right, Other)>"
        );
    }
}
