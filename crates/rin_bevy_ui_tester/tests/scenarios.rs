//! Scenarios over a plain Bevy UI, no game.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    input::{ButtonInput, keyboard::KeyCode},
    prelude::*,
    ui::px,
    ui_widgets::{Activate, Button},
};
use rin_bevy_ui_tester::{
    AppCheck, AppTarget, AppTargetValue, Check, CheckResult, HeadlessPlugins, Modifiers, Outcome,
    Probe, ReflectAppCheck, ReflectAppTarget, ResolvedTarget, Scenario, ScenarioPlayerPlugin,
    ScenarioReport, Step, Target,
};

/// How many times a button was activated.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Activations(u32);

/// Total distance dragged over the drag pad.
#[derive(Resource, Default)]
struct Dragged(Vec2);

/// How many drag events the drag pad saw.
#[derive(Resource, Default)]
struct DragEvents(u32);

/// Spawns a camera, a "Pause" button, and a drag pad.
fn spawn_ui(world: &mut World) {
    world.init_resource::<Activations>();
    world.init_resource::<Dragged>();
    world.init_resource::<DragEvents>();
    world.spawn(Camera2d);
    world
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: px(100),
                top: px(25),
                width: px(100),
                height: px(50),
                ..Node::default()
            },
            BackgroundColor(Color::WHITE),
            children![Text::new("Pause")],
        ))
        .observe(|_: On<Activate>, mut count: ResMut<Activations>| count.0 += 1);
    world
        .spawn((
            Name::new("Drag pad"),
            Node {
                position_type: PositionType::Absolute,
                left: px(100),
                top: px(300),
                width: px(300),
                height: px(100),
                ..Node::default()
            },
            BackgroundColor(Color::BLACK),
        ))
        .observe(
            |drag: On<Pointer<Drag>>, mut total: ResMut<Dragged>, mut count: ResMut<DragEvents>| {
                total.0 += drag.delta;
                count.0 += 1;
            },
        );
}

/// This app's own checks over the pad, the cursor and the keyboard.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(AppCheck)]
enum PadCheck {
    /// The window's cursor is at this position.
    CursorAt(Vec2),
    /// The pad was dragged this far to the right and not up or down.
    DraggedRight(f32),
    /// The pad saw this many drag events.
    DragEvents(u32),
    /// Control and A are both held.
    ShortcutHeld,
}

impl AppCheck for PadCheck {
    fn run(&self, probe: &Probe<'_>) -> CheckResult {
        match self {
            Self::CursorAt(wanted) => {
                let found = probe.world().try_query::<&Window>().and_then(|query| {
                    query
                        .iter_manual(probe.world())
                        .next()
                        .and_then(Window::cursor_position)
                });
                match found {
                    Some(at) if at.distance(*wanted) < 0.01 => Ok(()),
                    Some(at) => Err(format!("the cursor is at {at}, not {wanted}")),
                    None => Err("the cursor is outside the window".to_owned()),
                }
            }
            Self::DraggedRight(wanted) => {
                let total = probe.resource::<Dragged>().map_or(Vec2::ZERO, |d| d.0);
                if (total.x - wanted).abs() < 1.0 && total.y.abs() < 1.0 {
                    Ok(())
                } else {
                    Err(format!("dragged {total}"))
                }
            }
            Self::DragEvents(wanted) => {
                let found = probe.resource::<DragEvents>().map_or(0, |events| events.0);
                if found == *wanted {
                    Ok(())
                } else {
                    Err(format!("the pad saw {found} drag events"))
                }
            }
            Self::ShortcutHeld => {
                let held = probe
                    .resource::<ButtonInput<KeyCode>>()
                    .is_some_and(|keys| {
                        keys.pressed(KeyCode::ControlLeft) && keys.pressed(KeyCode::KeyA)
                    });
                if held {
                    Ok(())
                } else {
                    Err("Control and A are not both held".to_owned())
                }
            }
        }
    }
}

/// Covers the button with a named panel.
fn spawn_cover(world: &mut World) {
    world.spawn((
        Name::new("Modal panel"),
        Node {
            position_type: PositionType::Absolute,
            left: px(80),
            top: px(10),
            width: px(200),
            height: px(100),
            ..Node::default()
        },
        BackgroundColor(Color::srgb(1.0, 0.0, 0.0)),
        GlobalZIndex(10),
    ));
}

/// A scenario over the plain UI that clicks "Pause".
fn click_pause() -> Scenario {
    Scenario::new("click pause").setup(spawn_ui).steps([
        Step::move_to(Target::text("Pause")),
        Step::click(),
        Step::wait(1),
        Step::check(activated_once()),
    ])
}

/// The button was activated exactly once.
fn activated_once() -> Check {
    Check::resource::<Activations>(".0").eq(1_u32)
}

