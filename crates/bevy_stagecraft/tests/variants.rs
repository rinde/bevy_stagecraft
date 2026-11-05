//! Scenarios run once per variant: window size, scale factor, theme.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    prelude::*,
    ui::{percent, px},
    ui_widgets::{Activate, Button},
    window::PrimaryWindow,
};
use bevy_stagecraft::{
    AppSetup, Check, FileOutcome, HeadlessPlugins, LoadError, NoiseFilter, ReflectAppSetup,
    Scenario, ScenarioReport, Step, Target, Variant, run_ron_variants,
};

/// How many times the button was activated.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Activations(u32);

/// The theme the UI was built in.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
struct Theme(String);

/// The width of the panel in front of the button, in logical pixels.
const PANEL_WIDTH: i32 = 700;

/// The test UI: a row that does not wrap, a wide panel and then the "Pause"
/// button, so the button ends at 800 px and lies outside any narrower window.
#[derive(Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(AppSetup)]
enum RowUi {
    /// The light theme.
    Light,
    /// The dark theme.
    Dark,
}

impl AppSetup for RowUi {
    fn build(&self, world: &mut World) -> Result<(), String> {
        let (theme, background) = match self {
            Self::Light => ("light", Color::WHITE),
            Self::Dark => ("dark", Color::BLACK),
        };
        world.init_resource::<Activations>();
        world.insert_resource(Theme(theme.to_owned()));
        world.spawn(Camera2d);
        let root = world
            .spawn(Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::NoWrap,
                ..Node::default()
            })
            .id();
        let panel = world
            .spawn((
                Node {
                    width: px(PANEL_WIDTH),
                    height: px(100),
                    flex_shrink: 0.0,
                    ..Node::default()
                },
                BackgroundColor(background),
                ChildOf(root),
            ))
            .id();
        let _ = panel;
        world
            .spawn((
                Button,
                Node {
                    width: px(100),
                    height: px(50),
                    flex_shrink: 0.0,
                    ..Node::default()
                },
                BackgroundColor(Color::WHITE),
                children![Text::new("Pause")],
                ChildOf(root),
            ))
            .observe(|_: On<Activate>, mut count: ResMut<Activations>| count.0 += 1);
        Ok(())
    }
}

/// Presses the "Pause" button and expects one activation.
fn press_pause() -> Scenario {
    Scenario::new("wrap")
        .app_setup(RowUi::Light)
        .action_timeout(5)
        .step(Step::move_to(Target::text("Pause")))
        .step(Step::click())
        .expect_check(Check::resource::<Activations>(".0").eq(1_u32))
}

/// The two window sizes of the acceptance case.
fn wide_and_narrow() -> [Variant; 2] {
    [
        Variant::new("wide").window_size(1280, 720),
        Variant::new("narrow").window_size(640, 480),
    ]
}

/// Plays `scenario` in `variant`, or the error that it cannot run there.
fn play(scenario: &Scenario, variant: &Variant) -> Result<ScenarioReport, LoadError> {
    variant.apply(scenario).map(|varied| varied.run_headless())
}

#[test]
fn a_row_that_overflows_at_800_px_passes_wide_and_fails_narrow() {
    let [wide, narrow] = wide_and_narrow();
    let scenario = press_pause();
    let at_wide = play(&scenario, &wide).expect("the wide variant runs");
    let at_narrow = play(&scenario, &narrow).expect("the narrow variant runs");
    assert!(at_wide.passed(), "{at_wide:#?}");
    assert!(!at_narrow.passed(), "{at_narrow:#?}");
}

#[test]
fn each_variant_of_a_scenario_has_its_own_name() {
    let [wide, narrow] = wide_and_narrow();
    let scenario = press_pause();
    let at_wide = play(&scenario, &wide).expect("the wide variant runs");
    let at_narrow = play(&scenario, &narrow).expect("the narrow variant runs");
    assert_eq!(at_wide.name, "wrap [wide]");
    assert_eq!(at_narrow.name, "wrap [narrow]");
}

#[test]
fn the_default_variant_runs_the_scenario_as_written() {
    let scenario = press_pause();
    let report = play(&scenario, &Variant::default()).expect("the default variant runs");
    assert_eq!(report.name, "wrap");
    assert!(report.passed(), "{report:#?}");
}

#[test]
fn a_variant_leaves_the_scenario_it_was_applied_to_alone() {
    let scenario = press_pause();
    let _ = Variant::new("tiny")
        .window_size(10, 10)
        .scale_factor(3.0)
        .apply(&scenario)
        .expect("applies");
    assert_eq!(scenario.name(), "wrap");
    assert_eq!(scenario.window(), None);
    assert_eq!(scenario.scale(), None);
}

