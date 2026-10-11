//! UI tree snapshots as a check: the tree below a node as lines of role,
//! label and states, matched partially, with a line diff when it differs.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests may panic on a broken setup"
)]

mod common;

use bevy::{
    prelude::*,
    ui::{Checked, InteractionDisabled, Selected, px},
    ui_widgets::{Button, Checkbox, ListItem, Slider, SliderValue},
};
use common::{registry, spawn_ui};
use rin_bevy_ui_tester::{
    Check, HeadlessPlugins, NoiseFilter, NotUnderTest, Outcome, Picked, Scenario, Step, Target,
    WidgetRole, checks_that_hold, tree_snapshot,
};

/// The name of the panel the controls sit in.
const TOOLBAR: &str = "Toolbar";

/// A labelled button.
fn button(label: &'static str) -> impl Bundle {
    (Button, Node::default(), children![Text::new(label)])
}

/// Spawns a camera and a named panel holding two buttons (the second
/// disabled), a checked checkbox, a slider at one half, a node that is not
/// displayed and one the tooling owns.
fn spawn_toolbar(world: &mut World) {
    world.spawn(Camera2d);
    world
        .spawn((
            Name::new(TOOLBAR),
            Node {
                width: px(400),
                height: px(200),
                ..Node::default()
            },
        ))
        .with_children(|toolbar| {
            toolbar.spawn(button("Pause"));
            toolbar.spawn((button("Lay"), InteractionDisabled));
            toolbar.spawn((
                Checkbox,
                Checked,
                Node::default(),
                children![Text::new("Snap")],
            ));
            toolbar.spawn((Slider::default(), SliderValue(0.5), Node::default()));
            toolbar.spawn((
                Node {
                    display: Display::None,
                    ..Node::default()
                },
                children![Text::new("Secret")],
            ));
            toolbar.spawn((
                NotUnderTest,
                Node::default(),
                children![Text::new("Overlay")],
            ));
        });
}

/// A headless app with the toolbar laid out.
fn toolbar_app() -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    spawn_toolbar(app.world_mut());
    for _ in 0..3 {
        app.update();
    }
    app
}

/// The toolbar panel.
fn toolbar() -> Target {
    Target::name(TOOLBAR)
}

/// The outcome of the one check step of a scenario over the toolbar.
fn outcome_of(check: Check) -> Outcome {
    let report = Scenario::new("tree")
        .setup(spawn_toolbar)
        .steps([Step::check(check)])
        .run_headless();
    report.steps[0].outcome.clone()
}

#[test]
fn a_snapshot_lists_roles_labels_and_states_and_leaves_out_what_is_not_shown() {
    let app = toolbar_app();
    let snapshot = tree_snapshot(app.world(), &toolbar()).unwrap();
    assert_eq!(
        snapshot,
        "- panel \"Toolbar\":\n  \
           - button \"Pause\"\n  \
           - button \"Lay\" [disabled]\n  \
           - checkbox \"Snap\" [checked]\n  \
           - slider [value=0.5]"
    );
}

#[test]
fn a_matching_tree_passes() {
    let app = toolbar_app();
    let snapshot = tree_snapshot(app.world(), &toolbar()).unwrap();
    assert_eq!(outcome_of(Check::tree(toolbar(), snapshot)), Outcome::Pass);
}

#[test]
fn a_partial_tree_passes_when_the_siblings_it_lists_keep_their_order() {
    let tree = "- panel:\n  - button \"Pause\"\n  - checkbox \"Snap\" [checked]\n";
    assert_eq!(outcome_of(Check::tree(toolbar(), tree)), Outcome::Pass);
    let out_of_order = "- panel:\n  - checkbox \"Snap\"\n  - button \"Pause\"\n";
    assert!(matches!(
        outcome_of(Check::tree(toolbar(), out_of_order)),
        Outcome::Fail(_)
    ));
}

#[test]
fn a_changed_label_fails_with_a_line_diff() {
    let tree = "- panel:\n  - button \"Pause\"\n  - button \"Lie\"\n  - slider [value=0.5]\n";
    let Outcome::Fail(message) = outcome_of(Check::tree(toolbar(), tree)) else {
        panic!("a changed label must fail");
    };
    assert_eq!(
        message,
        "the tree of Name(\"Toolbar\") differs (- expected, + actual):\n\
         \x20 - panel:\n\
         \x20   - button \"Pause\"\n\
         -   - button \"Lie\"\n\
         +   - button \"Lay\" [disabled]\n\
         \x20   - slider [value=0.5]"
    );
}

