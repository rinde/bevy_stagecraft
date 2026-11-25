//! Turning a log of real input into readable steps.

use bevy::{
    input::{ButtonState, keyboard::KeyCode, mouse::MouseButton, touch::TouchPhase},
    math::Vec2,
};

use crate::{
    check::Check,
    scenario::{Modifiers, Step},
    targets::Target,
};

#[derive(Debug, Clone, PartialEq)]
pub enum RecordedKind {
    Moved(Vec2),
    Button {
        button: MouseButton,
        state: ButtonState,
        position: Vec2,
        target: Target,
    },
    Wheel {
        x: f32,
        y: f32,
        position: Vec2,
    },
    Key {
        key: KeyCode,
        state: ButtonState,
        /// Lets keys typed one after another read as one `Type` step whatever the keyboard layout.
        text: Option<String>,
    },
    Touch {
        /// Numbered from 0 by the lowest free number, not the system's touch id.
        finger: u8,
        phase: TouchPhase,
        position: Vec2,
        target: Target,
    },
    /// The checks it suggests go here, before anything else in the same frame.
    Checkpoint,
    Check(Check),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordedEvent {
    pub frame: u32,
    pub kind: RecordedKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepOptions {
    /// Ramer-Douglas-Peucker tolerance, in logical pixels.
    pub path_tolerance: f32,
    pub drag_distance: f32,
    pub hold_frames: u32,
    pub idle_frames: u32,
    pub idle_wait: u16,
    /// Playback waits for a widget to be actionable by itself, so idle time before one is cut more.
    pub widget_idle_frames: u32,
    pub widget_idle_wait: u16,
    /// Left at the end so the last input has taken effect before the checks look.
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

const SAME_PLACE: f32 = 0.5;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LoggedSteps {
    pub steps: Vec<Step>,
    /// How many of `steps` come before each checkpoint: its checks go between `steps[at - 1]` and `steps[at]`.
    pub checkpoints: Vec<usize>,
}

/// `end` is the frame the session stopped in. Checkpoints are left out; [`read_log`] says where they fall.
#[must_use]
pub fn steps_from_events(events: &[RecordedEvent], end: u32, options: &StepOptions) -> Vec<Step> {
    read_log(events, end, options).steps
}

/// A checkpoint ends the run of moves and brings the script's clock up to its frame, so its
/// checks see what the person saw.
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
            RecordedKind::Touch { .. } => next = out.touch(events, next - 1),
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

struct Emitter {
    options: StepOptions,
    steps: Vec<Step>,
    clock: u32,
    /// Frames of idle time cut, so recorded frames map to the script's.
    cut: u32,
    pointer: Option<Vec2>,
    run: Vec<(u32, Vec2)>,
    checkpoints: Vec<usize>,
    /// Each physical key is kept, so releasing one Control while the other is down leaves Control held.
    held: Vec<KeyCode>,
}

impl Emitter {
    const fn new(options: StepOptions) -> Self {
        Self {
            options,
            steps: Vec::new(),
            clock: 0,
            cut: 0,
            pointer: None,
            run: Vec::new(),
            checkpoints: Vec::new(),
            held: Vec::new(),
        }
    }

    fn checkpoint(&mut self, frame: u32) {
        self.flush(None);
        self.wait_until(frame);
        self.checkpoints.push(self.steps.len());
    }

    fn check(&mut self, frame: u32, check: &Check) {
        self.flush(None);
        self.wait_until(frame);
        self.emit(Step::Check(check.clone()), 0);
    }

    fn emit(&mut self, step: Step, cost: u32) {
        self.steps.push(step);
        self.clock = self.clock.saturating_add(cost);
    }

    fn wait_until(&mut self, recorded: u32) {
        self.wait_before(recorded, false);
    }

    /// Cuts more when `toward_widget`, since playback waits for the widget to be actionable.
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
        self.emit_wait(gap);
    }

    /// A `Wait` holds at most `u16::MAX` frames, so a longer pause is several.
    fn emit_wait(&mut self, frames: u32) {
        let mut left = frames;
        while left > 0 {
            let chunk = frames_u16(left);
            self.emit(Step::Wait(chunk), u32::from(chunk));
            left = left.saturating_sub(u32::from(chunk));
        }
    }

    /// A `Drag` holds at most `u16::MAX` frames; a longer one is spelled out as a move, press, glides and release.
    fn emit_drag(
        &mut self,
        button: MouseButton,
        (from, start): (&Target, Vec2),
        (to, end): (&Target, Vec2),
        frames: u32,
    ) {
        if u16::try_from(frames).is_ok() {
            self.emit(
                Step::Drag {
                    from: from.clone(),
                    to: to.clone(),
                    frames: frames_u16(frames),
                },
                frames.saturating_add(2),
            );
        } else {
            self.emit(Step::MoveTo(from.clone()), 1);
            self.pointer = Some(start);
            self.emit(Step::Press(button), 1);
            self.emit_glide(to.clone(), end, frames);
            self.emit(Step::Release(button), 1);
        }
    }

    /// A `Glide` longer than `u16::MAX` frames is cut into legs on the straight line; without a
    /// known start all legs aim at `target`.
    fn emit_glide(&mut self, target: Target, to: Vec2, frames: u32) {
        let from = self.pointer;
        let mut done = 0_u32;
        while done < frames {
            let chunk = frames_u16(frames.saturating_sub(done));
            done = done.saturating_add(u32::from(chunk));
            let leg_target = if done >= frames {
                target.clone()
            } else if let Some(from) = from {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "frame counts far below 2^24 lose nothing that shows in a pixel"
                )]
                let fraction = done as f32 / frames as f32;
                Target::screen(from.lerp(to, fraction))
            } else {
                target.clone()
            };
            self.emit(
                Step::Glide {
                    target: leg_target,
                    frames: chunk,
                },
                u32::from(chunk),
            );
        }
    }

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

