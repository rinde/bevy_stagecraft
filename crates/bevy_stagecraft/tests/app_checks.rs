//! The app's own checks: one reflected enum with `#[reflect(AppCheck)]`,
//! found by Bevy's type registry with no registration call.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

mod common;

use bevy::prelude::*;
use bevy_stagecraft::{
    AppCheck, AppField, Check, CheckResult, Extension, LoadError, Outcome, Probe, ReflectAppCheck,
    Registry, Scenario, Step, Target,
};
use common::{UiCheck, UiSetup, registry, spawn_ui};

/// A second app check type. It opts out of automatic registration, so only
/// the test that registers it by hand sees two app check types.
#[derive(Reflect, Debug, Clone)]
#[reflect(AppCheck, no_auto_register)]
enum OtherCheck {
    /// Always holds.
    Fine,
}

impl AppCheck for OtherCheck {
    fn run(&self, _probe: &Probe) -> CheckResult {
        Ok(())
    }
}

/// Loads a script of format version 3 with these steps.
fn load(steps: &str) -> Result<Scenario, LoadError> {
    Scenario::from_ron_with(
        &format!("(format_version: 3, name: \"x\", steps: [Wait(1), {steps}])"),
        &registry(),
    )
}

#[test]
fn an_app_check_in_a_file_is_found_with_no_registration_call_and_runs() {
    // A default registry: nothing about `UiCheck` is registered by hand.
    let scenario = Scenario::from_ron(
        "(format_version: 3, name: \"x\", steps: [\
            Check(App(Activated(0))),\
            Check(App(Activated(2))),\
            Check(Not(App(Found(Text(\"Resume\"))))),\
        ])",
    )
    .unwrap()
    .setup(spawn_ui);
    let report = scenario.run_headless();
    let outcomes: Vec<Outcome> = report
        .steps
        .iter()
        .map(|record| record.outcome.clone())
        .collect();
    assert_eq!(
        outcomes,
        [
            Outcome::Pass,
            Outcome::Fail("activated 0 times, not 2".into()),
            Outcome::Pass,
        ]
    );
    assert_eq!(report.checks[1].name, "Activated(2)");
}

