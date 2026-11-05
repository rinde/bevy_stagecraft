//! A panic in the app under test fails the step it happened in; the runner
//! goes on.

#![allow(
    clippy::expect_used,
    clippy::disallowed_methods,
    clippy::panic,
    reason = "tests may panic on a broken setup, and the app under test panics on purpose"
)]

use bevy::{
    ecs::error::{ErrorContext, ErrorHandler, FallbackErrorHandler},
    prelude::*,
    ui::px,
    ui_widgets::{Activate, Button},
};
use bevy_stagecraft::{Outcome, Registry, Scenario, Step, Target, files::run_ron_files};

/// A camera and a "Boom" button whose observer panics.
fn spawn_panicking_button(world: &mut World) {
    world.spawn(Camera2d);
    world
        .spawn((
            Button,
            Node {
                padding: UiRect::all(px(8)),
                ..Node::default()
            },
            children![Text::new("Boom")],
        ))
        .observe(|_: On<Activate>| {
            panic!("the button blew up");
        });
}

/// Clicks "Boom", then waits twice.
fn boom() -> Scenario {
    Scenario::new("boom")
        .window_size(200, 100)
        .setup(spawn_panicking_button)
        .steps([
            Step::move_to(Target::text("Boom")),
            Step::click(),
            Step::wait(2),
            Step::wait(2),
        ])
}

/// Does nothing and passes.
fn calm() -> Scenario {
    Scenario::new("calm").steps([Step::wait(2)])
}

/// The handler an app had before a run, which a run must put back.
fn app_handler(_: BevyError, _: ErrorContext) {}

#[test]
fn a_panicking_observer_fails_the_click_step_and_skips_the_rest() {
    let report = boom().run_headless();

    assert!(!report.passed());
    assert_eq!(
        report.steps.first().map(|step| step.outcome.is_pass()),
        Some(true)
    );
    let Some(Outcome::Fail(message)) = report.steps.get(1).map(|step| step.outcome.clone()) else {
        panic!("the click step should have failed: {report:?}");
    };
    assert!(message.contains("the button blew up"), "{message}");
    assert!(message.contains("failed"), "{message}");
    assert_eq!(
        report.steps.get(2).map(|step| &step.outcome),
        Some(&Outcome::Skipped)
    );
    assert_eq!(
        report.steps.get(3).map(|step| &step.outcome),
        Some(&Outcome::Skipped)
    );
}

#[test]
fn the_runner_goes_on_to_the_file_after_a_panicking_one() {
    let dir = std::env::temp_dir().join(format!("rin-tester-panics-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the temporary directory should be writable");
    // Named so that the panicking file is played first.
    for name in ["a_boom", "b_calm"] {
        std::fs::write(
            dir.join(format!("{name}.ron")),
            format!("(format_version: 3, name: \"{name}\", steps: [])"),
        )
        .expect("the file should be writable");
    }

    let results = run_ron_files(&dir, &Registry::default(), "", |scenario| {
        if scenario.name() == "a_boom" {
            boom().run_headless()
        } else {
            calm().run_headless()
        }
    })
    .expect("the directory should be readable");
    std::fs::remove_dir_all(&dir).expect("the temporary directory should be removable");

    let passed: Vec<_> = results
        .iter()
        .map(|result| (result.name.as_str(), result.passed()))
        .collect();
    assert_eq!(passed, [("a_boom", false), ("b_calm", true)]);
}

#[test]
fn the_apps_own_error_handler_is_restored_after_a_run() {
    let scenario = boom();
    let mut app = scenario.headless_app();
    app.insert_resource(FallbackErrorHandler(app_handler));

    assert!(!scenario.run_in(&mut app).passed());

    let handler = app.world().resource::<FallbackErrorHandler>().0;
    let expected: ErrorHandler = app_handler;
    assert!(core::ptr::fn_addr_eq(handler, expected));
}
