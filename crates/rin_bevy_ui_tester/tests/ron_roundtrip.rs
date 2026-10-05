//! Scenarios written in Rust and as RON are one value.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

mod common;

use bevy::{input::keyboard::KeyCode, math::Vec2};
use common::{Activations, UiCheck, UiSetup, UiTarget, UiView, registry};
use rin_bevy_ui_tester::{
    Check, Extension, FORMAT_VERSION, LoadError, Modifiers, Scenario, Script, Setup, Step, Target,
    View, files::run_ron_files,
};

/// A scenario written in Rust, with the app's own setup, view, targets and
/// checks as values.
fn in_rust() -> Scenario {
    Scenario::new("click pause")
        .window_size(800, 600)
        .app_setup(UiSetup::PlainUi)
        .app_view(UiView::Zoomed(1.5))
        .steps([
            Step::glide(Target::text("Pause"), 20),
            Step::click(),
            Step::wait(1),
            Step::check(Check::app(UiCheck::ActivatedOnce)),
            Step::tap_with(KeyCode::KeyP, Modifiers::CTRL | Modifiers::SHIFT),
            Step::drag(
                Target::screen(Vec2::new(1.0, 2.0)),
                Target::app(UiTarget::Showing("Pause".into())),
                5,
            ),
            Step::caption("done"),
        ])
        .expect_check(Check::app(UiCheck::ActivatedOnce))
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

/// The same scenario as [`in_rust`] without the drag step, played straight
/// from Rust with no registry.
fn in_rust_playable() -> Scenario {
    Scenario::new("click pause")
        .window_size(800, 600)
        .app_setup(UiSetup::PlainUi)
        .steps([
            Step::glide(Target::text("Pause"), 20),
            Step::click(),
            Step::wait(1),
            Step::check(Check::app(UiCheck::ActivatedOnce)),
            Step::tap_with(KeyCode::KeyP, Modifiers::CTRL | Modifiers::SHIFT),
            Step::caption("done"),
        ])
        .expect_check(Check::app(UiCheck::ActivatedOnce))
}

#[test]
fn both_spellings_play_identically() {
    // Played directly, as written in Rust.
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
fn the_apps_setup_view_and_targets_are_written_inside_app() {
    let ron = in_rust().to_ron().unwrap();
    for written in [
        "setup: App(PlainUi)",
        "view: App(Zoomed(1.5))",
        "to: App(Showing(\"Pause\"))",
    ] {
        assert!(ron.contains(written), "{written} in {ron}");
    }
}

/// Loads a script of format version 3 with this body.
fn load(body: &str) -> Result<Scenario, LoadError> {
    Scenario::from_ron_with(
        &format!("(format_version: 3, name: \"x\", {body})"),
        &registry(),
    )
}

#[test]
fn a_misspelt_setup_view_or_target_is_a_load_error_naming_where_and_listing_the_variants() {
    for (body, extension, used_by, variants) in [
        (
            "setup: App(PlainUj), steps: []",
            Extension::Setup,
            "the setup",
            vec!["PlainUi", "Activated(u32)", "MissingSave(String)"],
        ),
        (
            "view: App(Zoomed(\"far\")), steps: []",
            Extension::View,
            "the view",
            vec!["Plain", "Zoomed(f32)"],
        ),
        (
            "steps: [Wait(1), MoveTo(App(PauseButon))]",
            Extension::Target,
            "step 1",
            vec!["PauseButton", "Showing(String)"],
        ),
        (
            "steps: [Drag(from: Text(\"a\"), to: App(Showing(3)), frames: 2)]",
            Extension::Target,
            "step 0",
            vec!["PauseButton", "Showing(String)"],
        ),
        (
            "steps: [Check(Hovered(App(Showin(\"Pause\"))))]",
            Extension::Target,
            "step 0",
            vec!["PauseButton", "Showing(String)"],
        ),
        (
            "steps: [], expect: [App(Found(App(PauseButon)))]",
            Extension::Target,
            "the expect list",
            vec!["PauseButton", "Showing(String)"],
        ),
    ] {
        let error = load(body).unwrap_err();
        let LoadError::BadAppValue {
            extension: found,
            used_by: at,
            variants: listed,
            ..
        } = &error
        else {
            panic!("{body}: {error:?}");
        };
        assert_eq!((*found, at.as_str()), (extension, used_by), "{body}");
        assert_eq!(listed, &variants, "{body}");
        let text = error.to_string();
        assert!(text.starts_with(&format!("{used_by}: `")), "{text}");
        assert!(text.contains("is one of `"), "{text}");
    }
}

#[test]
fn the_old_named_setups_views_and_custom_targets_are_refused_saying_what_to_write_instead() {
    for (body, hint) in [
        ("setup: Named(\"plain_ui\"), steps: []", "`setup: App("),
        ("setup: Some(File(\"a.ron\")), steps: []", "`setup: App("),
        ("view: \"top_down\", steps: []", "`view: App("),
        ("view: Some(\"top_down\"), steps: []", "`view: App("),
        (
            "steps: [MoveTo(Custom(resolver: \"nope\", argument: \"\"))]",
            "`MoveTo(App(",
        ),
        (
            "steps: [Check(Exists(Custom(resolver: \"nope\", argument: \"\")))]",
            "`MoveTo(App(",
        ),
    ] {
        let error = load(body).unwrap_err();
        let message = error.to_string();
        assert!(matches!(error, LoadError::Syntax(_)), "{body}: {message}");
        assert!(
            message.contains("no longer looked up") && message.contains(hint),
            "{body}: {message}"
        );
    }
}

#[test]
fn an_app_value_with_no_type_for_it_is_a_load_error_saying_how_to_declare_one() {
    let registry = rin_bevy_ui_tester::Registry {
        types: bevy::reflect::TypeRegistryArc::default(),
    };
    let error = Scenario::from_ron_with(
        "(format_version: 3, name: \"x\", setup: App(PlainUi), steps: [])",
        &registry,
    )
    .unwrap_err();
    assert_eq!(
        error,
        LoadError::NoAppType {
            extension: Extension::Setup,
            used_by: "the setup".into()
        }
    );
    assert!(
        error.to_string().contains("#[reflect(AppSetup)]"),
        "{error}"
    );
}

#[test]
fn a_newer_format_is_refused_even_if_the_rest_is_unreadable() {
    let error = Scenario::from_ron("(format_version: 4, name: \"x\", brand_new: 7)").unwrap_err();
    assert_eq!(
        error,
        LoadError::UnsupportedVersion {
            found: 4,
            supported: 3
        }
    );
    assert!(error.to_string().contains("versions 1 to 3"), "{error}");
}

#[test]
fn typos_are_syntax_errors_and_the_apps_setup_and_view_load_as_its_values() {
    let error = Scenario::from_ron("(format_version: 1, name: \"x\", stpes: [])").unwrap_err();
    assert!(matches!(error, LoadError::Syntax(_)), "{error}");
    let scenario = Scenario::from_ron(
        "(format_version: 1, name: \"x\", setup: Some(App( Activated( 2 ) )), view: App(Plain), steps: [])",
    )
    .unwrap();
    assert_eq!(
        scenario.setup_value(),
        Some(&Setup::app(UiSetup::Activated(2)))
    );
    assert_eq!(scenario.view_value(), Some(&View::app(UiView::Plain)));
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
    assert_eq!(
        names,
        [
            "app_checks",
            "app_target",
            "click_pause",
            "reflected_checks",
            "typed_checks"
        ]
    );
    assert!(
        results.iter().all(rin_bevy_ui_tester::FileResult::passed),
        "{results:?}"
    );
}