#[test]
fn a_variant_window_size_replaces_the_scenarios_own() {
    let scenario = press_pause().window_size(1280, 720);
    let varied = Variant::new("narrow")
        .window_size(640, 480)
        .apply(&scenario)
        .expect("applies");
    assert_eq!(varied.window(), Some((640, 480)));
    let kept = Variant::new("themed")
        .app_setup(RowUi::Dark)
        .apply(&scenario)
        .expect("applies");
    assert_eq!(kept.window(), Some((1280, 720)));
}

#[test]
fn a_variant_scale_factor_reaches_the_window_and_the_layout() {
    let scenario = press_pause();
    let variant = Variant::new("retina")
        .window_size(1280, 720)
        .scale_factor(2.0);
    let varied = variant.apply(&scenario).expect("applies");
    let mut app = varied.headless_app();
    let report = varied.run_in(&mut app);
    assert!(report.passed(), "{report:#?}");
    let world = app.world_mut();
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .expect("a primary window");
    assert!((window.resolution.scale_factor() - 2.0).abs() < f32::EPSILON);
    assert!((window.resolution.width() - 1280.0).abs() < 0.5);
    assert!((window.resolution.height() - 720.0).abs() < 0.5);
    assert_eq!(window.resolution.physical_width(), 2560);
    assert_eq!(window.resolution.physical_height(), 1440);
}

#[test]
fn a_scale_factor_does_not_change_the_logical_size_the_layout_wraps_at() {
    let scenario = press_pause();
    let doubled = Variant::new("narrow-retina")
        .window_size(640, 480)
        .scale_factor(2.0);
    let report = play(&scenario, &doubled).expect("applies");
    assert!(
        !report.passed(),
        "640 logical pixels are narrow at any scale: {report:#?}"
    );
}

#[test]
fn a_scale_factor_that_is_not_above_zero_is_refused() {
    let scenario = press_pause();
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let error = Variant::new("bad")
            .scale_factor(bad)
            .apply(&scenario)
            .expect_err("refused");
        assert!(matches!(error, LoadError::Invalid { .. }), "{error}");
    }
}

#[test]
fn a_variant_app_setup_replaces_the_scenarios_own() {
    let scenario = press_pause().expect_check(Check::resource::<Theme>(".0").eq("dark".to_owned()));
    let light = play(&scenario, &Variant::default()).expect("runs");
    assert!(!light.passed(), "the scenario is built light: {light:#?}");
    let dark = Variant::new("dark").app_setup(RowUi::Dark);
    let report = play(&scenario, &dark).expect("runs");
    assert!(report.passed(), "{report:#?}");
    assert_eq!(report.name, "wrap [dark]");
}

#[test]
fn a_step_aimed_at_a_screen_position_is_refused_naming_it_in_other_variants() {
    let scenario = Scenario::new("fixed")
        .app_setup(RowUi::Light)
        .step(Step::move_to(Target::text("Pause")))
        .step(Step::move_to(Target::screen(Vec2::new(150.0, 25.0))))
        .step(Step::click());
    let narrow = Variant::new("narrow").window_size(640, 480);
    let error = narrow.apply(&scenario).expect_err("refused");
    let text = error.to_string();
    assert!(text.contains("step 1"), "{text}");
    assert!(text.contains("fixed [narrow]"), "{text}");
    assert!(text.contains("Screen"), "{text}");
}

#[test]
fn a_screen_position_inside_a_timeout_or_a_locator_is_found_too() {
    let scenario = Scenario::new("fixed")
        .app_setup(RowUi::Light)
        .step(Step::click())
        .step(Step::move_to(Target::screen(Vec2::new(1.0, 1.0))).with_timeout(3))
        .step(Step::glide(Target::screen(Vec2::new(2.0, 2.0)), 4).forced());
    let error = Variant::new("narrow")
        .window_size(640, 480)
        .apply(&scenario)
        .expect_err("refused");
    assert!(error.to_string().contains("steps 1, 2"), "{error}");
}

#[test]
fn a_screen_position_inside_a_group_is_refused_naming_the_group_path() {
    let scenario = Scenario::new("fixed")
        .app_setup(RowUi::Light)
        .step(Step::group(
            "raise the bridge",
            [
                Step::wait(1),
                Step::move_to(Target::screen(Vec2::new(150.0, 25.0))),
            ],
        ));
    let narrow = Variant::new("narrow").window_size(640, 480);
    let error = narrow.apply(&scenario).expect_err("refused");
    let text = error.to_string();
    assert!(text.contains("raise the bridge > step 1"), "{text}");
    assert!(text.contains("fixed [narrow]"), "{text}");
}

