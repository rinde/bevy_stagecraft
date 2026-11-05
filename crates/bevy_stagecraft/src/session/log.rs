//! Turning a log of real input into readable steps.
//!
//! The log is a list of [`RecordedEvent`]s stamped with a frame number at the
//! fixed 1/60 s, which is why playback does not depend on the frame rate the
//! session ran at. [`steps_from_events`] is a pure function of that log, so it
//! is tested without a window.

use bevy::{
    input::{ButtonState, keyboard::KeyCode, mouse::MouseButton},
    math::Vec2,
};

use crate::{
    check::Check,
    scenario::{Modifiers, Step},
    targets::Target,
};

/// What one piece of real input was.
#[derive(Debug, Clone, PartialEq)]
pub enum RecordedKind {
    /// The cursor ended an update here.
    Moved(Vec2),
    /// A mouse button changed state.
    Button {
        /// The button.
        button: MouseButton,
        /// Pressed or released.
        state: ButtonState,
        /// Where the cursor was, in logical pixels.
        position: Vec2,
        /// What `position` was worked out to while the session ran: a UI
        /// label, whatever the app's hook made of it, or the position itself.
        target: Target,
    },
    /// The wheel turned, in pixels.
    Wheel {
        /// Horizontal amount.
        x: f32,
        /// Vertical amount.
        y: f32,
        /// Where the cursor was.
        position: Vec2,
    },
    /// A key changed state.
    Key {
        /// The key.
        key: KeyCode,
        /// Pressed or released.
        state: ButtonState,
        /// The text the press produced, as the operating system reports it
        /// (`None` for a release, and for a key that types nothing). It is
        /// what lets keys typed one after the other read as one `Type` step
        /// whatever the keyboard layout.
        text: Option<String>,
    },
    /// The person took a checkpoint: the checks it suggests go here, before
    /// anything else that happened in the same frame.
    Checkpoint,
    /// The person chose a check here, such as one picked in the gallery: it
    /// becomes a `Check` step at this point.
    Check(Check),
}

/// One piece of real input and the frame it happened in.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedEvent {
    /// Frames of 1/60 s since the session began.
    pub frame: u32,
    /// What happened.
    pub kind: RecordedKind,
}

/// The thresholds that decide how a log reads as steps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepOptions {
    /// How far, in logical pixels, a path may stray from a straight line
    /// before it needs a vertex of its own (Ramer-Douglas-Peucker).
    pub path_tolerance: f32,
    /// How far the pointer may move between a press and a release for the
    /// two to count as a click rather than a drag, in logical pixels.
    pub drag_distance: f32,
    /// How many frames a press may last and still be a `Click` or `Tap`.
    pub hold_frames: u32,
    /// Idle time longer than this many frames is shortened.
    pub idle_frames: u32,
    /// What idle time is shortened to, in frames.
    pub idle_wait: u16,
    /// Idle time longer than this many frames is shortened when the pointer
    /// then goes to a widget (a target such as `Text(..)`), since playback
    /// waits for that widget to be actionable by itself.
    pub widget_idle_frames: u32,
    /// What idle time before the pointer goes to a widget is shortened to,
    /// in frames.
    pub widget_idle_wait: u16,
    /// At least this many frames are left at the end, so the last input has
    /// taken effect before the checks look.
    pub settle_frames: u32,
}

impl Default for StepOptions {
    fn default() -> Self {
        Self {
            path_tolerance: 3.0,
            drag_distance: 4.0,
            hold_frames: 20,
            idle_frames: 90,
            idle_wait: 30,
            widget_idle_frames: 60,
            widget_idle_wait: 10,
            settle_frames: 10,
        }
    }
}

/// Positions closer than this are the same place.
const SAME_PLACE: f32 = 0.5;

/// A log read as steps, with where each checkpoint falls among them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LoggedSteps {
    /// The steps, without any checks.
    pub steps: Vec<Step>,
    /// For each [`RecordedKind::Checkpoint`] in the log, in order, how many
    /// of `steps` come before it: its checks go between `steps[at - 1]` and
    /// `steps[at]`.
    pub checkpoints: Vec<usize>,
}

