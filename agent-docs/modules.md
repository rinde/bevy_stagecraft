# Module map

One line per module: what it is for and where it lives. Paths are under
`crates/bevy_stagecraft/src/`. The details of the core modules are in their
rustdoc, and how they fit together is in [architecture.md](architecture.md);
the others are in [module-details.md](module-details.md) until they move into
rustdoc.

| Module | Purpose | Source |
| ------ | ------- | ------ |
| `headless` | A real Bevy app and UI with no GPU or display (`HeadlessPlugins`). | `headless.rs` |
| `input` | Queued synthetic mouse, keyboard and touch input (`SyntheticInput`). | `input.rs` |
| `scenario` | The `Scenario` builder and its `Step`s. | `scenario.rs` |
| `check` | Checks as data (`Check`), their values, widget states and screenshots. | `check/` |
| `check::pick` | Describes one picked node: targets, checks that hold, app check forms. | `check/pick.rs` |
| `check::suggest` | World snapshots and the checks suggested from their diff. | `check/suggest.rs` |
| `annotation` | Tags, skip, fixme and should-fail for scenarios, and the tag filter. | `annotation.rs` |
| `extension` | The app's extension points as reflected enums (setup, view, target, check). | `extension/` |
| `script` | The serde data model of a scenario file (RON) and its load errors. | `script.rs` |
| `trace` | The per-step trace of a run (`ScenarioTrace`, `StepView`). | `trace.rs` |
| `files` | `run_ron_tests`: one test per `.ron` file in a directory. | `files.rs` |
| `variant` | A runner's environment for a scenario (window size, scale factor, rem size). | `variant.rs` |
| `player` | Plays a scenario frame by frame into a `ScenarioReport`. | `player.rs`, `player/` |
| `targets` | `Target` and how it resolves to one entity or position. | `targets.rs`, `targets/` |
| `targets::actionable` | Playwright-style auto-waiting: is a target ready for the pointer. | `targets/actionable.rs` |
| `probe` | `Probe`: generic helpers over the `World`. | `probe.rs` |
| `overlay` | Cursor, trail, caption and pick outlines drawn over the app. | `overlay.rs` |
| `recorder` | Offscreen rendering of scenarios to PNG, GIF and `index.html` (feature `record`). | `recorder.rs` |
| `capture` | Frame pixels, PNG and GIF encoding, filmstrips (feature `record`). | `capture.rs` |
| `index` | The per-scenario files, `trace.html` and `index.html` (feature `record`). | `index.rs` |
| `session` | Records real input into scenario steps and checkpoints. | `session/` |
| `save` | Saves a checkpoint as a setup other scenarios load. | `save.rs` |
| `gallery` | The window that plays, records and edits scenarios (feature `gallery`). | `gallery/` |
