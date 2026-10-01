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
    HeadlessPlugins, Modifiers, Outcome, ResolvedTarget, Scenario, ScenarioPlayerPlugin,
    ScenarioReport, Step, Target, TargetResolvers,
};

/// How many times a button was activated.
#[derive(Resource, Default)]
struct Activations(u32);

/// Total distance dragged over the drag pad.
#[derive(Resource, Default)]
struct Dragged(Vec2);

/// Spawns a camera, a "Pause" button, and a drag pad.
fn spawn_ui(world: &mut World) {
    world.init_resource::<Activations>();
    world.init_resource::<Dragged>();
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
        .observe(|drag: On<Pointer<Drag>>, mut total: ResMut<Dragged>| {
            total.0 += drag.delta;
        });
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
    Scenario::new("click pause")
        .setup(spawn_ui)
        .steps([
            Step::move_to(Target::text("Pause")),
            Step::click(),
            Step::wait(1),
            Step::check("activated once"),
        ])
        .check("activated once", |probe| {
            let count = probe.resource::<Activations>().map_or(0, |a| a.0);
            if count == 1 {
                Ok(())
            } else {
                Err(format!("activated {count} times"))
            }
        })
}

#[test]
fn clicking_a_button_by_its_text_activates_it() {
    let report = click_pause()
        .expect("still once at the end", |probe| {
            probe.resource::<Activations>().is_some_and(|a| a.0 == 1)
        })
        .run_headless();
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
    assert!(failures[0].contains("no UI node shows \"Resume\""));
}

#[test]
fn a_glide_ends_on_its_target() {
    let report = Scenario::new("glide")
        .setup(spawn_ui)
        .steps([
            Step::glide(Target::screen(Vec2::new(600.0, 600.0)), 10),
            Step::check("arrived"),
        ])
        .check("arrived", |probe| {
            probe
                .world()
                .try_query::<&Window>()
                .and_then(|q| {
                    q.iter_manual(probe.world())
                        .next()
                        .and_then(Window::cursor_position)
                })
                .is_some_and(|p| p.distance(Vec2::new(600.0, 600.0)) < 0.01)
        })
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
        .expect("dragged 200 pixels right", |probe| {
            let total = probe.resource::<Dragged>().map_or(Vec2::ZERO, |d| d.0);
            if (total.x - 200.0).abs() < 1.0 && total.y.abs() < 1.0 {
                Ok(())
            } else {
                Err(format!("dragged {total}"))
            }
        })
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn custom_targets_resolve_through_a_registered_resolver() {
    let mut app = App::new();
    app.add_plugins((HeadlessPlugins::default(), ScenarioPlayerPlugin));
    app.finish();
    app.cleanup();
    app.world_mut()
        .resource_mut::<TargetResolvers>()
        .register("lattice", |_, argument| {
            let cell: f32 = argument
                .parse()
                .map_err(|_| format!("`{argument}` is not a cell"))?;
            Ok(ResolvedTarget::at(Vec2::new(150.0, 40.0 + cell)))
        });
    let report = Scenario::new("custom")
        .setup(spawn_ui)
        .steps([
            Step::move_to(Target::custom("lattice", "10")),
            Step::click(),
            Step::wait(1),
        ])
        .expect("button activated", |probe| {
            probe.resource::<Activations>().is_some_and(|a| a.0 == 1)
        })
        .run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());

    let unknown = Scenario::new("unknown")
        .step(Step::move_to(Target::custom("nope", "")))
        .run_in(&mut app);
    assert!(unknown.failures()[0].contains("no target resolver named `nope`"));
}

#[test]
fn key_steps_hold_modifiers_until_released() {
    let held = |probe: &rin_bevy_ui_tester::Probe<'_>| {
        probe
            .resource::<ButtonInput<KeyCode>>()
            .is_some_and(|keys| keys.pressed(KeyCode::ControlLeft) && keys.pressed(KeyCode::KeyA))
    };
    let report = Scenario::new("keys")
        .setup(spawn_ui)
        .steps([
            Step::Key {
                key: KeyCode::KeyA,
                modifiers: Modifiers::CTRL,
            },
            Step::check("held"),
            Step::KeyUp {
                key: KeyCode::KeyA,
                modifiers: Modifiers::CTRL,
            },
            Step::wait(1),
            Step::check("released"),
        ])
        .check("held", held)
        .check("released", move |probe| !held(probe))
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn captions_and_unknown_checks_are_reported() {
    let report = Scenario::new("meta")
        .setup(spawn_ui)
        .steps([Step::caption("hello"), Step::check("ghost")])
        .run_headless();
    assert_eq!(report.caption.as_deref(), Some("hello"));
    assert!(!report.passed());
    assert!(report.failures()[0].contains("no check named `ghost`"));
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
