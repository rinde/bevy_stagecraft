//! Steps wait until their target can be acted on, over a plain Bevy UI.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    prelude::*,
    ui::{InteractionDisabled, px},
    ui_widgets::{Activate, Button},
};
use rin_bevy_ui_tester::{
    DEFAULT_ACTION_TIMEOUT, Outcome, Registry, Scenario, ScenarioPlayer, ScenarioReport, Script,
    Step, Target, TargetDescriptions, describe_step,
};

/// How many times the "Go" button was activated.
#[derive(Resource, Default)]
struct Activations(u32);

/// The player frame on which a late system does its thing.
#[derive(Resource, Clone, Copy)]
struct AtFrame(u32);

/// The "Go" button, at (100, 25) with a size of 100 by 50.
fn go_button() -> impl Bundle {
    (
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
        children![Text::new("Go")],
    )
}

/// Counts an activation of the button it observes.
fn count(_: On<Activate>, mut activations: ResMut<Activations>) {
    activations.0 += 1;
}

/// A camera and the "Go" button.
fn spawn_go(world: &mut World) {
    world.init_resource::<Activations>();
    world.spawn(Camera2d);
    world.spawn(go_button()).observe(count);
}

/// Spawns the "Go" button on the player frame [`AtFrame`] says.
fn spawn_late(mut commands: Commands, report: Res<ScenarioReport>, on: Res<AtFrame>) {
    if report.frames == on.0 {
        commands.spawn(go_button()).observe(count);
    }
}

/// Enables every button on the player frame [`AtFrame`] says.
fn enable_late(
    mut commands: Commands,
    report: Res<ScenarioReport>,
    on: Res<AtFrame>,
    disabled: Query<Entity, With<InteractionDisabled>>,
) {
    if report.frames == on.0 {
        for entity in &disabled {
            commands.entity(entity).remove::<InteractionDisabled>();
        }
    }
}

/// Moves every button right by 10 pixels each frame, from the first frame
/// of the app until the player frame [`AtFrame`] says.
fn slide(
    report: Res<ScenarioReport>,
    on: Res<AtFrame>,
    mut moved: Local<f32>,
    mut buttons: Query<&mut Node, With<Button>>,
) {
    if report.frames < on.0 {
        *moved += 10.0;
        for mut node in &mut buttons {
            node.left = Val::Px(100.0 + *moved);
        }
    }
}

/// Plays `scenario` with `system` running in `Update` and [`AtFrame`] at `on`,
/// and returns the report and the activations.
fn play_with<M>(
    scenario: &Scenario,
    on: u32,
    system: impl IntoScheduleConfigs<bevy::ecs::system::ScheduleSystem, M>,
) -> (ScenarioReport, u32) {
    let mut app = scenario.headless_app();
    app.insert_resource(AtFrame(on));
    app.init_resource::<ScenarioReport>();
    app.add_systems(Update, system);
    let report = scenario.run_in(&mut app);
    let activations = app.world().get_resource::<Activations>().map_or(0, |a| a.0);
    (report, activations)
}

/// Plays `scenario` and returns the report and the activations.
fn play(scenario: &Scenario) -> (ScenarioReport, u32) {
    let mut app = scenario.headless_app();
    let report = scenario.run_in(&mut app);
    let activations = app.world().get_resource::<Activations>().map_or(0, |a| a.0);
    (report, activations)
}

/// Move to "Go" and click, with no `Wait`.
fn click_go(name: &str) -> Scenario {
    Scenario::new(name).steps([Step::move_to(Target::text("Go")), Step::click()])
}

/// The failure message of the step at `index`.
fn failure(report: &ScenarioReport, index: usize) -> String {
    match &report.steps[index].outcome {
        Outcome::Fail(message) => message.clone(),
        other => panic!("step {index} should fail, but: {other:?}"),
    }
}

#[test]
fn a_button_that_appears_after_ten_frames_is_clicked_with_no_wait() {
    let scenario = click_go("late button").setup(|world| {
        world.init_resource::<Activations>();
        world.spawn(Camera2d);
    });
    let (report, activations) = play_with(&scenario, 10, spawn_late);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(activations, 1, "the button was clicked once");
    assert!(
        report.steps[1].frame > 10,
        "the click waited for the button: {:?}",
        report.steps
    );
}

#[test]
fn a_button_covered_by_another_node_fails_saying_it_is_covered() {
    let scenario = click_go("covered").action_timeout(5).setup(|world| {
        spawn_go(world);
        world.spawn((
            Name::new("Curtain"),
            Node {
                position_type: PositionType::Absolute,
                left: px(80),
                top: px(10),
                width: px(200),
                height: px(100),
                ..Node::default()
            },
            GlobalZIndex(10),
        ));
    });
    let (report, activations) = play(&scenario);
    let message = failure(&report, 0);
    assert!(
        message.contains("does not receive the pointer: it is covered by `Curtain`"),
        "{message}"
    );
    assert!(message.contains("still after 5 frames"), "{message}");
    assert_eq!(report.steps[1].outcome, Outcome::Skipped);
    assert_eq!(activations, 0);
}

