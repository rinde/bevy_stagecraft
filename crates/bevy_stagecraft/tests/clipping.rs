#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    picking::Pickable,
    prelude::*,
    ui::{CalculatedClip, FixedNode, Overflow, UiTransform, px},
    ui_widgets::{Activate, Button},
};
use bevy_stagecraft::{
    Check, HeadlessPlugins, Outcome, Scenario, ScenarioReport, Step, Target, node_at,
};

const SETTLE_FRAMES: u16 = 3;
const SHORT_TIMEOUT: u16 = 5;

#[derive(Resource, Default)]
struct Activations(u32);

fn count(_: On<Activate>, mut activations: ResMut<Activations>) {
    activations.0 += 1;
}

fn rotated_panel() -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: px(50),
            top: px(50),
            width: px(200),
            height: px(100),
            overflow: Overflow::clip(),
            ..Node::default()
        },
        UiTransform::from_rotation(Rot2::degrees(90.0)),
    )
}

fn small_button(left: i32, label: &str) -> impl Bundle {
    (
        Button,
        Node {
            position_type: PositionType::Absolute,
            left: px(left),
            top: px(10),
            width: px(40),
            height: px(30),
            ..Node::default()
        },
        BackgroundColor(Color::WHITE),
        children![Text::new(label.to_owned())],
    )
}

fn spawn_camera(world: &mut World) {
    world.init_resource::<Activations>();
    world.spawn(Camera2d);
}

fn play(scenario: &Scenario) -> (ScenarioReport, u32) {
    let mut app = scenario.headless_app();
    let report = scenario.run_in(&mut app);
    let activations = app.world().get_resource::<Activations>().map_or(0, |a| a.0);
    (report, activations)
}

fn failure(report: &ScenarioReport, index: usize) -> String {
    match &report.steps[index].outcome {
        Outcome::Fail(message) => message.clone(),
        other => panic!("step {index} should fail, it was {other:?}"),
    }
}

fn app_with(setup: impl FnOnce(&mut World)) -> App {
    let mut app = App::new();
    app.add_plugins(HeadlessPlugins::default());
    app.finish();
    app.cleanup();
    setup(app.world_mut());
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    app
}

fn text_node(app: &mut App, text: &str) -> Entity {
    let world = app.world_mut();
    world
        .query::<(Entity, &Text)>()
        .iter(world)
        .find(|(_, shown)| shown.0 == text)
        .expect("a node shows the text")
        .0
}

fn center(app: &App, entity: Entity) -> Vec2 {
    let world = app.world();
    let node = world.get::<ComputedNode>(entity).expect("laid out");
    let transform = world.get::<UiGlobalTransform>(entity).expect("positioned");
    transform.affine().translation * node.inverse_scale_factor
}

fn is_label_or_button(app: &App, hit: Entity, label: Entity) -> bool {
    hit == label
        || app
            .world()
            .get::<ChildOf>(label)
            .is_some_and(|parent| parent.parent() == hit)
}

fn rotated_clip(world: &mut World) {
    spawn_camera(world);
    world.spawn(rotated_panel()).with_children(|panel| {
        panel.spawn(small_button(150, "Shown")).observe(count);
        panel.spawn(small_button(220, "Away")).observe(count);
    });
}

#[test]
fn a_button_whose_middle_shows_in_a_rotated_clip_is_clicked() {
    let scenario = Scenario::new("rotated shown")
        .setup(rotated_clip)
        .steps([Step::move_to(Target::text("Shown")), Step::click()]);
    let (report, activations) = play(&scenario);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(activations, 1);
}

#[test]
fn a_button_whose_middle_is_clipped_away_by_a_rotated_clip_is_not_actionable() {
    let scenario = Scenario::new("rotated away")
        .setup(rotated_clip)
        .step(Step::move_to(Target::text("Away")).with_timeout(SHORT_TIMEOUT));
    let (report, activations) = play(&scenario);
    assert!(
        failure(&report, 0).contains("its middle is clipped away"),
        "{}",
        failure(&report, 0)
    );
    assert_eq!(activations, 0);
}

#[test]
fn picking_at_a_rotated_clip_finds_what_shows_and_nothing_where_it_is_clipped() {
    let mut app = app_with(rotated_clip);
    let shown = text_node(&mut app, "Shown");
    let away = text_node(&mut app, "Away");
    let hit = node_at(app.world(), center(&app, shown)).expect("the shown button is picked");
    assert!(is_label_or_button(&app, hit, shown));
    assert_eq!(node_at(app.world(), center(&app, away)), None);
}

fn fully_clipped(world: &mut World) {
    spawn_camera(world);
    world
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(50),
                top: px(50),
                width: px(200),
                height: px(100),
                overflow: Overflow::clip(),
                ..Node::default()
            },
            UiTransform::from_scale(Vec2::ZERO),
        ))
        .with_children(|panel| {
            panel.spawn(small_button(10, "Gone")).observe(count);
        });
}

#[test]
fn a_fully_clipped_button_is_never_actionable() {
    let scenario = Scenario::new("fully clipped")
        .setup(fully_clipped)
        .step(Step::move_to(Target::text("Gone")).with_timeout(SHORT_TIMEOUT));
    let (report, activations) = play(&scenario);
    assert!(matches!(report.steps[0].outcome, Outcome::Fail(_)));
    assert_eq!(activations, 0);
}

