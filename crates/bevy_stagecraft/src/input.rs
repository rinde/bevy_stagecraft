use alloc::collections::{BTreeMap, VecDeque};

use bevy::{
    app::{App, Last, Plugin, PreUpdate},
    camera::NormalizedRenderTarget,
    ecs::{
        entity::Entity,
        message::{MessageReader, MessageWriter},
        query::With,
        resource::Resource,
        schedule::{IntoScheduleConfigs, SystemSet},
        system::{Commands, Local, Query, ResMut},
    },
    input::{
        ButtonState, InputSystems,
        keyboard::{Key, KeyCode, KeyboardInput, NativeKey, NativeKeyCode},
        mouse::{MouseButton, MouseButtonInput, MouseScrollUnit, MouseWheel},
        touch::{TouchInput, TouchPhase},
    },
    log::warn,
    math::Vec2,
    picking::{
        PickingSystems,
        pointer::{
            Location, PointerAction, PointerButton, PointerId, PointerInput, PointerLocation,
        },
    },
    window::{CursorMoved, PrimaryWindow, Window, WindowRef},
};

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntheticInputSystems {
    /// Runs before `InputSystems` and the picking input systems.
    Drain,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SyntheticEvent {
    MoveCursor(Vec2),
    Button(MouseButton, ButtonState),
    Wheel {
        unit: MouseScrollUnit,
        x: f32,
        y: f32,
    },
    Key(KeyCode, ButtonState),
    /// Shift is a separate [`SyntheticEvent::Key`] around it; see [`keystroke`].
    Character {
        key: KeyCode,
        character: char,
        state: ButtonState,
    },
    /// Never moves the mouse.
    Touch {
        finger: u64,
        phase: TouchPhase,
        position: Vec2,
    },
}

/// Writes what `bevy_winit` would, so the app sees what real devices produce.
#[derive(Resource, Debug, Default)]
pub struct SyntheticInput {
    queue: VecDeque<SyntheticEvent>,
    target: Option<NormalizedRenderTarget>,
    cursor: Option<Vec2>,
}

impl SyntheticInput {
    pub fn push(&mut self, event: SyntheticEvent) {
        self.queue.push_back(event);
    }

    pub fn move_cursor(&mut self, position: Vec2) {
        self.push(SyntheticEvent::MoveCursor(position));
    }

    pub fn press_button(&mut self, button: MouseButton) {
        self.push(SyntheticEvent::Button(button, ButtonState::Pressed));
    }

    pub fn release_button(&mut self, button: MouseButton) {
        self.push(SyntheticEvent::Button(button, ButtonState::Released));
    }

    /// Same frame, because Bevy needs that to see a click.
    pub fn click(&mut self, button: MouseButton) {
        self.press_button(button);
        self.release_button(button);
    }

    pub fn touch_down(&mut self, finger: u64, position: Vec2) {
        self.touch(finger, TouchPhase::Started, position);
    }

    pub fn touch_move(&mut self, finger: u64, position: Vec2) {
        self.touch(finger, TouchPhase::Moved, position);
    }

    pub fn touch_up(&mut self, finger: u64, position: Vec2) {
        self.touch(finger, TouchPhase::Ended, position);
    }

    /// Does not send `Pointer<Cancel>`: on Bevy 0.20 a device's canceled touch
    /// reaches widgets as `Pointer<Out>` only, and a scenario must fail where a
    /// device would.
    pub fn touch_cancel(&mut self, finger: u64, position: Vec2) {
        self.touch(finger, TouchPhase::Canceled, position);
    }

    fn touch(&mut self, finger: u64, phase: TouchPhase, position: Vec2) {
        self.push(SyntheticEvent::Touch {
            finger,
            phase,
            position,
        });
    }

    pub fn scroll(&mut self, unit: MouseScrollUnit, x: f32, y: f32) {
        self.push(SyntheticEvent::Wheel { unit, x, y });
    }

    pub fn press_key(&mut self, key: KeyCode) {
        self.push(SyntheticEvent::Key(key, ButtonState::Pressed));
    }

    pub fn release_key(&mut self, key: KeyCode) {
        self.push(SyntheticEvent::Key(key, ButtonState::Released));
    }

    /// Bevy sees it as just pressed and just released at once; to observe the key
    /// held, press and release in different frames.
    pub fn tap_key(&mut self, key: KeyCode) {
        self.press_key(key);
        self.release_key(key);
    }

    pub fn type_character(&mut self, character: char) {
        for event in character_events(character) {
            self.push(event);
        }
    }

    pub fn set_target(&mut self, target: Option<NormalizedRenderTarget>) {
        self.target = target;
    }

    #[must_use]
    pub const fn cursor(&self) -> Option<Vec2> {
        self.cursor
    }

    pub const fn forget_cursor(&mut self) {
        self.cursor = None;
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SyntheticInputPlugin;

impl Plugin for SyntheticInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SyntheticInput>()
            .add_systems(
                PreUpdate,
                drain_synthetic_input
                    .in_set(SyntheticInputSystems::Drain)
                    .before(InputSystems)
                    .before(PickingSystems::ProcessInput),
            )
            .add_systems(Last, despawn_ended_touch_pointers);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keystroke {
    /// [`KeyCode::Unidentified`] for a character no key of the US layout types.
    pub key: KeyCode,
    pub shift: bool,
}

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

const SHIFTED_DIGITS: [char; 10] = [')', '!', '@', '#', '$', '%', '^', '&', '*', '('];

/// Assumes the US layout on every machine so scenarios play the same everywhere.
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

pub(crate) fn character_events(character: char) -> Vec<SyntheticEvent> {
    let Keystroke { key, shift } = keystroke(character);
    let character_event = |state| SyntheticEvent::Character {
        key,
        character,
        state,
    };
    let mut events = Vec::new();
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

/// Text widgets read it to tell Shift from a plain press; `None` for other keys,
/// whose logical key depends on a layout.
const fn modifier_logical_key(key: KeyCode) -> Option<Key> {
    match key {
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Some(Key::Shift),
        KeyCode::ControlLeft | KeyCode::ControlRight => Some(Key::Control),
        KeyCode::AltLeft | KeyCode::AltRight => Some(Key::Alt),
        KeyCode::SuperLeft | KeyCode::SuperRight => Some(Key::Super),
        _ => None,
    }
}

const fn pointer_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Primary),
        MouseButton::Right => Some(PointerButton::Secondary),
        MouseButton::Middle => Some(PointerButton::Middle),
        MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => None,
    }
}

const fn button_action(button: PointerButton, state: ButtonState) -> PointerAction {
    match state {
        ButtonState::Pressed => PointerAction::Press(button),
        ButtonState::Released => PointerAction::Release(button),
    }
}

const fn touch_input(finger: u64, phase: TouchPhase, position: Vec2, window: Entity) -> TouchInput {
    TouchInput {
        phase,
        position,
        window,
        force: None,
        id: finger,
    }
}

fn modifier_input(key_code: KeyCode, state: ButtonState, window: Entity) -> KeyboardInput {
    KeyboardInput {
        key_code,
        logical_key: modifier_logical_key(key_code)
            .unwrap_or(Key::Unidentified(NativeKey::Unidentified)),
        state,
        text: None,
        repeat: false,
        window,
    }
}

/// Spawn a finger's pointer on `Started`: Bevy's touch picking reads window
/// events a headless app never has.
fn touch_pointer_input(
    commands: &mut Commands,
    positions: &mut BTreeMap<u64, Vec2>,
    finger: u64,
    phase: TouchPhase,
    at: Location,
) -> PointerInput {
    let position = at.position;
    let pointer = PointerId::Touch(finger);
    let action = match phase {
        TouchPhase::Started => {
            commands.spawn((pointer, PointerLocation::new(at.clone())));
            PointerAction::Press(PointerButton::Primary)
        }
        TouchPhase::Moved => PointerAction::Move {
            delta: position - positions.get(&finger).copied().unwrap_or(position),
        },
        TouchPhase::Ended => PointerAction::Release(PointerButton::Primary),
        TouchPhase::Canceled => PointerAction::Cancel,
    };
    match phase {
        TouchPhase::Started | TouchPhase::Moved => {
            positions.insert(finger, position);
        }
        TouchPhase::Ended | TouchPhase::Canceled => {
            positions.remove(&finger);
        }
    }
    PointerInput::new(pointer, at, action)
}

fn drain_synthetic_input(
    mut input: ResMut<SyntheticInput>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut cursor_moved: MessageWriter<CursorMoved>,
    mut mouse_buttons: MessageWriter<MouseButtonInput>,
    mut wheel: MessageWriter<MouseWheel>,
    mut keys: MessageWriter<KeyboardInput>,
    mut touches: MessageWriter<TouchInput>,
    mut pointer_inputs: MessageWriter<PointerInput>,
    mut commands: Commands,
    mut last_position: Local<Vec2>,
    mut touch_positions: Local<BTreeMap<u64, Vec2>>,
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
                    pointer_inputs.write(PointerInput::new(
                        PointerId::Mouse,
                        location(*last_position),
                        button_action(pointer_button, state),
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
                keys.write(modifier_input(key_code, state, window_entity));
            }
            SyntheticEvent::Character {
                key,
                character,
                state,
            } => {
                keys.write(character_input(key, character, state, window_entity));
            }
            SyntheticEvent::Touch {
                finger,
                phase,
                position,
            } => {
                touches.write(touch_input(finger, phase, position, window_entity));
                pointer_inputs.write(touch_pointer_input(
                    &mut commands,
                    &mut touch_positions,
                    finger,
                    phase,
                    location(position),
                ));
            }
        }
    }
}

/// Text goes with a press only, as a real keyboard reports it.
fn character_input(
    key_code: KeyCode,
    character: char,
    state: ButtonState,
    window: Entity,
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

/// Run after the picking systems have seen the pointer's last message.
fn despawn_ended_touch_pointers(
    mut touches: MessageReader<TouchInput>,
    pointers: Query<(Entity, &PointerId)>,
    mut commands: Commands,
) {
    for touch in touches.read() {
        if !matches!(touch.phase, TouchPhase::Ended | TouchPhase::Canceled) {
            continue;
        }
        for (entity, id) in &pointers {
            if id.get_touch_id() == Some(touch.id) {
                commands.entity(entity).try_despawn();
            }
        }
    }
}
