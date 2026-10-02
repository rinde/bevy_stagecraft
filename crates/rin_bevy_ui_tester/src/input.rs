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
        keyboard::{Key, KeyCode, KeyboardInput},
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
                    logical_key: Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
                    state,
                    text: None,
                    repeat: false,
                    window: window_entity,
                });
            }
        }
    }
}