/// Turns a log into steps. `end` is the frame the session stopped in.
///
/// Checkpoints in the log are left out; [`read_log`] says where they fall.
/// A [`RecordedKind::Check`] becomes a `Check` step where it was chosen.
///
/// * runs of moves become `Glide`s, one per vertex of the simplified path;
/// * a press and release in place is a `Click`; a straight press, move and
///   release of the left button is a `Drag`; anything else is spelt out as
///   `Press`, glides and `Release`;
/// * a key pressed and released on its own is a `Tap`, and a modifier held
///   around one folds into it;
/// * idle time beyond [`StepOptions::idle_frames`] becomes a short `Wait`;
///   before the pointer goes to a widget, idle time beyond the lower
///   [`StepOptions::widget_idle_frames`] does, as playback waits for the
///   widget to be actionable anyway.
#[must_use]
pub fn steps_from_events(events: &[RecordedEvent], end: u32, options: &StepOptions) -> Vec<Step> {
    read_log(events, end, options).steps
}

/// Turns a log into steps as [`steps_from_events`] does, and says where
/// among them each checkpoint falls.
///
/// A checkpoint ends the run of moves before it, so the pointer is where the
/// person had it when the checkpoint was taken, and the script's clock is
/// brought up to the checkpoint's frame. A checkpoint taken while a button or
/// key is held keeps the press and the release apart, so its checks see what
/// the person saw.
#[must_use]
pub fn read_log(events: &[RecordedEvent], end: u32, options: &StepOptions) -> LoggedSteps {
    let mut out = Emitter::new(*options);
    let mut next = 0;
    while let Some(event) = events.get(next) {
        next += 1;
        match &event.kind {
            RecordedKind::Moved(position) => out.moved(event.frame, *position),
            RecordedKind::Button {
                button,
                state: ButtonState::Pressed,
                ..
            } => next = out.press(events, next - 1, *button),
            RecordedKind::Button {
                button,
                state: ButtonState::Released,
                position,
                target,
            } => out.release(event.frame, *button, *position, target),
            RecordedKind::Wheel { x, y, position } => {
                // One turn of the wheel often arrives as several messages.
                let (mut x, mut y) = (*x, *y);
                while let Some(RecordedEvent {
                    frame,
                    kind:
                        RecordedKind::Wheel {
                            x: more_x,
                            y: more_y,
                            ..
                        },
                }) = events.get(next)
                {
                    if *frame != event.frame {
                        break;
                    }
                    x += more_x;
                    y += more_y;
                    next += 1;
                }
                out.wheel(event.frame, *position, x, y);
            }
            RecordedKind::Key { key, state, .. } => {
                next = out.key(events, next - 1, *key, *state);
            }
            RecordedKind::Checkpoint => out.checkpoint(event.frame),
            RecordedKind::Check(check) => out.check(event.frame, check),
        }
    }
    out.flush(None);
    let last = events.last().map_or(0, |event| event.frame);
    out.wait_until(end.max(last.saturating_add(options.settle_frames)));
    let (steps, checkpoints) = fold_chords(out.steps, &out.checkpoints);
    LoggedSteps { steps, checkpoints }
}

/// Builds the steps while keeping track of the script's own clock.
struct Emitter {
    /// The thresholds.
    options: StepOptions,
    /// The steps so far.
    steps: Vec<Step>,
    /// Where the script's clock stands, in frames.
    clock: u32,
    /// How many frames of idle time were cut, so recorded frames map to the
    /// script's.
    cut: u32,
    /// Where the script's pointer is, once something has put it somewhere.
    pointer: Option<Vec2>,
    /// Moves not yet turned into glides.
    run: Vec<(u32, Vec2)>,
    /// How many steps came before each checkpoint so far.
    checkpoints: Vec<usize>,
    /// The modifier keys held down, as far as the log has said.
    held: Modifiers,
}

