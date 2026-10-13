//! The cursor overlay and caption strip over a plain Bevy UI, headless.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    camera::{Camera, RenderTarget, visibility::Visibility},
    ecs::query::Or,
    input::keyboard::KeyCode,
    prelude::*,
    ui::{UiTargetCamera, px, widget::Text},
    ui_widgets::{Activate, Button},
    window::WindowRef,
};
use rin_bevy_ui_tester::{
    AppCheck, AppTarget, Check, CheckResult, HeadlessPlugins, Probe, ReflectAppCheck,
    ReflectAppTarget, ResolvedTarget, Scenario, Step, SyntheticInput, Target, TargetDescriptions,
    describe_step,
    overlay::{
        CursorOverlayPlugin, OverlayCamera, OverlayCaptionStrip, OverlayChecksText, OverlayCursor,
        OverlayHeld, OverlayRing, OverlayRoot, OverlayStepText, OverlayTheme, OverlayTrailDot,
    },
};

/// How many times the button was activated.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Activations(u32);

/// This test's own checks, over the overlay.
#[derive(Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(AppCheck)]
enum OverlayCheck {
    /// The overlay exists and took none of the picking hits.
    NotHovered,
    /// Never holds.
    Never,
}

impl AppCheck for OverlayCheck {
    fn run(&self, probe: &Probe<'_>) -> CheckResult {
        match self {
            Self::NotHovered => {
                let overlay = overlay_entities(probe.world());
                if overlay.is_empty() {
                    return Err("the overlay should exist".to_owned());
                }
                let hovered = probe.hovered();
                if hovered.is_empty() {
                    return Err("the button should be hovered".to_owned());
                }
                if hovered.iter().any(|entity| overlay.contains(entity)) {
                    return Err("the overlay took a picking hit".to_owned());
                }
                Ok(())
            }
            Self::Never => Err("always fails".to_owned()),
        }
    }
}

/// Spawns a camera and a "Pause" button.
fn spawn_ui(world: &mut World) {
    world.init_resource::<Activations>();
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
}

/// A finished headless app with the overlay on top.
fn overlay_app() -> App {
    let mut app = App::new();
    app.add_plugins((HeadlessPlugins::default(), CursorOverlayPlugin));
    app.finish();
    app.cleanup();
    app
}

/// The position of the arrow's top-left corner, as laid out.
fn cursor_position(app: &mut App) -> Option<Vec2> {
    let world = app.world_mut();
    let mut query = world.query_filtered::<&Node, With<OverlayCursor>>();
    let node = query.iter(world).next()?;
    match (node.left, node.top) {
        (Val::Px(left), Val::Px(top)) => Some(Vec2::new(left, top)),
        _ => None,
    }
}

/// Every entity the overlay draws.
fn overlay_entities(world: &World) -> Vec<Entity> {
    let Some(query) = world.try_query_filtered::<Entity, Or<(
        With<OverlayCursor>,
        With<OverlayHeld>,
        With<OverlayRing>,
        With<OverlayTrailDot>,
        With<OverlayCaptionStrip>,
    )>>() else {
        return Vec::new();
    };
    query.iter_manual(world).collect()
}

#[test]
fn the_overlay_cursor_follows_the_scripted_pointer() {
    let mut app = overlay_app();
    let scenario = Scenario::new("follow")
        .setup(spawn_ui)
        .step(Step::move_to(Target::screen(Vec2::new(300.0, 200.0))))
        .step(Step::wait(1));
    let report = scenario.run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(cursor_position(&mut app), Some(Vec2::new(300.0, 200.0)));

    let report = Scenario::new("again")
        .setup(spawn_ui)
        .step(Step::move_to(Target::screen(Vec2::new(640.0, 360.0))))
        .step(Step::wait(1))
        .run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(cursor_position(&mut app), Some(Vec2::new(640.0, 360.0)));
}