#[test]
fn a_screen_position_in_a_group_inside_a_group_names_the_whole_path() {
    let scenario = Scenario::new("fixed")
        .app_setup(RowUi::Light)
        .step(Step::group(
            "open the door",
            [
                Step::click(),
                Step::group(
                    "pull the lever",
                    [
                        Step::move_to(Target::screen(Vec2::new(1.0, 1.0))).with_timeout(3),
                        Step::move_to(Target::text("Pause")),
                    ],
                ),
            ],
        ))
        .step(Step::group(
            "leave",
            [Step::glide(Target::screen(Vec2::new(2.0, 2.0)), 4)],
        ));
    let error = Variant::new("narrow")
        .window_size(640, 480)
        .apply(&scenario)
        .expect_err("refused");
    let text = error.to_string();
    assert!(
        text.contains("open the door > pull the lever > 1"),
        "the nested one is found inside its timeout: {text}"
    );
    assert!(
        text.contains("leave > 3"),
        "so is the one in another group: {text}"
    );
}

#[test]
fn a_group_without_a_screen_position_is_fine_in_another_variant() {
    let scenario = Scenario::new("grouped")
        .app_setup(RowUi::Light)
        .step(Step::group(
            "press it",
            [Step::move_to(Target::text("Pause")), Step::click()],
        ));
    assert!(
        Variant::new("dark")
            .app_setup(RowUi::Dark)
            .apply(&scenario)
            .is_ok()
    );
}

#[test]
fn a_screen_position_in_a_scale_factor_only_variant_runs() {
    // The position is logical, so a scale factor leaves it just as valid as
    // at 1.0: only a window-size change makes it ambiguous.
    let scenario = Scenario::new("fixed")
        .app_setup(RowUi::Light)
        // The middle of the "Pause" button, which is logical at any scale.
        .step(Step::move_to(Target::screen(Vec2::new(750.0, 25.0))))
        .step(Step::click())
        .expect_check(Check::resource::<Activations>(".0").eq(1_u32));
    let retina = Variant::new("retina").scale_factor(2.0);
    assert!(
        retina.apply(&scenario).is_ok(),
        "the scale-only variant runs"
    );
    let report = play(&scenario, &retina).expect("runs");
    assert!(report.passed(), "{report:#?}");
    assert_eq!(report.name, "fixed [retina]");
}

#[test]
fn a_screen_position_in_a_setup_only_variant_runs() {
    // A theme changes how things look, not where logical positions are.
    let scenario = Scenario::new("fixed")
        .app_setup(RowUi::Light)
        .step(Step::move_to(Target::screen(Vec2::new(750.0, 25.0))))
        .step(Step::click())
        .expect_check(Check::resource::<Activations>(".0").eq(1_u32));
    let dark = Variant::new("dark").app_setup(RowUi::Dark);
    assert!(dark.apply(&scenario).is_ok(), "the setup-only variant runs");
    let report = play(&scenario, &dark).expect("runs");
    assert!(report.passed(), "{report:#?}");
}

#[test]
fn a_scenario_restricted_to_the_default_variant_is_skipped_in_the_others() {
    let scenario = Scenario::new("fixed")
        .app_setup(RowUi::Light)
        .variants(["default"])
        .step(Step::move_to(Target::screen(Vec2::new(150.0, 25.0))));
    assert!(!Variant::default().skips(&scenario));
    assert!(
        Variant::new("narrow")
            .window_size(640, 480)
            .skips(&scenario)
    );
    assert!(Variant::new("retina").scale_factor(2.0).skips(&scenario));
    assert_eq!(scenario.variant_names(), ["default".to_owned()]);
}

#[test]
fn a_scenario_that_names_no_variants_runs_in_every_one() {
    let scenario = Scenario::new("plain").step(Step::wait(1));
    assert!(!Variant::default().skips(&scenario));
    assert!(scenario.variant_names().is_empty());
}

#[test]
fn a_screen_position_is_fine_in_the_default_variant() {
    let scenario = Scenario::new("fixed")
        .app_setup(RowUi::Light)
        .step(Step::move_to(Target::screen(Vec2::new(150.0, 25.0))));
    assert!(Variant::default().apply(&scenario).is_ok());
}

#[test]
fn a_move_burst_is_refused_in_other_variants_and_fine_in_the_default_one() {
    let scenario = Scenario::new("burst")
        .app_setup(RowUi::Light)
        .step(Step::move_burst([Vec2::new(1.0, 1.0)]));
    let error = Variant::new("narrow")
        .window_size(640, 480)
        .apply(&scenario)
        .expect_err("refused");
    assert!(error.to_string().contains("MoveBurst"), "{error}");
    assert!(Variant::default().apply(&scenario).is_ok());
}

