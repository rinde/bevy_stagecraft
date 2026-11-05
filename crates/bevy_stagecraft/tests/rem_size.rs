//! Scenarios run at another root font size (Bevy's `RemSize`): a variant's
//! text size setting.

#![allow(
    clippy::expect_used,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    prelude::*,
    text::RemSize,
    ui::{percent, px},
    ui_widgets::{Activate, Button},
};
use bevy_stagecraft::{
    AppSetup, Check, ReflectAppSetup, Scenario, ScenarioReport, Step, Target, Variant,
};

/// How many times the button was activated.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Activations(u32);

/// The root font size the scenario's setup closure saw.
#[derive(Resource)]
struct SeenRem(f32);

/// The width of the panel in front of the button, in logical pixels.
const PANEL_WIDTH: i32 = 700;

/// The rem size of the "large text" variant: ten times Bevy's default.
const LARGE_REM: f32 = 200.0;

/// A row whose last item is a button labelled "Pause" in the default font
/// size, which is one rem, so the button grows with the root font size. It
/// starts 700 px in and does not shrink, so it fits an 800 px window only
/// while its label is narrow enough.
#[derive(Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(AppSetup)]
enum LabelRow {
    /// The only layout.
    Default,
}

impl AppSetup for LabelRow {
    fn build(&self, world: &mut World) -> Result<(), String> {
        world.init_resource::<Activations>();
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
        world.spawn((
            Node {
                width: px(PANEL_WIDTH),
                height: px(100),
                flex_shrink: 0.0,
                ..Node::default()
            },
            ChildOf(root),
        ));
        world
            .spawn((
                Button,
                Node {
                    height: px(50),
                    flex_shrink: 0.0,
                    ..Node::default()
                },
                children![Text::new("Pause")],
                ChildOf(root),
            ))
            .observe(|_: On<Activate>, mut count: ResMut<Activations>| count.0 += 1);
        Ok(())
    }
}

/// Presses the label row's button in an 800 px window.
fn press_label() -> Scenario {
    Scenario::new("label")
        .app_setup(LabelRow::Default)
        .window_size(800, 600)
        .action_timeout(5)
        .step(Step::move_to(Target::text("Pause")))
        .step(Step::click())
        .expect_check(Check::resource::<Activations>(".0").eq(1_u32))
}

/// Plays `scenario` in `variant`.
fn play(scenario: &Scenario, variant: &Variant) -> ScenarioReport {
    variant
        .apply(scenario)
        .expect("the variant applies")
        .run_headless()
}

#[test]
fn a_label_that_overflows_at_a_larger_rem_size_fails_a_check_that_passes_at_the_default() {
    let scenario = press_label();
    let at_default = play(&scenario, &Variant::default());
    let at_large = play(&scenario, &Variant::new("large text").rem_size(LARGE_REM));
    assert!(at_default.passed(), "{at_default:#?}");
    assert!(!at_large.passed(), "{at_large:#?}");
}

#[test]
fn a_variant_rem_size_reaches_the_app_and_run_in_puts_it_back() {
    let varied = Variant::new("large text")
        .rem_size(LARGE_REM)
        .apply(&press_label())
        .expect("applies")
        .setup(|world| {
            let rem = world.get_resource::<RemSize>().map_or(0.0, |rem| rem.0);
            world.insert_resource(SeenRem(rem));
        });
    assert_eq!(varied.rem_size(), Some(LARGE_REM));
    let mut app = varied.headless_app();
    let before = app
        .world()
        .get_resource::<RemSize>()
        .copied()
        .unwrap_or_default();
    let _ = varied.run_in(&mut app);
    let seen = app.world().get_resource::<SeenRem>().map(|seen| seen.0);
    assert_eq!(
        seen,
        Some(LARGE_REM),
        "the setup runs at the variant's size"
    );
    assert_eq!(app.world().get_resource::<RemSize>(), Some(&before));
}

#[test]
fn a_rem_size_that_is_not_usable_is_refused() {
    for rem in [0.0, -4.0, f32::NAN, f32::INFINITY] {
        let error = Variant::new("bad")
            .rem_size(rem)
            .apply(&press_label())
            .expect_err("refused");
        assert!(error.to_string().contains("rem size"), "{error}");
    }
}

#[test]
fn a_screen_target_stays_valid_under_another_rem_size() {
    let scenario = Scenario::new("fixed")
        .app_setup(LabelRow::Default)
        .step(Step::move_to(Target::screen(Vec2::new(10.0, 10.0))));
    assert!(
        Variant::new("large text")
            .rem_size(LARGE_REM)
            .apply(&scenario)
            .is_ok()
    );
}
