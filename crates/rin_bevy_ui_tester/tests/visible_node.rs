//! `Visible` is about the named node, not the widget above it: a hidden list
//! inside a visible selectable row is hidden.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    prelude::*,
    ui::{Selectable, px},
};
use rin_bevy_ui_tester::{Check, Scenario, Step, Target};

/// Frames for layout and visibility propagation.
const SETTLE_FRAMES: u16 = 3;

/// A visible `Selectable` row holding a non-widget node called "list",
/// hidden or shown.
fn row_with_list(name: &str, list: Visibility) -> Scenario {
    Scenario::new(name)
        .setup(move |world| {
            world.spawn(Camera2d);
            world.spawn((
                Name::new("row"),
                Selectable,
                Node {
                    width: px(200),
                    height: px(100),
                    ..Node::default()
                },
                children![(
                    Name::new("list"),
                    Node {
                        width: px(50),
                        height: px(50),
                        ..Node::default()
                    },
                    list,
                )],
            ));
        })
        .step(Step::wait(SETTLE_FRAMES))
}

#[test]
fn a_hidden_node_inside_a_visible_selectable_row_is_not_visible() {
    let report = row_with_list("hidden list", Visibility::Hidden)
        .steps([
            Step::check(Check::visible(Target::name("row"))),
            Step::check(!Check::visible(Target::name("list"))),
        ])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_shown_node_inside_a_visible_selectable_row_is_visible() {
    let report = row_with_list("shown list", Visibility::Inherited)
        .step(Step::check(Check::visible(Target::name("list"))))
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}