#[test]
fn a_scenario_with_checks_writes_out_and_reads_back_the_same() {
    let scenario = click_pause()
        .expect_check(activated_once())
        .expect_check(Check::app(PadCheck::DraggedRight(200.0)))
        .expect_check(!Check::app(PadCheck::ShortcutHeld));
    let text = scenario.to_ron().expect("every check is data");
    let read = Scenario::from_ron(&text).expect("the file reads back");
    assert_eq!(read.to_script(), scenario.to_script());
}

#[test]
fn clicking_a_button_by_its_text_activates_it() {
    let report = click_pause().expect_check(activated_once()).run_headless();
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(
        report.checks.len(),
        2,
        "one check and one expect should run"
    );
}

#[test]
fn a_covered_button_fails_naming_what_covers_it() {
    let scenario = click_pause().setup(|world| {
        spawn_ui(world);
        spawn_cover(world);
    });
    let report = scenario.run_headless();
    assert!(!report.passed());
    let failures = report.failures();
    assert!(
        failures.iter().any(|f| f.contains("Modal panel")),
        "the failure should name the cover: {failures:?}"
    );
    let outcomes: Vec<_> = report.steps.iter().map(|s| s.outcome.clone()).collect();
    assert!(matches!(outcomes[0], Outcome::Fail(_)), "the move fails");
    assert_eq!(outcomes[1], Outcome::Skipped, "the click is skipped");
    assert_eq!(outcomes[3], Outcome::Skipped, "the check is skipped");
}

#[test]
fn a_covered_button_is_never_clicked() {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    let scenario = click_pause().setup(|world| {
        spawn_ui(world);
        spawn_cover(world);
    });
    let report = scenario.run_in(&mut app);
    assert!(!report.passed());
    assert_eq!(app.world().resource::<Activations>().0, 0);
}

#[test]
fn text_that_is_not_on_screen_fails_the_step_clearly() {
    let report = Scenario::new("missing")
        .setup(spawn_ui)
        .step(Step::move_to(Target::text("Resume")))
        .run_headless();
    let failures = report.failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].contains("no widget shows \"Resume\""),
        "{failures:?}"
    );
}

#[test]
fn a_glide_ends_on_its_target() {
    let report = Scenario::new("glide")
        .setup(spawn_ui)
        .steps([
            Step::glide(Target::screen(Vec2::new(600.0, 600.0)), 10),
            Step::check(Check::app(PadCheck::CursorAt(Vec2::new(600.0, 600.0)))),
        ])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_glide_passes_through_points_between_start_and_end() {
    let report = Scenario::new("glide frames")
        .setup(spawn_ui)
        .steps([
            Step::move_to(Target::screen(Vec2::new(10.0, 10.0))),
            Step::glide(Target::screen(Vec2::new(500.0, 10.0)), 10),
            Step::wait(1),
        ])
        .run_headless();
    let frames: Vec<u32> = report.steps.iter().map(|s| s.frame).collect();
    assert_eq!(frames[1] + 10, frames[2], "the glide lasts ten frames");
    assert_eq!(frames[0] + 1, frames[1], "a jump lasts one frame");
}

#[test]
fn a_move_burst_delivers_every_move_in_one_frame() {
    let report = Scenario::new("burst")
        .setup(spawn_ui)
        .steps([
            Step::move_to(Target::screen(Vec2::new(150.0, 350.0))),
            Step::press(),
            Step::move_burst([
                Vec2::new(150.0, 355.0),
                Vec2::new(150.0, 360.0),
                Vec2::new(150.0, 365.0),
            ]),
            Step::release(),
        ])
        .expect_check(Check::app(PadCheck::DragEvents(3)))
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
    let frames: Vec<u32> = report.steps.iter().map(|s| s.frame).collect();
    assert_eq!(
        frames[2],
        frames[1] + 1,
        "the burst is one frame, however many moves it holds"
    );
}

#[test]
fn dragging_across_a_pad_reports_the_distance() {
    let report = Scenario::new("drag")
        .setup(spawn_ui)
        .steps([
            Step::drag(
                Target::screen(Vec2::new(150.0, 350.0)),
                Target::screen(Vec2::new(350.0, 350.0)),
                20,
            ),
            Step::wait(1),
        ])
        .expect_check(Check::app(PadCheck::DraggedRight(200.0)))
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

/// This app's own targets, found by Bevy with no registration call.
#[derive(Reflect, Debug, Clone)]
#[reflect(AppTarget)]
enum LatticeTarget {
    /// A row of the lattice, counted in pixels below the top of the button.
    Row(f32),
}

impl AppTarget for LatticeTarget {
    fn resolve(&self, _world: &World) -> Result<ResolvedTarget, String> {
        match self {
            Self::Row(row) if *row < 0.0 => Err(format!("there is no row {row}")),
            Self::Row(row) => Ok(ResolvedTarget::at(Vec2::new(150.0, 40.0 + row))),
        }
    }
}

#[test]
fn the_apps_own_targets_resolve_with_no_registration_call() {
    let mut app = App::new();
    app.add_plugins((HeadlessPlugins::default(), ScenarioPlayerPlugin));
    app.finish();
    app.cleanup();
    let report = Scenario::new("app target")
        .setup(spawn_ui)
        .steps([
            Step::move_to(Target::app(LatticeTarget::Row(10.0))),
            Step::click(),
            Step::wait(1),
        ])
        .expect_check(activated_once())
        .run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());

    let nowhere = Scenario::new("nowhere")
        .step(Step::move_to(Target::app(LatticeTarget::Row(-1.0))))
        .run_in(&mut app);
    assert!(
        nowhere.failures()[0].contains("there is no row -1"),
        "{:?}",
        nowhere.failures()
    );
}

#[test]
fn an_app_target_written_as_ron_in_rust_is_read_as_the_apps_type_when_it_is_used() {
    let written = |ron: &str| Target::App(AppTargetValue::from_ron(ron).expect("one RON value"));
    let report = Scenario::new("from RON")
        .setup(spawn_ui)
        .steps([
            Step::move_to(written("Row( 10.0 )")),
            Step::click(),
            Step::wait(1),
        ])
        .expect_check(activated_once())
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());

    let misspelt = Scenario::new("misspelt")
        .step(Step::move_to(written("Rwo(1.0)")))
        .run_headless();
    let failure = &misspelt.failures()[0];
    assert!(
        failure.contains("`Rwo(1.0)` is not a `LatticeTarget`") && failure.contains("`Row(f32)`"),
        "{failure}"
    );
}

