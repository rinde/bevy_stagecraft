# Seeing what the script does

`CursorOverlayPlugin` draws the script's pointer for people watching: an arrow
at the cursor, a ring that expands and fades on each press, a held marker
while a button is down, a short fading trail, and a caption strip along the
bottom naming the current step ("Click: Rail", "Glide to (5, 0)", "Tap =") and
each check's pass or fail mark. For touch steps it draws a filled circle under
each finger that is down (one per finger, a different colour from the arrow,
which a touch step never moves) and a short mark that shrinks and fades where
a finger went down and where it lifted or was cancelled. Everything is UI with
`Pickable::IGNORE`, so it never takes a picking hit. Only the gallery and the
recorder add it; headless tests do not.

Replace the `OverlayTheme` resource to change colours, sizes, font and marks.
The default marks are the words `PASS` and `FAIL`, because the bundled font has
no `\u{2714}` or `\u{2718}` (they render as empty boxes); set a font that has
them if you want the glyphs. The app's own targets read as their RON
(`Station("Alpha")`); add describers to `TargetDescriptions` to word them
otherwise.

The overlay draws through Bevy's default UI camera, which is never a camera
that renders to an image. If your cameras have custom targets, set the
`OverlayCamera` resource to name the one the overlay should use.

The overlay's top-level nodes can be told apart from the app's own: each is
`NotUnderTest` and `OverlayRoot`, and carries a `UiTargetCamera` from the very
bundle that adds its `Node`, so an observer on `Add<Node>` (such as one that
marks the app's UI root) sees it already aimed. For that the overlay only
exists while there is a camera to aim at: it is spawned the first frame one
exists, follows the camera if it changes, and is despawned if it goes. An app
that treats every parentless node without a `UiTargetCamera` as its root can
therefore ignore the overlay.

## Recording for review (feature `record`)

```rust
use bevy_stagecraft::Recorder;

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
pictures, drawn by the overlay) and `scenario.gif`. Touch steps are in all of
them: the fingers and marks are drawn into the pictures, and `trace.html` lists
each touch step like any other. Overall it writes
`index.html`, a contact sheet grouped by the groups you gave, groups with a
failure first and failures first within each. A failed check fails the
scenario, and `any_failed()` tells the calling binary to exit non-zero.

## Checking without recording

A CI job that only needs the checks, not the pictures, calls `check()` instead
of `record(..)`: the same real rendering and outcomes, but no PNGs, filmstrips,
GIFs, traces or `index.html` are made or written.

```rust
let checked = Recorder::new(|app| app.add_plugins(MyUiPlugin))
    .scenario(click_pause_scenario())
    .check()?;
if checked.any_failed() {
    std::process::exit(1);
}
```

Offer a flag to call `record(dir)` instead when someone wants to review the
recordings. Screenshot checks still take their pictures and read or write
their baselines, which are test data.

## Screenshot checks

```ron
Check(Screenshot(name: "home")),
Check(Screenshot(name: "toolbar", of: Name("Toolbar"), tolerance: (color: 4, pixels: 0.01))),
```

`Check::screenshot("home")` and `Check::screenshot_of("toolbar", target)` (with
`.with_tolerance(Tolerance::default().with_color(4).with_pixels(0.01))`)
compare the frame, or the rectangle the target's node covers, with the
baseline `<base dir>/<scenario name as a slug>/<name>.png`. The base directory
is the one set with `Scenario::with_base_dir`, and for a `.ron` file the
directory the file is in. A pixel differs when a channel is further than
`color` (default 2, of 255) from the baseline's; the check fails when more
than the share `pixels` (default 0, so none) of the pixels differ.

- Names that make the same slug (`A B` and `A-B`, or a variant label such as
  `Home [dark mode]` and `Home [dark-mode]`) would share a directory, so each
  directory records the exact scenario name that owns it in `.scenario` (commit
  it with the baselines). Another name finds it taken and its screenshot checks
  are broken, naming both: rename one of them. Migration: baselines made before
  this have no `.scenario` file; the first scenario to use the directory adopts
  it, so nothing moves, but if two scenarios were already sharing one, give them
  distinct names and move or regenerate the baselines
  (`RIN_UPDATE_SNAPSHOTS=1`).
- A missing baseline fails the first time and is written, for you to look at,
  commit and run again.
- On a mismatch the picture taken (`<name>.actual.png`) and one of what
  differs (`<name>.diff.png`, differing pixels magenta, the rest dimmed) are
  written in `.diagnostics/` inside the baseline directory (a name may not
  start with `.`, so they can never be or overwrite a baseline, even for checks
  called `home` and `home.actual`), and `index.html` links them. Migration:
  versions before this wrote them beside the baselines as `<name>.actual.png`
  and `<name>.diff.png`; those files are never read, changed or deleted now,
  because one may be a baseline. Delete the stale ones by hand (and ignore
  `.diagnostics/` in version control).
- `RIN_UPDATE_SNAPSHOTS=1` (or inserting `ScreenshotSettings { update: true }`)
  rewrites every baseline with the picture taken, and the check passes.
- The overlay (cursor, caption, pick outlines) is hidden from the picture.

Pixels need the renderer, so only the recorder (feature `record`: the
`Recorder`, or `Scenario::run_in` on an app with `RecorderPlugins`) takes
them. In a plain headless run, in the gallery or anywhere else the check is
**skipped**: it is recorded as `Outcome::Skipped`, which does not fail the
scenario (`ScenarioReport::passed` only counts failed checks), and nothing is
written.

Render the baselines on the same kind of GPU as the one that checks them: CI's
`gpu` job uses lavapipe (software Vulkan), so baselines made there are the
reference. A local GPU draws edges and text a little differently; use
`just test-gpu-headless` (lavapipe) to compare, or loosen the tolerance. Text
depends on the font and the driver, so give a picture with text a tolerance.

## Comparing with a value captured earlier

A check can only compare with a literal, which is no use for "the number of
views did not change" or "the popup moved 40 pixels to the right". A
`Capture` step reads something once and keeps it under a name; a later check
compares with it.

```ron
Capture(name: "views", value: Count(with: ["ItemView"])),
Capture(name: "x", value: Component(target: Name("popup"), path: "UiGlobalTransform.0.translation.x")),
// ... the steps that should change nothing, or one thing ...
Check(Count(with: ["ItemView"], is: Eq(Captured("views")))),
Check(Component(target: Name("popup"), path: "UiGlobalTransform.0.translation.x",
                is: Eq(Captured(name: "x", plus: 40)))),
```

```rust,ignore
Step::capture("views", Measure::count::<ItemView>()),
Step::capture("score", Measure::resource::<Score>("0")),
Step::check(Check::count::<ItemView>().is(Comparison::Eq(Value::captured("views")))),
Step::check(Check::resource::<Score>("0").is(Comparison::Ge(Value::captured("score").plus(1.0)))),
```

- `value` is `Count(with: [..])`, `Resource(path: ..)` or
  `Component(target: .., path: ..)`: the check of the same name without its
  `is`, read the same way. A capture takes no frame and sees the world as the
  step before it left it. A `Resource(..)` or `Component(..)` measure may add
  `unit: Logical` (Rust: `.logical()`) to store a physical value as its
  logical number; see "Reading physical pixels as logical" in [checks.md](checks.md#reading-physical-pixels-as-logical).
- Any value of a comparison (`Eq`, `Ne`, `Lt`, `Le`, `Gt`, `Ge`, both ends of
  `Between`, and `InState`'s value) may be `Captured("name")`, or
  `Captured(name: "name", plus: n)` to add `n` to a number. (The offset is
  written with the name as a field because RON does not allow
  `Captured("name", plus: n)`, a positional and a named field in one.) A whole
  number stays whole (and is added exactly, even above 2^53); a fractional `plus`
  on one is an error. Other numbers are added as `f64`. The long form refuses a
  key it does not know (`pluss`). Anything written `Captured(..)` is read as a
  captured operand, so an app enum's variant named `Captured` cannot be written
  as a value.
- A capture is read as RON written from the reflected field, then against the
  type of what it is compared with, as a literal is, so a count captured as `3`
  can be compared with a `u32` field.
- Names are unique in a script. A check may use a name only after its
  `Capture` (also inside `Not`, `All`, `Any` and `Within`); the `expect` list
  runs last and may use any. Loading refuses a name that is not captured
  before the check, a name captured twice, and a capture of an unknown type or
  field (with the usual close-name suggestions), each naming the step.
- At run time a capture whose target is missing, whose resource is not there or
  whose field cannot be read fails its step, naming the capture, and the rest
  of the script is skipped. A comparison that does not fit (adding to a
  non-number, an enum compared with a float) fails naming the check and the
  captured value. A failure shows the resolved value:
  `count of `Ghost` is 3, expected Captured("ghosts") = 2`.
- The trace (`trace.html`, the gallery's Trace tab) lists what a step captured.
  The session recorder and suggested checks do not write captures.

## The trace: what the world looked like at each step

Every recorded scenario is also traced (`Recorder::with_noise_filter` chooses
which reflected types the snapshots leave out; the default ignores `bevy_*` and
this crate). Between steps the player takes a point in `Last`, the moment the
recorder asks for its picture: a `WorldSnapshot` (the one suggested checks are
made from: reflected resources, counts, UI texts, widget states), the UI tree,
and the frame. Per step it also keeps the targets it looked up and where it
found them, the values it captured and the checks it ran. `trace.html` in each scenario's folder shows,
per step, the failure message, the targets, the checks, the pictures before and
after, how the snapshot changed, and the UI tree and snapshot afterwards. In
`index.html` a failed scenario's card lists its steps, each linking to its
section.

Without the recorder the same data comes from `Scenario::run_headless_traced()`
(or `run_in_traced(app, filter)`), which returns a `ScenarioTrace`:
`before(k)` and `after(k)` are the points around step `k`, and
`WorldSnapshot::changes_from` diffs two of them. A step that takes no frame (a
check, a caption) shares its world with the step before it.

`RecorderPlugins` is the render set: a headless app with the real renderer and
`bevy_ui_render`, every camera that would render to the primary window pointed
at an offscreen `Image` of the scenario's window size, and the synthetic
pointer aimed at that image. The primary window stays virtual and carries the
cursor position. The camera that would have been the default UI camera is
marked `IsDefaultUiCamera`, since Bevy never picks a camera that renders to an
image. Do not add the player or the overlay plugin yourself.

It needs a GPU, or lavapipe without one: `WGPU_BACKEND=vulkan` after
`sudo apt-get install -y mesa-vulkan-drivers libvulkan1`. `just test-gpu` runs
the recorder's tests on the backend wgpu picks, and `just test-gpu-headless`
forces Vulkan.
