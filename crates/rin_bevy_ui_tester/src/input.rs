//! Synthetic input: drive a Bevy UI like a mouse and keyboard.

use alloc::collections::VecDeque;

use bevy::{
    app::{App, Plugin, PreUpdate},
    camera::NormalizedRenderTarget,
    ecs::{
        message::MessageWriter,
        query::With,
        resource::Resource,
        schedule::{IntoScheduleConfigs, SystemSet},
        system::{Local, Query, ResMut},
    },
    input::{
        ButtonState, InputSystems,
        keyboard::{Key, KeyCode, KeyboardInput, NativeKey, NativeKeyCode},
        mouse::{MouseButton, MouseButtonInput, MouseScrollUnit, MouseWheel},
        touch::TouchPhase,
    },
    log::warn,
    math::Vec2,
    picking::{
        PickingSystems,
        pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput},
    },
    window::{CursorMoved, PrimaryWindow, Window, WindowRef},
};

/// System sets of the synthetic input plugin, all in `PreUpdate`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntheticInputSystems {
    /// Drains [`SyntheticInput`] into Bevy's input messages. Runs before
    /// `InputSystems` and the picking input systems.
    Drain,
}

/// One thing a user could do with a mouse or keyboard.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SyntheticEvent {
    /// Move the cursor to a logical position in the window.
    MoveCursor(Vec2),
    /// Press or release a mouse button at the current cursor position.
    Button(MouseButton, ButtonState),
    /// Turn the mouse wheel at the current cursor position.
    Wheel {
        /// Whether `x` and `y` are lines or pixels.
        unit: MouseScrollUnit,
        /// Horizontal amount.
        x: f32,
        /// Vertical amount.
        y: f32,
    },
    /// Press or release a keyboard key.
    Key(KeyCode, ButtonState),
    /// Press or release a key that types `character`: Bevy sees the logical
    /// key (`Key::Character`, or `Key::Space`) and, on a press, the text it
    /// produces, as it would from a real keyboard. Shift is a separate
    /// [`SyntheticEvent::Key`] around it; see [`keystroke`].
    Character {
        /// The physical key.
        key: KeyCode,
        /// The character it types.
        character: char,
        /// Pressed or released.
        state: ButtonState,
    },
}

/// A queue of synthetic input, drained once per frame in `PreUpdate`.
///
/// It writes what `bevy_winit` would: `Window::set_cursor_position`,
/// `CursorMoved`, `MouseButtonInput`, `MouseWheel` and `KeyboardInput`
/// messages, plus `PointerInput` for `PointerId::Mouse`, written directly.
#[derive(Resource, Debug, Default)]
pub struct SyntheticInput {
    /// Events waiting for the next drain, oldest first.
    queue: VecDeque<SyntheticEvent>,
    /// The render target pointer locations refer to; the primary window when
    /// `None`. An offscreen image when recording.
    target: Option<NormalizedRenderTarget>,
    /// Where the last drained [`SyntheticEvent::MoveCursor`] put the cursor.
    cursor: Option<Vec2>,
}

impl SyntheticInput {
    /// Queues an arbitrary event.
    pub fn push(&mut self, event: SyntheticEvent) {
        self.queue.push_back(event);
    }

    /// Queues moving the cursor to `position` (logical pixels).
    pub fn move_cursor(&mut self, position: Vec2) {
        self.push(SyntheticEvent::MoveCursor(position));
    }

    /// Queues pressing `button`.
    pub fn press_button(&mut self, button: MouseButton) {
        self.push(SyntheticEvent::Button(button, ButtonState::Pressed));
    }

    /// Queues releasing `button`.
    pub fn release_button(&mut self, button: MouseButton) {
        self.push(SyntheticEvent::Button(button, ButtonState::Released));
    }

    /// Queues a press and release of `button` in the same frame, which is
    /// what Bevy needs to see a click.
    pub fn click(&mut self, button: MouseButton) {
        self.press_button(button);
        self.release_button(button);
    }

    /// Queues a turn of the mouse wheel.
    pub fn scroll(&mut self, unit: MouseScrollUnit, x: f32, y: f32) {
        self.push(SyntheticEvent::Wheel { unit, x, y });
    }

    /// Queues pressing `key`.
    pub fn press_key(&mut self, key: KeyCode) {
        self.push(SyntheticEvent::Key(key, ButtonState::Pressed));
    }

    /// Queues releasing `key`.
    pub fn release_key(&mut self, key: KeyCode) {
        self.push(SyntheticEvent::Key(key, ButtonState::Released));
    }

    /// Queues a press and release of `key` in the same frame. Bevy then sees
    /// it as just pressed and just released at once; to observe the key held,
    /// use [`Self::press_key`] and [`Self::release_key`] in different frames.
    pub fn tap_key(&mut self, key: KeyCode) {
        self.press_key(key);
        self.release_key(key);
    }

    /// Queues typing `character`: its key pressed and released in the same
    /// frame, with Shift held around it where the character needs it.
    pub fn type_character(&mut self, character: char) {
        for event in character_events(character) {
            self.push(event);
        }
    }