#[test]
fn the_overlay_takes_no_picking_hits_and_clicks_reach_the_app() {
    let scenario = Scenario::new("no hits").setup(spawn_ui).steps([
        Step::move_to(Target::text("Pause")),
        Step::click(),
        Step::wait(1),
        Step::check(Check::app(OverlayCheck::NotHovered)),
        Step::check(Check::resource::<Activations>(".0").eq(1_u32)),
    ]);
    let report = scenario.run_in(&mut overlay_app());
    assert!(report.passed(), "{:?}", report.failures());
}

#[test]
fn a_press_ring_appears_on_a_click_and_fades_away() {
    let mut app = overlay_app();
    let report = Scenario::new("ring")
        .setup(spawn_ui)
        .steps([Step::move_to(Target::text("Pause")), Step::click()])
        .run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());
    let rings = |app: &mut App| {
        let world = app.world_mut();
        world.query::<&OverlayRing>().iter(world).count()
    };
    assert_eq!(rings(&mut app), 1, "the click should start a ring");
    for _ in 0..rin_bevy_ui_tester::overlay::RING_FRAMES {
        app.update();
    }
    assert_eq!(rings(&mut app), 0, "the ring should fade away");
}

#[test]
fn the_held_marker_shows_only_while_a_button_is_down() {
    let mut app = overlay_app();
    let held_visible = |app: &mut App| {
        let world = app.world_mut();
        world
            .query_filtered::<&Visibility, With<OverlayHeld>>()
            .iter(world)
            .next()
            .is_some_and(|visibility| *visibility != Visibility::Hidden)
    };
    let report = Scenario::new("held")
        .setup(spawn_ui)
        .steps([
            Step::move_to(Target::text("Pause")),
            Step::press(),
            Step::wait(1),
        ])
        .run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());
    assert!(held_visible(&mut app), "held while the button is down");
    let report = Scenario::new("released")
        .setup(spawn_ui)
        .steps([Step::release(), Step::wait(1)])
        .run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());
    assert!(!held_visible(&mut app), "hidden once it is released");
}

#[test]
fn the_caption_strip_names_the_step_and_marks_each_check() {
    let mut app = overlay_app();
    let report = Scenario::new("caption")
        .setup(spawn_ui)
        .steps([
            Step::move_to(Target::text("Pause")),
            Step::click(),
            Step::check(Check::resource::<Activations>(".0").eq(1_u32)),
            Step::check(Check::app(OverlayCheck::Never)),
        ])
        .run_in(&mut app);
    assert!(!report.passed());
    app.update();
    let world = app.world_mut();
    let step = world
        .query_filtered::<&Text, With<OverlayStepText>>()
        .iter(world)
        .map(|text| text.0.clone())
        .next()
        .expect("a step line");
    assert_eq!(step, "Check: Never", "the last step that ran");
    let checks = world
        .query_filtered::<&Text, With<OverlayChecksText>>()
        .iter(world)
        .map(|text| text.0.clone())
        .next()
        .expect("a checks line");
    assert_eq!(checks, "PASS `Activations.0` is 1  FAIL Never");
}

#[test]
fn steps_read_in_words() {
    let words = TargetDescriptions::default();
    let glide = Step::glide(Target::screen(Vec2::new(5.0, 0.0)), 10);
    assert_eq!(describe_step(&glide, &[], &words), "Glide to (5, 0)");
    let tap = Step::tap(KeyCode::Equal);
    assert_eq!(describe_step(&tap, &[], &words), "Tap =");
    let burst = Step::move_burst([Vec2::new(1.0, 2.0), Vec2::new(3.0, 4.0)]);
    assert_eq!(
        describe_step(&burst, &[], &words),
        "Move burst of 2 positions"
    );
    let before = [Step::move_to(Target::text("Rail"))];
    assert_eq!(
        describe_step(&Step::click(), &before, &words),
        "Click: Rail"
    );
}

/// An app's own targets, for the caption strip's words.
#[derive(Reflect, Debug, Clone)]
#[reflect(AppTarget)]
enum LatticeTarget {
    /// A lattice point.
    Point(i32, i32),
}

