//! The offscreen recorder. Every test that plays a scenario needs a (software)
//! GPU, so those are ignored by `just test` and run by `just test-gpu`; a
//! scenario that is only skipped records nothing and runs without one.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "tests may panic on a broken setup"
)]

mod common;

use std::{fs, path::PathBuf};

use bevy::{camera::RenderTarget, prelude::*};
use common::{Activations, spawn_ui};
use rin_bevy_ui_tester::{
    CaptureMeta, Captures, Check, Frame, Recorder, RecorderPlugins, Scenario, Step, Target,
    Variant,
    overlay::{OverlayTheme, PickHighlight},
    recorder::request_capture,
};

/// A scenario over the plain UI that clicks "Pause" and checks it was
/// activated, or, when `broken`, expects it twice.
fn click_pause(name: &str, broken: bool) -> Scenario {
    let wanted: u32 = if broken { 2 } else { 1 };
    Scenario::new(name)
        .window_size(320, 240)
        .setup(spawn_ui)
        .steps([
            Step::caption("Click the button"),
            Step::move_to(Target::text("Pause")),
            Step::glide(Target::text("Pause"), 6),
            Step::click(),
            Step::wait(2),
            Step::check(Check::resource::<Activations>(".0").eq(wanted)),
        ])
}

/// A fresh output directory under cargo's scratch space.
fn output_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// Runs updates until every picture asked for has come back.
fn wait_for_pictures(app: &mut App) {
    for _ in 0..240 {
        if app
            .world()
            .get_resource::<Captures>()
            .is_some_and(|captures| captures.outstanding() == 0)
        {
            return;
        }
        app.update();
    }
    panic!("the picture never came back from the GPU");
}

#[test]
#[ignore = "needs a GPU: run by `just test-gpu`"]
fn gpu_a_click_reaches_a_button_through_an_image_targeted_camera() {
    let scenario = click_pause("click through an image", false);
    let mut app = App::new();
    app.add_plugins(RecorderPlugins::default().with_window_size(320, 240));
    app.finish();
    app.cleanup();

    let report = scenario.run_in(&mut app);

    assert!(report.passed(), "failures: {:?}", report.failures());
    assert_eq!(
        app.world().resource::<Activations>().0,
        1,
        "the button was activated once"
    );
    let mut cameras = app.world_mut().query::<&RenderTarget>();
    assert!(
        cameras
            .iter(app.world())
            .all(|target| matches!(target, RenderTarget::Image(_))),
        "every camera renders to the image, none to the window"
    );

    // And the picture really holds the UI: the white button on the clear
    // colour, and the overlay's caption strip over it.
    request_capture(
        app.world_mut(),
        CaptureMeta {
            frame: 1,
            steps: Vec::new(),
            step_text: String::new(),
            checks_text: String::new(),
            caption: None,
            failed: false,
        },
    );
    wait_for_pictures(&mut app);
    let captures = app.world().resource::<Captures>();
    let picture: &Frame = &captures
        .captures()
        .next()
        .expect("one picture came back")
        .frame;
    assert_eq!((picture.width(), picture.height()), (320, 240));
    let button = picture.pixel(120, 40).expect("inside the frame");
    assert!(
        button.iter().take(3).all(|c| *c > 240),
        "button is white: {button:?}"
    );
    let background = picture.pixel(280, 40).expect("inside the frame");
    assert!(
        background.iter().take(3).all(|c| *c < 128),
        "clear colour is dark: {background:?}"
    );
}

