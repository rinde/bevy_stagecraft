//! Keeping the real mouse and keyboard out of the app while a scenario plays,
//! and routing what the Scenarios window gets.
//!
//! While the script has the primary window ([`GalleryState::input_is_scripted`]):
//!
//! * `PointerInputSettings::is_mouse_enabled` (and touch) is off, so Bevy's own
//!   mouse picking stays quiet;
//! * the real mouse, keyboard and touch messages aimed at the primary window
//!   are removed before `InputSystems` reads them;
//! * the Scenarios window's mouse goes to a pointer of its own
//!   ([`scenarios_pointer`]), so its buttons work while the script's pointer
//!   is somewhere else;
//! * the synthetic cursor position is set on the primary window each frame
//!   for the update, and put back before the window system sees it, so the
//!   operating system's pointer is never moved.

use bevy::camera::NormalizedRenderTarget;
use bevy::{
    app::AppExit,
    asset::uuid::Uuid,
    ecs::{
        entity::Entity,
        message::{Message, MessageWriter, Messages},
        query::With,
        resource::Resource,
        system::{Commands, Query, Res, ResMut},
    },
    input::{
        ButtonState,
        keyboard::{KeyCode, KeyboardInput},
        mouse::{MouseButton, MouseButtonInput, MouseScrollUnit, MouseWheel},
        touch::TouchInput,
    },
    math::Vec2,
    picking::{
        input::PointerInputSettings,
        pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput},
    },
    window::{CursorMoved, PrimaryWindow, Window, WindowRef},
};

use super::{
    ScenariosWindow,
    state::{GalleryAction, GalleryState},
};
use crate::input::SyntheticInput;

/// Stable identity of the pointer the Scenarios window's real mouse drives.
const SCENARIOS_POINTER: Uuid = Uuid::from_u128(0x7363_656e_6172_696f_732d_706f_696e_7465);

/// How many pixels one line of mouse wheel scrolls the catalogue.
pub(super) const LINE_PIXELS: f32 = 24.0;

/// The pointer the Scenarios window's real mouse drives while the script has
/// the primary window.
#[must_use]
pub const fn scenarios_pointer() -> PointerId {
    PointerId::Custom(SCENARIOS_POINTER)
}

/// How far the mouse wheel has asked the catalogue to scroll, in pixels,
/// since the list last used it.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub(super) struct ScenariosScroll(pub(super) f32);

/// The real cursor position, set aside while the script's is on show.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub(super) struct RealCursor {
    /// Whether the script's position is in, to be taken out again.
    active: bool,
    /// What the window's cursor was before the script's went in.
    cursor: Option<Vec2>,
}

/// Spawns the pointer for the Scenarios window.
pub(super) fn spawn_scenarios_pointer(mut commands: Commands) {
    commands.spawn(scenarios_pointer());
}

/// Where a message goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// Leave it for the app and Bevy.
    Keep,
    /// Remove it.
    Drop,
    /// Remove it and hand it to the gallery.
    Take,
}

/// Empties `messages`, writes back those routed [`Route::Keep`], and returns
/// those routed [`Route::Take`].
fn sort_messages<T: Message>(
    messages: &mut Messages<T>,
    window: impl Fn(&T) -> Entity,
    route: impl Fn(Entity) -> Route,
) -> Vec<T> {
    if messages.is_empty() {
        return Vec::new();
    }
    let all: Vec<T> = messages.drain().collect();
    let mut taken = Vec::new();
    for message in all {
        match route(window(&message)) {
            Route::Keep => {
                messages.write(message);
            }
            Route::Drop => {}
            Route::Take => taken.push(message),
        }
    }
    taken
}

/// The action a key in the Scenarios window stands for.
const fn shortcut(key: KeyCode) -> Option<GalleryAction> {
    match key {
        KeyCode::Space => Some(GalleryAction::TogglePlay),
        KeyCode::ArrowRight => Some(GalleryAction::StepFrame),
        KeyCode::ArrowDown => Some(GalleryAction::StepStep),
        KeyCode::ArrowLeft => Some(GalleryAction::Restart),
        _ => None,
    }
}

/// The pointer location for a point in the Scenarios window.
fn location(window: Entity, position: Vec2) -> Option<Location> {
    WindowRef::Entity(window)
        .normalize(Some(window))
        .map(|target| Location {
            target: NormalizedRenderTarget::Window(target),
            position,
        })
}

