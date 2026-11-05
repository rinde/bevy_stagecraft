//! Tabs of `bevy_ui_widgets`: the `Tab` and `TabList` roles, and `Selected`
//! holding for the tab a `SelectedTab` names.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    prelude::*,
    ui::px,
    ui_widgets::{SelectedTab, Tab, TabList, tablist_self_update},
};
use bevy_stagecraft::{
    Check, HeadlessPlugins, Scenario, Step, Target, WidgetRole, tree_snapshot,
};

/// The labels of the tabs, in order.
const LABELS: [&str; 3] = ["Rendering", "Audio", "Input"];

/// Spawns a camera and a tab list of three tabs, the first selected, that
/// updates its own selection.
fn spawn_tabs(world: &mut World) {
    world.spawn(Camera2d);
    let list = world
        .spawn((
            TabList::default(),
            Node {
                width: px(300),
                height: px(40),
                ..Node::default()
            },
        ))
        .observe(tablist_self_update)
        .id();
    let tabs: Vec<Entity> = LABELS
        .iter()
        .map(|label| {
            world
                .spawn((
                    Tab,
                    Node {
                        width: px(100),
                        height: px(40),
                        ..Node::default()
                    },
                    children![Text::new(*label)],
                    ChildOf(list),
                ))
                .id()
        })
        .collect();
    world
        .entity_mut(list)
        .insert(SelectedTab(tabs.first().copied()));
}

/// The tab with this label.
fn tab(label: &str) -> Target {
    Target::role(WidgetRole::Tab, label)
}

#[test]
fn clicking_a_tab_selects_it_and_the_role_and_state_checks_follow() {
    let report = Scenario::new("tabs")
        .setup(spawn_tabs)
        .steps([
            Step::check(Check::exists(Target::of_role(WidgetRole::TabList))),
            Step::check(Check::selected(tab("Rendering"))),
            Step::check(!Check::selected(tab("Audio"))),
            Step::move_to(tab("Audio")),
            Step::click(),
            Step::wait(2),
            Step::check(Check::selected(tab("Audio"))),
            Step::check(!Check::selected(tab("Rendering"))),
        ])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_tree_snapshot_shows_the_tab_list_and_the_selected_tab() {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    spawn_tabs(app.world_mut());
    for _ in 0..3 {
        app.update();
    }
    let snapshot = tree_snapshot(app.world(), &Target::of_role(WidgetRole::TabList)).unwrap();
    assert!(snapshot.contains("tab-list"), "{snapshot}");
    assert!(
        snapshot.contains("tab \"Rendering\" [selected]"),
        "{snapshot}"
    );
    assert!(snapshot.contains("tab \"Audio\""), "{snapshot}");
    assert!(!snapshot.contains("\"Audio\" [selected]"), "{snapshot}");
}