#[test]
fn a_missing_line_is_marked_as_expected_and_a_wrong_state_names_the_one_held() {
    let Outcome::Fail(message) = outcome_of(Check::tree(
        toolbar(),
        "- panel:\n  - button \"Pause\" [pressed]\n  - menu-item \"Open\"\n",
    )) else {
        panic!("a tree that is not there must fail");
    };
    assert!(
        message.contains("-   - button \"Pause\" [pressed]"),
        "{message}"
    );
    assert!(message.contains("+   - button \"Pause\"\n"), "{message}");
    assert!(message.contains("-   - menu-item \"Open\""), "{message}");
}

#[test]
fn a_tree_that_cannot_be_read_is_refused_when_the_script_loads() {
    let ron = r#"(
        format_version: 3,
        name: "bad tree",
        steps: [Check(Tree(of: Name("Toolbar"), matches: "- buton \"x\""))],
    )"#;
    let message = Scenario::from_ron(ron).err().unwrap().to_string();
    assert!(message.contains("did you mean button"), "{message}");
}

#[test]
fn a_tree_check_round_trips_through_ron() {
    let snapshot = "- panel \"Toolbar\":\n  - button \"Say \\\"hi\\\"\"\n  - slider [value=0.5]";
    let check = Check::tree(toolbar(), snapshot);
    let scenario = Scenario::new("tree").steps([Step::check(check.clone())]);
    let ron = scenario.to_ron().unwrap();
    let loaded = Scenario::from_ron_with(&ron, &registry()).unwrap();
    assert_eq!(loaded.to_script(), scenario.to_script());
    assert_eq!(loaded.to_ron().unwrap(), ron, "export is stable");
    assert_eq!(check.to_string(), "tree of Name(\"Toolbar\") matches");
}

#[test]
fn a_tree_written_in_a_raw_string_loads_and_passes() {
    let ron = r###"(
        format_version: 3,
        name: "raw tree",
        steps: [
            Check(Tree(of: Name("Toolbar"), matches: r##"
                - panel:
                  - button "Pause"
                  - slider [value=0.50]
            "##)),
        ],
    )"###;
    let report = Scenario::from_ron(ron)
        .unwrap()
        .setup(spawn_toolbar)
        .run_headless();
    assert!(report.passed(), "{report:?}");
}

#[test]
fn a_target_that_matches_nothing_fails_and_one_that_matches_several_is_broken() {
    let tree = "- panel";
    let Outcome::Fail(message) = outcome_of(Check::tree(Target::name("Nothing"), tree)) else {
        panic!("a missing target must fail");
    };
    assert!(message.contains("no entity is named"), "{message}");
    let several = Target::of_role(WidgetRole::Button);
    let Outcome::Fail(message) = outcome_of(!Check::tree(several, "- button")) else {
        panic!("an ambiguous target is broken even inside Not");
    };
    assert!(message.contains("ambiguous"), "{message}");
}

#[test]
fn a_widget_snapshots_as_one_line_with_its_text_as_the_label() {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    spawn_ui(app.world_mut());
    app.update();
    let snapshot = tree_snapshot(app.world(), &Target::role(WidgetRole::Button, "Pause")).unwrap();
    assert_eq!(snapshot, "- button \"Pause\"");
    assert!(tree_snapshot(app.world(), &Target::text("Nothing")).is_err());
}

#[test]
fn pick_mode_offers_the_snapshot_of_the_node_last() {
    let app = toolbar_app();
    let panel = app
        .world()
        .iter_entities()
        .find(|entity| {
            entity
                .get::<Name>()
                .is_some_and(|name| name.as_str() == TOOLBAR)
        })
        .unwrap()
        .id();
    let checks = checks_that_hold(app.world(), panel, &toolbar(), &NoiseFilter::default());
    let snapshot = tree_snapshot(app.world(), &toolbar()).unwrap();
    assert_eq!(checks.last(), Some(&Check::tree(toolbar(), snapshot)));
    let picked = Picked::new(app.world(), panel, &NoiseFilter::default(), None);
    assert!(
        picked
            .checks()
            .iter()
            .any(|check| matches!(check, Check::Tree { .. })),
        "{:?}",
        picked.checks()
    );
}

/// Spawns a camera and a named list of three items, the second selected.
fn spawn_countries(world: &mut World) {
    world.spawn(Camera2d);
    world
        .spawn((Name::new(COUNTRIES), Node::default()))
        .with_children(|list| {
            list.spawn((ListItem, Node::default(), children![Text::new("Belgium")]));
            list.spawn((
                ListItem,
                Selected,
                Node::default(),
                children![Text::new("Canada")],
            ));
            list.spawn((ListItem, Node::default(), children![Text::new("Denmark")]));
        });
}

/// The name of the panel the list items sit in.
const COUNTRIES: &str = "Countries";

/// A headless app with the countries laid out.
fn countries_app() -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    spawn_countries(app.world_mut());
    for _ in 0..3 {
        app.update();
    }
    app
}

