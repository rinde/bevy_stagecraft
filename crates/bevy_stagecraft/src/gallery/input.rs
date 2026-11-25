//! Keeps the real mouse and keyboard out of the app while a scenario plays and
//! routes what the Scenarios window gets.
//!
//! While the script has the primary window ([`GalleryState::input_is_scripted`])
//! its real mouse, keyboard and touch messages are removed, Bevy's mouse picking
//! is off, and the synthetic cursor position is put back before the window
//! system sees it. Pick mode ([`GalleryState::is_picking`]) does the same for
//! the app's mouse and keyboard and hides the cursor from the app.

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

const SCENARIOS_POINTER: Uuid = Uuid::from_u128(0x7363_656e_6172_696f_732d_706f_696e_7465);

pub(super) const LINE_PIXELS: f32 = 24.0;

#[must_use]
pub const fn scenarios_pointer() -> PointerId {
    PointerId::Custom(SCENARIOS_POINTER)
}

#[derive(Resource, Debug, Default, Clone, Copy)]
pub(super) struct ScenariosScroll(pub(super) f32);

#[derive(Resource, Debug, Default, Clone, Copy)]
pub(super) struct RealCursor {
    active: bool,
    cursor: Option<Vec2>,
}

impl RealCursor {
    pub(super) const fn position(&self) -> Option<Vec2> {
        self.cursor
    }
}

#[derive(Resource, Debug, Default, Clone, Copy)]
pub(super) struct PickClicks(pub(super) u32);

pub(super) fn spawn_scenarios_pointer(mut commands: Commands) {
    commands.spawn(scenarios_pointer());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Keep,
    Drop,
    Take,
}

fn sort_messages<T: Message>(
    messages: &mut Messages<T>,
    window: impl Fn(&T) -> Entity,
    route: impl Fn(Entity) -> Route,
) -> Vec<T> {
    // Rewriting messages gives them new ids so readers see them again: only do
    // it when something is removed, or unwanted messages replay every update.
    let needs_routing = messages
        .get_cursor()
        .read(messages)
        .any(|message| route(window(message)) != Route::Keep);
    if !needs_routing {
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

const fn shortcut(key: KeyCode) -> Option<GalleryAction> {
    match key {
        KeyCode::Space => Some(GalleryAction::TogglePlay),
        KeyCode::ArrowRight => Some(GalleryAction::StepFrame),
        KeyCode::ArrowDown => Some(GalleryAction::StepStep),
        KeyCode::ArrowLeft => Some(GalleryAction::Restart),
        _ => None,
    }
}

fn location(window: Entity, position: Vec2) -> Option<Location> {
    WindowRef::Entity(window)
        .normalize(Some(window))
        .map(|target| Location {
            target: NormalizedRenderTarget::Window(target),
            position,
        })
}

fn key_action(key: &KeyboardInput, scenarios: Option<Entity>) -> Option<GalleryAction> {
    if key.state != ButtonState::Pressed || key.repeat {
        return None;
    }
    if Some(key.window) == scenarios {
        shortcut(key.key_code)
    } else {
        (key.key_code == KeyCode::Escape).then_some(GalleryAction::TogglePick)
    }
}

/// Counts clicks outside the Scenarios window; pick mode took them from the app.
fn app_clicks(presses: &[MouseButtonInput], scenarios: Option<Entity>) -> u32 {
    let clicks = presses
        .iter()
        .filter(|press| {
            Some(press.window) != scenarios
                && press.button == MouseButton::Left
                && press.state == ButtonState::Pressed
        })
        .count();
    u32::try_from(clicks).unwrap_or(u32::MAX)
}

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
    mut pick_clicks: ResMut<PickClicks>,
    mut last_position: bevy::ecs::system::Local<Vec2>,
) {
    let picking = state.is_picking();
    let scripted = state.input_is_scripted() || picking;
    // Bevy's mouse picking reads every window's events, so it is off throughout
    // and the Scenarios window gets a pointer of its own.
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
            if picking { Route::Take } else { Route::Drop }
        } else if Some(window) == scenarios {
            Route::Take
        } else {
            Route::Keep
        }
    };
    // The wheel in the Scenarios window is always the gallery's: nothing else
    // scrolls the catalog.
    let route_wheel = |window: Entity| {
        if Some(window) == scenarios {
            Route::Take
        } else {
            route_mouse(window)
        }
    };
    let route_keys = |window: Entity| {
        if Some(window) == scenarios || (picking && Some(window) == primary) {
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
        if let Some(action) = key_action(&key, scenarios) {
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
    let turns = sort_messages(&mut wheel, |message| message.window, route_wheel);
    let pointer = scenarios_pointer();
    let in_scenarios = |window: Entity| Some(window) == scenarios;
    pick_clicks.0 = pick_clicks
        .0
        .saturating_add(app_clicks(&presses, scenarios));
    for moved in moves.into_iter().filter(|moved| in_scenarios(moved.window)) {
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
    for press in presses
        .into_iter()
        .filter(|press| in_scenarios(press.window))
    {
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
    for turn in turns.into_iter().filter(|turn| in_scenarios(turn.window)) {
        let pixels = match turn.unit {
            MouseScrollUnit::Line => turn.y * LINE_PIXELS,
            MouseScrollUnit::Pixel => turn.y,
        };
        scroll.0 += pixels;
    }
}

/// Runs before the synthetic input is drained, which moves the cursor too.
pub(super) fn save_real_cursor(
    state: Res<GalleryState>,
    mut real: ResMut<RealCursor>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    real.active = state.input_is_scripted() || state.is_picking();
    real.cursor = windows.iter().next().and_then(Window::cursor_position);
}

pub(super) fn assert_script_cursor(
    real: Res<RealCursor>,
    state: Res<GalleryState>,
    input: Res<SyntheticInput>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !real.active {
        return;
    }
    let shown = if state.input_is_scripted() {
        input.cursor()
    } else {
        None
    };
    for mut window in &mut windows {
        window.set_cursor_position(shown);
    }
}

/// Runs before the window system compares positions, so the pointer is never warped.
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

pub(super) fn exit_with_either_window(
    mut primary: bevy::ecs::lifecycle::RemovedComponents<PrimaryWindow>,
    mut scenarios: bevy::ecs::lifecycle::RemovedComponents<ScenariosWindow>,
    mut exit: MessageWriter<AppExit>,
) {
    // Both readers are drained so a stale removal cannot fire a later exit.
    let primary_closed = primary.read().next().is_some();
    let scenarios_closed = scenarios.read().next().is_some();
    if primary_closed || scenarios_closed {
        exit.write(AppExit::Success);
    }
}
