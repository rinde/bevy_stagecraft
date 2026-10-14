//! Text with images and boxes set inline: read as its text alone, and shown
//! in tree snapshots as lines of their own.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    prelude::*,
    text::{InlineBox, InlineBoxKind},
    ui::{px, widget::InlineImage},
    ui_widgets::Button,
};
use rin_bevy_ui_tester::{
    Check, HeadlessPlugins, Outcome, Scenario, Step, Target, TextMatch, WidgetRole, tree_snapshot,
};

/// The button the tests look at.
fn button() -> Target {
    Target::role(WidgetRole::Button, "Save now")
}

/// A button whose label has an icon in front of it and a box after it.
fn spawn_button_with_icon(world: &mut World) {
    world.spawn(Camera2d);
    world.spawn((
        Button,
        Node {
            width: px(200),
            height: px(40),
            ..Node::default()
        },
        children![(
            Text::new("Save"),
            children![
                InlineImage::default(),
                TextSpan::new(" now"),
                (
                    InlineBox {
                        kind: InlineBoxKind::InFlow,
                        size: Vec2::splat(8.0),
                    },
                    // Nothing in a box is part of the label.
                    children![Text::new("Hidden")],
                ),
            ],
        )],
    ));
}

/// The outcome of the one step of a scenario over the button.
fn outcome_of(step: Step) -> Outcome {
    let report = Scenario::new("inline")
        .setup(spawn_button_with_icon)
        .steps([step])
        .run_headless();
    report.steps[0].outcome.clone()
}

#[test]
fn a_label_with_inline_boxes_is_found_as_its_text_alone() {
    assert_eq!(
        outcome_of(Step::check(Check::exists(button()))),
        Outcome::Pass
    );
}

#[test]
fn shows_and_text_of_read_text_with_inline_boxes_without_gaps_or_strays() {
    assert_eq!(outcome_of(Step::check(Check::shows("Save"))), Outcome::Pass);
    assert_eq!(
        outcome_of(Step::check(Check::text_of(
            button(),
            TextMatch::Exact("Save now".into())
        ))),
        Outcome::Pass
    );
    assert_eq!(
        outcome_of(Step::check(Check::exists(Target::text("Save")))),
        Outcome::Pass
    );
}

#[test]
fn a_snapshot_shows_an_inline_image_and_box_under_the_label() {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    spawn_button_with_icon(app.world_mut());
    for _ in 0..3 {
        app.update();
    }
    let snapshot = tree_snapshot(app.world(), &button()).unwrap();
    assert_eq!(
        snapshot,
        "- button \"Save now\":\n  - image\n  - inline-box"
    );
    assert_eq!(
        outcome_of(Step::check(Check::tree(
            button(),
            "- button \"Save now\":\n  - image\n"
        ))),
        Outcome::Pass
    );
}
