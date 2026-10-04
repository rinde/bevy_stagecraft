# rin-bevy-ui-tester

A UI test driver for Bevy. It runs scripted pointer and keyboard scenarios
against a Bevy UI, headless inside `cargo test`, so interface behaviour can be
asserted like any other code.

The same scenarios can be recorded offscreen for review and played back in a
gallery. The crate knows nothing about any game; it takes Bevy types only.

This is private software.

## Status

The crate is being built up ticket by ticket (see the tickets on GitHub).
Headless scenarios, the offscreen recorder, the gallery and recording a
scenario from a real session work today.

## Headless app and synthetic input

`HeadlessPlugins` runs a real Bevy app, UI included, with no GPU and no
display. `SyntheticInput` drives it like a mouse and keyboard:

```rust,ignore
let mut app = App::new();
app.add_plugins(HeadlessPlugins::default());
app.finish();
app.cleanup();
// ... spawn a Camera2d and a UI ...
app.world_mut().resource_mut::<SyntheticInput>().move_cursor(Vec2::new(150.0, 50.0));
app.update();
```

## Example scenario

A scenario is plain data: steps, with time counted in frames of 1/60 s, and
checks. `run_headless` builds a headless app, runs the setup hook, plays the
steps and returns a `ScenarioReport`.

```rust,ignore
use rin_bevy_ui_tester::{Check, Scenario, Step, Target};

#[test]
fn clicking_the_button_increments_the_counter() {
    let report = Scenario::new("counter")
        .setup(spawn_counter_ui)                 // build a plain Bevy UI
        .steps([
            Step::move_to(Target::text("Add")),  // fails if something covers it
            Step::click(),
            Step::wait(1),
            Step::check(Check::shows("1")),
            Step::check("counter is 1"),         // a check defined in Rust
        ])
        .check("counter is 1", |probe| probe.resource::<Counter>().is_some_and(|c| c.0 == 1))
        .expect_check(!Check::shows("Error"))   // at the end
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}
```

A `Check` is one of:

- a name (`Check::named`, or just a `&str`): a check defined in Rust with
  `Scenario::check` or `Scenario::expect`;
- `Check::custom(name, argument)`: a check defined in Rust that takes an
  argument (`Scenario::custom_check`);
- `Check::shows(text)`: some UI node of the app shows exactly this text;
- `Check::hovered(target)`: the target's widget, or something below it, is
  under the pointer;
- `Check::state(key, value)`: the app's `StateProbe` (`Scenario::with_state`)
  reports this value;
- `!check`: the check does not hold. A check that cannot run at all, such as
  a name nobody defined, still fails.

A check step takes no frames. A failed one fails its step but does not stop
the script. Reports name each check in words, such as `"1" is shown`.

Targets are `Target::screen`, `Target::text` (any Bevy UI toolkit),
`Target::world` and `Target::custom`, which an app backs with a resolver
registered in `TargetResolvers`. After moving onto a widget target the player
checks that the widget, or a descendant, is under the pointer, and otherwise
fails naming what covers it. A failed step skips the rest of the script.

## Scenarios as RON

`Scenario::to_script()` gives a `Script`, plain serde data (format version,
name, window size, setup, view, steps, expect list), and `Scenario::from_ron`
reads one back, so a scenario can be written in Rust or as a file and
converted either way. Closures cannot be serialised, so a script names its
checks defined in Rust, setups, views and custom targets, and a `Registry`
maps the names to code: `registry.checks.register(..)` for named checks,
`register_custom(..)` for checks that take an argument (the generic checks
`pointer_over_ui` and `pointer_over_nothing` are built in), and
`Registry::with_state` for `State` checks. A file naming something nobody
registered, or written in a `format_version` this crate does not read, is
rejected when it loads.

The current format is version 2. Version 1 files are still read: they are
version 2 files with only named checks.

```ron
(
    format_version: 2,
    name: "click pause",
    setup: Named("plain_ui"),
    steps: [
        Check(Not(Hovered(Text("Pause")))),
        Glide(target: Text("Pause"), frames: 20),
        Check(Hovered(Text("Pause"))),
        Click(Left),
        Tap(key: KeyP, modifiers: [Ctrl, Shift]),
        Check("activated once"),                 // a check defined in Rust
        Check(Custom(check: "activated", argument: "1")),
        Check(State(key: "activations", value: "1")),
    ],
    expect: ["activated once", Shows("Resume"), Not(Shows("Error"))],
)
```