impl Emitter {
    /// An emitter at the start of a script.
    const fn new(options: StepOptions) -> Self {
        Self {
            options,
            steps: Vec::new(),
            clock: 0,
            cut: 0,
            pointer: None,
            run: Vec::new(),
            checkpoints: Vec::new(),
            held: Modifiers::NONE,
        }
    }

    /// A checkpoint was taken in recorded frame `frame`.
    fn checkpoint(&mut self, frame: u32) {
        self.flush(None);
        self.wait_until(frame);
        self.checkpoints.push(self.steps.len());
    }

    /// The person chose `check` in recorded frame `frame`. Like a checkpoint
    /// it ends the run of moves before it and brings the clock up to its
    /// frame, so it sees what the person saw.
    fn check(&mut self, frame: u32, check: &Check) {
        self.flush(None);
        self.wait_until(frame);
        self.emit(Step::Check(check.clone()), 0);
    }

    /// Adds a step that takes `cost` frames.
    fn emit(&mut self, step: Step, cost: u32) {
        self.steps.push(step);
        self.clock = self.clock.saturating_add(cost);
    }

    /// Waits until the script's clock reaches recorded frame `recorded`,
    /// shortening a long idle stretch.
    fn wait_until(&mut self, recorded: u32) {
        self.wait_before(recorded, false);
    }

    /// [`Emitter::wait_until`] before a step that sends the pointer to a
    /// widget when `toward_widget` says so: playback waits for the widget to
    /// be actionable, so a shorter stretch of idle time is shortened, and
    /// further.
    fn wait_before(&mut self, recorded: u32, toward_widget: bool) {
        let wanted = recorded.saturating_sub(self.cut);
        let gap = wanted.saturating_sub(self.clock);
        if gap == 0 {
            return;
        }
        let (idle_frames, idle_wait) = if toward_widget {
            (
                self.options
                    .widget_idle_frames
                    .min(self.options.idle_frames),
                self.options.widget_idle_wait.min(self.options.idle_wait),
            )
        } else {
            (self.options.idle_frames, self.options.idle_wait)
        };
        let gap = if gap > idle_frames {
            let kept = u32::from(idle_wait);
            self.cut = self.cut.saturating_add(gap.saturating_sub(kept));
            kept
        } else {
            gap
        };
        self.emit(Step::Wait(frames_u16(gap)), gap);
    }

    /// Notes that the cursor was at `position` in `frame`.
    fn moved(&mut self, frame: u32, position: Vec2) {
        let paused = self
            .run
            .last()
            .is_some_and(|(last, _)| frame.saturating_sub(*last) > self.options.idle_frames);
        if paused {
            self.flush(None);
        }
        self.run.push((frame, position));
    }

