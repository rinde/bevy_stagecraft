//! A `bevy_ui_widgets` scroll bar thumb, which has a `ComputedNode` and a
//! `UiGlobalTransform` but no `Node`, is a UI node steps can act on.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    prelude::*,
    ui::{ScrollPosition, px},
    ui_widgets::{ControlOrientation, Scrollbar, ScrollbarThumb},
};
use bevy_stagecraft::{Check, Outcome, Scenario, ScenarioReport, Step, Target};

/// The height of the visible scroll area, and of its bar, in logical pixels.
const VISIBLE: f32 = 100.0;

/// How many rows the scroll area holds.
const ROWS: usize = 8;

/// The height of one row: the content is four times the visible height.
const ROW: f32 = 50.0;

/// The player frame on which the scroll area appears.
const APPEARS_AT: u32 = 10;

/// The scroll area, then its vertical `Scrollbar` at its right with a thumb
/// named `ScrollThumb`.
fn spawn_scroll_area(world: &mut World) {
    let area = world
        .spawn((
            Name::new("ScrollArea"),
            Node {
                position_type: PositionType::Absolute,
                left: px(50),
                top: px(50),
                width: px(200),
                height: px(VISIBLE),
                flex_direction: FlexDirection::Column,
                overflow: Overflow::scroll_y(),
                ..Node::default()
            },
            ScrollPosition::default(),
        ))
        .with_children(|area| {
            for row in 0..ROWS {
                area.spawn((
                    Node {
                        height: px(ROW),
                        flex_shrink: 0.0,
                        ..Node::default()
                    },
                    children![Text::new(format!("Row {row}"))],
                ));
            }
        })
        .id();
    world.spawn((
        Name::new("ScrollTrack"),
        Node {
            position_type: PositionType::Absolute,
            left: px(260),
            top: px(50),
            width: px(12),
            height: px(VISIBLE),
            ..Node::default()
        },
        BackgroundColor(Color::srgb(0.2, 0.2, 0.2)),
        Scrollbar::new(area, ControlOrientation::Vertical, 8.0),
        children![(
            Name::new("ScrollThumb"),
            ScrollbarThumb::default(),
            BackgroundColor(Color::WHITE),
        )],
    ));
}

/// Spawns the scroll area on the player frame [`APPEARS_AT`].
fn spawn_late(world: &mut World) {
    let due = world
        .get_resource::<ScenarioReport>()
        .is_some_and(|report| report.frames == APPEARS_AT);
    if due {
        spawn_scroll_area(world);
    }
}

/// Plays `scenario` with the scroll area appearing late, and returns the
/// report and the area's scroll position, if it has appeared.
fn play(scenario: &Scenario) -> (ScenarioReport, Option<Vec2>) {
    let mut app = scenario.headless_app();
    app.init_resource::<ScenarioReport>();
    app.add_systems(Update, spawn_late);
    let report = scenario.run_in(&mut app);
    let world = app.world_mut();
    let position = world
        .query::<(&Name, &ScrollPosition)>()
        .iter(world)
        .find(|(name, _)| name.as_str() == "ScrollArea")
        .map(|(_, position)| position.0);
    (report, position)
}

/// A camera and nothing else, until the scroll area appears.
fn camera(world: &mut World) {
    world.spawn(Camera2d);
}

#[test]
fn dragging_a_scrollbar_thumb_by_name_waits_for_it_and_scrolls_the_area() {
    // The thumb is 25 pixels tall at the top of the 100-pixel track; 50
    // pixels down the track is half of the 400 pixels of content.
    let scenario = Scenario::new("thumb drag").setup(camera).steps([
        Step::drag(
            Target::name("ScrollThumb"),
            Target::screen(Vec2::new(266.0, 112.5)),
            10,
        ),
        Step::check(Check::visible(Target::name("ScrollThumb"))),
        Step::check(Check::on_top(Target::name("ScrollThumb"))),
    ]);
    let (report, position) = play(&scenario);
    assert!(report.passed(), "{:?}", report.failures());
    let position = position.expect("the scroll area appeared");
    assert!(
        report.frames > APPEARS_AT,
        "the drag waited for the thumb: {} frames",
        report.frames
    );
    assert!(
        (position.y - 200.0).abs() < 1.0,
        "the area scrolled by half its content: {position}"
    );
}

#[test]
fn a_scrollbar_thumb_is_actionable_once_its_scrollbar_has_placed_it() {
    let scenario = Scenario::new("thumb move")
        .setup(camera)
        .steps([Step::move_to(Target::name("ScrollThumb"))]);
    let (report, position) = play(&scenario);
    assert!(report.passed(), "{:?}", report.failures());
    assert!(
        report.frames > APPEARS_AT,
        "the move waited for the thumb: {} frames",
        report.frames
    );
    assert_eq!(
        position,
        Some(Vec2::ZERO),
        "moving the pointer scrolls nothing"
    );
}

#[test]
fn dragging_from_a_name_on_a_camera_is_still_refused_as_not_a_ui_node() {
    let scenario = Scenario::new("camera drag")
        .setup(|world| {
            world.spawn((Camera2d, Name::new("main camera")));
        })
        .steps([Step::drag(
            Target::name("main camera"),
            Target::screen(Vec2::new(10.0, 10.0)),
            5,
        )]);
    let (report, _) = play(&scenario);
    let Outcome::Fail(message) = &report.steps[0].outcome else {
        panic!("the drag should fail: {:?}", report.steps[0].outcome);
    };
    assert!(
        message.starts_with("Name(\"main camera\") names `main camera` (")
            && message.ends_with("), which is not a UI node"),
        "{message}"
    );
}