#[test]
fn a_disabled_button_fails_saying_it_is_disabled() {
    let scenario = click_go("disabled").action_timeout(5).setup(|world| {
        spawn_go(world);
        let mut buttons = world.query_filtered::<Entity, With<Button>>();
        let button = buttons.iter(world).next().expect("the button is there");
        world.entity_mut(button).insert(InteractionDisabled);
    });
    let (report, activations) = play(&scenario);
    assert_eq!(
        report.steps[0].outcome,
        Outcome::Pass,
        "a disabled button can still be hovered"
    );
    let message = failure(&report, 1);
    assert!(message.starts_with("Text(\"Go\") is disabled"), "{message}");
    assert_eq!(activations, 0);
}

#[test]
fn a_button_inside_a_disabled_panel_is_disabled_too() {
    let scenario = click_go("disabled panel").action_timeout(0).setup(|world| {
        world.init_resource::<Activations>();
        world.spawn(Camera2d);
        world
            .spawn((Name::new("Panel"), Node::default(), InteractionDisabled))
            .with_children(|panel| {
                panel.spawn(go_button()).observe(count);
            });
    });
    let (report, _) = play(&scenario);
    let message = failure(&report, 1);
    assert!(
        message.contains("is disabled: `Panel` above it is"),
        "{message}"
    );
}

#[test]
fn a_button_that_becomes_enabled_is_clicked_once_it_is() {
    let scenario = click_go("enabled later").setup(|world| {
        spawn_go(world);
        let mut buttons = world.query_filtered::<Entity, With<Button>>();
        let button = buttons.iter(world).next().expect("the button is there");
        world.entity_mut(button).insert(InteractionDisabled);
    });
    let (report, activations) = play_with(&scenario, 8, enable_late);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(activations, 1);
}

#[test]
fn forcing_a_click_on_a_disabled_button_clicks_without_waiting() {
    let scenario = Scenario::new("forced")
        .steps([
            Step::move_to(Target::text("Go")),
            Step::click().forced(),
            Step::check(rin_bevy_ui_tester::Check::disabled(Target::text("Go"))),
        ])
        .setup(|world| {
            spawn_go(world);
            let mut buttons = world.query_filtered::<Entity, With<Button>>();
            let button = buttons.iter(world).next().expect("the button is there");
            world.entity_mut(button).insert(InteractionDisabled);
        });
    let (report, activations) = play(&scenario);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(activations, 0, "Bevy's button ignores a disabled click");
}

#[test]
fn a_hidden_button_fails_saying_it_is_not_visible() {
    let scenario = click_go("hidden").action_timeout(3).setup(|world| {
        spawn_go(world);
        let mut buttons = world.query_filtered::<Entity, With<Button>>();
        let button = buttons.iter(world).next().expect("the button is there");
        world.entity_mut(button).insert(Visibility::Hidden);
    });
    let (report, _) = play(&scenario);
    let message = failure(&report, 0);
    assert!(
        message.starts_with("Text(\"Go\") is not visible: it is hidden"),
        "{message}"
    );
}

#[test]
fn a_target_that_never_appears_fails_after_the_timeout_with_close_matches() {
    let scenario = Scenario::new("misspelt")
        .action_timeout(7)
        .steps([Step::move_to(Target::text("Goo"))])
        .setup(spawn_go);
    let (report, _) = play(&scenario);
    let message = failure(&report, 0);
    assert_eq!(
        message,
        "no widget shows \"Goo\"; did you mean \"Go\"? (still after 7 frames)"
    );
    assert_eq!(
        report.frames, 8,
        "it looked on 8 frames: the first and 7 more"
    );
}

#[test]
fn an_ambiguous_target_fails_at_once_without_waiting() {
    let scenario = click_go("ambiguous").setup(|world| {
        spawn_go(world);
        world.spawn(go_button());
    });
    let (report, _) = play(&scenario);
    let message = failure(&report, 0);
    assert!(
        message.contains("is ambiguous: it matches 2 widgets"),
        "{message}"
    );
    assert_eq!(report.frames, 1, "strictness does not wait");
}

#[test]
fn a_sliding_button_is_acted_on_only_once_it_stops() {
    let scenario = click_go("sliding").setup(spawn_go);
    let (report, activations) = play_with(&scenario, 6, slide);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(activations, 1);
    assert!(
        report.steps[1].frame >= 6,
        "the move waited for the button to stop: {:?}",
        report.steps
    );

    let impatient = click_go("sliding, impatient")
        .action_timeout(2)
        .setup(spawn_go);
    let (report, _) = play_with(&impatient, 20, slide);
    let message = failure(&report, 0);
    assert!(
        message.starts_with("Text(\"Go\") is not stable"),
        "{message}"
    );
}

