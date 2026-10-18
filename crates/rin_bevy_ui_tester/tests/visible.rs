//! `Visible` asks about the node a target names, not the widget around it: a
//! hidden list inside a visible selectable row is not visible.

#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    prelude::*,
    ui::{Selectable, px},
};
use rin_bevy_ui_tester::{Check, Scenario, Step, Target};

/// How many frames the UI gets to be laid out before a check.
const SETTLE_FRAMES: u16 = 3;

/// A selectable row holding a plain node called "Popup", hidden or not.
fn row_with_popup(name: &str, visibility: Visibility) -> Scenario {
    Scenario::new(name)
        .setup(move |world| {
            world.spawn(Camera2d);
            world.spawn((
                Name::new("Row"),
                Selectable,
                Node {
                    width: px(200),
                    height: px(100),
                    ..Node::default()
                },
                children![(
                    Name::new("Popup"),
                    Node {
                        width: px(50),
                        height: px(50),
                        ..Node::default()
                    },
                    visibility,
                )],
            ));
        })
        .step(Step::wait(SETTLE_FRAMES))
}

#[test]
fn a_hidden_node_inside_a_visible_selectable_row_is_not_visible() {
    let report = row_with_popup("hidden", Visibility::Hidden)
        .steps([
            Step::check(Check::visible(Target::name("Row"))),
            Step::check(!Check::visible(Target::name("Popup"))),
        ])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_visible_node_inside_a_visible_selectable_row_is_visible() {
    let report = row_with_popup("shown", Visibility::Inherited)
        .step(Step::check(Check::visible(Target::name("Popup"))))
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}