    /// Turns the moves so far into glides along the simplified path. The last
    /// one ends at `end` when that is given, so a path that ends on a button
    /// ends on the button's label rather than a pixel.
    fn flush(&mut self, end: Option<Target>) {
        let run = core::mem::take(&mut self.run);
        let Some(&(first_frame, first_position)) = run.first() else {
            return;
        };
        // The path starts where the script's pointer is, a frame before the
        // first sample; with no pointer yet the first sample is a jump.
        let mut path: Vec<(u32, Vec2)> = Vec::with_capacity(run.len() + 1);
        // A run that ends on a widget is the pointer going to it.
        let toward_widget = end.as_ref().is_some_and(names_widget);
        let rest = if let Some(pointer) = self.pointer {
            path.push((first_frame.saturating_sub(1), pointer));
            run.as_slice()
        } else {
            self.wait_before(first_frame.saturating_sub(1), toward_widget);
            // A jump to a place the run never leaves is the whole move.
            let stays = run
                .last()
                .is_some_and(|(_, last)| last.distance(first_position) < SAME_PLACE);
            let target = match &end {
                Some(end) if stays => end.clone(),
                _ => Target::screen(first_position),
            };
            self.emit(Step::MoveTo(target), 1);
            self.pointer = Some(first_position);
            path.push((first_frame, first_position));
            run.get(1..).unwrap_or_default()
        };
        path.extend_from_slice(rest);
        let positions: Vec<Vec2> = path.iter().map(|(_, position)| *position).collect();
        let kept = simplify(&positions, self.options.path_tolerance);
        let last = kept.last().copied();
        let mut previous = path.first().map_or(first_frame, |(frame, _)| *frame);
        for index in kept.into_iter().skip(1) {
            let Some(&(frame, position)) = path.get(index) else {
                continue;
            };
            let target = match (&end, Some(index) == last) {
                (Some(end), true) => end.clone(),
                _ => Target::screen(position),
            };
            let there = self
                .pointer
                .is_some_and(|pointer| pointer.distance(position) < SAME_PLACE);
            if !there {
                self.wait_before(previous, toward_widget);
                let frames = frame.saturating_sub(previous).max(1);
                self.emit(
                    Step::Glide {
                        target,
                        frames: frames_u16(frames),
                    },
                    frames,
                );
                self.pointer = Some(position);
            }
            previous = frame;
        }
    }

    /// Makes sure the script's pointer is where the real one was.
    fn ensure_pointer(&mut self, frame: u32, position: Vec2, target: &Target) {
        let there = self
            .pointer
            .is_some_and(|pointer| pointer.distance(position) < SAME_PLACE);
        if !there {
            self.wait_before(frame.saturating_sub(1), names_widget(target));
            self.emit(Step::MoveTo(target.clone()), 1);
            self.pointer = Some(position);
        }
    }

    /// A button went down at `at`. Returns the index to carry on from.
    fn press(&mut self, events: &[RecordedEvent], at: usize, button: MouseButton) -> usize {
        let Some(RecordedEvent {
            frame,
            kind: RecordedKind::Button {
                position, target, ..
            },
        }) = events.get(at)
        else {
            return at + 1;
        };
        let (frame, position) = (*frame, *position);
        let lands_here = self
            .run
            .last()
            .is_some_and(|(_, last)| last.distance(position) < SAME_PLACE);
        self.flush(lands_here.then(|| target.clone()));
        self.ensure_pointer(frame, position, target);

        let release = events.iter().enumerate().skip(at + 1).find(|(_, event)| {
            matches!(
                &event.kind,
                RecordedKind::Button { button: other, state: ButtonState::Released, .. }
                    if *other == button
            )
        });
        let Some((release_at, release)) = release else {
            self.wait_until(frame);
            self.emit(Step::Press(button), 1);
            return at + 1;
        };
        let RecordedKind::Button {
            position: release_position,
            target: release_target,
            ..
        } = &release.kind
        else {
            return at + 1;
        };
        let between = events.get(at + 1..release_at).unwrap_or_default();
        let moves: Vec<(u32, Vec2)> = between
            .iter()
            .filter_map(|event| match event.kind {
                RecordedKind::Moved(position) => Some((event.frame, position)),
                _ => None,
            })
            .collect();
        if moves.len() != between.len() {
            // Something else happened while the button was down: keep the
            // press and let the rest play out in order.
            self.wait_until(frame);
            self.emit(Step::Press(button), 1);
            return at + 1;
        }
        let held = release.frame.saturating_sub(frame);
        let mut path = vec![position];
        path.extend(moves.iter().map(|(_, position)| *position));
        path.push(*release_position);
        let travelled = path
            .iter()
            .map(|point| point.distance(position))
            .fold(0.0, f32::max);
        if travelled <= self.options.drag_distance {
            self.wait_until(frame);
            if held <= self.options.hold_frames {
                self.emit(Step::Click(button), 1);
            } else {
                self.emit(Step::Press(button), 1);
                self.wait_until(release.frame);
                self.emit(Step::Release(button), 1);
            }
        } else if button == MouseButton::Left
            && simplify(&path, self.options.path_tolerance).len() <= 2
        {
            self.wait_until(frame);
            let frames = held.saturating_sub(2).max(1);
            self.emit(
                Step::Drag {
                    from: target.clone(),
                    to: release_target.clone(),
                    frames: frames_u16(frames),
                },
                frames.saturating_add(2),
            );
            self.pointer = Some(*release_position);
        } else {
            self.wait_until(frame);
            self.emit(Step::Press(button), 1);
            self.run.extend(moves);
            if self
                .run
                .last()
                .is_none_or(|(_, last)| last.distance(*release_position) >= SAME_PLACE)
            {
                self.run.push((release.frame, *release_position));
            }
            self.flush(Some(release_target.clone()));
            self.wait_until(release.frame);
            self.emit(Step::Release(button), 1);
        }
        release_at + 1
    }

