//! `OnTop` asks what the person sees: is the widget painted topmost at its
//! middle, whatever a covering node lets the pointer through.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    picking::Pickable,
    prelude::*,
    ui::{BackgroundGradient, ColorStop, Gradient, LinearGradient, px, widget::Text},
    ui_widgets::Button,
};
use bevy_stagecraft::{Check, Outcome, Scenario, Step, Target, WidgetRole};

/// How many frames the UI gets to be laid out and stacked before a check.
const SETTLE_FRAMES: u16 = 3;

/// A camera and the "Go" button, at (100, 25) with a size of 100 by 50.
fn spawn_go(world: &mut World) {
    world.spawn(Camera2d);
    world.spawn((
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
        children![Text::new("Go")],
    ));
}

/// A node over the middle of the "Go" button, at `z`, with `colour` as its
/// background, that lets the pointer through like a floating popup does.
fn spawn_popup(world: &mut World, z: i32, colour: Color) {
    world.spawn((
        Name::new("Popup"),
        Node {
            position_type: PositionType::Absolute,
            left: px(80),
            top: px(10),
            width: px(200),
            height: px(100),
            ..Node::default()
        },
        BackgroundColor(colour),
        GlobalZIndex(z),
        Pickable {
            should_block_lower: false,
            is_hoverable: false,
        },
    ));
}

/// The "Go" button, and a popup at `z` painted `colour` over it.
fn with_popup(name: &str, z: i32, colour: Color) -> Scenario {
    Scenario::new(name)
        .setup(move |world| {
            spawn_go(world);
            spawn_popup(world, z, colour);
        })
        .step(Step::wait(SETTLE_FRAMES))
}

#[test]
fn a_popup_that_lets_the_pointer_through_still_covers_the_widget_under_it() {
    let report = with_popup("covered", 10, Color::BLACK)
        .step(Step::check(Check::on_top(Target::text("Go"))))
        .run_headless();
    let Outcome::Fail(message) = &report.steps[1].outcome else {
        panic!("OnTop should fail: {:?}", report.steps);
    };
    assert!(
        message.contains("is not on top: it is covered by `Popup`"),
        "{message}"
    );
}

#[test]
fn the_pointer_reaches_a_widget_that_a_popup_covers_without_blocking() {
    let report = with_popup("click through", 10, Color::BLACK)
        .steps([
            Step::check(Check::visible(Target::text("Go"))),
            Step::move_to(Target::text("Go")),
            Step::check(Check::hovered(Target::text("Go"))),
            Step::check(!Check::on_top(Target::text("Go"))),
        ])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

/// A node over the middle of the "Go" button that paints only through
/// `painter`, whatever it is, and lets the pointer through.
fn with_painted_popup(name: &str, painter: impl Bundle + Clone) -> Scenario {
    Scenario::new(name)
        .setup(move |world| {
            spawn_go(world);
            world.spawn((
                Name::new("Popup"),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(80),
                    top: px(10),
                    width: px(200),
                    height: px(100),
                    ..Node::default()
                },
                painter.clone(),
                GlobalZIndex(10),
                Pickable {
                    should_block_lower: false,
                    is_hoverable: false,
                },
            ));
        })
        .step(Step::wait(SETTLE_FRAMES))
}

/// Plays `scenario` with `check` as its last step and returns that step's
/// failure message, if it failed.
fn failure_of(scenario: Scenario, check: Check) -> Option<String> {
    let report = scenario.step(Step::check(check)).run_headless();
    match &report.steps.last()?.outcome {
        Outcome::Fail(message) => Some(message.clone()),
        _ => None,
    }
}

#[test]
fn a_popup_painted_only_with_a_gradient_covers_the_widget_under_it() {
    let gradient = BackgroundGradient::from(Gradient::Linear(LinearGradient::new(
        0.0,
        vec![
            ColorStop::new(Color::BLACK, Val::Percent(0.0)),
            ColorStop::new(Color::WHITE, Val::Percent(100.0)),
        ],
    )));
    let message = failure_of(
        with_painted_popup("gradient", gradient),
        Check::on_top(Target::text("Go")),
    )
    .expect("OnTop should fail");
    assert!(
        message.contains("is not on top: it is covered by `Popup`"),
        "{message}"
    );
}

#[test]
fn a_popup_with_text_covers_the_widget_under_it() {
    let message = failure_of(
        with_painted_popup("text", Text::new("Over")),
        Check::on_top(Target::text("Go")),
    )
    .expect("OnTop should fail");
    assert!(message.contains("it is covered by `Popup`"), "{message}");
}

#[test]
fn a_popup_with_an_image_covers_the_widget_under_it() {
    let message = failure_of(
        with_painted_popup("image", ImageNode::default()),
        Check::on_top(Target::text("Go")),
    )
    .expect("OnTop should fail");
    assert!(message.contains("it is covered by `Popup`"), "{message}");
}

