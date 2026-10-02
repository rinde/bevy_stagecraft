# rin-bevy-ui-tester

A UI test driver for Bevy. It runs scripted pointer and keyboard scenarios
against a Bevy UI, headless inside `cargo test`, so interface behaviour can be
asserted like any other code.

The same scenarios can be recorded offscreen for review and played back in a
gallery. The crate knows nothing about any game; it takes Bevy types only.

This is private software.

## Status

The crate is being built up ticket by ticket (see the tickets on GitHub).
Headless scenarios work today; recording and the gallery are still to come.

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
checks referred to by name. `run_headless` builds a headless app, runs the
setup hook, plays the steps and returns a `ScenarioReport`.

```rust,ignore
use rin_bevy_ui_tester::{Scenario, Step, Target};

#[test]
fn clicking_the_button_increments_the_counter() {
    let report = Scenario::new("counter")
        .setup(spawn_counter_ui)                 // build a plain Bevy UI
        .steps([
            Step::move_to(Target::text("Add")),  // fails if something covers it
            Step::click(),
            Step::wait(1),
            Step::check("counter shows 1"),
        ])
        .check("counter shows 1", |probe| probe.has_text("1"))
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}
```

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
checks, setups, views and custom targets, and a `Registry` maps the names to
code (the generic checks `pointer_over_ui` and `pointer_over_nothing` are
built in). A file naming something nobody registered, or written in a newer
`format_version`, is rejected when it loads.

```ron
(
    format_version: 1,
    name: "click pause",
    setup: Named("plain_ui"),
    steps: [
        Glide(target: Text("Pause"), frames: 20),
        Click(Left),
        Tap(key: KeyP, modifiers: [Ctrl, Shift]),
        Check("activated once"),
    ],
    expect: ["activated once"],
)
```

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

Replace the `OverlayTheme` resource to change colours, sizes, font and marks
(the bundled default font may lack the check marks). Add describers to
`TargetDescriptions` to word an app's own custom targets.

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
3. **Gallery**: enable feature `gallery` to play scenarios back in a window.

Scenarios use fixed 1/60 s steps, so all three runners produce the same frames.

## Working on the crate

See [`AGENTS.md`](AGENTS.md). In short: `just ci` runs everything CI runs.
