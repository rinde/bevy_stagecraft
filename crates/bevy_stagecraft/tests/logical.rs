//! Reading physical reflected values as logical, for scale-factor variants.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    prelude::*,
    ui::{ComputedNode, UiGlobalTransform, px},
};
use bevy_stagecraft::{
    Check, Comparison, HeadlessPlugins, LoadError, Measure, Registry, Scenario, ScenarioReport,
    Step, Target, Unit, Value, Variant,
};

/// A label with text, so a logical check can name a non-number field.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
struct Tag {
    /// The text.
    text: String,
}

/// A headless app with a 100 by 50 panel at logical (20, 10), named "panel".
fn app() -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default())
        .register_type::<Tag>();
    app.finish();
    app.cleanup();
    app
}

/// A registry that knows every type the app registered.
fn registry() -> Registry {
    Registry::default().with_types_of(app().world())
}

/// Spawns the panel the checks read.
fn spawn(world: &mut World) {
    world.spawn(Camera2d);
    world.spawn((
        Name::new("panel"),
        Tag {
            text: "panel".into(),
        },
        Node {
            position_type: PositionType::Absolute,
            left: px(20),
            top: px(10),
            width: px(100),
            height: px(50),
            ..Node::default()
        },
    ));
}

/// The panel's logical size and position, in logical pixels.
fn logical_scenario() -> Scenario {
    let panel = || Target::name("panel");
    Scenario::new("logical")
        .setup(spawn)
        .window_size(1280, 720)
        .steps([
            Step::wait(2),
            Step::check(
                Check::component::<ComputedNode>(panel(), "size.x")
                    .logical()
                    .eq(100.0_f32),
            ),
            Step::check(
                Check::component::<ComputedNode>(panel(), "size.y")
                    .logical()
                    .eq(50.0_f32),
            ),
            Step::check(
                Check::component::<ComputedNode>(panel(), "size.x")
                    .logical()
                    .between(99.0_f32, 101.0_f32),
            ),
            Step::check(
                Check::component::<UiGlobalTransform>(panel(), ".0.translation.x")
                    .logical()
                    .eq(70.0_f32),
            ),
            Step::check(
                Check::component::<UiGlobalTransform>(panel(), ".0.translation.y")
                    .logical()
                    .eq(35.0_f32),
            ),
        ])
}

/// The default variant and the same window at scale factor 2.
fn at_both_scales() -> [Variant; 2] {
    [
        Variant::default(),
        Variant::new("retina")
            .window_size(1280, 720)
            .scale_factor(2.0),
    ]
}

/// Plays `scenario` in `variant`.
fn play(scenario: &Scenario, variant: &Variant) -> ScenarioReport {
    variant
        .apply(scenario)
        .expect("the variant applies")
        .run_headless()
}

#[test]
fn a_logical_component_check_passes_at_scale_factors_one_and_two() {
    let scenario = logical_scenario();
    for variant in at_both_scales() {
        let report = play(&scenario, &variant);
        assert!(report.passed(), "in {variant}: {:?}", report.failures());
    }
}

#[test]
fn the_same_size_read_unscaled_fails_at_scale_factor_two() {
    let scenario = Scenario::new("physical")
        .setup(spawn)
        .window_size(1280, 720)
        .steps([
            Step::wait(2),
            Step::check(
                Check::component::<ComputedNode>(Target::name("panel"), "size.x").eq(100.0_f32),
            ),
        ]);
    let [plain, retina] = at_both_scales();
    assert!(play(&scenario, &plain).passed(), "it holds at scale 1");
    let report = play(&scenario, &retina);
    assert!(!report.passed(), "it does not hold at scale 2");
    let message = report.failures().join(" ");
    assert!(message.contains("is 200"), "{message}");
}

#[test]
fn a_captured_logical_size_works_in_a_later_check() {
    let panel = || Target::name("panel");
    let scenario = Scenario::new("capture")
        .setup(spawn)
        .window_size(1280, 720)
        .steps([
            Step::wait(2),
            Step::capture(
                "width",
                Measure::component::<ComputedNode>(panel(), "size.x").logical(),
            ),
            Step::check(
                Check::component::<ComputedNode>(panel(), "size.x")
                    .logical()
                    .is(Comparison::Eq(Value::captured("width"))),
            ),
            Step::check(
                Check::component::<ComputedNode>(panel(), "size.x")
                    .logical()
                    .is(Comparison::Eq(Value::captured("width").plus(0.0))),
            ),
        ]);
    for variant in at_both_scales() {
        let report = play(&scenario, &variant);
        assert!(report.passed(), "in {variant}: {:?}", report.failures());
    }
}

#[test]
fn a_logical_check_and_capture_round_trip_through_ron() {
    let panel = || Target::name("panel");
    let scenario = Scenario::new("ron").steps([
        Step::capture(
            "width",
            Measure::component::<ComputedNode>(panel(), "size.x").logical(),
        ),
        Step::check(
            Check::component::<ComputedNode>(panel(), "size.x")
                .logical()
                .is(Comparison::Eq(Value::captured("width"))),
        ),
    ]);
    let ron = scenario.to_ron().unwrap();
    assert!(ron.contains("unit: Logical"), "{ron}");
    assert!(!ron.contains("unit: Physical"), "{ron}");
    let loaded = Scenario::from_ron_with(&ron, &registry()).unwrap();
    assert_eq!(loaded.to_script(), scenario.to_script());
    assert_eq!(loaded.to_ron().unwrap(), ron, "export is stable");
}

#[test]
fn a_logical_check_on_a_non_number_is_a_load_error() {
    let text = "(format_version: 3, name: \"bad\", steps: [\
        Check(Component(target: Name(\"panel\"), path: \"Tag.text\", unit: Logical, is: Eq(\"x\")))\
    ])";
    let error = Scenario::from_ron_with(text, &registry()).unwrap_err();
    assert!(
        matches!(&error, LoadError::BadCheck { used_by, .. } if used_by == "step 0"),
        "{error}"
    );
    assert!(error.to_string().contains("not a number"), "{error}");
}

#[test]
fn a_logical_check_on_a_count_measure_does_nothing() {
    // A count has no unit; `logical()` leaves the measure as it is.
    let measure = Measure::count::<Tag>();
    assert_eq!(measure.clone().logical(), measure);
}

#[test]
fn the_unit_is_physical_by_default_in_rust_and_in_a_file() {
    assert_eq!(Unit::default(), Unit::Physical);
    let check = Check::component::<ComputedNode>(Target::name("panel"), "size.x").eq(100.0_f32);
    assert!(
        matches!(
            &check,
            Check::Component {
                unit: Unit::Physical,
                ..
            }
        ),
        "{check:?}"
    );
    let ron = Scenario::new("plain")
        .steps([Step::check(check)])
        .to_ron()
        .unwrap();
    assert!(!ron.contains("unit:"), "{ron}");
}
