//! The app's own setups, views and targets: one reflected enum each, found
//! by Bevy's type registry with no registration call.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

mod common;

use bevy::prelude::*;
use common::{Activations, Arranged, UiCheck, UiSetup, UiTarget, UiView};
use rin_bevy_ui_tester::{
    AppSetup, AppTarget, AppView, Check, Extension, LoadError, Outcome, ReflectAppSetup,
    ReflectAppTarget, ReflectAppView, Registry, ResolvedTarget, Scenario, Step, Target,
};

/// A second setup type. It opts out of automatic registration, so only the
/// tests that register it by hand see two setup types.
#[derive(Reflect, Debug, Clone)]
#[reflect(AppSetup, no_auto_register)]
enum OtherSetup {
    /// Builds nothing.
    Nothing,
}

impl AppSetup for OtherSetup {
    fn build(&self, _world: &mut World) -> Result<(), String> {
        Ok(())
    }
}

/// A second view type, registered by hand only.
#[derive(Reflect, Debug, Clone)]
#[reflect(AppView, no_auto_register)]
enum OtherView {
    /// Arranges nothing.
    Nothing,
}

impl AppView for OtherView {
    fn arrange(&self, _world: &mut World) {}
}

/// A second target type, registered by hand only.
#[derive(Reflect, Debug, Clone)]
#[reflect(AppTarget, no_auto_register)]
enum OtherTarget {
    /// The top left corner.
    Corner,
}

impl AppTarget for OtherTarget {
    fn resolve(&self, _world: &World) -> Result<ResolvedTarget, String> {
        Ok(ResolvedTarget::at(Vec2::ZERO))
    }
}

/// A file using the app's setup, view and target, and checking through them.
const FILE: &str = "(
    format_version: 3,
    name: \"from a save\",
    setup: App(Activated(2)),
    view: App(Zoomed(2.0)),
    steps: [
        MoveTo(App(PauseButton)),
        Click(Left),
        Wait(1),
        Check(Exists(App(Showing(\"Pause\")))),
    ],
    expect: [App(Activated(3)), App(Found(App(Showing(\"Pause\"))))],
)";

#[test]
fn the_apps_setup_view_and_targets_in_a_file_work_with_no_registration_call() {
    // A default registry: nothing about the app's types is registered by hand.
    let scenario = Scenario::from_ron(FILE).unwrap();
    let report = scenario.run_headless();
    assert!(report.passed(), "{:?}", report.failures());

    // Headless runs arrange no view; the recorder and the gallery call this.
    let mut world = World::new();
    assert_eq!(scenario.run_view(&mut world), Ok(true));
    assert_eq!(
        world.get_resource::<Arranged>(),
        Some(&Arranged(UiView::Zoomed(2.0)))
    );
}

#[test]
fn the_apps_setup_view_and_targets_round_trip_through_ron() {
    let scenario = Scenario::from_ron(FILE).unwrap();
    let ron = scenario.to_ron().unwrap();
    for written in [
        "setup: App(Activated(2))",
        "view: App(Zoomed(2.0))",
        "MoveTo(App(PauseButton))",
        "Check(Exists(App(Showing(\"Pause\"))))",
        "App(Found(App(Showing(\"Pause\"))))",
    ] {
        assert!(ron.contains(written), "{written} in {ron}");
    }
    let again = Scenario::from_ron(&ron).unwrap();
    assert_eq!(again.to_script(), scenario.to_script());
    assert_eq!(again.to_ron().unwrap(), ron, "export is stable");

    let in_rust = Scenario::new("from a save")
        .app_setup(UiSetup::Activated(2))
        .app_view(UiView::Zoomed(2.0))
        .steps([
            Step::move_to(Target::app(UiTarget::PauseButton)),
            Step::click(),
            Step::wait(1),
            Step::check(Check::exists(Target::app(UiTarget::Showing(
                "Pause".into(),
            )))),
        ])
        .expect_check(Check::app(UiCheck::Activated(3)))
        .expect_check(Check::app(UiCheck::Found(Target::app(UiTarget::Showing(
            "Pause".into(),
        )))));
    assert_eq!(in_rust.to_script(), scenario.to_script());
}

#[test]
fn a_setup_that_fails_fails_the_scenario_without_playing_a_step() {
    let report = Scenario::new("no save")
        .app_setup(UiSetup::MissingSave("gone.ron".into()))
        .steps([Step::click(), Step::wait(1)])
        .expect_check(Check::shows("Pause"))
        .run_headless();
    assert!(!report.passed());
    assert!(report.finished);
    assert_eq!(
        report.failures(),
        ["setup: the setup `MissingSave(\"gone.ron\")` failed: there is no save `gone.ron`"]
    );
    assert!(
        report
            .steps
            .iter()
            .all(|record| record.outcome == Outcome::Skipped)
    );
    assert!(report.checks.is_empty(), "{:?}", report.checks);
}

#[test]
fn the_apps_setup_runs_before_a_setup_closure() {
    let report = Scenario::new("both")
        .app_setup(UiSetup::Activated(4))
        .setup(|world| world.resource_mut::<Activations>().0 += 1)
        .expect_check(Check::app(UiCheck::Activated(5)))
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_second_type_for_an_extension_point_is_a_load_error_naming_both() {
    let registry = Registry::default();
    {
        let mut types = registry.types.write();
        types.register::<OtherSetup>();
        types.register::<OtherView>();
        types.register::<OtherTarget>();
    }
    let load = |body: &str| {
        Scenario::from_ron_with(
            &format!("(format_version: 3, name: \"x\", {body})"),
            &registry,
        )
        .unwrap_err()
    };
    for (body, extension, used_by, ours, theirs) in [
        (
            "setup: App(PlainUi), steps: []",
            Extension::Setup,
            "the setup",
            "app_extensions::OtherSetup",
            "app_extensions::common::UiSetup",
        ),
        (
            "view: App(Plain), steps: []",
            Extension::View,
            "the view",
            "app_extensions::OtherView",
            "app_extensions::common::UiView",
        ),
        (
            "steps: [Wait(1), MoveTo(App(PauseButton))]",
            Extension::Target,
            "step 1",
            "app_extensions::OtherTarget",
            "app_extensions::common::UiTarget",
        ),
    ] {
        let error = load(body);
        assert_eq!(
            error,
            LoadError::SeveralAppTypes {
                extension,
                used_by: used_by.into(),
                types: vec![ours.into(), theirs.into()],
            }
        );
        let text = error.to_string();
        assert!(
            text.contains(&format!("`{ours}` and `{theirs}`"))
                && text.contains(&format!("#[reflect({})]", extension.trait_name())),
            "{text}"
        );
        let several = registry.app_type(extension).unwrap_err();
        assert_eq!(several.types.len(), 2);
    }
}

#[test]
fn the_registry_lists_the_variants_of_each_extension_points_type() {
    let registry = Registry::default();
    let forms = |extension| -> Vec<String> {
        registry
            .app_type(extension)
            .unwrap()
            .unwrap()
            .variants
            .iter()
            .map(rin_bevy_ui_tester::AppVariant::form)
            .collect()
    };
    assert_eq!(
        forms(Extension::Setup),
        ["PlainUi", "Activated(u32)", "MissingSave(String)"]
    );
    assert_eq!(forms(Extension::View), ["Plain", "Zoomed(f32)"]);
    assert_eq!(forms(Extension::Target), ["PauseButton", "Showing(String)"]);
    let setup = registry.app_type(Extension::Setup).unwrap().unwrap();
    assert_eq!(setup.type_path, "app_extensions::common::UiSetup");
}