#[test]
fn not_on_top_holds_for_a_widget_that_a_covering_popup_hides() {
    let report = with_popup("covered", 10, Color::BLACK)
        .step(Step::check(!Check::on_top(Target::text("Go"))))
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

/// A widget with no `Node`, so nothing ever lays it out, around the text "Go".
fn spawn_unlaid_go(world: &mut World) {
    world.spawn(Camera2d);
    world.spawn((Button, children![Text::new("Go")]));
}

#[test]
fn a_widget_that_is_never_laid_out_is_not_on_top() {
    let scenario = Scenario::new("unlaid")
        .setup(spawn_unlaid_go)
        .step(Step::wait(SETTLE_FRAMES));
    let message = failure_of(scenario, Check::on_top(Target::text("Go")))
        .expect("OnTop should fail for a widget without layout");
    assert!(
        message.contains("is not on top: it has not been laid out yet"),
        "{message}"
    );
}

#[test]
fn not_on_top_holds_for_a_widget_that_is_never_laid_out() {
    let report = Scenario::new("unlaid not")
        .setup(spawn_unlaid_go)
        .steps([
            Step::wait(SETTLE_FRAMES),
            Step::check(!Check::on_top(Target::text("Go"))),
        ])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

/// A "Go" button that lies wholly outside the box its parent clips to.
fn spawn_clipped_go(world: &mut World) {
    world.spawn(Camera2d);
    world.spawn((
        Node {
            width: px(50),
            height: px(50),
            overflow: Overflow::clip(),
            ..Node::default()
        },
        children![(
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: px(200),
                top: px(200),
                width: px(100),
                height: px(50),
                ..Node::default()
            },
            BackgroundColor(Color::WHITE),
            children![Text::new("Go")],
        )],
    ));
}

#[test]
fn a_widget_clipped_away_by_its_parent_is_not_on_top() {
    let scenario = Scenario::new("clipped")
        .setup(spawn_clipped_go)
        .step(Step::wait(SETTLE_FRAMES));
    let message = failure_of(scenario, Check::on_top(Target::text("Go")))
        .expect("OnTop should fail for a clipped widget");
    assert!(
        message.contains("is not on top: nothing is drawn at its middle"),
        "{message}"
    );
}

#[test]
fn not_on_top_holds_for_a_widget_that_is_clipped_away() {
    let report = Scenario::new("clipped not")
        .setup(spawn_clipped_go)
        .steps([
            Step::wait(SETTLE_FRAMES),
            Step::check(!Check::on_top(Target::text("Go"))),
        ])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_widget_is_on_top_of_a_popup_that_is_painted_behind_it() {
    let report = with_popup("behind", -10, Color::BLACK)
        .step(Step::check(Check::on_top(Target::text("Go"))))
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_node_that_paints_nothing_covers_nothing() {
    let report = with_popup("transparent", 10, Color::NONE)
        .step(Step::check(Check::on_top(Target::text("Go"))))
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_widget_that_nothing_covers_is_on_top_when_named_by_its_label() {
    let report = Scenario::new("plain")
        .setup(spawn_go)
        .steps([
            Step::wait(SETTLE_FRAMES),
            Step::check(Check::on_top(Target::text("Go"))),
        ])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn on_top_reads_and_writes_as_ron() {
    let text = r#"(
        format_version: 3,
        name: "ron",
        steps: [Check(OnTop(Text("Go"))), Check(Not(OnTop(Role(role: Button, label: "Go"))))],
    )"#;
    let scenario = Scenario::from_ron(text).expect("the script loads");
    assert_eq!(
        scenario.script(),
        [
            Step::check(Check::on_top(Target::text("Go"))),
            Step::check(!Check::on_top(Target::role(WidgetRole::Button, "Go"))),
        ]
    );
    let written = scenario.to_ron().expect("written");
    assert!(written.contains("OnTop(Text(\"Go\"))"), "{written}");
}

/// A "Go" button holding a plain node "Inner" (no widget) whose middle a popup
/// covers, while the button's own middle stays free.
fn spawn_button_with_covered_inner(world: &mut World) {
    world.spawn(Camera2d);
    world.spawn((
        Name::new("Outer"),
        Button,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: px(300),
            height: px(300),
            ..Node::default()
        },
        BackgroundColor(Color::WHITE),
        children![(
            Name::new("Inner"),
            Node {
                position_type: PositionType::Absolute,
                left: px(10),
                top: px(10),
                width: px(40),
                height: px(40),
                ..Node::default()
            },
        )],
    ));
    world.spawn((
        Name::new("Popup"),
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: px(60),
            height: px(60),
            ..Node::default()
        },
        BackgroundColor(Color::BLACK),
        GlobalZIndex(10),
        Pickable {
            should_block_lower: false,
            is_hoverable: false,
        },
    ));
}

#[test]
fn a_plain_node_inside_a_widget_is_judged_by_itself_not_by_the_widget() {
    let scenario = Scenario::new("inner covered")
        .setup(spawn_button_with_covered_inner)
        .step(Step::wait(SETTLE_FRAMES));
    // The button's own middle is free, so it stays on top while its inner
    // node is covered: the node and the widget are judged apart.
    let report = scenario
        .clone()
        .step(Step::check(Check::on_top(Target::name("Outer"))))
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
    let message = failure_of(scenario, Check::on_top(Target::name("Inner")))
        .expect("OnTop should fail for the covered inner node");
    assert!(message.contains("it is covered by `Popup`"), "{message}");
}
