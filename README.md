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
checks. `run_headless` builds a headless app, builds the world with the
scenario's setup, plays the steps and returns a `ScenarioReport`.

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
            // `CounterCheck` is the app's own `#[reflect(AppCheck)]` enum (see below)
            Step::check(Check::app(CounterCheck::Is(1))),
        ])
        .expect_check(!Check::shows("Error"))   // at the end
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}
```

A `Check` is one of:

- `Check::app(value)`: one of the app's own checks (see below);
- `Check::shows(text)`: some UI node of the app shows exactly this text;
- `Check::hovered(target)`: the target's widget, or something below it, is
  under the pointer;
- the built-in checks on plain Bevy data below, such as
  `Check::resource::<Score>(".0").eq(3)`;
- `!check`: the check does not hold. A check that cannot run at all, such as
  a name nobody defined, still fails;
- a label (`Check::named`, or just a `&str`) for a closure defined on the
  scenario with `Scenario::check` or `Scenario::expect`. The label is never
  looked up from a file, and exporting a scenario that uses one is an error
  naming it.

### The app's own checks

An app declares its checks as one enum that derives `Reflect` and carries
`#[reflect(AppCheck)]`, and implements `AppCheck` for it. Bevy's type registry
finds the enum by itself: there is no registration call and no generic
parameter.

```rust,ignore
use rin_bevy_ui_tester::{AppCheck, CheckResult, Probe, ReflectAppCheck, Target};

#[derive(Reflect, Debug, Clone)]
#[reflect(AppCheck)]
enum RailCheck {
    TrackRaisedOnBridge,
    StationNamed(String),
    Joined { from: Target, to: Target },
}

impl AppCheck for RailCheck {
    fn run(&self, probe: &Probe) -> CheckResult {
        match self { /* .. */ }
    }
}

Step::check(Check::app(RailCheck::StationNamed("Alpha".into())))
```