    /// The last glide ends at `end` when given, so a path ending on a button ends on its label.
    fn flush(&mut self, end: Option<Target>) {
        let run = core::mem::take(&mut self.run);
        let Some(&(first_frame, first_position)) = run.first() else {
            return;
        };
        // The path starts a frame before the first sample; with no pointer yet the first sample is a jump.
        let mut path: Vec<(u32, Vec2)> = Vec::with_capacity(run.len() + 1);
        let toward_widget = end.as_ref().is_some_and(names_widget);
        let rest = if let Some(pointer) = self.pointer {
            path.push((first_frame.saturating_sub(1), pointer));
            run.as_slice()
        } else {
            self.wait_before(first_frame.saturating_sub(1), toward_widget);
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
                self.emit_glide(target, position, frames);
                self.pointer = Some(position);
            }
            previous = frame;
        }
    }

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
            // Something else happened while the button was down: keep the press and play the rest in order.
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
            self.emit_drag(
                button,
                (target, position),
                (release_target, *release_position),
                frames,
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

    fn release(&mut self, frame: u32, button: MouseButton, position: Vec2, target: &Target) {
        let lands_here = self
            .run
            .last()
            .is_some_and(|(_, last)| last.distance(position) < SAME_PLACE);
        self.flush(lands_here.then(|| target.clone()));
        self.wait_until(frame);
        self.emit(Step::Release(button), 1);
    }

    fn wheel(&mut self, frame: u32, position: Vec2, x: f32, y: f32) {
        self.flush(None);
        self.ensure_pointer(frame, position, &Target::screen(position));
        self.wait_until(frame);
        self.emit(Step::Wheel { x, y }, 1);
    }

    /// Touch never moves the script's mouse pointer.
    fn touch(&mut self, events: &[RecordedEvent], at: usize) -> usize {
        let Some(RecordedEvent {
            frame,
            kind:
                RecordedKind::Touch {
                    finger,
                    phase,
                    position,
                    target,
                },
        }) = events.get(at)
        else {
            return at + 1;
        };
        let (frame, finger) = (*frame, *finger);
        self.flush(None);
        match phase {
            TouchPhase::Started => return self.finger_down(events, at, finger, *position, target),
            TouchPhase::Moved => {
                self.wait_until(frame);
                self.emit(
                    Step::touch_move(Target::screen(*position)).with_finger(finger),
                    1,
                );
            }
            TouchPhase::Ended => {
                self.wait_until(frame);
                self.emit(Step::touch_up().with_finger(finger), 1);
            }
            TouchPhase::Canceled => {
                self.wait_until(frame);
                self.emit(Step::touch_cancel().with_finger(finger), 1);
            }
        }
        at + 1
    }

    /// When nothing but its own moves happens before it ends, the whole gesture is read at once.
    fn finger_down(
        &mut self,
        events: &[RecordedEvent],
        at: usize,
        finger: u8,
        position: Vec2,
        target: &Target,
    ) -> usize {
        let Some(down) = events.get(at) else {
            return at + 1;
        };
        let frame = down.frame;
        let ends = events.iter().enumerate().skip(at + 1).find(|(_, event)| {
            matches!(
                &event.kind,
                RecordedKind::Touch {
                    finger: other,
                    phase: TouchPhase::Ended | TouchPhase::Canceled,
                    ..
                } if *other == finger
            )
        });
        let gesture = ends.and_then(|(end_at, end)| {
            let RecordedKind::Touch {
                phase,
                position: end_position,
                target: end_target,
                ..
            } = &end.kind
            else {
                return None;
            };
            let between = events.get(at + 1..end_at).unwrap_or_default();
            let moves: Option<Vec<(u32, Vec2)>> = between
                .iter()
                .map(|event| match &event.kind {
                    RecordedKind::Touch {
                        finger: other,
                        phase: TouchPhase::Moved,
                        position,
                        ..
                    } if *other == finger => Some((event.frame, *position)),
                    _ => None,
                })
                .collect();
            Some(Gesture {
                end_at,
                end_frame: end.frame,
                phase: *phase,
                end_position: *end_position,
                end_target,
                moves: moves?,
            })
        });
        let Some(gesture) = gesture else {
            self.wait_until(frame);
            self.emit(Step::touch_down(target.clone()).with_finger(finger), 1);
            return at + 1;
        };
        self.wait_until(frame);
        let held = gesture.end_frame.saturating_sub(frame);
        let mut path = vec![position];
        path.extend(gesture.moves.iter().map(|(_, moved)| *moved));
        path.push(gesture.end_position);
        let travelled = path
            .iter()
            .map(|point| point.distance(position))
            .fold(0.0, f32::max);
        let lifted = gesture.phase == TouchPhase::Ended;
        if finger == 0 && lifted && travelled <= self.options.drag_distance {
            if held <= self.options.hold_frames {
                self.emit(Step::touch_tap(target.clone()), 2);
            } else {
                self.emit(Step::touch_down(target.clone()), 1);
                self.wait_until(gesture.end_frame);
                self.emit(Step::touch_up(), 1);
            }
        } else if finger == 0
            && lifted
            && simplify(&path, self.options.path_tolerance).len() <= 2
            && held > TOUCH_DRAG_OVERHEAD
            && u16::try_from(held - TOUCH_DRAG_OVERHEAD).is_ok()
        {
            let frames = held - TOUCH_DRAG_OVERHEAD;
            self.emit(
                Step::touch_drag(
                    target.clone(),
                    gesture.end_target.clone(),
                    frames_u16(frames),
                ),
                frames + TOUCH_DRAG_OVERHEAD,
            );
        } else {
            self.emit(Step::touch_down(target.clone()).with_finger(finger), 1);
            self.touch_path(finger, &gesture, &path, frame);
        }
        gesture.end_at + 1
    }

    fn touch_path(&mut self, finger: u8, gesture: &Gesture<'_>, path: &[Vec2], start: u32) {
        let mut frames = vec![start];
        frames.extend(gesture.moves.iter().map(|(frame, _)| *frame));
        frames.push(gesture.end_frame);
        let kept = simplify(path, self.options.path_tolerance);
        let mut there = path.first().copied();
        for index in kept.into_iter().skip(1) {
            let (Some(&frame), Some(&position)) = (frames.get(index), path.get(index)) else {
                continue;
            };
            if there.is_some_and(|there| there.distance(position) < SAME_PLACE) {
                continue;
            }
            self.wait_until(frame);
            self.emit(
                Step::touch_move(Target::screen(position)).with_finger(finger),
                1,
            );
            there = Some(position);
        }
        self.wait_until(gesture.end_frame);
        let end = if gesture.phase == TouchPhase::Ended {
            Step::touch_up()
        } else {
            Step::touch_cancel()
        };
        self.emit(end.with_finger(finger), 1);
    }

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
            // Playback types a character a frame, however slowly the person did; the rest of the log is
            // measured from where they stopped.
            let wanted = typing.end_frame.saturating_sub(self.cut);
            self.cut = self.cut.saturating_add(wanted.saturating_sub(self.clock));
            return typing.next;
        }
        self.wait_until(event.frame);
        if modifier_of(key).is_some() {
            match state {
                ButtonState::Pressed => {
                    if !self.held.contains(&key) {
                        self.held.push(key);
                    }
                }
                ButtonState::Released => self.held.retain(|held| *held != key),
            }
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
            // The tap swallows the release, so the key is up again.
            self.held.retain(|held| *held != key);
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

    /// Control characters and text typed with Control, Alt or Super held are not typing; those stay `Tap`s.
    fn typing_at(&self, events: &[RecordedEvent], at: usize) -> Option<Typing> {
        let held_back = self.held.iter().any(|key| {
            [Modifiers::CTRL, Modifiers::ALT, Modifiers::SUPER]
                .contains(&modifier_of(*key).unwrap_or(Modifiers::NONE))
        });
        let mut last_frame = events.get(at)?.frame;
        let mut text = String::new();
        let mut typed = 0_u32;
        let mut pending: Vec<(KeyCode, u32)> = Vec::new();
        let mut shifts = 0_u32;
        // Kept as numbers so the text is copied once, not at every boundary (quadratic otherwise).
        let mut found: Option<(usize, u32, usize, u32)> = None;
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
                found = Some((text.len(), typed, index + 1, event.frame));
            }
        }
        let (length, frames, next, end_frame) = found?;
        text.truncate(length);
        Some(Typing {
            text,
            frames,
            next,
            end_frame,
        })
    }
}