    /// Sets the render target pointer locations refer to. `None` means the
    /// primary window.
    pub fn set_target(&mut self, target: Option<NormalizedRenderTarget>) {
        self.target = target;
    }

    /// Where the script last moved the cursor, or `None` if it has not since
    /// the start or the last [`Self::forget_cursor`].
    #[must_use]
    pub const fn cursor(&self) -> Option<Vec2> {
        self.cursor
    }

    /// Forgets where the cursor was, as a fresh scenario starts without one.
    pub const fn forget_cursor(&mut self) {
        self.cursor = None;
    }

    /// Whether nothing is waiting to be drained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Registers [`SyntheticInput`] and the system that drains it.
#[derive(Debug, Default, Clone, Copy)]
pub struct SyntheticInputPlugin;

impl Plugin for SyntheticInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SyntheticInput>().add_systems(
            PreUpdate,
            drain_synthetic_input
                .in_set(SyntheticInputSystems::Drain)
                .before(InputSystems)
                .before(PickingSystems::ProcessInput),
        );
    }
}

/// The key that types a character, and whether Shift must be held for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keystroke {
    /// The physical key; [`KeyCode::Unidentified`] for a character no key of
    /// the US layout types.
    pub key: KeyCode,
    /// Whether Shift is held to type it.
    pub shift: bool,
}

/// The letter keys, in alphabetical order.
const LETTER_KEYS: [KeyCode; 26] = [
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyI,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyR,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
];

/// The digit keys, `0` to `9`.
const DIGIT_KEYS: [KeyCode; 10] = [
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

/// What the digit keys type with Shift held, `0` to `9`, on the US layout.
const SHIFTED_DIGITS: [char; 10] = [')', '!', '@', '#', '$', '%', '^', '&', '*', '('];

/// The key that types `character` on the US layout.
///
/// A scenario assumes one layout whatever the machine it runs on, as it has
/// to play the same everywhere. A character no key types (an accented letter,
/// an emoji) comes from [`KeyCode::Unidentified`] without Shift; the text is
/// what widgets read.
#[must_use]
pub fn keystroke(character: char) -> Keystroke {
    let plain = |key| Keystroke { key, shift: false };
    let shifted = |key| Keystroke { key, shift: true };
    let letter = |lower: char| {
        let index = ('a'..='z').position(|candidate| candidate == lower)?;
        LETTER_KEYS.get(index).copied()
    };
    let digit = |digit: char| {
        let index = usize::try_from(digit.to_digit(10)?).ok()?;
        DIGIT_KEYS.get(index).copied()
    };
    let symbol = match character {
        ' ' => Some(plain(KeyCode::Space)),
        '-' => Some(plain(KeyCode::Minus)),
        '_' => Some(shifted(KeyCode::Minus)),
        '=' => Some(plain(KeyCode::Equal)),
        '+' => Some(shifted(KeyCode::Equal)),
        '[' => Some(plain(KeyCode::BracketLeft)),
        '{' => Some(shifted(KeyCode::BracketLeft)),
        ']' => Some(plain(KeyCode::BracketRight)),
        '}' => Some(shifted(KeyCode::BracketRight)),
        '\\' => Some(plain(KeyCode::Backslash)),
        '|' => Some(shifted(KeyCode::Backslash)),
        ';' => Some(plain(KeyCode::Semicolon)),
        ':' => Some(shifted(KeyCode::Semicolon)),
        '\'' => Some(plain(KeyCode::Quote)),
        '"' => Some(shifted(KeyCode::Quote)),
        '`' => Some(plain(KeyCode::Backquote)),
        '~' => Some(shifted(KeyCode::Backquote)),
        ',' => Some(plain(KeyCode::Comma)),
        '<' => Some(shifted(KeyCode::Comma)),
        '.' => Some(plain(KeyCode::Period)),
        '>' => Some(shifted(KeyCode::Period)),
        '/' => Some(plain(KeyCode::Slash)),
        '?' => Some(shifted(KeyCode::Slash)),
        _ => None,
    };
    if let Some(symbol) = symbol {
        return symbol;
    }
    if character.is_ascii_lowercase()
        && let Some(key) = letter(character)
    {
        return plain(key);
    }
    if character.is_ascii_uppercase()
        && let Some(key) = letter(character.to_ascii_lowercase())
    {
        return shifted(key);
    }
    if let Some(key) = digit(character) {
        return plain(key);
    }
    if let Some(index) = SHIFTED_DIGITS
        .iter()
        .position(|shifted| *shifted == character)
        && let Some(key) = DIGIT_KEYS.get(index).copied()
    {
        return shifted(key);
    }
    plain(KeyCode::Unidentified(NativeKeyCode::Unidentified))
}

/// The events that type `character` in one frame: Shift down where needed,
/// the key down and up, Shift up.
pub(crate) fn character_events(character: char) -> Vec<SyntheticEvent> {
    let Keystroke { key, shift } = keystroke(character);
    let character_event = |state| SyntheticEvent::Character {
        key,
        character,
        state,
    };
    let mut events = Vec::with_capacity(4);
    if shift {
        events.push(SyntheticEvent::Key(
            KeyCode::ShiftLeft,
            ButtonState::Pressed,
        ));
    }
    events.push(character_event(ButtonState::Pressed));
    events.push(character_event(ButtonState::Released));
    if shift {
        events.push(SyntheticEvent::Key(
            KeyCode::ShiftLeft,
            ButtonState::Released,
        ));
    }
    events
}

/// The logical key of a modifier's physical key, which `bevy_winit` reports
/// and the text widgets read to tell Shift from a plain press; `None` for any
/// other key, whose logical key depends on a layout.
const fn modifier_logical_key(key: KeyCode) -> Option<Key> {
    match key {
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Some(Key::Shift),
        KeyCode::ControlLeft | KeyCode::ControlRight => Some(Key::Control),
        KeyCode::AltLeft | KeyCode::AltRight => Some(Key::Alt),
        KeyCode::SuperLeft | KeyCode::SuperRight => Some(Key::Super),
        _ => None,
    }
}

/// Maps a mouse button to its picking counterpart, if it has one.
const fn pointer_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Primary),
        MouseButton::Right => Some(PointerButton::Secondary),
        MouseButton::Middle => Some(PointerButton::Middle),
        MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => None,
    }
}

