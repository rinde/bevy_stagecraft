#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    input::{
        ButtonInput,
        keyboard::KeyCode,
        mouse::{MouseButton, MouseScrollUnit},
    },
    prelude::*,
    ui::{ScrollPosition, px},
    ui_widgets::{Activate, Button, ScrollArea},
    window::PrimaryWindow,
};
use bevy_stagecraft::{HeadlessPlugins, SyntheticInput};

#[derive(Resource, Default)]
struct Activations(u32);

const BUTTON_CENTER: Vec2 = Vec2::new(150.0, 50.0);
const SCROLL_CENTER: Vec2 = Vec2::new(150.0, 300.0);

fn ui_app() -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default())
        .init_resource::<Activations>();
    app.finish();
    app.cleanup();

    app.world_mut().spawn(Camera2d);
    app.world_mut().spawn((
        Button,
        Node {
            position_type: PositionType::Absolute,
            left: px(100),
            top: px(25),
            width: px(100),
            height: px(50),
            ..Node::default()
        },
        BackgroundColor(Color::WHITE),
    ));
    app.world_mut()
        .add_observer(|_: On<Activate>, mut count: ResMut<Activations>| count.0 += 1);

    let area = app
        .world_mut()
        .spawn((
            ScrollArea,
            Node {
                position_type: PositionType::Absolute,
                left: px(100),
                top: px(200),
                width: px(100),
                height: px(200),
                overflow: Overflow::scroll_y(),
                ..Node::default()
            },
        ))
        .id();
    app.world_mut().spawn((
        Node {
            width: px(100),
            height: px(1000),
            ..Node::default()
        },
        ChildOf(area),
    ));

    // One frame to lay out the UI, one more for picking to see it.
    app.update();
    app.update();
    app
}

fn settle(app: &mut App) {
    for _ in 0..3 {
        app.update();
    }
}

#[test]
fn a_synthetic_click_on_a_button_fires_activate() {
    let mut app = ui_app();
    {
        let mut input = app.world_mut().resource_mut::<SyntheticInput>();
        input.move_cursor(BUTTON_CENTER);
    }
    settle(&mut app);
    {
        let mut input = app.world_mut().resource_mut::<SyntheticInput>();
        input.press_button(MouseButton::Left);
    }
    settle(&mut app);
    {
        let mut input = app.world_mut().resource_mut::<SyntheticInput>();
        input.release_button(MouseButton::Left);
    }
    settle(&mut app);
    assert_eq!(
        app.world().resource::<Activations>().0,
        1,
        "the button should have been activated exactly once"
    );
}

#[test]
fn a_synthetic_click_beside_the_button_does_nothing() {
    let mut app = ui_app();
    app.world_mut()
        .resource_mut::<SyntheticInput>()
        .move_cursor(Vec2::new(600.0, 600.0));
    settle(&mut app);
    app.world_mut()
        .resource_mut::<SyntheticInput>()
        .press_button(MouseButton::Left);
    settle(&mut app);
    app.world_mut()
        .resource_mut::<SyntheticInput>()
        .release_button(MouseButton::Left);
    settle(&mut app);
    assert_eq!(app.world().resource::<Activations>().0, 0);
}

#[test]
fn a_wheel_turn_over_a_scroll_area_scrolls_it() {
    let mut app = ui_app();
    app.world_mut()
        .resource_mut::<SyntheticInput>()
        .move_cursor(SCROLL_CENTER);
    settle(&mut app);
    app.world_mut()
        .resource_mut::<SyntheticInput>()
        .scroll(MouseScrollUnit::Pixel, 0.0, -40.0);
    settle(&mut app);
    let mut query = app.world_mut().query::<(&ScrollArea, &ScrollPosition)>();
    let (_, position) = query
        .iter(app.world())
        .next()
        .expect("the scroll area exists");
    assert!(
        (position.y - 40.0).abs() < f32::EPSILON,
        "scrolled to {} instead of 40",
        position.y
    );
}

#[test]
fn key_taps_show_up_in_button_input() {
    let mut app = ui_app();
    app.world_mut()
        .resource_mut::<SyntheticInput>()
        .press_key(KeyCode::KeyA);
    app.update();
    assert!(
        app.world()
            .resource::<ButtonInput<KeyCode>>()
            .just_pressed(KeyCode::KeyA),
        "A should be just pressed"
    );
    app.world_mut()
        .resource_mut::<SyntheticInput>()
        .release_key(KeyCode::KeyA);
    app.update();
    let keys = app.world().resource::<ButtonInput<KeyCode>>();
    assert!(keys.just_released(KeyCode::KeyA), "A should be released");
    assert!(!keys.pressed(KeyCode::KeyA), "A should no longer be held");
}

#[test]
fn the_cursor_position_is_readable_from_the_primary_window() {
    let mut app = ui_app();
    app.world_mut()
        .resource_mut::<SyntheticInput>()
        .move_cursor(Vec2::new(321.0, 123.0));
    app.update();
    let mut query = app
        .world_mut()
        .query_filtered::<&Window, With<PrimaryWindow>>();
    let window = query
        .iter(app.world())
        .next()
        .expect("there is a primary window");
    assert_eq!(window.cursor_position(), Some(Vec2::new(321.0, 123.0)));
}