impl AppTarget for LatticeTarget {
    fn resolve(&self, _world: &World) -> Result<ResolvedTarget, String> {
        Err("not on screen in this test".into())
    }
}

#[test]
fn an_apps_target_reads_as_its_ron_unless_the_app_words_it() {
    let step = Step::glide(Target::app(LatticeTarget::Point(3, 4)), 5);
    assert_eq!(
        describe_step(&step, &[], &TargetDescriptions::default()),
        "Glide to Point(3,4)"
    );
    let mut words = TargetDescriptions::default();
    words.add(|target| match target {
        Target::App(value) => value
            .value()?
            .downcast_ref::<LatticeTarget>()
            .map(|LatticeTarget::Point(x, y)| format!("lattice point {x}, {y}")),
        _ => None,
    });
    assert_eq!(
        describe_step(&step, &[], &words),
        "Glide to lattice point 3, 4"
    );
    let other = Step::glide(Target::text("Rail"), 5);
    assert_eq!(describe_step(&other, &[], &words), "Glide to Rail");
}

#[test]
fn naming_an_overlay_camera_aims_every_overlay_node_at_it() {
    let mut app = overlay_app();
    let scenario = Scenario::new("aimed")
        .setup(|world| {
            spawn_ui(world);
            let second = world.spawn(Camera2d).id();
            world.insert_resource(OverlayCamera(Some(second)));
        })
        .steps([
            Step::move_to(Target::text("Pause")),
            Step::click(),
            Step::wait(2),
        ]);
    let report = scenario.run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());

    let world = app.world_mut();
    let wanted = world
        .resource::<OverlayCamera>()
        .0
        .expect("the camera was named");
    let mut roots = world.query_filtered::<(Entity, Option<&UiTargetCamera>), With<OverlayRoot>>();
    let roots: Vec<_> = roots.iter(world).collect();
    assert!(
        roots.len() > 10,
        "cursor, trail, strip and a ring: {}",
        roots.len()
    );
    assert!(
        roots
            .iter()
            .all(|(_, aimed)| aimed.is_some_and(|aimed| aimed.0 == wanted)),
        "every overlay root, rings included, targets the named camera"
    );
}

#[test]
fn without_an_overlay_camera_the_overlay_aims_at_the_default_ui_camera() {
    let mut app = overlay_app();
    let report = Scenario::new("default")
        .setup(spawn_ui)
        .step(Step::wait(2))
        .run_in(&mut app);
    assert!(report.passed(), "{:?}", report.failures());
    let world = app.world_mut();
    let camera = world
        .query_filtered::<Entity, With<Camera2d>>()
        .single(world)
        .expect("the app's camera");
    let mut roots = world.query_filtered::<Option<&UiTargetCamera>, With<OverlayRoot>>();
    let roots: Vec<_> = roots.iter(world).collect();
    assert!(!roots.is_empty());
    assert!(
        roots
            .iter()
            .all(|aimed| aimed.is_some_and(|aimed| aimed.0 == camera)),
        "every root aims at the default UI camera"
    );
}

/// Records every parentless node as it is added: whether it carries a
/// `UiTargetCamera` and whether it is the overlay's. This is what an app's
/// `UiRoot` observer sees.
#[derive(Resource, Default)]
struct AddedRoots(Vec<AddedRoot>);

/// One parentless node, as it was when `Node` was added.
struct AddedRoot {
    /// Whether it already carried a `UiTargetCamera`.
    aimed: bool,
    /// Whether it is one of the overlay's.
    overlay: bool,
}

/// Notes a parentless node as it appears.
fn note_root(
    added: On<Add, Node>,
    nodes: Query<(Has<ChildOf>, Has<UiTargetCamera>, Has<OverlayRoot>)>,
    mut roots: ResMut<AddedRoots>,
) {
    if let Ok((child, aimed, overlay)) = nodes.get(added.entity)
        && !child
    {
        roots.0.push(AddedRoot { aimed, overlay });
    }
}