#[test]
fn an_app_check_round_trips_through_ron_and_exports_stably() {
    let scenario = Scenario::new("round trip")
        .app_setup(UiSetup::PlainUi)
        .steps([
            Step::check(Check::app(UiCheck::Found(Target::text("Pause")))),
            Step::check(!Check::app(UiCheck::ActivatedOnce)),
        ])
        .expect_check(Check::app(UiCheck::ActivatedBetween {
            at_least: 0,
            at_most: 1,
        }));
    let ron = scenario.to_ron().unwrap();
    for written in [
        "Check(App(Found(Text(\"Pause\"))))",
        "Check(Not(App(ActivatedOnce)))",
        "App(ActivatedBetween(at_least:0,at_most:1))",
    ] {
        assert!(ron.contains(written), "{written} in {ron}");
    }
    let loaded = Scenario::from_ron_with(&ron, &registry()).unwrap();
    assert_eq!(loaded.to_script(), scenario.to_script());
    assert_eq!(loaded.to_ron().unwrap(), ron, "export is stable");
    let report = loaded.run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_file_value_is_read_as_the_app_type_and_written_back_in_its_own_form() {
    let scenario = load("Check(App( Activated( 3 ) ))").unwrap();
    assert_eq!(
        scenario.script()[1],
        Step::check(Check::app(UiCheck::Activated(3)))
    );
    assert!(
        scenario
            .to_ron()
            .unwrap()
            .contains("Check(App(Activated(3)))")
    );
}

#[test]
fn a_misspelt_variant_is_a_load_error_naming_the_step_and_listing_the_variants() {
    let error = load("Check(App(Actvated(1)))").unwrap_err();
    let LoadError::BadAppValue {
        extension,
        used_by,
        value,
        type_name,
        message,
        variants,
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(*extension, Extension::Check);
    assert_eq!(used_by, "step 1");
    assert_eq!(value, "Actvated(1)");
    assert_eq!(*type_name, "UiCheck");
    assert_eq!(
        message,
        "unknown variant `Actvated`, expected one of `ActivatedOnce`, `Activated`, `Found`, `ActivatedBetween`"
    );
    assert_eq!(
        variants,
        &[
            "ActivatedOnce",
            "Activated(u32)",
            "Found(Target)",
            "ActivatedBetween(at_least: u32, at_most: u32)"
        ]
    );
    let text = error.to_string();
    assert!(
        text.starts_with("step 1: `Actvated(1)` is not a `UiCheck`"),
        "{text}"
    );
    assert!(text.contains("`Activated(u32)`"), "{text}");
}

#[test]
fn a_wrong_argument_is_a_load_error_naming_the_step_and_listing_the_variants() {
    for (steps, step) in [
        ("Check(App(Activated(\"one\")))", "step 1"),
        (
            "Wait(1), Check(Within(5, App(Found(Txt(\"Pause\")))))",
            "step 2",
        ),
        ("Check(App(ActivatedBetween(at_least: 1)))", "step 1"),
    ] {
        let error = load(steps).unwrap_err();
        assert!(
            matches!(&error, LoadError::BadAppValue { used_by, variants, .. }
                if used_by == step && variants.len() == 4),
            "{steps}: {error:?}"
        );
    }
}

#[test]
fn a_second_app_check_type_is_a_load_error_naming_both() {
    let registry = Registry::default();
    registry.types.write().register::<OtherCheck>();
    let error = Scenario::from_ron_with(
        "(format_version: 3, name: \"x\", steps: [], expect: [App(Fine)])",
        &registry,
    )
    .unwrap_err();
    assert_eq!(
        error,
        LoadError::SeveralAppTypes {
            extension: Extension::Check,
            used_by: "the expect list".into(),
            types: vec![
                "app_checks::OtherCheck".into(),
                "app_checks::common::UiCheck".into()
            ],
        }
    );
    let text = error.to_string();
    assert!(
        text.contains("`app_checks::OtherCheck` and `app_checks::common::UiCheck`"),
        "{text}"
    );
    let several = registry.app_type(Extension::Check).unwrap_err();
    assert_eq!(several.types.len(), 2);
}

#[test]
fn an_app_check_with_no_app_check_type_is_a_load_error_saying_how_to_declare_one() {
    let registry = Registry {
        types: bevy::reflect::TypeRegistryArc::default(),
    };
    let error = Scenario::from_ron_with(
        "(format_version: 3, name: \"x\", steps: [Check(App(Fine))])",
        &registry,
    )
    .unwrap_err();
    assert_eq!(
        error,
        LoadError::NoAppType {
            extension: Extension::Check,
            used_by: "step 0".into()
        }
    );
    assert!(
        error.to_string().contains("#[reflect(AppCheck)]"),
        "{error}"
    );
    assert_eq!(registry.app_type(Extension::Check), Ok(None));
}

#[test]
fn the_registry_lists_the_app_check_variants_and_their_field_types() {
    let found = registry().app_type(Extension::Check).unwrap().unwrap();
    assert_eq!(found.type_path, "app_checks::common::UiCheck");
    assert_eq!(found.short_type_path, "UiCheck");
    let names: Vec<&str> = found.variants.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        ["ActivatedOnce", "Activated", "Found", "ActivatedBetween"]
    );
    assert_eq!(found.variants[0].fields.len(), 0);
    let found_target = &found.variants[2].fields[0];
    assert_eq!(found_target.name, None);
    assert_eq!(found_target.short_type_path, "Target");
    assert_eq!(found_target.type_path, "bevy_stagecraft::targets::Target");
    let between: Vec<&AppField> = found.variants[3].fields.iter().collect();
    assert_eq!(between[1].name.as_deref(), Some("at_most"));
    assert_eq!(between[1].short_type_path, "u32");
    assert!(found.variants[3].named_fields);
}