A check defined in Rust is written as just its name. A mistake in a spelled
out check is reported as a syntax error at its position, listing the forms a
check can take.

```rust,ignore
let scenario = Scenario::from_ron_with(include_str!("x.ron"), &registry)?
    .expect("also true in Rust", |probe| probe.has_text("1"));
```

`scenario.to_ron()` / `export_ron(&script)` write pretty RON. A test target
with `harness = false` calling `run_ron_tests(dir, &registry, run)` turns
every `.ron` file in a directory into one test named after the file.

## Seeing what the script does

`CursorOverlayPlugin` draws the script's pointer for people watching: an arrow
at the cursor, a ring that expands and fades on each press, a held marker
while a button is down, a short fading trail, and a caption strip along the
bottom naming the current step ("Click: Rail", "Glide to (5, 0)", "Tap =") and
each check's pass or fail mark. Everything is UI with `Pickable::IGNORE`, so it
never takes a picking hit. Only the gallery and the recorder add it; headless
tests do not.

Replace the `OverlayTheme` resource to change colours, sizes, font and marks.
The default marks are the words `PASS` and `FAIL`, because the bundled font has
no `\u{2714}` or `\u{2718}` (they render as empty boxes); set a font that has
them if you want the glyphs. Add describers to `TargetDescriptions` to word an
app's own custom targets.

The overlay draws through Bevy's default UI camera, which is never a camera
that renders to an image. If your cameras have custom targets, set the
`OverlayCamera` resource to name the one the overlay should use.

## Recording for review (feature `record`)

```rust
use rin_bevy_ui_tester::Recorder;

let recording = Recorder::new(|app| {
    // Your own plugins and resources go on top of `RecorderPlugins`.
    app.add_plugins(MyUiPlugin);
})
.every(10) // a picture every 10th frame, as well as at each step boundary
.settle(|world| my_world_has_settled(world)) // optional
.grouped("Buttons", click_pause_scenario())
.record("target/recordings")?;
if recording.any_failed() {
    std::process::exit(1);
}
```

Per scenario it writes `frame-NNNNN.png`, `filmstrip.png` (thumbnails with a
tick or cross band and the frame number; the step and checks are in the
pictures, drawn by the overlay) and `scenario.gif`. Overall it writes
`index.html`, a contact sheet grouped by the groups you gave, groups with a
failure first and failures first within each. There is no golden-image
comparison; a failed check fails the scenario, and `any_failed()` tells the
calling binary to exit non-zero.

`RecorderPlugins` is the render set: a headless app with the real renderer and
`bevy_ui_render`, every camera that would render to the primary window pointed
at an offscreen `Image` of the scenario's window size, and the synthetic
pointer aimed at that image. The primary window stays virtual and carries the
cursor position. The camera that would have been the default UI camera is
marked `IsDefaultUiCamera`, since Bevy never picks a camera that renders to an
image. Do not add the player or the overlay plugin yourself.

It needs a GPU, or lavapipe without one: `WGPU_BACKEND=vulkan` after
`sudo apt-get install -y mesa-vulkan-drivers libvulkan1`. `just test-gpu` runs
the recorder's tests.

## The gallery (feature `gallery`)