#[test]
fn a_steps_own_timeout_overrides_the_scenarios() {
    let late = |name: &str, move_step: Step| {
        Scenario::new(name)
            .action_timeout(2)
            .steps([move_step, Step::click()])
            .setup(|world| {
                world.init_resource::<Activations>();
                world.spawn(Camera2d);
            })
    };
    let (report, _) = play_with(
        &late("scenario timeout", Step::move_to(Target::text("Go"))),
        10,
        spawn_late,
    );
    assert!(!report.passed(), "two frames are too few");
    let (report, activations) = play_with(
        &late(
            "step timeout",
            Step::move_to(Target::text("Go")).with_timeout(30),
        ),
        10,
        spawn_late,
    );
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(activations, 1);
}

#[test]
fn a_scenario_waits_sixty_frames_unless_it_says_otherwise() {
    assert_eq!(DEFAULT_ACTION_TIMEOUT, 60);
    let scenario = Scenario::new("default");
    assert_eq!(scenario.action_timeout_frames(), DEFAULT_ACTION_TIMEOUT);
    assert_eq!(scenario.action_timeout(5).action_timeout_frames(), 5);
}

#[test]
fn a_step_waiting_for_its_target_is_not_at_a_step_boundary() {
    let scenario = click_go("boundary").setup(|world| {
        world.init_resource::<Activations>();
        world.spawn(Camera2d);
    });
    let mut app = scenario.headless_app();
    app.insert_resource(AtFrame(10));
    Plugin::build(&rin_bevy_ui_tester::ScenarioPlayerPlugin, &mut app);
    app.add_systems(Update, spawn_late);
    scenario
        .run_setup(app.world_mut())
        .expect("the setup builds");
    app.update();
    app.update();
    app.world_mut()
        .resource_mut::<ScenarioPlayer>()
        .start(scenario);
    for _ in 0..4 {
        app.update();
    }
    let player = app.world().resource::<ScenarioPlayer>();
    assert!(player.is_active());
    assert_eq!(player.steps_started(), 1, "the move has begun");
    assert!(
        !player.at_step_boundary(),
        "a move waiting for its target is in the middle of its step"
    );
}

#[test]
fn the_action_timeout_and_the_step_wrappers_read_and_write_as_ron() {
    let text = r#"(
        format_version: 3,
        name: "patient",
        action_timeout: 30,
        steps: [
            Timeout(120, MoveTo(Text("Go"))),
            Force(Click(Left)),
        ],
    )"#;
    let scenario = Scenario::from_ron(text).expect("the script loads");
    assert_eq!(scenario.action_timeout_frames(), 30);
    assert_eq!(
        scenario.script(),
        [
            Step::move_to(Target::text("Go")).with_timeout(120),
            Step::click().forced(),
        ]
    );
    let written = scenario.to_ron().expect("the script is written");
    assert!(written.contains("action_timeout: 30"), "{written}");
    assert_eq!(
        Script::from_ron(&written).expect("it reads back"),
        scenario.to_script()
    );
    let old = r#"(format_version: 3, name: "old", steps: [MoveTo(Text("Go"))])"#;
    let old = Scenario::from_ron(old).expect("a file without them still loads");
    assert_eq!(old.action_timeout_frames(), DEFAULT_ACTION_TIMEOUT);
    assert!(
        !old.to_ron().expect("written").contains("action_timeout"),
        "the default is not written out"
    );
}

#[test]
fn a_timeout_around_a_step_that_does_not_wait_is_refused_when_the_script_loads() {
    let text = r#"(format_version: 3, name: "odd", steps: [Timeout(5, Wait(2))])"#;
    let error = Scenario::from_ron_with(text, &Registry::default())
        .expect_err("a wait has no target to wait for")
        .to_string();
    assert!(error.starts_with("step 0: "), "{error}");
    assert!(error.contains("Within(frames, ..)"), "{error}");
}

#[test]
fn the_wrappers_read_as_the_step_inside_them() {
    let descriptions = TargetDescriptions::default();
    let before = [Step::move_to(Target::text("Go"))];
    assert_eq!(
        describe_step(&Step::click().forced(), &before, &descriptions),
        format!(
            "{} (forced)",
            describe_step(&Step::click(), &before, &descriptions)
        )
    );
    assert!(
        describe_step(
            &Step::move_to(Target::text("Go")).with_timeout(9),
            &[],
            &descriptions
        )
        .ends_with("(waits up to 9 frames)")
    );
}