#[test]
#[ignore = "needs a GPU: run by `just test-gpu`"]
fn gpu_recording_a_plain_ui_scenario_writes_a_filmstrip_a_gif_and_an_index() {
    let dir = output_dir("recorder-plain-ui");
    let recording = Recorder::new(|_app| {})
        .every(5)
        .grouped("Buttons", click_pause("click the pause button", false))
        .grouped("Buttons", click_pause("click it twice", true))
        .record(&dir)
        .expect("recording works");

    assert!(recording.any_failed(), "the broken scenario is reported");
    let passed: Vec<_> = recording
        .scenarios
        .iter()
        .map(rin_bevy_ui_tester::ScenarioRecording::passed)
        .collect();
    assert_eq!(passed, [true, false], "one passed, one failed");
    for slug in ["click-the-pause-button", "click-it-twice"] {
        let folder = dir.join(slug);
        let png = fs::read(folder.join("filmstrip.png")).expect("a filmstrip");
        assert_eq!(png.get(..4), Some(&[137, 80, 78, 71][..]), "a PNG");
        let gif = fs::read(folder.join("scenario.gif")).expect("a GIF");
        assert_eq!(gif.get(..3), Some(&b"GIF"[..]), "a GIF");
        assert!(folder.join("frame-00000.png").exists(), "frame 0 is taken");
    }
    let index = fs::read_to_string(&recording.index).expect("an index");
    assert!(
        index.contains("Buttons"),
        "grouped by the catalogue's groups"
    );
    let failed = index.find("id=\"click-it-twice\"").expect("failed card");
    let passed = index
        .find("id=\"click-the-pause-button\"")
        .expect("passed card");
    assert!(failed < passed, "failures come first");
}

#[test]
#[ignore = "needs a GPU: run by `just test-gpu`"]
fn gpu_a_failed_scenario_gets_a_trace_page_with_snapshots_and_pictures_per_step() {
    let dir = output_dir("recorder-trace");
    let recording = Recorder::new(|_app| {})
        .every(5)
        .grouped("Buttons", click_pause("click it twice", true))
        .record(&dir)
        .expect("recording works");

    let folder = dir.join("click-it-twice");
    let page = fs::read_to_string(folder.join("trace.html")).expect("a trace page");
    for step in 0..6 {
        assert!(page.contains(&format!("id=\"step-{step}\"")), "step {step}");
    }
    assert!(
        page.contains("`Activations.0` is 1, expected 2"),
        "the failure is shown"
    );
    assert!(
        page.contains("Activations"),
        "the snapshot lists the resource"
    );
    assert!(page.contains("<img"), "the steps show pictures");
    let trace = recording
        .scenarios
        .first()
        .and_then(|scenario| scenario.trace.as_ref())
        .expect("the recording keeps the trace");
    // Every picture the page refers to was written next to it.
    for point in trace.points() {
        let name = format!("frame-{:05}.png", point.frame);
        if page.contains(&name) {
            assert!(folder.join(&name).exists(), "{name} is on disk");
        }
    }
    let index = fs::read_to_string(&recording.index).expect("an index");
    assert!(
        index.contains("click-it-twice/trace.html#step-"),
        "the failed card lists its steps"
    );
}

#[test]
#[ignore = "needs a GPU: run by `just test-gpu`"]
fn gpu_recorded_frame_zero_is_the_same_on_every_run() {
    let first = output_dir("recorder-determinism-a");
    let second = output_dir("recorder-determinism-b");
    for dir in [&first, &second] {
        Recorder::new(|_app| {})
            .scenario(click_pause("determinism", false))
            .record(dir)
            .expect("recording works");
    }
    let frame_zero =
        |dir: &PathBuf| fs::read(dir.join("determinism/frame-00000.png")).expect("frame 0");
    assert_eq!(
        frame_zero(&first),
        frame_zero(&second),
        "frame 0 is identical"
    );
}

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

