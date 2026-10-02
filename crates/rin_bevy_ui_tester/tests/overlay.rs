//! The cursor overlay and caption strip over a plain Bevy UI, headless.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

use bevy::{
    camera::visibility::Visibility,
    ecs::query::Or,
    input::keyboard::KeyCode,
    prelude::*,
    ui::{px, widget::Text},
    ui_widgets::{Activate, Button},
};
use rin_bevy_ui_tester::{
    HeadlessPlugins, Probe, Scenario, Step, Target, TargetDescriptions, describe_step,
    overlay::{
        CursorOverlayPlugin, OverlayCaptionStrip, OverlayChecksText, OverlayCursor, OverlayHeld,
        OverlayRing, OverlayStepText, OverlayTrailDot,
    },
};

/// How many times the button was activated.
#[derive(Resource, Default)]
struct Activations(u32);

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
    let scenario = Scenario::new("no hits")
        .setup(spawn_ui)
        .steps([
            Step::move_to(Target::text("Pause")),
            Step::click(),
            Step::wait(1),
            Step::check("overlay is not hovered"),
            Step::check("activated"),
        ])
        .check("overlay is not hovered", |probe: &Probe| {
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
        })
        .check("activated", |probe| {
            let count = probe.resource::<Activations>().map_or(0, |a| a.0);
            if count == 1 {
                Ok(())
            } else {
                Err(format!("activated {count} times"))
            }
        });
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
            Step::check("activated"),
            Step::check("never"),
        ])
        .check("activated", |probe| {
            probe.resource::<Activations>().is_some_and(|a| a.0 == 1)
        })
        .check("never", |_: &Probe| Err("always fails".to_owned()))
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
    assert_eq!(step, "Check: never", "the last step that ran");
    let checks = world
        .query_filtered::<&Text, With<OverlayChecksText>>()
        .iter(world)
        .map(|text| text.0.clone())
        .next()
        .expect("a checks line");
    assert_eq!(checks, "\u{2714} activated  \u{2718} never");
}

#[test]
fn steps_read_in_words() {
    let words = TargetDescriptions::default();
    let glide = Step::glide(Target::screen(Vec2::new(5.0, 0.0)), 10);
    assert_eq!(describe_step(&glide, &[], &words), "Glide to (5, 0)");
    let tap = Step::tap(KeyCode::Equal);
    assert_eq!(describe_step(&tap, &[], &words), "Tap =");
    let before = [Step::move_to(Target::text("Rail"))];
    assert_eq!(
        describe_step(&Step::click(), &before, &words),
        "Click: Rail"
    );
}

#[test]
fn an_app_can_override_how_its_targets_read() {
    let mut words = TargetDescriptions::default();
    words.add(|target| match target {
        Target::Custom { resolver, argument } if resolver == "lattice" => {
            Some(format!("lattice point {argument}"))
        }
        _ => None,
    });
    let step = Step::glide(Target::custom("lattice", "3,4"), 5);
    assert_eq!(
        describe_step(&step, &[], &words),
        "Glide to lattice point 3,4"
    );
    let other = Step::glide(Target::text("Rail"), 5);
    assert_eq!(describe_step(&other, &[], &words), "Glide to Rail");
}