/// Moves queued synthetic events into Bevy's input messages.
fn drain_synthetic_input(
    mut input: ResMut<SyntheticInput>,
    mut windows: Query<(bevy::ecs::entity::Entity, &mut Window), With<PrimaryWindow>>,
    mut cursor_moved: MessageWriter<CursorMoved>,
    mut mouse_buttons: MessageWriter<MouseButtonInput>,
    mut wheel: MessageWriter<MouseWheel>,
    mut keys: MessageWriter<KeyboardInput>,
    mut pointer_inputs: MessageWriter<PointerInput>,
    mut last_position: Local<Vec2>,
) {
    if input.queue.is_empty() {
        return;
    }
    let Some((window_entity, mut window)) = windows.iter_mut().next() else {
        warn!("synthetic input dropped: there is no primary window");
        input.queue.clear();
        return;
    };
    let target = input.target.clone().or_else(|| {
        WindowRef::Entity(window_entity)
            .normalize(Some(window_entity))
            .map(NormalizedRenderTarget::Window)
    });
    let Some(target) = target else {
        warn!("synthetic input dropped: the primary window has no render target");
        input.queue.clear();
        return;
    };

    while let Some(event) = input.queue.pop_front() {
        let location = |position: Vec2| Location {
            target: target.clone(),
            position,
        };
        match event {
            SyntheticEvent::MoveCursor(position) => {
                let delta = position - *last_position;
                *last_position = position;
                input.cursor = Some(position);
                window.set_cursor_position(Some(position));
                cursor_moved.write(CursorMoved {
                    window: window_entity,
                    position,
                    delta: Some(delta),
                });
                pointer_inputs.write(PointerInput::new(
                    PointerId::Mouse,
                    location(position),
                    PointerAction::Move { delta },
                ));
            }
            SyntheticEvent::Button(button, state) => {
                mouse_buttons.write(MouseButtonInput {
                    button,
                    state,
                    window: window_entity,
                });
                if let Some(pointer_button) = pointer_button(button) {
                    let action = match state {
                        ButtonState::Pressed => PointerAction::Press(pointer_button),
                        ButtonState::Released => PointerAction::Release(pointer_button),
                    };
                    pointer_inputs.write(PointerInput::new(
                        PointerId::Mouse,
                        location(*last_position),
                        action,
                    ));
                }
            }
            SyntheticEvent::Wheel { unit, x, y } => {
                wheel.write(MouseWheel {
                    unit,
                    x,
                    y,
                    window: window_entity,
                    phase: TouchPhase::Moved,
                });
                pointer_inputs.write(PointerInput::new(
                    PointerId::Mouse,
                    location(*last_position),
                    PointerAction::Scroll {
                        unit,
                        x,
                        y,
                        phase: TouchPhase::Moved,
                    },
                ));
            }
            SyntheticEvent::Key(key_code, state) => {
                keys.write(KeyboardInput {
                    key_code,
                    logical_key: modifier_logical_key(key_code)
                        .unwrap_or(Key::Unidentified(NativeKey::Unidentified)),
                    state,
                    text: None,
                    repeat: false,
                    window: window_entity,
                });
            }
            SyntheticEvent::Character {
                key,
                character,
                state,
            } => {
                keys.write(character_input(key, character, state, window_entity));
            }
        }
    }
}

/// The keyboard message for a character key: the logical key and, on a
/// press only, the text, as a real keyboard reports them.
fn character_input(
    key_code: KeyCode,
    character: char,
    state: ButtonState,
    window: bevy::ecs::entity::Entity,
) -> KeyboardInput {
    let text = character.to_string();
    let logical_key = if character == ' ' {
        Key::Space
    } else {
        Key::Character(text.clone().into())
    };
    KeyboardInput {
        key_code,
        logical_key,
        state,
        text: (state == ButtonState::Pressed).then(|| text.into()),
        repeat: false,
        window,
    }
}
