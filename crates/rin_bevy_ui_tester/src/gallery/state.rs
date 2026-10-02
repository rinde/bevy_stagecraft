//! What the gallery is doing, and what can be asked of it.

use alloc::collections::BTreeMap;
use std::path::PathBuf;

use bevy::ecs::resource::Resource;

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

/// Something to do to the gallery. Buttons, keys and code all ask the same way,
/// through [`GalleryState::request`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GalleryAction {
    /// Reset the app and play the entry with this index.
    Select(usize),
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
    /// Stop recording and write the scenario into the export directory.
    StopRecording,
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
            pending: Vec::new(),
            results: BTreeMap::new(),
            export_dir,
            status: String::new(),
            recording: Recording::Off,
        }
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

    /// Whether the script, not the real mouse and keyboard, has the primary
    /// window: something is selected and it has not been taken over.
    #[must_use]
    pub const fn input_is_scripted(&self) -> bool {
        self.selected.is_some() && !self.taken_over
    }
}