/// Sorts the real input of this update: what the script must not see, what the
/// Scenarios window gets, and what is left for the app.
pub(super) fn route_real_input(
    mut state: ResMut<GalleryState>,
    mut settings: ResMut<PointerInputSettings>,
    mut scroll: ResMut<ScenariosScroll>,
    primary: Query<Entity, With<PrimaryWindow>>,
    scenarios: Query<Entity, With<ScenariosWindow>>,
    mut cursor_moved: ResMut<Messages<CursorMoved>>,
    mut buttons: ResMut<Messages<MouseButtonInput>>,
    mut wheel: ResMut<Messages<MouseWheel>>,
    mut keys: ResMut<Messages<KeyboardInput>>,
    mut touches: ResMut<Messages<TouchInput>>,
    mut pointer_inputs: MessageWriter<PointerInput>,
    mut last_position: bevy::ecs::system::Local<Vec2>,
) {
    let scripted = state.input_is_scripted();
    // Bevy's mouse picking reads every window's events, the Scenarios
    // window's too, so it is off for the whole time and the Scenarios
    // window gets a pointer of its own instead.
    if settings.is_mouse_enabled == scripted {
        settings.is_mouse_enabled = !scripted;
    }
    if settings.is_touch_enabled == scripted {
        settings.is_touch_enabled = !scripted;
    }
    let primary = primary.iter().next();
    let scenarios = scenarios.iter().next();
    let route_mouse = |window: Entity| {
        if !scripted {
            Route::Keep
        } else if Some(window) == primary {
            Route::Drop
        } else if Some(window) == scenarios {
            Route::Take
        } else {
            Route::Keep
        }
    };
    let route_keys = |window: Entity| {
        if Some(window) == scenarios {
            Route::Take
        } else if scripted && Some(window) == primary {
            Route::Drop
        } else {
            Route::Keep
        }
    };
    let route_touch = |window: Entity| {
        if scripted && Some(window) == primary {
            Route::Drop
        } else {
            Route::Keep
        }
    };

    for key in sort_messages(&mut keys, |message| message.window, route_keys) {
        if key.state == ButtonState::Pressed
            && !key.repeat
            && let Some(action) = shortcut(key.key_code)
        {
            state.request(action);
        }
    }
    drop(sort_messages(
        &mut touches,
        |message| message.window,
        route_touch,
    ));
    let moves = sort_messages(&mut cursor_moved, |message| message.window, route_mouse);
    let presses = sort_messages(&mut buttons, |message| message.window, route_mouse);
    let turns = sort_messages(&mut wheel, |message| message.window, route_mouse);
    let pointer = scenarios_pointer();
    for moved in moves {
        let Some(at) = location(moved.window, moved.position) else {
            continue;
        };
        let delta = moved.position - *last_position;
        *last_position = moved.position;
        pointer_inputs.write(PointerInput::new(
            pointer,
            at,
            PointerAction::Move { delta },
        ));
    }
    for press in presses {
        let button = match press.button {
            MouseButton::Left => PointerButton::Primary,
            MouseButton::Right => PointerButton::Secondary,
            MouseButton::Middle => PointerButton::Middle,
            MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => continue,
        };
        let Some(at) = location(press.window, *last_position) else {
            continue;
        };
        let action = match press.state {
            ButtonState::Pressed => PointerAction::Press(button),
            ButtonState::Released => PointerAction::Release(button),
        };
        pointer_inputs.write(PointerInput::new(pointer, at, action));
    }
    for turn in turns {
        let pixels = match turn.unit {
            MouseScrollUnit::Line => turn.y * LINE_PIXELS,
            MouseScrollUnit::Pixel => turn.y,
        };
        scroll.0 += pixels;
    }
}

/// Keeps the real cursor position to put back later. It runs before the
/// synthetic input is drained, because draining moves the window's cursor too.
pub(super) fn save_real_cursor(
    state: Res<GalleryState>,
    mut real: ResMut<RealCursor>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    real.active = state.input_is_scripted();
    real.cursor = windows.iter().next().and_then(Window::cursor_position);
}

/// Puts the script's cursor on the primary window for this update.
pub(super) fn assert_script_cursor(
    real: Res<RealCursor>,
    input: Res<SyntheticInput>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !real.active {
        return;
    }
    for mut window in &mut windows {
        window.set_cursor_position(input.cursor());
    }
}

/// Puts the real cursor back before the window system compares it with what it
/// last told the operating system, so the pointer is never warped.
pub(super) fn restore_real_cursor(
    mut real: ResMut<RealCursor>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !real.active {
        return;
    }
    real.active = false;
    for mut window in &mut windows {
        window.set_cursor_position(real.cursor);
    }
}

/// Ends the app when the primary window closes, even though the Scenarios
/// window is still open.
pub(super) fn exit_with_primary(
    mut removed: bevy::ecs::lifecycle::RemovedComponents<PrimaryWindow>,
    mut exit: MessageWriter<AppExit>,
) {
    if removed.read().next().is_some() {
        exit.write(AppExit::Success);
    }
}