#[test]
#[ignore = "needs a GPU: run by `just test-gpu`"]
fn gpu_the_pick_outlines_are_drawn_just_outside_the_node_in_a_recorded_frame() {
    let dir = output_dir("recorder-pick-outline");
    let scenario = Scenario::new("pick outline")
        .window_size(320, 240)
        .setup(|world| {
            spawn_ui(world);
            let label = world
                .query::<(Entity, &Text)>()
                .iter(world)
                .find(|(_, text)| text.0 == "Pause")
                .expect("the label")
                .0;
            let button = world.get::<ChildOf>(label).expect("its button").parent();
            world.insert_resource(PickHighlight {
                hovered: Some(button),
                selected: Some(button),
                tag: "Text(\"Pause\")".into(),
            });
        })
        .steps([Step::wait(3)]);
    Recorder::new(|_app| {})
        .scenario(scenario)
        .record(&dir)
        .expect("recording works");
    let path = dir.join("pick-outline/frame-00000.png");
    let picture = image::open(&path).expect("frame 0").to_rgba8();
    let pixel = |x: u32, y: u32| picture.get_pixel(x, y).0;
    let theme = OverlayTheme::default();
    // The button spans x 100..200 and y 25..75; the hovered outline is the
    // two pixels just outside it, the picked one the two beyond those.
    for (x, y) in [(99, 50), (150, 23), (201, 50), (150, 76)] {
        assert!(
            near(pixel(x, y), srgb8(theme.pick_hovered)),
            "hovered outline at ({x}, {y}): {:?} in {}",
            pixel(x, y),
            path.display()
        );
    }
    for (x, y) in [(97, 50), (202, 60)] {
        assert!(
            near(pixel(x, y), srgb8(theme.pick_selected)),
            "picked outline at ({x}, {y}): {:?}",
            pixel(x, y)
        );
    }
    assert!(
        pixel(150, 50).iter().take(3).all(|channel| *channel > 200)
            || pixel(110, 35).iter().take(3).all(|channel| *channel > 200),
        "the button itself is not covered"
    );
}

/// The width and height in a PNG's header.
fn png_size(path: &std::path::Path) -> (u32, u32) {
    let bytes = fs::read(path).expect("a picture");
    let number = |at: usize| {
        let word: [u8; 4] = bytes
            .get(at..at + 4)
            .and_then(|slice| slice.try_into().ok())
            .expect("a PNG header");
        u32::from_be_bytes(word)
    };
    (number(16), number(20))
}

#[test]
#[ignore = "needs a GPU: run by `just test-gpu`"]
fn gpu_recording_variants_writes_one_entry_per_scenario_and_variant() {
    let dir = output_dir("recorder-variants");
    let recording = Recorder::new(|_app| {})
        .every(5)
        .variants([
            Variant::default(),
            Variant::new("retina").scale_factor(2.0),
            Variant::new("small").window_size(240, 180),
        ])
        .grouped("Buttons", click_pause("click the pause button", false))
        .record(&dir)
        .expect("recording works");

    let names: Vec<_> = recording
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "click the pause button",
            "click the pause button [retina]",
            "click the pause button [small]"
        ]
    );
    let slugs: std::collections::BTreeSet<_> = recording
        .scenarios
        .iter()
        .map(|scenario| scenario.slug.clone())
        .collect();
    assert_eq!(slugs.len(), 3, "each entry has a folder of its own");
    let sizes: Vec<_> = recording
        .scenarios
        .iter()
        .map(|scenario| png_size(&dir.join(&scenario.slug).join("frame-00000.png")))
        .collect();
    assert_eq!(
        sizes,
        [(320, 240), (640, 480), (240, 180)],
        "the picture has the window's physical size"
    );
    for scenario in &recording.scenarios {
        // A picture at another scale factor than the cameras' target is
        // black, the right size and empty.
        let picture = image::open(dir.join(&scenario.slug).join("frame-00005.png"))
            .expect("a picture")
            .to_rgba8();
        let first = picture.pixels().next().expect("a pixel");
        assert!(
            picture.pixels().any(|pixel| pixel != first),
            "{} has something drawn in it",
            scenario.slug
        );
    }
    assert!(!recording.any_failed(), "all three pass");
}