Watch scenarios play in a real app, in a loop. `GalleryPlugin` goes into an
ordinary windowed app (for example `DefaultPlugins`) next to the app's own
plugins. The app under test runs untouched in the primary window; a second OS
window, **Scenarios**, with its own UI camera, holds the catalogue (grouped,
with descriptions and each entry's last result), play and pause, step one
frame, step to the next step, restart, speed (1/4x, 1/2x, 1x, 2x), loop on and
off, **Take over**, **Export RON**, and the live list of the selected scenario's
checks. Space plays and pauses, Right steps a frame, Down steps to the next
step and Left restarts, read from the Scenarios window only.

```rust,ignore
App::new()
    .add_plugins(DefaultPlugins)
    .add_plugins(
        GalleryPlugin::new(
            Gallery::new().group("Counter", [GalleryEntry::new(scenario).described("Press +1")]),
        )
        .with_reset(|world| { /* despawn the app's UI, reset its resources */ }),
    )
    .run();
```

Picking a scenario calls the app's reset hook, then the scenario's setup and
view, resizes the primary window to the scenario's window size and plays it
with the cursor overlay. The reset hook must only put the app's own state back:
it must not despawn the gallery's, the overlay's or the Scenarios window's
entities. Everything the gallery spawns carries `NotUnderTest`, which target
lookups and `Probe::has_text` skip, so a script that clicks "Reset" never finds
a gallery button.

While a scenario plays the script owns the primary window: the real mouse,
keyboard and touch messages aimed at it are removed before Bevy reads them,
Bevy's own mouse picking is off, and the script's cursor position is put on the
window for each update and the real one put back afterwards (so the operating
system's pointer is never moved; an app reading `Window::cursor_position` in
`First` or `Last` sees the real one). The Scenarios window keeps working: its
mouse drives a pointer of its own. **Take over** pauses the script and hands
the window back; **Resume script** carries on.

Try it with `cargo run --example gallery --features gallery` (needs a display).
The gallery feature turns on the windowing backends and the renderer that draws
UI into a window, which a library test never needs.

## Recording a scenario from a session

Use the app by hand and save what you did as a `.ron` scenario. `SessionPlugin`
(added by `GalleryPlugin` for you) logs the real mouse and keyboard of the
primary window against a frame counter at the fixed 1/60 s, so playback is
deterministic whatever frame rate the session ran at. Start and stop with the
gallery's **Record** / **Stop recording** button, with
`SessionRecorder::request_start` / `request_stop` (at the next update), or
straight away with `session::start(world, name)` / `session::stop(world)`; a
command-line flag is a system that calls one of them (see the example's
`--record`).

On stop the log becomes steps: runs of moves become `Glide`s along a path
simplified with Ramer-Douglas-Peucker (3 px by default), a press and release in
place is a `Click`, a straight press, move and release is a `Drag`, a click on
a UI node with a label is `Target::text(..)`, idle time over 1.5 s is shortened
to a 30 frame `Wait`, and a modifier held around a key folds into a `Tap`. The
thresholds are `StepOptions`. The app plugs in through `SessionHooks`:

- `capture_setup` runs on start and writes what the app needs to get back to
  this state into the output directory (rails writes a save file), returning
  the script's `setup` (`Setup::File(..)`).
- `capture_view` names the view.
- `resolve_target` is offered every click that hit no label (rails turns
  ground clicks into lattice points); the rest stay `screen(..)`.
- `state` is a `StateProbe` that reads the app's state as `key: value` text.
  What changed between start and stop is written as `expect_state`, which the
  person keeps, edits or deletes; set the same probe on the `Registry`
  (`Registry::with_state`) so a loaded file checks it. Each entry runs as a
  `State` check at the end, and the same probe answers `State` checks added
  part-way through.

```ron
// Recorded from a session. Edit freely. ...
(
    format_version: 2,
    name: "Recording",
    window_size: (640, 360),
    setup: File("recording.setup.txt"),
    steps: [
        MoveTo(Screen((22.5, 27.8))),
        Glide(target: Text("+1"), frames: 12),
        Click(Left),
        Wait(26),
        Click(Left),
        Wait(25),
    ],
    expect_state: { "count": "2" },
)
```

The file and the setup file go to the gallery's export directory (or the
`SessionPlugin`'s). Load it like any other script: the app reads the setup file
the script names and calls `.setup(..)` on the loaded scenario; see
`tests/gallery.rs` and the example.

Limits: only the primary window is recorded and nothing marked `NotUnderTest`
is ever named as a target; a click on a label replays on the middle of the
labelled node, not the exact pixel; typed text and touch are not recorded (key
presses are). Selecting, restarting and stepping are refused in the gallery
while recording.

## Depending on it from a private git repository

In the Bevy app's `Cargo.toml`, as a dev-dependency, pinned to a revision (the
Bevy version must match the one the crate uses, `=0.19.1`):

```toml
[dev-dependencies]
rin_bevy_ui_tester = { git = "https://github.com/rinde/rin-bevy-ui-tester", rev = "<commit>" }
```

Cargo needs credentials for the private repository: in CI, a token with read
access configured for git; locally, your usual git credentials (set
`CARGO_NET_GIT_FETCH_WITH_CLI=true` to make Cargo use the `git` binary and so
your credential helper or SSH agent).

## The three runners

1. **Headless**: `run_headless()` inside `cargo test`. No window, no GPU.
2. **Recorded**: enable feature `record` to render a scenario offscreen and
   save frames for review. Needs a GPU, or lavapipe on a machine without one.
3. **Gallery**: enable feature `gallery` to play scenarios back in a real window, in a loop.

Scenarios use fixed 1/60 s steps, so all three runners produce the same frames.

## Working on the crate

See [`AGENTS.md`](AGENTS.md). In short: `just ci` runs everything CI runs.
