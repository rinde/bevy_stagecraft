//! Scenarios written in Rust and as RON are one value.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests may panic on a broken setup"
)]

mod common;

use bevy::{input::keyboard::KeyCode, math::Vec2};
use common::{Activations, registry, spawn_ui};
use rin_bevy_ui_tester::{
    FORMAT_VERSION, LoadError, Modifiers, Scenario, Script, Setup, Step, Target,
    files::run_ron_files,
};

/// A scenario written in Rust, with names where RON needs them.
fn in_rust() -> Scenario {
    Scenario::new("click pause")
        .window_size(800, 600)
        .setup_named("plain_ui", spawn_ui)
        .steps([
            Step::glide(Target::text("Pause"), 20),
            Step::click(),
            Step::wait(1),
            Step::check("activated once"),
            Step::tap_with(KeyCode::KeyP, Modifiers::CTRL | Modifiers::SHIFT),
            Step::drag(
                Target::screen(Vec2::new(1.0, 2.0)),
                Target::custom("pause_button", "x"),
                5,
            ),
            Step::caption("done"),
        ])
        .check("activated once", |probe| {
            probe
                .resource::<Activations>()
                .is_some_and(|count| count.0 == 1)
        })
        .expect("activated once", |probe| {
            probe
                .resource::<Activations>()
                .is_some_and(|count| count.0 == 1)
        })
}

#[test]
fn rust_and_ron_are_the_same_value() {
    let ron = in_rust().to_ron().unwrap();
    let loaded = Scenario::from_ron_with(&ron, &registry()).unwrap();
    assert_eq!(loaded.to_script(), in_rust().to_script());
    assert_eq!(loaded.to_ron().unwrap(), ron, "export is stable");
}

#[test]
fn modifiers_read_as_a_list_in_ron() {
    let ron = in_rust().to_ron().unwrap();
    assert!(
        ron.contains("modifiers: [") && ron.contains("Shift,") && ron.contains("Ctrl,"),
        "{ron}"
    );
    assert!(!ron.contains("bits"), "{ron}");
    assert!(!ron.contains("Some("), "{ron}");
}

/// The same scenario as [`in_rust`] without the drag step, written out in
/// Rust with its own inline closures and no registry.
fn in_rust_playable() -> Scenario {
    Scenario::new("click pause")
        .window_size(800, 600)
        .setup_named("plain_ui", spawn_ui)
        .steps([
            Step::glide(Target::text("Pause"), 20),
            Step::click(),
            Step::wait(1),
            Step::check("activated once"),
            Step::tap_with(KeyCode::KeyP, Modifiers::CTRL | Modifiers::SHIFT),
            Step::caption("done"),
        ])
        .check("activated once", |probe| {
            probe
                .resource::<Activations>()
                .is_some_and(|count| count.0 == 1)
        })
        .expect("activated once", |probe| {
            probe
                .resource::<Activations>()
                .is_some_and(|count| count.0 == 1)
        })
}

#[test]
fn both_spellings_play_identically() {
    // Played directly, with the closures written inline.
    let from_rust = in_rust_playable().run_headless();
    // Exported to RON and loaded back through the registry.
    let ron = in_rust_playable().to_ron().unwrap();
    let from_file = Scenario::from_ron_with(&ron, &registry())
        .unwrap()
        .run_headless();
    assert!(from_rust.passed(), "{:?}", from_rust.failures());
    assert_eq!(from_rust, from_file);
}

#[test]
fn mixed_use_adds_rust_assertions_to_a_recorded_file() {
    let text = include_str!("ron/click_pause.ron");
    let scenario = Scenario::from_ron_with(text, &registry())
        .unwrap()
        .expect("also once", |probe| {
            probe.resource::<Activations>().is_some_and(|c| c.0 == 1)
        });
    assert_eq!(scenario.expects().len(), 2);
    let report = scenario.run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn unknown_names_are_load_errors() {
    let registry = registry();
    let load = |body: &str| {
        Scenario::from_ron_with(
            &format!("(format_version: 1, name: \"x\", {body})"),
            &registry,
        )
        .unwrap_err()
    };
    assert!(matches!(
        load("steps: [Check(\"nope\")]"),
        LoadError::UnknownCheck { ref name, .. } if name == "nope"
    ));
    assert!(matches!(
        load("steps: [], expect: [\"nope\"]"),
        LoadError::UnknownCheck { .. }
    ));
    assert!(matches!(
        load("steps: [], setup: Some(Named(\"nope\"))"),
        LoadError::UnknownSetup(_)
    ));
    assert!(matches!(
        load("steps: [], view: Some(\"nope\")"),
        LoadError::UnknownView(_)
    ));
    assert!(matches!(
        load("steps: [Click(Left), MoveTo(Custom(resolver: \"nope\", argument: \"\"))]"),
        LoadError::UnknownTarget { ref used_by, .. } if used_by == "step 1"
    ));
    let message = load("steps: [Check(\"nope\")]").to_string();
    assert!(
        message.contains("step 0") && message.contains("nope"),
        "{message}"
    );
}

#[test]
fn a_newer_format_is_refused_even_if_the_rest_is_unreadable() {
    let error = Scenario::from_ron("(format_version: 3, name: \"x\", brand_new: 7)").unwrap_err();
    assert_eq!(
        error,
        LoadError::UnsupportedVersion {
            found: 3,
            supported: 2
        }
    );
    assert!(error.to_string().contains("versions 1 to 2"), "{error}");
}

#[test]
fn typos_are_syntax_errors_and_setup_files_pass_through() {
    let error = Scenario::from_ron("(format_version: 1, name: \"x\", stpes: [])").unwrap_err();
    assert!(matches!(error, LoadError::Syntax(_)), "{error}");
    let scenario = Scenario::from_ron(
        "(format_version: 1, name: \"x\", setup: Some(File(\"a.ron\")), steps: [])",
    )
    .unwrap();
    assert_eq!(
        scenario.setup_reference(),
        Some(&Setup::File("a.ron".into()))
    );
    let script: Script = scenario.to_script();
    assert_eq!(script.format_version, FORMAT_VERSION);
}

#[test]
fn a_directory_of_files_becomes_results_named_after_the_files() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ron");
    let registry = registry();
    let results = run_ron_files(dir, &registry, "", |scenario| {
        let mut app = scenario.headless_app();
        registry.install(app.world_mut());
        scenario.run_in(&mut app)
    })
    .unwrap();
    let names: Vec<_> = results.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["click_pause", "custom_target", "typed_checks"]);
    assert!(
        results.iter().all(rin_bevy_ui_tester::FileResult::passed),
        "{results:?}"
    );
}