/// The overlay's roots that were added without a target camera.
fn unaimed_overlay_roots(app: &App) -> usize {
    app.world()
        .resource::<AddedRoots>()
        .0
        .iter()
        .filter(|root| root.overlay && !root.aimed)
        .count()
}

/// The overlay's roots now, with the camera each aims at.
fn overlay_aims(app: &mut App) -> Vec<Option<Entity>> {
    let world = app.world_mut();
    let mut roots = world.query_filtered::<Option<&UiTargetCamera>, With<OverlayRoot>>();
    roots
        .iter(world)
        .map(|aimed| aimed.map(|aimed| aimed.0))
        .collect()
}

#[test]
fn the_overlay_never_adds_a_root_without_a_target_camera() {
    let mut app = App::new();
    app.add_plugins((HeadlessPlugins::default(), CursorOverlayPlugin))
        .init_resource::<AddedRoots>()
        .add_observer(note_root);
    app.finish();
    app.cleanup();

    // No camera yet: the overlay waits.
    for _ in 0..3 {
        app.update();
    }
    assert!(
        overlay_aims(&mut app).is_empty(),
        "nothing without a camera"
    );
    assert_eq!(unaimed_overlay_roots(&app), 0);

    // A camera spawned in a later frame.
    let first = app.world_mut().spawn(Camera2d).id();
    app.update();
    app.update();
    let aims = overlay_aims(&mut app);
    assert!(aims.len() > 10, "the overlay exists now: {}", aims.len());
    assert!(aims.iter().all(|aim| *aim == Some(first)));

    // A press starts a ring, the one root that appears long after the rest.
    {
        let mut input = app.world_mut().resource_mut::<SyntheticInput>();
        input.move_cursor(Vec2::new(40.0, 30.0));
        input.press_button(MouseButton::Left);
    }
    app.update();
    let world = app.world_mut();
    let mut rings = world.query_filtered::<Option<&UiTargetCamera>, With<OverlayRing>>();
    let rings: Vec<_> = rings.iter(world).collect();
    assert_eq!(rings.len(), 1, "the press started a ring");
    assert!(rings[0].is_some_and(|aimed| aimed.0 == first));

    // Another camera takes over as the default UI camera (a despawned
    // camera needs the renderer's world to sync to, which this app lacks).
    let second = app
        .world_mut()
        .spawn((
            Camera2d,
            Camera {
                order: 1,
                ..Camera::default()
            },
        ))
        .id();
    app.update();
    app.update();
    let aims = overlay_aims(&mut app);
    assert!(aims.len() > 10);
    assert!(aims.iter().all(|aim| *aim == Some(second)));

    // No camera renders to the window any more: no overlay, so no unaimed root.
    for camera in [first, second] {
        app.world_mut()
            .entity_mut(camera)
            .insert(RenderTarget::None { size: UVec2::ONE });
    }
    app.update();
    app.update();
    assert!(overlay_aims(&mut app).is_empty());

    // And back again.
    app.world_mut()
        .entity_mut(first)
        .insert(RenderTarget::Window(WindowRef::Primary));
    app.update();
    app.update();
    let aims = overlay_aims(&mut app);
    assert!(aims.len() > 10);
    assert!(aims.iter().all(|aim| *aim == Some(first)));

    let roots = &app.world().resource::<AddedRoots>().0;
    assert!(
        roots.iter().any(|root| root.overlay),
        "the overlay was seen"
    );
    assert_eq!(
        unaimed_overlay_roots(&app),
        0,
        "no overlay root was ever added without a UiTargetCamera"
    );
}

#[test]
fn the_default_check_marks_are_ascii_the_bundled_font_can_draw() {
    let theme = OverlayTheme::default();
    for mark in [&theme.pass_mark, &theme.fail_mark, &theme.skipped_mark] {
        assert!(mark.is_ascii(), "{mark:?} would render as an empty box");
    }
}