/// The outcome of the one check step of a scenario over the countries.
fn countries_outcome(check: Check) -> Outcome {
    Scenario::new("countries")
        .setup(spawn_countries)
        .steps([Step::check(check)])
        .run_headless()
        .steps[0]
        .outcome
        .clone()
}

#[test]
fn a_snapshot_shows_a_selected_list_item_as_selected() {
    let app = countries_app();
    let snapshot = tree_snapshot(app.world(), &Target::name(COUNTRIES)).unwrap();
    assert_eq!(
        snapshot,
        "- panel \"Countries\":\n  \
           - list-item \"Belgium\"\n  \
           - list-item \"Canada\" [selected]\n  \
           - list-item \"Denmark\""
    );
}

#[test]
fn an_expected_tree_can_demand_a_selected_state() {
    let selected = "- panel:\n  - list-item \"Canada\" [selected]\n";
    assert_eq!(
        countries_outcome(Check::tree(Target::name(COUNTRIES), selected)),
        Outcome::Pass
    );
    let wrong = "- panel:\n  - list-item \"Belgium\" [selected]\n";
    let Outcome::Fail(message) = countries_outcome(Check::tree(Target::name(COUNTRIES), wrong))
    else {
        panic!("an item that is not selected must fail");
    };
    assert!(
        message.contains("-   - list-item \"Belgium\" [selected]"),
        "{message}"
    );
    assert!(
        message.ends_with("+   - list-item \"Belgium\""),
        "{message}"
    );
}

#[test]
fn a_state_with_a_wrong_name_is_refused_listing_selected() {
    let ron = r#"(
        format_version: 3,
        name: "bad state",
        steps: [Check(Tree(of: Name("Countries"), matches: "- list-item [chosen]"))],
    )"#;
    let message = Scenario::from_ron(ron).err().unwrap().to_string();
    assert!(message.contains("selected"), "{message}");
}

#[test]
fn the_selected_check_holds_only_for_the_selected_item() {
    let canada = Target::text("Canada");
    assert_eq!(
        countries_outcome(Check::selected(canada.clone())),
        Outcome::Pass
    );
    assert_eq!(
        countries_outcome(!Check::selected(canada)),
        Outcome::Fail("Text(\"Canada\") is selected holds, but should not".into())
    );
    assert_eq!(
        countries_outcome(Check::selected(Target::text("Denmark"))),
        Outcome::Fail("Text(\"Denmark\") is not selected".into())
    );
    assert_eq!(
        Check::selected(Target::text("Canada")).to_string(),
        "Text(\"Canada\") is selected"
    );
}

#[test]
fn a_selected_check_round_trips_through_ron() {
    let scenario =
        Scenario::new("selected").steps([Step::check(!Check::selected(Target::text("Belgium")))]);
    let ron = scenario.to_ron().unwrap();
    assert!(ron.contains("Selected(Text(\"Belgium\"))"), "{ron}");
    let loaded = Scenario::from_ron_with(&ron, &registry()).unwrap();
    assert_eq!(loaded.to_script(), scenario.to_script());
    assert_eq!(loaded.to_ron().unwrap(), ron, "export is stable");
}

#[test]
fn pick_mode_offers_selected_for_what_can_be_selected() {
    let app = countries_app();
    let item = |label: &str| {
        let text = app
            .world()
            .iter_entities()
            .find(|entity| entity.get::<Text>().is_some_and(|text| text.0 == label))
            .unwrap()
            .id();
        let parent = app.world().get::<ChildOf>(text).unwrap().parent();
        (parent, Target::text(label))
    };
    let filter = NoiseFilter::default();
    let (canada, target) = item("Canada");
    let checks = checks_that_hold(app.world(), canada, &target, &filter);
    assert!(
        checks.contains(&Check::selected(target.clone())),
        "{checks:?}"
    );
    let (belgium, target) = item("Belgium");
    let checks = checks_that_hold(app.world(), belgium, &target, &filter);
    assert!(
        checks.contains(&!Check::selected(target.clone())),
        "{checks:?}"
    );
    assert!(!checks.contains(&Check::selected(target)), "{checks:?}");
}