#[test]
fn key_steps_hold_modifiers_until_released() {
    let held = || Check::app(PadCheck::ShortcutHeld);
    let report = Scenario::new("keys")
        .setup(spawn_ui)
        .steps([
            Step::Key {
                key: KeyCode::KeyA,
                modifiers: Modifiers::CTRL,
            },
            Step::check(held()),
            Step::KeyUp {
                key: KeyCode::KeyA,
                modifiers: Modifiers::CTRL,
            },
            Step::wait(1),
            Step::check(!held()),
        ])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn captions_are_reported() {
    let report = Scenario::new("meta")
        .setup(spawn_ui)
        .steps([Step::caption("hello"), Step::wait(1)])
        .run_headless();
    assert_eq!(report.caption.as_deref(), Some("hello"));
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn the_report_resource_is_readable_after_the_run() {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    let _ = click_pause().run_in(&mut app);
    let report = app.world().resource::<ScenarioReport>();
    assert!(report.finished);
    assert_eq!(report.name, "click pause");
}

#[test]
fn a_failing_step_in_a_group_names_the_group_path() {
    let report = Scenario::new("grouped")
        .setup(spawn_ui)
        .steps([
            Step::wait(1),
            Step::group(
                "raise the bridge",
                [
                    Step::wait(1),
                    Step::group("pull the lever", [Step::move_to(Target::text("Resume"))]),
                ],
            ),
        ])
        .run_headless();
    let failures = report.failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].starts_with("raise the bridge > pull the lever > step 2"),
        "the failure should lead with the group path and the step's flat number: {failures:?}"
    );
    assert_eq!(
        report.steps[2].group,
        ["raise the bridge", "pull the lever"],
        "the report keeps each step's path"
    );
    assert!(report.steps[0].group.is_empty());
}

#[test]
fn a_scenario_plays_frame_for_frame_the_same_with_and_without_groups() {
    let flat = click_pause();
    let grouped = Scenario::new("click pause").setup(spawn_ui).steps([
        Step::group(
            "press the button",
            [
                Step::move_to(Target::text("Pause")),
                Step::group("click it", [Step::click()]),
            ],
        ),
        Step::group(
            "see the result",
            [Step::wait(1), Step::check(activated_once())],
        ),
    ]);
    let (flat, grouped) = (flat.run_headless(), grouped.run_headless());
    assert!(grouped.passed(), "{:?}", grouped.failures());
    assert_eq!(grouped.frames, flat.frames);
    let outcomes = |report: &ScenarioReport| -> Vec<Outcome> {
        report.steps.iter().map(|s| s.outcome.clone()).collect()
    };
    assert_eq!(outcomes(&grouped), outcomes(&flat));
    let steps = |report: &ScenarioReport| -> Vec<Step> {
        report.steps.iter().map(|s| s.step.clone()).collect()
    };
    assert_eq!(steps(&grouped), steps(&flat));
}

#[test]
fn an_empty_group_plays_nothing() {
    let report = Scenario::new("empty")
        .setup(spawn_ui)
        .steps([Step::group("nothing", []), Step::wait(1)])
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(report.steps.len(), 1, "a group is not a step of its own");
}