    /// A button went up with no press the log could pair it with.
    fn release(&mut self, frame: u32, button: MouseButton, position: Vec2, target: &Target) {
        let lands_here = self
            .run
            .last()
            .is_some_and(|(_, last)| last.distance(position) < SAME_PLACE);
        self.flush(lands_here.then(|| target.clone()));
        self.wait_until(frame);
        self.emit(Step::Release(button), 1);
    }

    /// The wheel turned by `x` and `y` pixels at `position`.
    fn wheel(&mut self, frame: u32, position: Vec2, x: f32, y: f32) {
        self.flush(None);
        self.ensure_pointer(frame, position, &Target::screen(position));
        self.wait_until(frame);
        self.emit(Step::Wheel { x, y }, 1);
    }

    /// A key changed state. Returns the index to carry on from.
    fn key(
        &mut self,
        events: &[RecordedEvent],
        at: usize,
        key: KeyCode,
        state: ButtonState,
    ) -> usize {
        let Some(event) = events.get(at) else {
            return at + 1;
        };
        self.flush(None);
        if let Some(typing) = self.typing_at(events, at) {
            self.wait_until(event.frame);
            self.emit(Step::type_text(typing.text), typing.frames);
            // Playback types a character a frame, however slowly the person
            // did; the rest of the log is measured from where they stopped.
            let wanted = typing.end_frame.saturating_sub(self.cut);
            self.cut = self.cut.saturating_add(wanted.saturating_sub(self.clock));
            return typing.next;
        }
        self.wait_until(event.frame);
        if let Some(modifier) = modifier_of(key) {
            self.held = match state {
                ButtonState::Pressed => self.held | modifier,
                ButtonState::Released => self.held.without(modifier),
            };
        }
        if state == ButtonState::Released {
            self.emit(
                Step::KeyUp {
                    key,
                    modifiers: Modifiers::NONE,
                },
                1,
            );
            return at + 1;
        }
        let release = events.iter().enumerate().skip(at + 1).find(|(_, other)| {
            matches!(
                &other.kind,
                RecordedKind::Key { key: other_key, state: ButtonState::Released, .. }
                    if *other_key == key
            )
        });
        if let Some((release_at, release)) = release
            && release_at == at + 1
            && release.frame.saturating_sub(event.frame) <= self.options.hold_frames
        {
            self.emit(Step::tap(key), 1);
            return release_at + 1;
        }
        self.emit(
            Step::Key {
                key,
                modifiers: Modifiers::NONE,
            },
            1,
        );
        at + 1
    }