/// A gesture held no longer than this would play longer as a `TouchDrag` than recorded, so it is spelled out.
const TOUCH_DRAG_OVERHEAD: u32 = 2;

struct Gesture<'a> {
    end_at: usize,
    end_frame: u32,
    phase: TouchPhase,
    end_position: Vec2,
    end_target: &'a Target,
    moves: Vec<(u32, Vec2)>,
}

struct Typing {
    text: String,
    frames: u32,
    next: usize,
    end_frame: u32,
}

fn is_typed(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|character| !character.is_control())
}

/// A locator, not a position nor one of the app's own targets.
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

const fn modifier_of(key: KeyCode) -> Option<Modifiers> {
    match key {
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Some(Modifiers::SHIFT),
        KeyCode::ControlLeft | KeyCode::ControlRight => Some(Modifiers::CTRL),
        KeyCode::AltLeft | KeyCode::AltRight => Some(Modifiers::ALT),
        KeyCode::SuperLeft | KeyCode::SuperRight => Some(Modifiers::SUPER),
        _ => None,
    }
}

const CHORD_WAIT_FRAMES: u32 = 6;

/// A checkpoint that fell inside a chord moves to after it.
fn fold_chords(steps: Vec<Step>, checkpoints: &[usize]) -> (Vec<Step>, Vec<usize>) {
    let mut out: Vec<Step> = Vec::with_capacity(steps.len());
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

/// Callers split longer durations into several steps rather than lose frames.
fn frames_u16(frames: u32) -> u16 {
    u16::try_from(frames).unwrap_or(u16::MAX)
}

fn simplify(points: &[Vec2], tolerance: f32) -> Vec<usize> {
    let indexed: Vec<(usize, Vec2)> = points.iter().copied().enumerate().collect();
    let mut kept = Vec::new();
    if let Some((first, _)) = indexed.first() {
        kept.push(*first);
    }
    simplify_from(&indexed, tolerance, &mut kept);
    kept
}

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

fn distance_to_segment(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let along = end - start;
    let length_squared = along.length_squared();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let t = ((point - start).dot(along) / length_squared).clamp(0.0, 1.0);
    point.distance(start + along * t)
}