#[test]
#[ignore = "needs a GPU: run by `just test-gpu`"]
fn gpu_a_variant_a_scenario_cannot_run_in_is_recorded_as_a_failed_entry() {
    let dir = output_dir("recorder-variant-refused");
    let fixed = Scenario::new("fixed spot")
        .window_size(320, 240)
        .setup(spawn_ui)
        .step(Step::move_to(Target::screen(Vec2::new(150.0, 50.0))));
    let recording = Recorder::new(|_app| {})
        .variants([
            Variant::default(),
            Variant::new("small").window_size(240, 180),
        ])
        .grouped("Buttons", fixed)
        .record(&dir)
        .expect("recording works");
    let passed: Vec<_> = recording
        .scenarios
        .iter()
        .map(rin_bevy_ui_tester::ScenarioRecording::passed)
        .collect();
    assert_eq!(passed, [true, false]);
    let failure = recording
        .scenarios
        .get(1)
        .and_then(|scenario| scenario.report.setup_failure.clone())
        .expect("the refusal is the setup failure");
    assert!(failure.contains("step 0"), "{failure}");
}

#[test]
fn a_scenario_restricted_to_another_variant_is_recorded_as_skipped() {
    let dir = output_dir("recorder-variant-skipped");
    // A scenario limited to the default variant, recorded only in a named one:
    // nothing plays, so no GPU is needed.
    let scenario = Scenario::new("only the default").variants(["default"]);
    let recording = Recorder::new(|_app| {})
        .variants([Variant::new("wide")])
        .scenario(scenario)
        .record(&dir)
        .expect("recording works");

    assert!(
        !recording.any_failed(),
        "a skip is nothing to fail over: {:?}",
        recording
            .scenarios
            .iter()
            .map(|scenario| scenario.report.failures())
            .collect::<Vec<_>>()
    );
    let recorded = recording.scenarios.first().expect("one entry");
    assert!(recorded.skipped(), "it did not run in this variant");
    assert!(!recorded.passed(), "it never finished");
    assert!(
        !dir.join(&recorded.slug).exists(),
        "nothing is written for a skipped entry"
    );
    let index = fs::read_to_string(&recording.index).expect("an index");
    assert!(index.contains("SKIPPED"), "the index says so: {index}");
    assert!(
        index.contains("1 skipped") && !index.contains("1 failed"),
        "counted as skipped, not failed: {index}"
    );
}

#[test]
#[ignore = "needs a GPU: run by `just test-gpu`"]
fn gpu_the_trace_page_and_the_index_name_the_group_of_each_step() {
    let dir = output_dir("recorder-groups");
    let grouped = Scenario::new("grouped click")
        .window_size(320, 240)
        .setup(spawn_ui)
        .steps([
            Step::group(
                "press the button",
                [
                    Step::move_to(Target::text("Pause")),
                    Step::group("click it", [Step::click()]),
                ],
            ),
            Step::group(
                "see the result",
                [
                    Step::wait(2),
                    Step::check(Check::resource::<Activations>(".0").eq(2_u32)),
                ],
            ),
        ]);
    let recording = Recorder::new(|_app| {})
        .every(5)
        .grouped("Buttons", grouped)
        .record(&dir)
        .expect("recording works");
    assert!(recording.any_failed());

    let page =
        fs::read_to_string(dir.join("grouped-click").join("trace.html")).expect("a trace page");
    for step in 0..4 {
        assert!(page.contains(&format!("id=\"step-{step}\"")), "step {step}");
    }
    assert!(
        page.contains("press the button &gt; click it &gt; 2."),
        "a nested step is titled with its path"
    );
    assert!(
        page.contains("see the result &gt; 4."),
        "so is a check inside a group"
    );
    let index = fs::read_to_string(&recording.index).expect("an index");
    assert!(
        index.contains("see the result &gt; step 3"),
        "the failure names the group: {index}"
    );
    assert!(
        index.contains("grouped-click/trace.html#step-"),
        "the failed card lists its steps"
    );
}