    /// The longest stretch of typing that starts at the key event `at`: keys
    /// that produced text, each released within [`StepOptions::hold_frames`]
    /// of its press, with Shift pressed and released around them, and no
    /// pause longer than [`StepOptions::idle_frames`]. Ends where every key
    /// and Shift is up again, and is `None` unless it typed something.
    ///
    /// A control character (Enter, Tab, Backspace) is not typing, nor is text
    /// typed with Control, Alt or Super held: those stay `Tap`s, which say
    /// what the person pressed.
    fn typing_at(&self, events: &[RecordedEvent], at: usize) -> Option<Typing> {
        let held_back = self.held.contains(Modifiers::CTRL)
            || self.held.contains(Modifiers::ALT)
            || self.held.contains(Modifiers::SUPER);
        let mut last_frame = events.get(at)?.frame;
        let mut text = String::new();
        let mut typed = 0_u32;
        let mut pending: Vec<(KeyCode, u32)> = Vec::new();
        let mut shifts = 0_u32;
        let mut found = None;
        for (index, event) in events.iter().enumerate().skip(at) {
            if event.frame.saturating_sub(last_frame) > self.options.idle_frames {
                break;
            }
            last_frame = event.frame;
            let RecordedKind::Key {
                key,
                state,
                text: produced,
            } = &event.kind
            else {
                break;
            };
            let is_shift = modifier_of(*key) == Some(Modifiers::SHIFT);
            match state {
                ButtonState::Pressed if is_shift => shifts = shifts.saturating_add(1),
                ButtonState::Pressed => {
                    let Some(produced) = produced.as_deref().filter(|produced| is_typed(produced))
                    else {
                        break;
                    };
                    if held_back || pending.iter().any(|(pressed, _)| pressed == key) {
                        break;
                    }
                    pending.push((*key, event.frame));
                    text.push_str(produced);
                    typed = typed.saturating_add(
                        u32::try_from(produced.chars().count()).unwrap_or(u32::MAX),
                    );
                }
                ButtonState::Released if is_shift => {
                    let Some(left) = shifts.checked_sub(1) else {
                        break;
                    };
                    shifts = left;
                }
                ButtonState::Released => {
                    let Some(position) = pending.iter().position(|(pressed, _)| pressed == key)
                    else {
                        break;
                    };
                    let (_, pressed_at) = pending.remove(position);
                    if event.frame.saturating_sub(pressed_at) > self.options.hold_frames {
                        break;
                    }
                }
            }
            if pending.is_empty() && shifts == 0 && typed > 0 {
                found = Some(Typing {
                    text: text.clone(),
                    frames: typed,
                    next: index + 1,
                    end_frame: event.frame,
                });
            }
        }
        found
    }
}

/// A run of keys read as one `Type` step.
struct Typing {
    /// What was typed.
    text: String,
    /// How many frames playback takes to type it: one per character.
    frames: u32,
    /// The index of the first event after the run.
    next: usize,
    /// The recorded frame of the run's last event.
    end_frame: u32,
}

/// Whether the text a key press produced is something typed into a field:
/// not empty and no control character.
fn is_typed(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|character| !character.is_control())
}

/// Whether `target` names a widget that playback waits for: a locator, not
/// a position nor one of the app's own targets, which may be either.
const fn names_widget(target: &Target) -> bool {
    matches!(
        target,
        Target::Text(_)
            | Target::Component(_)
            | Target::Name(_)
            | Target::Role { .. }
            | Target::Within(..)
            | Target::Nth(..)
    )
}

/// The modifier a key is one of.
const fn modifier_of(key: KeyCode) -> Option<Modifiers> {
    match key {
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Some(Modifiers::SHIFT),
        KeyCode::ControlLeft | KeyCode::ControlRight => Some(Modifiers::CTRL),
        KeyCode::AltLeft | KeyCode::AltRight => Some(Modifiers::ALT),
        KeyCode::SuperLeft | KeyCode::SuperRight => Some(Modifiers::SUPER),
        _ => None,
    }
}

/// How many frames of waiting may sit inside a chord and still let it fold.
const CHORD_WAIT_FRAMES: u32 = 6;