#[test]
fn run_ron_variants_names_each_file_and_variant_and_filters_by_that_name() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("variants_ron");
    std::fs::create_dir_all(&dir).expect("a directory");
    let scenario = Scenario::new("wrap")
        .step(Step::wait(1))
        .to_ron()
        .expect("exports");
    std::fs::write(dir.join("wrap.ron"), scenario).expect("writes");
    let registry = bevy_stagecraft::Registry::default();
    let variants = [
        Variant::default(),
        wide_and_narrow()[0].clone(),
        wide_and_narrow()[1].clone(),
    ];
    let all = run_ron_variants(&dir, &registry, "", &variants, Scenario::run_headless)
        .expect("reads the directory");
    let names: Vec<&str> = all.iter().map(|result| result.name.as_str()).collect();
    assert_eq!(names, ["wrap", "wrap [wide]", "wrap [narrow]"]);
    let narrow = run_ron_variants(&dir, &registry, "narrow", &variants, Scenario::run_headless)
        .expect("reads the directory");
    assert_eq!(narrow.len(), 1);
    assert_eq!(narrow[0].variant, "narrow");
    assert!(
        all.iter()
            .all(|result| matches!(result.outcome, FileOutcome::Ran(Ok(_))))
    );
}

/// A finished headless app at the given logical window size, built without
/// any help from the scenario.
fn app_with_window(width: u32, height: u32) -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default().with_window_size(width, height));
    app.finish();
    app.cleanup();
    app
}

/// The primary window's resolution as `(logical width, logical height,
/// scale factor, physical width)`.
fn primary_window(app: &mut App) -> (f32, f32, f32, u32) {
    let world = app.world_mut();
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .expect("a primary window");
    (
        window.resolution.width(),
        window.resolution.height(),
        window.resolution.scale_factor(),
        window.resolution.physical_width(),
    )
}

#[test]
fn run_in_applies_the_scenarios_window_size_and_scale_factor_to_the_app_it_is_given() {
    // The button lies outside a 640 px wide window, so this passes only if
    // the scenario's own size reaches the app it is played in.
    let scenario = press_pause().window_size(1280, 720).scale_factor(2.0);
    let mut app = app_with_window(640, 480);

    let report = scenario.run_in(&mut app);

    assert!(report.passed(), "{report:#?}");
    let (width, height, scale, physical_width) = primary_window(&mut app);
    assert!((width - 1280.0).abs() < 0.5 && (height - 720.0).abs() < 0.5);
    assert!((scale - 2.0).abs() < f32::EPSILON);
    assert_eq!(physical_width, 2560);
}

#[test]
fn run_in_leaves_the_window_alone_where_the_scenario_sets_no_size() {
    let mut app = app_with_window(1280, 720);

    let report = press_pause().run_in(&mut app);

    assert!(report.passed(), "{report:#?}");
    let (width, height, scale, _) = primary_window(&mut app);
    assert!((width - 1280.0).abs() < 0.5 && (height - 720.0).abs() < 0.5);
    assert!((scale - 1.0).abs() < f32::EPSILON);
}

#[test]
fn run_in_keeps_the_logical_size_where_the_scenario_sets_only_a_scale_factor() {
    let scenario = press_pause().scale_factor(2.0);
    let mut app = app_with_window(1280, 720);

    let report = scenario.run_in(&mut app);

    assert!(report.passed(), "{report:#?}");
    let (width, height, scale, physical_width) = primary_window(&mut app);
    assert!((width - 1280.0).abs() < 0.5 && (height - 720.0).abs() < 0.5);
    assert!((scale - 2.0).abs() < f32::EPSILON);
    assert_eq!(physical_width, 2560);
}

#[test]
fn a_scenario_loaded_from_a_file_with_a_window_size_fits_when_played_with_run_in() {
    let text = press_pause()
        .window_size(1280, 720)
        .to_ron()
        .expect("exports");
    let scenario = Scenario::from_ron(&text).expect("loads");
    let mut app = app_with_window(640, 480);

    let report = scenario.run_in(&mut app);

    assert!(report.passed(), "{report:#?}");
}

#[test]
fn run_in_traced_applies_the_window_size_too() {
    let scenario = press_pause().window_size(1280, 720);
    let mut app = app_with_window(640, 480);

    let (report, _) = scenario.run_in_traced(&mut app, NoiseFilter::default());

    assert!(report.passed(), "{report:#?}");
}