`AppCheck::facts(probe)` is optional: it lists the app's checks that hold in
a world, and a recording suggests the ones that hold at its end but not at
its start (see [Suggested checks](#suggested-checks)). It has no receiver and
`Self: Sized`, so `dyn AppCheck` still works; `#[reflect(AppCheck)]` stores a
function pointer to it in the type data (`ReflectAppCheck::facts`), which is
how the recorder calls it with only the type registry at hand.

A file writes the value inside `App(..)`: `Check(App(StationNamed("Alpha")))`.
The payload is kept untouched when the file is read, then read as the app's
type with Bevy's `TypedReflectDeserializer` when the scenario loads, and
written back with `TypedReflectSerializer`. So a misspelt variant or a wrong
argument is a load error that names the step and lists the variants with their
field types:

```text
step 2: `StationNamd("Alpha")` is not a `RailCheck`: unknown variant `StationNamd`,
expected one of `TrackRaisedOnBridge`, `StationNamed`, `Joined`; `RailCheck` is one
of `TrackRaisedOnBridge`, `StationNamed(String)`, `Joined(from: Target, to: Target)`
```

- Every field type must be `Reflect`; primitives, `String` and `Target` read
  best in a file.
- One app check type per app: a second type with `#[reflect(AppCheck)]` is a
  load error naming both.
- `Registry::app_type(Extension::Check)` (or `app_type(&types, ..)`) lists
  the type's variants and their field types, for offering app checks with a
  form.
- Automatic registration needs Bevy's `reflect_auto_register` feature, which
  is part of Bevy's default features and which this crate turns on. A type
  that is not found that way can still be added with
  `registry.register_type::<T>()`.

### The app's own setups, views and targets

Setups, views and targets follow the same pattern: one enum per extension
point, deriving `Reflect` and carrying the attribute, found by Bevy with no
registration call.

```rust,ignore
use rin_bevy_ui_tester::{
    AppSetup, AppTarget, AppView, ReflectAppSetup, ReflectAppTarget, ReflectAppView,
    ResolvedTarget,
};

#[derive(Reflect, Debug, Clone)]
#[reflect(AppSetup)]
enum RailSetup { Empty, SavedWorld(String) }

impl AppSetup for RailSetup {
    // Builds the world the scenario starts from; an error fails the scenario
    // before its first step.
    fn build(&self, world: &mut World) -> Result<(), String> { /* .. */ }
}

#[derive(Reflect, Debug, Clone)]
#[reflect(AppView)]
enum RailView { Standard, TopDown }

impl AppView for RailView {
    // Arranges the camera and layout; the recorder and the gallery call it.
    fn arrange(&self, world: &mut World) { /* .. */ }
}

#[derive(Reflect, Debug, Clone)]
#[reflect(AppTarget)]
enum RailTarget { Piece(u32), Station(String) }

impl AppTarget for RailTarget {
    fn resolve(&self, world: &World) -> Result<ResolvedTarget, String> { /* .. */ }
}

Scenario::new("two stops")
    .app_setup(RailSetup::SavedWorld("two_stops_and_a_train.ron".into()))
    .app_view(RailView::TopDown)
    .step(Step::move_to(Target::app(RailTarget::Station("Alpha".into()))))
```

A file writes each inside `App(..)`: `setup: App(SavedWorld("two_stops_and_a_train.ron"))`,
`view: App(TopDown)`, `MoveTo(App(Station("Alpha")))`, and the same anywhere
a target goes (`Check(Hovered(App(Piece(3))))`, a `Target` field of an app
check). As with checks, the value is read as the app's type when the scenario
loads, so a misspelt variant or a wrong argument is a load error naming where
it is (`the setup`, `the view`, `step 3`) and listing the variants; one type
per extension point, and a second one is a load error naming both.

- A setup closure (`Scenario::setup(|world| ..)`) and a view closure still
  work for scenarios written in Rust. They run after the app's setup and view
  and are never written to a file.
- A setup variant that names a file is the app's to find: the crate passes the
  name along, so resolve it against the directory the app keeps its scenarios
  in.
- Each trait has an optional associated function for recordings:
  `AppSetup::capture(world, dir, stem)` turns the app's state into a setup
  when a recording starts (writing a save into `dir` if it needs one),
  `AppView::current(world)` names the view, and `AppTarget::at(world, position)`
  names a click that hit no label. They replace the session hooks of earlier
  versions.
- `Registry::app_type(Extension::Setup)` and so on list the variants.

Most questions about plain Bevy data are built in, typed in Rust and found
through Bevy's type registry (`AppTypeRegistry`), so writing app code for a
check is rare:

```rust,ignore
Check::count::<Ghost>().eq(1)                        // entities with Ghost
Check::count::<Ghost>().and::<Selected>().between(1, 3)
Check::resource::<Editor>("tool").eq(Tool::Lay)      // a reflected resource field
Check::component::<Gauge>(Target::text("Speed"), "value").gt(0.5_f32)
Check::in_state(Mode::Editing)                       // a Bevy State<S>
Check::disabled(Target::text("Save"))                // also checked, pressed,
                                                     // focused, visible, exists
Check::text_of(Target::text("Score"), TextMatch::starts_with("Score: "))
Check::all([a, b]), Check::any([a, b])
Check::within(30, Check::shows("Saved"))             // waits up to 30 frames
```

- Comparisons are `eq`, `ne`, `lt`, `le`, `gt`, `ge` and `between` (both ends
  included); a failure says what was found: `count of `Ghost` is 2, expected
  1`.
- `Count` leaves out disabled entities, as queries do. The types must be
  registered in the app (`app.register_type::<T>()`, or Bevy's automatic
  registration); `Resource` and `Component` need `#[reflect(Resource)]` or
  `#[reflect(Component)]`, and `InState` needs `app.register_type_state::<S>()`.
- A short name that two registered types share (Bevy has two `Button`s) is
  refused, listing their full type paths; write the full path instead, such as
  `bevy_ui_widgets::button::Button`.
- `Component` reads the component on the target's node or, failing that, the
  nearest node above it that has one, so a label finds the widget it sits in.
- The widget states read the components Bevy's widgets keep:
  `InteractionDisabled`, `Checked`, `Pressed` (or `Interaction::Pressed`),
  `InputFocus` (the widget or something below it), `InheritedVisibility` and a
  laid-out size. A text target names the nearest widget at or above the text.
- `TextOf` reads every text node at or below the target's widget, spans
  included, joined by spaces, and matches `Exact`, `Contains` or `StartsWith`.
- `Within(frames, check)` holds if the check holds now or on one of the next
  `frames` frames (Playwright's `expect.poll`), so a step waits for something
  without a guessed `Wait`, and takes only the frames it waited. It works in
  the expect list too.

A check step takes no frames unless it waits. A failed one fails its step but
does not stop the script. Reports name each check in words, such as `"1" is
shown`.

Targets are `Target::screen`, `Target::text` (any Bevy UI toolkit),
`Target::world` and `Target::app`, one of the app's own targets (see above).
After moving onto a widget target the player
checks that the widget, or a descendant, is under the pointer, and otherwise
fails naming what covers it. A failed step skips the rest of the script.

## Scenarios as RON

`Scenario::to_script()` gives a `Script`, plain serde data (format version,
name, window size, setup, view, steps, expect list), and `Scenario::from_ron`
reads one back, so a scenario can be written in Rust or as a file and
converted either way. Closures cannot be serialised, so a script holds the
app's setup, view, targets and checks as values of its own reflected enums,
inside `App(..)`; nothing is looked up by a name. Types that checks name, and
the app's own types, are looked up in `Registry::types`, so a whole file is
checked when it loads, with no world at hand. It starts with every type that
derives `Reflect`; share the app's own registry instead with
`Registry::default().with_types_of(app.world())`, or add types one by one with
`register_type::<T>()` and `register_state::<S>()` (`Registry::install` then
adds them to the app too). A file naming something nobody registered, or
written in a `format_version` this crate does not read, is rejected when it
loads. That includes a type or field that does not exist (the error names the
step and suggests close names) and a value that does not fit its field: values
are read with Bevy's `TypedReflectDeserializer` against the field's type.

The current format is version 3. Versions 1 and 2 are still read: version 2
added checks spelled out as data, version 3 the built-in checks below and the
app's own checks as `App(..)`. Version 3 also dropped checks looked up by name
(a bare `"name"`, `Named(..)` and `Custom(..)`): an older file that uses one is
refused with a load error saying to write `App(..)` instead. The string state
probe is gone too: a file with `expect_state` or a `State(key: .., value: ..)`
check is refused with a load error saying to publish the state as a reflected
resource and check it with `Resource(..)`. Setups, views and targets went
the same way later in version 3: a `Named(..)` or `File(..)` setup, a view
given by name (`view: "top_down"`) and a `Custom(resolver: .., argument: ..)`
target are refused with a load error saying to write `App(..)`. Any other
older file is also a newer one.

```ron
(
    format_version: 3,
    name: "click pause",
    setup: App(PlainUi),                        // the app's own setup
    view: App(Zoomed(2.0)),                     // and view
    steps: [
        Check(Not(Hovered(Text("Pause")))),
        Glide(target: Text("Pause"), frames: 20),
        Check(Hovered(Text("Pause"))),
        Click(Left),
        MoveTo(App(PauseButton)),                // the app's own targets
        Tap(key: KeyP, modifiers: [Ctrl, Shift]),
        Check(App(ActivatedOnce)),               // the app's own checks
        Check(App(Activated(1))),
        Check(Resource(path: "Activations.0", is: Eq(1))),
        Check(Count(with: ["Ghost"], is: Eq(1))),
        Check(Resource(path: "Editor.tool", is: Eq(Lay))),
        Check(Component(target: Text("Speed"), path: "Gauge.value", is: Gt(0.5))),
        Check(InState(state: "Mode", value: Editing)),
        Check(Disabled(Text("Save"))),
        Check(TextOf(Text("Score"), StartsWith("Score: "))),
        Check(Within(30, Shows("Saved"))),
    ],
    expect: [
        App(ActivatedOnce),
        Shows("Resume"),
        All([Not(Shows("Error")), Exists(Text("Pause"))]),
    ],
)
```

Types are written as short type paths, followed by a reflect path for a field
(`Editor.tool`, `Score.0`, `Board.cells[2]`). A value is plain RON read as the
field's type, so `Eq(Lay)` is the variant `Lay` of whatever enum the field
holds.

A mistake in the form of a check is reported as a syntax error, listing the
forms a check can take.
The check is read whole before it is parsed, so the position given is where
the whole check ends, not where the mistake is: in a check spread over several
lines, look at the lines before the one reported.

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
them if you want the glyphs. The app's own targets read as their RON
(`Station("Alpha")`); add describers to `TargetDescriptions` to word them
otherwise.

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
off, **Take over**, **Export RON**, **Record**, **Pick**, and the live list of
the selected scenario's checks. Space plays and pauses, Right steps a frame, Down steps to the next
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
view (a setup that fails is shown in the status line, and the run fails with
every step skipped), resizes the primary window to the scenario's window size and plays it
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
thresholds are `StepOptions`. The app plugs in through its own reflected
types (see [The app's own setups, views and targets](#the-apps-own-setups-views-and-targets)):

- `AppSetup::capture` runs on start and turns the state into one of the app's
  setups, writing what it needs into the output directory (rails writes a save
  file); that is the script's `setup: App(..)`. An error stops the recording
  from starting.
- `AppView::current` names the view, the script's `view: App(..)`.
- `AppTarget::at` is offered every click that hit no label (rails turns ground
  clicks into lattice points); the rest stay `screen(..)`.

and through `SessionHooks`:

- `noise_filter` says which types the snapshots behind the suggested checks
  leave out (below).
- `checkpoint_key` picks the key that takes a checkpoint (below); F8 unless
  set.

```ron
// Recorded from a session. Edit freely. ...
(
    format_version: 3,
    name: "Recording",
    window_size: (640, 360),
    setup: App(Counter(count: 0, switch: false)),
    steps: [
        MoveTo(Screen((22.5, 27.8))),
        Glide(target: Text("+1"), frames: 12),
        Click(Left),
        Wait(26),
        Click(Left),
        Wait(25),
    ],
    expect: [
        Resource(path: "Counter.0", is: Eq(2)),
        Shows("Count: 2"),
        Not(Shows("Count: 0")),
    ],
)
```

### Suggested checks

While recording you know what should have changed; the world can propose the
checks. On start and on stop the recorder takes a `WorldSnapshot` through
reflection: the fields of every reflected resource, how many entities have
each reflected component, the texts of the app's UI nodes and the states of
the widgets they label, every reflected Bevy `State`, and the app's own
`AppCheck::facts`. `suggest_checks` turns the difference into built-in checks:

| What changed                   | Suggested check                              |
| ------------------------------ | -------------------------------------------- |
| a resource field               | `Resource(path: "Counter.0", is: Eq(1))`     |
| a component count              | `Count(with: ["Ghost"], is: Eq(2))`          |
| a state                        | `InState(state: "Mode", value: Editing)`     |
| text now shown / no longer     | `Shows("1")` / `Not(Shows("0"))`             |
| a labelled widget's state      | `Disabled(Text("Save"))`, `Not(Checked(..))`, also pressed, focused, visible |
| an app fact now / no longer    | `App(CounterIs(1))` / `Not(App(CounterIs(0)))` |

Every suggestion is run against the world at the end and kept only if it
holds there. The order is fixed (resources, counts, states, texts, widgets,
facts; within each by name, field order or text), so the same recording gives
the same list on every run. The suggestions are the file's `expect` and are
also in `SavedSession::candidates`; `SavedSession::save_expect` writes the
file again with the ones you keep. In the gallery they appear as a checklist
in the Scenarios window when **Stop recording** is pressed, all ticked;
clicking one unticks it (or ticks it again) and rewrites the file with the
ticked ones as its `expect`.

A `NoiseFilter` leaves out what moves without meaning: by default every type
of Bevy's own crates (`bevy_*`: layout such as `ComputedNode`, `Transform`
and `GlobalTransform`, picking, rendering, input, time) and of this crate.
The UI Bevy draws still counts through its texts and widget states. Change it
with `SessionHooks::new().noise_filter(NoiseFilter::default().ignore("my_game::debug::*").keep("Sprite"))`;
a pattern is a short or full type path, a name without generics, or a prefix
ending in `*`.

### Checkpoints

Checks at the end cannot say what was true half-way through. Press **F8**
(or the key set with `SessionHooks::new().checkpoint_key(KeyCode::F2)`) in the
app's window while recording to take a checkpoint; `session::checkpoint(world)`
does the same from code. Each checkpoint takes a snapshot and compares it with
the checkpoint before, or with the start for the first one. What changed
becomes `Check` steps at that point of the script, each one a check that held
when the checkpoint was taken; the end's `expect` is then what changed since the
last checkpoint.

```ron
steps: [
    MoveTo(Text("+1")),
    Click(Left),
    Wait(11),
    Check(Resource(path: "Counter.0", is: Eq(1))),
    Check(Shows("Count: 1")),
    Check(Not(Shows("Count: 0"))),
    Click(Left),
    Wait(10),
],
expect: [
    Resource(path: "Counter.0", is: Eq(2)),
    Shows("Count: 2"),
    Not(Shows("Count: 1")),
],
```

While a session is recorded the key never reaches the app: its presses and
releases in the app's window are taken out of Bevy's keyboard messages (and
the `WindowEvent`s) before `InputSystems`, and are not logged as steps. Pick a
key the app does not use. The cursor overlay shows each checkpoint with a short
flash over the window and a caption such as `Checkpoint 2: 3 checks suggested`
(the `CheckpointTaken` message). Each checkpoint's candidates are in
`SavedSession::checkpoints`, and `SavedSession::save_checks` writes the file
again with the ones you keep at each checkpoint and at the end. In the gallery,
**Stop recording** lists one checklist per checkpoint and then the end's; the
wheel scrolls them while the pointer is over them.

An app whose state lives outside the ECS (rails keeps a Bevy-free `Sim`)
publishes what matters of it as a reflected facts resource, updated each
frame; the snapshot reads it like any other, and the suggested
`Resource(..)` checks read it back on playback. This replaces the string
`StateProbe` of earlier versions.

The file, and whatever the app's `AppSetup::capture` wrote, go to the
gallery's export directory (or the `SessionPlugin`'s). Load it like any other
script: its setup is the app's own value, so it plays with nothing else added;
see `tests/session.rs`, `tests/gallery.rs` and the example.

### Picking a check by clicking a node

Diffs propose what changed; sometimes the check you want is about one widget
("this button is disabled", "this label reads 3"). **Pick** in the Scenarios
window turns on pick mode, while recording or with a scenario selected (which
it pauses). The app gets none of the mouse or keyboard meanwhile: hovering the
app window outlines the node under the pointer (the overlay draws the outline
and a tag naming its best target, both `Pickable::IGNORE`), and a click picks
it instead of reaching the app. Escape in the app window, or **Stop picking**,
ends it.

The Scenarios window then shows, for the picked node:

- its targets, best first: `Text(..)` of each label that only it shows, then
  what the app's `AppTarget::at` makes of the position, then
  `Screen(..)` of its middle. Clicking another one works the checks out again
  through it;
- the checks that hold for it now: `Shows(..)` or `TextOf(..)`, `Visible`,
  `Disabled` or `Not(Disabled)`, `Checked` or `Not(Checked)`, `Focused`,
  `Hovered`, and each field of its reflected components as
  `Component(target: .., path: "Level.0", is: Eq(3))`;
- a form per variant of the app's check type (`#[reflect(AppCheck)]`), each
  field filled from the node: a `Target` field with the chosen target, a
  `String` with its text, a number with the first number in its text, with
  `-`/`+` to change numbers, flip a `bool` or cycle a target, and `PASS` or
  `FAIL` for whether the check holds now.

Clicking a check takes it. While recording it becomes a `Check` step at this
point of the script (`session::add_check(world, check)` does the same from
code), which stays whatever is ticked in the checklists afterwards:

```ron
steps: [
    Wait(30),
    Check(Disabled(Text("Save"))),
],
```

While paused on a played scenario it is added to the scenario's `expect`, in
the catalogue and in the run being played, so it runs at the end; **Export
RON** writes it out. The lists behind the panel are `check::pick`
(`node_at`, `target_alternatives`, `checks_that_hold`, `app_check_forms`,
`Picked`), which read the world only, so picking changes nothing in the app.
Ranking the targets is one function, `target_alternatives`, where better
locators plug in.

Limits: only the primary window is recorded and nothing marked `NotUnderTest`
is ever named as a target; a click on a label replays on the middle of the
labelled node, not the exact pixel; typed text and touch are not recorded (key
presses are, except the checkpoint key's). Selecting, restarting and stepping
are refused in the gallery while recording.

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