#[test]
fn a_fully_clipped_button_is_never_picked() {
    let mut app = app_with(fully_clipped);
    let gone = text_node(&mut app, "Gone");
    let world = app.world();
    assert!(matches!(
        world.get::<CalculatedClip>(gone),
        Some(CalculatedClip::FullyClipped)
    ));
    assert_eq!(node_at(world, center(&app, gone)), None);
    assert_eq!(node_at(world, Vec2::new(100.0, 75.0)), None);
}

fn fixed_in_clip(world: &mut World) {
    spawn_camera(world);
    world
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: px(50),
            top: px(50),
            width: px(100),
            height: px(100),
            overflow: Overflow::clip(),
            ..Node::default()
        })
        .with_children(|container| {
            container
                .spawn((
                    FixedNode,
                    Button,
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(300),
                        top: px(200),
                        width: px(80),
                        height: px(40),
                        ..Node::default()
                    },
                    BackgroundColor(Color::WHITE),
                    children![Text::new("Fixed")],
                ))
                .observe(count);
        });
}

#[test]
fn a_fixed_node_outside_its_parents_clip_is_clicked_where_it_lies_on_screen() {
    let scenario = Scenario::new("fixed clicked").setup(fixed_in_clip).steps([
        Step::move_to(Target::text("Fixed")),
        Step::check(Check::on_top(Target::text("Fixed"))),
        Step::click(),
    ]);
    let (report, activations) = play(&scenario);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(activations, 1);
}

#[test]
fn a_fixed_node_outside_its_parents_clip_is_picked() {
    let mut app = app_with(fixed_in_clip);
    let label = text_node(&mut app, "Fixed");
    let hit = node_at(app.world(), center(&app, label)).expect("the fixed node is picked");
    assert!(is_label_or_button(&app, hit, label));
}

#[test]
fn a_fixed_node_ignores_the_transform_of_its_parent() {
    let mut app = app_with(|world| {
        spawn_camera(world);
        world.spawn(rotated_panel()).with_children(|panel| {
            panel.spawn((
                FixedNode,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(300),
                    top: px(200),
                    width: px(80),
                    height: px(40),
                    ..Node::default()
                },
                BackgroundColor(Color::WHITE),
                children![Text::new("Fixed")],
            ));
        });
    });
    let label = text_node(&mut app, "Fixed");
    let middle = center(&app, label);
    assert!(
        (300.0..380.0).contains(&middle.x) && (200.0..240.0).contains(&middle.y),
        "{middle}"
    );
    assert!(node_at(app.world(), middle).is_some());
}

fn fixed_over_button(world: &mut World, z: i32, pickable: Pickable) {
    spawn_camera(world);
    world
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: px(50),
            top: px(50),
            width: px(200),
            height: px(100),
            overflow: Overflow::clip(),
            ..Node::default()
        })
        .with_children(|container| {
            container.spawn(small_button(20, "Go")).observe(count);
            container.spawn((
                FixedNode,
                Name::new("Cover"),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(60),
                    top: px(55),
                    width: px(100),
                    height: px(50),
                    ..Node::default()
                },
                BackgroundColor(Color::BLACK),
                GlobalZIndex(z),
                pickable,
            ));
        });
}

#[test]
fn a_fixed_node_painted_over_a_button_covers_it_for_on_top_and_the_pointer() {
    let scenario = Scenario::new("fixed covers")
        .setup(|world| fixed_over_button(world, 10, Pickable::default()))
        .steps([
            Step::wait(SETTLE_FRAMES),
            Step::check(!Check::on_top(Target::text("Go"))),
            Step::move_to(Target::text("Go")).with_timeout(SHORT_TIMEOUT),
        ]);
    let (report, activations) = play(&scenario);
    assert!(
        report.steps[1].outcome == Outcome::Pass,
        "{:?}",
        report.steps
    );
    let message = failure(&report, 2);
    assert!(message.contains("covered by `Cover`"), "{message}");
    assert_eq!(activations, 0);
}

#[test]
fn a_fixed_node_below_a_button_in_the_stack_does_not_cover_it() {
    let scenario = Scenario::new("fixed below")
        .setup(|world| fixed_over_button(world, -10, Pickable::default()))
        .steps([
            Step::wait(SETTLE_FRAMES),
            Step::check(Check::on_top(Target::text("Go"))),
            Step::move_to(Target::text("Go")),
            Step::click(),
        ]);
    let (report, activations) = play(&scenario);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(activations, 1);
}

#[test]
fn a_fixed_cover_that_lets_the_pointer_through_covers_for_on_top_but_not_the_pointer() {
    let scenario = Scenario::new("fixed passes pointer")
        .setup(|world| {
            fixed_over_button(
                world,
                10,
                Pickable {
                    should_block_lower: false,
                    is_hoverable: false,
                },
            );
        })
        .steps([
            Step::wait(SETTLE_FRAMES),
            Step::check(!Check::on_top(Target::text("Go"))),
            Step::move_to(Target::text("Go")),
            Step::click(),
        ]);
    let (report, activations) = play(&scenario);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(activations, 1);
}
