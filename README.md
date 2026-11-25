# bevy_stagecraft

A UI test driver for Bevy. It runs scripted pointer and keyboard scenarios
against a Bevy UI, headless inside `cargo test`, so interface behavior can be
asserted like any other code.

The same scenarios can be recorded offscreen for review and played back in a
gallery.

## Depending on it from a private git repository

In the Bevy app's `Cargo.toml`, as a dev-dependency, pinned to a revision (the
Bevy version must match the one the crate uses, `=0.20.1`):

```toml
[dev-dependencies]
bevy_stagecraft = { git = "https://github.com/rinde/bevy_stagecraft", rev = "<commit>" }
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

## An example scenario

A scenario is plain data: steps, with time counted in frames of 1/60 s, and
checks. `run_headless` builds a headless app, builds the world with the
scenario's setup, plays the steps and returns a `ScenarioReport`.

```rust,ignore
use bevy_stagecraft::{Check, Scenario, Step, Target};

#[test]
fn clicking_the_button_increments_the_counter() {
    let report = Scenario::new("counter")
        .setup(spawn_counter_ui)                 // build a plain Bevy UI
        .steps([
            Step::move_to(Target::text("Add")),  // fails if something covers it
            Step::click(),
            Step::wait(1),
            Step::check(Check::shows("1")),
        ])
        .expect_check(!Check::shows("Error"))   // at the end
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}
```

The same scenario as a RON file (`Scenario::from_ron` reads it back):

```ron
(
    format_version: 3,
    name: "counter",
    steps: [
        MoveTo(Text("Add")),
        Click(Left),
        Wait(1),
        Check(Shows("1")),
    ],
)
```

## Guides

The API is documented in rustdoc: `cargo doc --open`.

| Guide | Read it when |
| ----- | ------------ |
| [Scenarios](agent-docs/usage/scenarios.md) | you drive an app with synthetic input, wait for a target, type text, or move the pointer in bursts or by touch |
| [Extension points](agent-docs/usage/extension-points.md) | the app needs its own checks, setups, views or targets |
| [Checks](agent-docs/usage/checks.md) | you assert on text, state, counts, targets, tree snapshots or captured values |
| [Scenarios as RON](agent-docs/usage/ron-format.md) | you write scenarios as files, or group steps by name |
| [Variants](agent-docs/usage/variants.md) | a scenario must run at other window sizes, scale factors, text sizes or themes |
| [Annotations](agent-docs/usage/annotations.md) | you tag a scenario, skip it, mark it fixme or expect it to fail |
| [Recording](agent-docs/usage/recording.md) | you record for review, check screenshots, or read the trace |
| [Gallery](agent-docs/usage/gallery.md) | you play scenarios in a window or edit their files while it runs |
| [Recording from a session](agent-docs/usage/session-recording.md) | you save what you did by hand as a scenario, with suggested checks |
| [Checkpoints](agent-docs/usage/checkpoints.md) | you take, name or save checkpoints during a session, or pick a check by clicking |

## Working on the crate

See [`AGENTS.md`](AGENTS.md) for the repository workflow and development notes.
In short: `just ci` runs everything CI runs.