/// Folds `Key(Ctrl)`, `Tap(S)`, `KeyUp(Ctrl)` into `Tap(S, [Ctrl])`, ignoring
/// the odd frame of waiting between them. `checkpoints` count the steps
/// before each checkpoint and are counted again in the folded steps; one
/// that fell inside a chord moves to after it.
fn fold_chords(steps: Vec<Step>, checkpoints: &[usize]) -> (Vec<Step>, Vec<usize>) {
    let mut out: Vec<Step> = Vec::with_capacity(steps.len());
    // How many folded steps stand for the first `n` original ones, by `n`.
    let mut folded_before = vec![0; steps.len() + 1];
    let mut at = 0;
    while let Some(step) = steps.get(at) {
        let used = if let Some((tap, used)) = chord_at(&steps, at) {
            out.push(tap);
            used
        } else {
            out.push(step.clone());
            1
        };
        for boundary in folded_before.iter_mut().skip(at + 1).take(used) {
            *boundary = out.len();
        }
        at += used;
    }
    let checkpoints = checkpoints
        .iter()
        .map(|before| folded_before.get(*before).copied().unwrap_or(out.len()))
        .collect();
    (out, checkpoints)
}

/// The chord starting at `at`, as the tap it folds into and how many steps it
/// used.
fn chord_at(steps: &[Step], at: usize) -> Option<(Step, usize)> {
    let Some(Step::Key {
        key,
        modifiers: Modifiers::NONE,
    }) = steps.get(at)
    else {
        return None;
    };
    let modifier = modifier_of(*key)?;
    let mut waited = 0;
    let mut tapped = None;
    for (offset, step) in steps.iter().enumerate().skip(at + 1) {
        match (step, tapped) {
            (Step::Wait(frames), _) => waited += u32::from(*frames),
            (
                Step::Tap {
                    key: pressed,
                    modifiers: Modifiers::NONE,
                },
                None,
            ) if modifier_of(*pressed).is_none() => tapped = Some(*pressed),
            (
                Step::KeyUp {
                    key: released,
                    modifiers: Modifiers::NONE,
                },
                Some(pressed),
            ) if released == key && waited <= CHORD_WAIT_FRAMES => {
                return Some((
                    Step::Tap {
                        key: pressed,
                        modifiers: modifier,
                    },
                    offset + 1 - at,
                ));
            }
            _ => return None,
        }
    }
    None
}

/// A frame count as the `u16` steps take; longer is not a real pause.
fn frames_u16(frames: u32) -> u16 {
    u16::try_from(frames).unwrap_or(u16::MAX)
}

/// The indices of `points` that stay when the path is simplified with
/// Ramer-Douglas-Peucker: the first and last always, and any point that
/// strays more than `tolerance` from the line between its neighbours.
fn simplify(points: &[Vec2], tolerance: f32) -> Vec<usize> {
    let indexed: Vec<(usize, Vec2)> = points.iter().copied().enumerate().collect();
    let mut kept = Vec::new();
    if let Some((first, _)) = indexed.first() {
        kept.push(*first);
    }
    simplify_from(&indexed, tolerance, &mut kept);
    kept
}

/// Adds the indices to keep after the first of `points`, up to and including
/// the last.
fn simplify_from(points: &[(usize, Vec2)], tolerance: f32, kept: &mut Vec<usize>) {
    let (Some(&(_, start)), Some(&(last_index, end))) = (points.first(), points.last()) else {
        return;
    };
    if points.len() < 2 {
        return;
    }
    let farthest = points
        .iter()
        .enumerate()
        .skip(1)
        .take(points.len().saturating_sub(2))
        .map(|(at, (_, point))| (at, distance_to_segment(*point, start, end)))
        .max_by(|(_, a), (_, b)| a.total_cmp(b));
    match farthest {
        Some((at, distance)) if distance > tolerance => {
            simplify_from(points.get(..=at).unwrap_or_default(), tolerance, kept);
            simplify_from(points.get(at..).unwrap_or_default(), tolerance, kept);
        }
        _ => kept.push(last_index),
    }
}

/// How far `point` is from the segment `start`-`end`.
fn distance_to_segment(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let along = end - start;
    let length_squared = along.length_squared();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let t = ((point - start).dot(along) / length_squared).clamp(0.0, 1.0);
    point.distance(start + along * t)
}
