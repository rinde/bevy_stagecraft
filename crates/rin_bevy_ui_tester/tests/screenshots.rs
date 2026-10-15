//! Screenshot checks against stored baselines. Every test that renders needs
//! a (software) GPU, so those are ignored by `just test` and run by
//! `just test-gpu`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "tests may panic on a broken setup"
)]

use std::{fs, path::Path, path::PathBuf};

use bevy::prelude::*;
use rin_bevy_ui_tester::{
    Check, Recorder, RecorderPlugins, Scenario, ScreenshotSettings, Step, Target,
};

/// The colour of `colour` as 8-bit sRGB, as a picture holds it.
fn srgb8(colour: Color) -> [u8; 3] {
    let [red, green, blue, _] = colour.to_srgba().to_u8_array();
    [red, green, blue]
}

/// Whether `pixel` is within a few levels of `wanted` in every channel.
fn near(pixel: [u8; 4], wanted: [u8; 3]) -> bool {
    pixel
        .iter()
        .zip(wanted)
        .all(|(have, want)| have.abs_diff(want) <= 6)
}

/// A fresh output directory under cargo's scratch space.
fn output_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// A scenario over two solid-coloured nodes and no text, so its pictures do
/// not depend on a driver's glyph rendering, that moves the pointer onto the
/// inner node and compares the frame with the baseline `home`, kept in `dir`.
/// `inner` is the colour of the inner node.
fn solid_colours(dir: &Path, inner: Color) -> Scenario {
    Scenario::new("Solid colours")
        .window_size(320, 240)
        .with_base_dir(dir)
        .setup(move |world| {
            world.spawn(Camera2d);
            world
                .spawn((
                    Node {
                        width: percent(100),
                        height: percent(100),
                        ..Node::default()
                    },
                    BackgroundColor(Color::srgb_u8(200, 40, 40)),
                ))
                .with_child((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(100),
                        top: px(25),
                        width: px(100),
                        height: px(50),
                        ..Node::default()
                    },
                    BackgroundColor(inner),
                ));
        })
        .steps([
            Step::move_to(Target::screen(Vec2::new(150.0, 50.0))),
            Step::check(Check::screenshot("home")),
        ])
}

/// A recorder app, with screenshot baselines rewritten when `update`.
fn recorder_app(update: bool) -> App {
    let mut app = App::new();
    app.add_plugins(RecorderPlugins::default().with_window_size(320, 240));
    app.insert_resource(ScreenshotSettings { update });
    app.finish();
    app.cleanup();
    app
}

#[test]
fn a_screenshot_check_is_skipped_in_a_plain_headless_run() {
    let dir = output_dir("screenshots-headless");
    let report = solid_colours(&dir, Color::WHITE).run_headless();
    assert!(report.passed(), "failures: {:?}", report.failures());
    assert!(report.screenshots.is_empty());
    assert!(!dir.exists(), "nothing is written without a renderer");
}

#[test]
#[ignore = "needs a GPU: run by `just test-gpu`"]
fn gpu_a_screenshot_check_compares_with_its_baseline_and_writes_a_diff_on_a_change() {
    let dir = output_dir("recorder-screenshot-check");
    let blue = Color::srgb_u8(30, 60, 220);
    let baseline = dir.join("solid-colours/home.png");

    // No baseline yet: the first run fails and writes it.
    let first = solid_colours(&dir, blue).run_in(&mut recorder_app(false));
    assert!(!first.passed(), "a missing baseline fails the first run");
    assert!(baseline.exists(), "the picture is kept as the baseline");

    // The overlay's cursor is out of the picture: its tip is on the inner node.
    let picture = image::open(&baseline).expect("the baseline").to_rgba8();
    assert_eq!(picture.dimensions(), (320, 240));
    assert!(
        near(picture.get_pixel(150, 50).0, srgb8(blue)),
        "the overlay is hidden from the picture: {:?}",
        picture.get_pixel(150, 50)
    );
    assert!(near(picture.get_pixel(5, 5).0, [200, 40, 40]));

    // The same scene matches.
    let second = solid_colours(&dir, blue).run_in(&mut recorder_app(false));
    assert!(second.passed(), "failures: {:?}", second.failures());

    // One colour changes: the check fails and leaves the picture and a diff.
    let green = Color::srgb_u8(30, 200, 60);
    let changed = solid_colours(&dir, green).run_in(&mut recorder_app(false));
    assert!(!changed.passed());
    let actual = dir.join("solid-colours/home.actual.png");
    let diff = dir.join("solid-colours/home.diff.png");
    assert!(actual.exists() && diff.exists(), "{:?}", changed.failures());
    let diff_picture = image::open(&diff).expect("the diff").to_rgba8();
    assert_eq!(diff_picture.get_pixel(150, 50).0, [255, 0, 255, 255]);
    assert_ne!(diff_picture.get_pixel(5, 5).0, [255, 0, 255, 255]);

    // Updating rewrites the baseline, and the new colour then matches.
    let updated = solid_colours(&dir, green).run_in(&mut recorder_app(true));
    assert!(updated.passed(), "failures: {:?}", updated.failures());
    let after = solid_colours(&dir, green).run_in(&mut recorder_app(false));
    assert!(after.passed(), "failures: {:?}", after.failures());
}

#[test]
#[ignore = "needs a GPU: run by `just test-gpu`"]
fn gpu_the_index_links_the_pictures_of_a_screenshot_mismatch() {
    let dir = output_dir("recorder-screenshot-index");
    let baselines = dir.join("baselines");
    let blue = Color::srgb_u8(30, 60, 220);
    let _ = solid_colours(&baselines, blue).run_in(&mut recorder_app(false));
    let out = dir.join("out");
    Recorder::new(|_app| {})
        .scenario(solid_colours(&baselines, Color::srgb_u8(30, 200, 60)))
        .record(&out)
        .expect("recording works");
    let index = fs::read_to_string(out.join("index.html")).expect("the index");
    assert!(
        index.contains("solid-colours/snapshots/home.diff.png"),
        "{index}"
    );
    assert!(out.join("solid-colours/snapshots/home.diff.png").exists());
    assert!(out.join("solid-colours/snapshots/home.actual.png").exists());
    assert!(
        out.join("solid-colours/snapshots/home.baseline.png")
            .exists()
    );
}
