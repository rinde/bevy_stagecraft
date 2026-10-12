# Agent notes

This file is for whoever changes the code. If it and the code disagree, the
code wins, and both documents need fixing in the same change.

## What this is

A test driver for Bevy UIs and the apps built on them. A scenario (setup,
view, scripted pointer and keyboard steps, checks) runs three ways:

- headless in `cargo test`;
- recorded offscreen for review (feature `record`);
- played in a gallery (feature `gallery`).

## Tickets live on GitHub

Read tickets with the GitHub tools. Label a ticket **"In progress"** before
starting work on it. Treat the title as the specification, and ask if it is
ambiguous.

Issues labelled `ready-for-agent` can be run by agents end to end with
`/coordinate`; see [Agents](#agents).

## Always work with PRs

Work on a branch named `issue-<n>-<short-slug>`, branched from an up-to-date
`main`. Before stopping, create the PR (body says `Closes #<n>`) and check
whether it needs a rebase. Never merge it yourself: only the coordinator
merges, and only what [Agents](#agents) allows.

## Agents

A **coordinator** picks up issues that are ready, an **implementer** turns
each into a pull request, a **reviewer** approves it or requests changes, and
the coordinator sends it back or merges it. They live in `.claude/agents/` and
`.claude/skills/` and follow the same rules as anyone else: this file. Nothing
runs on GitHub's side; the coordinator starts every agent, in its own session.
The same set-up runs on `rinde/rin-bevy-ui`.

| Agent | Model | Does |
| --- | --- | --- |
| `coordinator` | Sonnet | Decides what is ready, dispatches implementers and reviewers, checks on GitHub that each did its job, merges. Writes no code. |
| `implementer-sonnet`, `implementer-opus` | Sonnet, Opus | Implements one issue in a worktree under `.claude/worktrees/` on a branch `issue-<number>-<slug>`, opens a pull request that `Closes #N`, and follows it up on review, red CI or a merge conflict. Runs the `implement-issue` skill. |
| `reviewer-sonnet`, `reviewer-opus` | Sonnet, Opus | Reviews one pull request in a worktree of its own, makes sure CI passed on its head, and posts one review that approves or requests changes, with inline comments. Runs the `review-pr` skill. |

The variants differ only in their model; the instructions are the skills, which
also run by hand: `/implement-issue 82`, `/review-pr 90 opus`.

| Label | Meaning |
| --- | --- |
| `ready-for-agent` | The coordinator may start the issue. Agents leave every other issue alone. |
| `opus` | The Opus implementer, and an Opus approval. Without it, Sonnet. |
| `require-human-review` | A human approves and merges; no agent approves or merges it. |
| `In progress` | Someone is working on it; set on dispatch. |
| `Needs refinement` | Not ready. An implementer that finds the issue ambiguous asks on it, sets this and removes `ready-for-agent`. |

An issue starts only when everything it depends on is **closed as completed**:
the issues blocking it in its *Relationships*, its open sub-issues, and any
issue its body names as `Depends on #N`, `Blocked by #N` or `After #N`. A
dependency closed as not planned waits for a human.

**The review verdict.** A reviewer approves only when CI passed on the pull
request's head: GitHub's **lint**, **test**, **features**, **unused-deps** and
**deny** jobs, which together are `just ci`, and **gpu** (`just test-gpu`).
Where one did not run (missing, cancelled, queued, or failed for billing
reasons; see [CI billing failures](#ci-billing-failures)) the reviewer runs
`just ci` and `just test-gpu` on the head itself. Every agent review starts
with a marker the coordinator reads:

```
<!-- agent-review verdict=<approve|request_changes> reviewer=<sonnet|opus> sha=<commit> ci=<github|local|none> -->
```

A verdict counts only for the commit it names, so a push makes it stale. An
approved pull request with CI passed and no conflict is squash-merged (a plain
merge where the repository refuses squashing) unless the issue has
`require-human-review`. After three rounds of changes requested, the
coordinator stops, comments and takes `ready-for-agent` off the issue.

| How | What |
| --- | --- |
| `/coordinate` | Passes until nothing is left to do, then a report. |
| `/coordinate #82 #86` | Only those issues and their pull requests. |
| `/coordinate max 5` | Five implementers at once instead of three. |
| `/coordinate dry-run` | Report what it would do and change nothing. |
| `/loop 30m /coordinate` | Keep coordinating every half hour. |
| `claude --agent coordinator` | The coordinator as the session's main agent. |

Agents talk to GitHub through `gh api` and its REST endpoints, because cloud
sessions block the GraphQL API that `gh issue` and `gh pr` use. An unattended
run needs its tools allowed, or it stops at the first permission prompt. Each
worktree starts with an empty `target/`; the skills point cargo at the main
checkout's build so Bevy is not compiled again.

A change here reaches `rinde/rin-bevy-ui` only through a `rev` bump there, and
issues there may wait on issues here. The coordinator does not follow
dependencies across repositories, so such an issue there gets
`ready-for-agent` by hand once the issue here is closed.

## The one architectural rule

No dependency on any game crate (`rail_core`, `rail_game`, ...) and no game
state. The crate takes Bevy types only. Games extend it through its extension
points, each the app's one reflected enum found by Bevy's automatic
registration (no registration call, nothing looked up by name): setups
(`#[reflect(AppSetup)]`, the world builders), views (`#[reflect(AppView)]`),
targets (`#[reflect(AppTarget)]`) and checks (`#[reflect(AppCheck)]`); and
probes over the world. This is the same rule as `rail_ui` and `rin-bevy-ui`.

## Bevy version policy

The Bevy version is the same as rails and is upgraded together with it, never
in one repository alone. It is pinned exactly (`=0.19.1`) in the workspace
`Cargo.toml`.

The Bevy feature list in `crates/rin_bevy_ui_tester/Cargo.toml` is curated by
hand, with `default-features = false` in the workspace. Do not "simplify" it to
a bundle such as `default` or `ui`: a library must not force audio, gamepad,
dynamic linking or a windowing backend on whoever depends on it. Windowing and
rendering sit behind the `gallery` and `record` features.

## Where to find what

The crate is `crates/rin_bevy_ui_tester`. Fill in this table as each module
lands.

| Module     | Purpose                                      | Status  |
| ---------- | -------------------------------------------- | ------- |
| `headless` | `HeadlessPlugins`: a real Bevy app and UI with no GPU or display (virtual primary window, fixed 1/60 s time; `with_scale_factor`: the window keeps its logical size, its physical size is that times the factor, `DEFAULT_SCALE_FACTOR` = 1) | done |
| `input`    | `SyntheticInput`: queued mouse and keyboard input, drained in `PreUpdate` into Bevy's input messages and `PointerInput`; `type_character` queues a character as a `Character` event (Shift down, logical key with its text, release, Shift up, all in one frame); `keystroke` / `Keystroke` is the US-layout mapping from a character to a key and whether Shift is held; modifier key events carry `Key::Shift` etc. so `ButtonInput<Key>` sees them as text fields do | done |
| `scenario` | `Scenario` builder (the app's setup and view, `app_setup` / `scene_setup` / `app_view`, which a file holds; `with_base_dir` / `base_dir` (where a setup that names a file looks for it; `load_ron_file` sets the file's directory); `scale_factor` / `scale` (physical pixels per logical pixel, `None` = 1; `in_environment`, the crate-private copy a `Variant` makes); `action_timeout` (frames a step waits for its target, `DEFAULT_ACTION_TIMEOUT` = 60 otherwise); setup and view closures, which run after them and are never in a file; steps, and expects as `Check`s (`expect_check`), all data, so every scenario exports: there are no check closures)), `Step` (`Group { name, steps }`, nestable and only a label: `flatten` gives the leaf steps as `FlatStep { group, step }` with the path of names, and everything that plays, numbers or reports steps works on that flat list, so playback is frame for frame the same as without groups; `with_group_path` / `GROUP_SEPARATOR` (ASCII `" > "`, the bundled font has no `›`) put the path before a line; `Step::group`; with `Type { text, into }` / `type_text` / `type_into`: one character per frame through `SyntheticInput`, `into` clicks its target first, file form `Type(text: "..", into: ..)`; the wrappers `Timeout(frames, step)` / `with_timeout` and `Force(step)` / `forced`, around a `MoveTo`, `Glide`, `Drag`, `Click`, `Press` or a `Type` with an `into` only; `inner`, `waits_for_target`), `Modifiers` (serde form is a list, `[Ctrl, Shift]`); `to_script` / `from_script` / `from_ron` / `to_ron` | done |
| `check`    | `Check`, a check as data (every check is; no check is a closure): `App` (the app's own check: a value of its one `#[reflect(AppCheck)]` enum, found by Bevy's automatic registration; `AppCheckValue` keeps a file's payload as raw RON until the script loads, reads it with `TypedReflectDeserializer` and writes it with `TypedReflectSerializer`), `Shows`, `Hovered`, `Tree { of, matches }` (the UI below a node as indented lines of role, label and states, matched partially: the expected tree lists only what matters, siblings may be left out, their order must hold; a mismatch is a `-`/`+` line diff; the text is read at load time; submodule `tree`; `tree_snapshot` at the crate root, also the last of the checks pick mode offers), the reflected built-ins `Count`, `Resource`, `Component`, `InState` (types by short type path, fields by reflect path, values as raw RON read with `TypedReflectDeserializer`), the widget states `Disabled`, `Checked`, `Selected`, `Pressed`, `Focused`, `Visible`, `Exists`, `TextOf`, and `Not`, `All`, `Any`, `Within` (polled by the player frame by frame); typed builders `CountOf`, `FieldOf`; `Comparison`, `TextMatch`, `Value`; `AppCheck` (with the optional `facts`, an associated fn with `Self: Sized`), `ReflectAppCheck` (written by hand, not with `#[reflect_trait]`: it holds the cast to `dyn AppCheck` and a function pointer to the type's `facts`); `AppCheckValue` and the variant listing are the generic ones in `extension`. A check is read from RON whole (`RawValue`) so values keep their spelling; the serde form goes through a private mirror enum so no derived functions are public; a bare name, `Named(..)` or `Custom(..)` in a file is refused saying to write `App(..)`, and the old `State(..)` saying to check a reflected resource; a check that cannot run fails even inside `Not`. Submodules: `app`, `value`, `reflect` (type and field lookup, close-name suggestions, load-time validation), `widget`, `tree` (snapshot of a node's UI tree, the expected-tree parser, the partial matcher and the diff), `suggest` (public, see its own row) | done |
| `extension` | The app's extension points as reflected enums: `AppSetup` (`build`; optional `capture` for recordings), `AppView` (`arrange`; optional `current`), `AppTarget` (`resolve`; optional `at` for recordings and pick mode), each with hand-written type data (`ReflectAppSetup`, `ReflectAppView`, `ReflectAppTarget`: the cast and a function pointer to the associated fn, as `ReflectAppCheck`). `Extension` names the four points (checks included); `AppValue<K>` (aliases `AppSetupValue`, `AppViewValue`, `AppTargetValue`, `AppCheckValue`) keeps a file's payload as raw RON until the script loads, reads it with `TypedReflectDeserializer` (and every `Target` inside it, so an app target in an app check is checked too) and writes it with `TypedReflectSerializer`; a value built from RON in Rust is typed against the world's registry when used. It is an opaque `Reflect` type, as is `Target` itself. `Setup` (the script's `setup: App(..)` or the built-in `setup: Scene("x.scene.ron")`, a world saved by `save`; other forms refused with a hint) and `View` (`view: App(..)`), `app_type` / `AppType` / `AppVariant` / `AppField` (variants and field types, for forms), `SeveralAppTypes`. One type per point: two is a load error naming both | done |
| `script`   | `Script` (the serde data model; writes `format_version` 3, reads 1 to 3; optional `action_timeout` and `scale_factor` (finite and above zero, else `LoadError::Invalid`); a `Timeout(..)`/`Force(..)` around any other step is a `LoadError::Invalid`; a `Group(name: .., steps: [..])` only adds a form, an empty or blank name is a `LoadError::Invalid`, and every error inside one carries the group path in `used_by`), `Setup` and `View` (re-exported from `extension`), `Registry` (only `types`: the type registry reflected checks are validated against and every value inside `App(..)` is typed against when a script loads; the default holds every type that derives `Reflect`; no named setups, views or target resolvers), `LoadError` (unknown types and fields name the step and suggest close names; `expect_state` is refused with a hint; `BadAppValue` names the extension point and where, and lists the variants; `SeveralAppTypes` names them all; `NoAppType` says how to declare one; the old `Named(..)`/`File(..)` setups, a view's bare name and `Custom(resolver: ..)` targets are refused with a hint saying to write `App(..)`), `export_ron` | done |
| `check::pick` | One picked node: `node_at` (the app's topmost UI node at a position, walking `UiStack` as UI picking does, without moving a pointer), `subject_of` (its widget), `target_alternatives` (the one ranking of targets, used by pick mode and the session recorder: test-id `Component(..)` (a unit-struct reflected component only this entity has, not `bevy_*` or this crate's), `Role(..)` with its label, unique `Text(..)`, `Name(..)`, each kept only if it leads back; then `AppTarget::at` and `Screen` of the position), `checks_that_hold` (`Shows`/`TextOf`, widget states and their `Not`s, `Hovered`, reflected component fields as `Component(..)`, last the node's `Tree(..)` snapshot, each kept only if it loads and holds), `app_check_forms` (`AppCheckForm`, `FieldValue`: each app check variant filled from the node), `Picked`. Reads the world only. Re-exported at the crate root | done |
| `check::suggest` | `WorldSnapshot::take` (reflected resources flattened into field paths, counts per reflected component, UI texts and the states of the widgets they uniquely label, reflected `State`s, `AppCheck::facts`), `suggest_checks` (the diff as `Resource`, `Count`, `InState`, `Shows`/`Not(Shows)`, widget states, `App`/`Not(App)`, each kept only if it loads and holds at the end; fixed order, BTreeMaps only), `NoiseFilter` (default ignores `bevy_*` and this crate; `ignore`/`keep` patterns by short or full path, name without generics, or `prefix*`). Re-exported at the crate root. Replaces the old string `StateProbe`; `entries` and `changes_from` (the diff of two snapshots as `SnapshotChange`s, used by the trace and `index::trace_html`) | done |
| `trace`    | `ScenarioTrace` (`TracePoint`: frame, steps started, finished, `WorldSnapshot`, UI tree as `UiLine`s, tooling left out; `StepTrace` with `TargetTrace`s and `TraceCheck`s; `before(k)` is the last point with `started <= k`, `after(k)` the first with `started > k` or finished); `step_title` (with the group path in front) and `StepView` (plain data bringing together one step's outcome, targets, checks, snapshot diff and the points before and after it: the one description `index::trace_html` and the gallery's Trace tab both render) | done |
| `files`    | `run_ron_tests`: a `harness = false` runner turning every `.ron` file in a directory into one test (`*.scene.ron` files are saved worlds and are skipped by `ron_files`; `load_ron_file` gives the scenario its file's directory as `base_dir`); `run_ron_tests_with_variants` / `run_ron_variants` play each file once per `Variant`, as the test `file [variant]` (`FileResult::variant`) | done |
| `variant`  | `Variant`: a runner's environment for a scenario, not part of it (Playwright projects): a name, optionally a window size (logical), a scale factor, and an app setup or view that replace the scenario's own (its setup and view closures still run after). `apply` (the scenario as the variant runs it, named `label` = `scenario [variant]`, or the plain name for the default one), crate-private `problem` / `varied` (the runners use them to make a refused variant a failed setup rather than an abort). `is_default` = nothing set: only there may a step aim at `Target::Screen` (also inside `Timeout`, `Force`, `Within`, `Nth`, and in any `Group`, found through `flatten`); elsewhere `LoadError::Invalid` names the steps, with their group paths. A runner's variant list is exactly what runs: list `Variant::default()` to cover the scenario as written | done |
| `player`   | `ScenarioPlayerPlugin`: plays a scenario through `SyntheticInput` one frame per update into the `ScenarioReport` resource; an action on a target that names a widget (`MoveTo`, `Glide`, `Drag`, and a `Click`/`Press` right after a move to a widget, which checks that widget) waits frame by frame until the target is actionable (`targets::actionable`), up to the step's or scenario's timeout, then fails naming the first condition that did not hold; an ambiguous or non-UI target fails at once; `Force(..)` skips the checks and the hover check; a waiting action is not at a step boundary; a check that waits (`Check::Within`, also in the expect list) is polled once per frame until it settles; `Scenario::run_headless` / `run_in`; a setup that fails (`AppSetup::build` returns an error) makes a finished report with `setup_failure` set and every step skipped (`ScenarioPlayer::start_after_setup`); the `Last` system `trace_point` (`ScenarioSystems::Trace`) takes a `TracePoint` at each step boundary where a later step became due, and at the end, when `enable_trace(filter)` is on; the player plays `flatten(script)`, so a `Group` is no step of its own (`expand` ignores one) and each `StepRecord` keeps its `group` path, which `failures()` puts in front of the line; the player records each step's targets and checks into the `ScenarioTrace`; `Scenario::run_in_traced` / `run_headless_traced`; `pause_before(Some(k))` halts inside `advance_frame` before step k's ops are expanded and takes the frame it began back (`report.frames -= 1`), so the landed frame and world equal the trace's `before(k)`; `halted()` | done |
| `targets`  | `Target` (screen, text, world, `App(..)`: the app's own `#[reflect(AppTarget)]` value, and the locators `Component(..)` (a reflected marker component by short type path, checked at load time like `Count`), `Name(..)`, `Role(role: .., label: ..)`, `Within(outer, inner)`, `Nth(n, target)`; an opaque `Reflect` type read and written as RON, so the nested ones can be boxed), `WidgetRole` (roles read from `bevy_ui_widgets` components, not AccessKit), `resolve`, `locate` (crate-private: the one node a target names, strictly: several matches are `Lookup::Ambiguous`, which a check reports as broken (failing even inside `Not`/`Any`) listing them; none is `Lookup::Missing`, suggesting close matches). Submodules: `locate` (matching in document order, the messages), `role`, `actionable` (see its own row) | done |
| `targets::actionable` | Actionability, as Playwright's auto-waiting: `actionable` (exists, visible incl. clip and window, stable by change detection of `ComputedNode`/`UiGlobalTransform` since the player's system last ran, enabled (no `InteractionDisabled` on the widget or above it, for presses only), receives the pointer by a `UiStack` walk honouring `Pickable` and the widget's camera), `pressable` (the widget a move went to, before a press), `DEFAULT_ACTION_TIMEOUT` (public, re-exported). Reads the world only; `Action` and `NotActionable` (`Yet` waits, `Never` fails at once) and the crate-private wrappers live in `targets` | done |
| `probe`    | `Probe` over the `World` with generic helpers (`count`, `hovered`, `resource`, `has_text`) | done |
| `overlay`  | `CursorOverlayPlugin`: arrow cursor, press ring, held marker, trail and caption strip (`OverlayTheme`, `TargetDescriptions`, `OverlayCamera`), and the pick outlines (`PickHighlight`: the hovered node with a tag naming its best target, the picked one a ring further out); all `Pickable::IGNORE`; added only by the gallery and the recorder. A `CheckpointTaken` message from the session shows as a short flash over the window and a caption line (`checkpoint_text`); while the session is asking for a checkpoint's name (`SessionRecorder::naming`) the line is the prompt instead (`naming_text`, ASCII); the step caption (`describe_step`, `current_step_text`) shows a step's group path, counted in frames. Default marks are ASCII (`PASS`/`FAIL`): the bundled font draws `\u{2714}`/`\u{2718}` as empty boxes | done |
| `recorder` | `RecorderPlugins` (headless app plus the real renderer, every camera retargeted to an `Image`, the synthetic pointer aimed at it) and `Recorder` (per scenario: PNGs at step boundaries and every nth frame, filmstrip, GIF; overall `index.html`, failures first). Feature `record`; `Recorder::variants` records every scenario once per variant, one index entry each, `RecorderPlugins::with_scale_factor` (the image has the physical size; the cameras' `ImageRenderTarget`, the synthetic pointer's and each `Screenshot`'s carry the scale factor, or the pointer is on no camera and the picture is black); every recorded scenario is traced (`Recorder::with_noise_filter`, `ScenarioRecording::trace`) and `request_frames` runs after `ScenarioSystems::Trace` so each trace point's frame has a picture | done |
| `capture`  | `Frame` (RGBA pixels), PNG and GIF encoding, filmstrip drawing (marks and numbers drawn from shapes, no font). Feature `record` | done |
| `index`    | The per-scenario files and `index.html`. Feature `record`; `trace.html` per traced scenario (`trace_html`: per step the failure, targets, checks, pictures before and after, snapshot diff, UI tree and snapshot; `<details id="step-k">`; it renders `StepView`s), and failed cards list their steps linking to it (a step in a group is titled with its path) | done |
| `session`  | `SessionPlugin`, `SessionRecorder`, `session::start`/`stop`/`checkpoint`/`add_check`: real input logged against a frame counter at 1/60 s, turned into steps on stop (`steps_from_events`, `read_log` (also says where each checkpoint falls): RDP-simplified `Glide`s, `Click`, `Drag`, `Tap`, shortened idle `Wait`s (shortened further before the pointer goes to a locator target: `StepOptions::widget_idle_frames` / `widget_idle_wait`, as playback waits for it), clicks named by the first of `target_alternatives` for the widget with a role (not a slider) or text under them, a `RecordedKind::Check` as a `Check` step where it was chosen; `RecordedKind::Key` carries the text the key produced, and consecutive keys that typed text, each released within `hold_frames`, merge into one `Type` step, never Enter, Tab or Backspace nor a Ctrl, Alt or Super chord). The setup (`AppSetup::capture`), the view (`AppView::current`) and app targets for clicks on no nameable widget (`AppTarget::at`) come from the app's reflected types; `SessionHooks` holds the `NoiseFilter` and the checkpoint key, F8 by default. Snapshots of the world on start, at each checkpoint and on stop; a checkpoint can be named (`Checkpoint::name`): after the key a prompt takes the primary window's keys (`MAX_NAME_CHARS` letters; Enter keeps, Escape skips, Backspace edits; a mouse press, the next checkpoint or stop keeps what was typed; their releases are swallowed too; `SessionHooks::checkpoint_names(false)` turns it off), or `checkpoint_named(world, name)` names it from code, and the steps since the checkpoint before, with its own `Check` steps, become a `Step::Group` called that (`with_checks` builds it, so `save_checks` keeps the groups); each checkpoint's suggested checks (diffed against the checkpoint before, or the start) become `Check` steps there and `SavedSession::checkpoints`, the last diff becomes the script's `expect` and `SavedSession::candidates`; `save_expect` / `save_checks` rewrite the file with the kept ones. The checkpoint key is taken out of the primary window's `KeyboardInput` and `WindowEvent` messages before `InputSystems` while recording (`SessionSystems::Checkpoint`), so the app never sees it; each checkpoint writes a `CheckpointTaken` message the overlay shows. Submodules: `log` (pure log to steps), `hooks`, `target` (what a click names, through `target_alternatives`). Not feature-gated; the gallery's **Record** button drives it | done |
| `save`     | A checkpoint saved as a setup for other scenarios (`Setup::Scene`). `save_setup(world, dir, stem, noise)` first tries the app's own `AppSetup::capture` (a returned setup is the result, nothing written by the crate), else writes `{stem}.scene.ron` (numbered so nothing is overwritten) holding the app's reflected resources and entities as a Bevy `DynamicWorld` (feature `bevy_world_serialization`); `scene_ron` / `load_scene_ron` / `load_scene_file` (resources replaced, entities added, handles through the `AssetServer` if any). Left out: types the `NoiseFilter` ignores (default `bevy_*` and this crate), UI nodes, cameras, windows and their descendants, `NotUnderTest`, entities left empty. Deterministic: resources and components sorted by type path, entities by id. `SCENE_SUFFIX`, `is_scene_file`, `SaveError`, `SavedSetup`. Re-exported at the crate root | done |
| `gallery`  | `GalleryPlugin` (feature `gallery`): the app in the primary window, a second `Scenarios` window with its own UI camera, placed directly left of the app window (macOS automatic window tabbing is turned off so the two never become tabs) (catalogue, play/pause, step frame/step, restart, speed, loop, Take over, Export, Save as setup (`GalleryAction::SaveAsSetup`: `save::save_setup` into the export directory, named for the selected scenario, allowed while recording; the status line says where), Record, New (`NewEmpty`: reset hook, nothing selected, then record; `NewFromCurrent`: the app as it is, then record), Pick, live checks). Submodules: `catalogue` (`Gallery`), `state` (`GalleryState`, `GalleryAction`, `Speed`, `Picking`, `Suggestions`: the checklists shown when **Record** stops, one per checkpoint and one for the end, each click on a row rewriting the file's `Check` steps at that checkpoint or its `expect`; the wheel scrolls them while hovered), `control` (requests and pacing, drives the player through `advance_world`; a picked check goes to `session::add_check` while recording, else to the selected scenario's expect and the running player), `input` (drains real input aimed at the primary window while a script plays or pick mode is on, touching messages only when something is to be removed so kept ones are not replayed, counts pick clicks, bridges the Scenarios window to its own pointer, asserts and restores the cursor, shows the app no cursor while the person picks), `pick` (pick mode: outlines the node under the real cursor through `PickHighlight` and picks it on a click), `ui` (also the pick panel: targets, checks that hold, app check forms). `Gallery::variants` and `GalleryAction::SelectVariant(i)` (a **Variant** button row when there are several; results are kept per entry and variant; run-all plays every entry in every variant; the primary window's size and scale factor of before the first variant are put back where a scenario sets none; Export and the pick checks still use the catalogue's scenario as written). `NotUnderTest` (in `targets`) hides gallery and overlay UI from target lookups; `GalleryAction::JumpTo(k)` / `Control::JumpTo` (the step list in the details panel, whose lines and the checks' show each step's group path, numbered flat): restart, `pause_before(Some(k))`, one script frame per update while jumping (`GalleryState::jumping_to`), then paused; refused while recording. The **Trace** tab (`GalleryTab`, `GalleryAction::ShowTab`) beside the scenario view in the details panel: tracing is on for every run except run-all; the complete trace is stored in `GalleryState` when a run ends (kept across jumps, cleared on `Select` and `Restart`; the tab reads `state.trace()` else the player's live one); **Follow** (`ToggleTraceFollow`) shows the step the script is at, else once over the first failing step or the last; `InspectStep(k)` looks at one (turning Follow off, as does a jump, which also shows the Before world); `ShowTraceWorld` picks the UI tree before or after the step; the tab redraws on a cheap `TraceKey` (not every frame) and on `revision`; picking and the suggestion checklists take precedence over it | done |

## Running things

`just` is the entry point; the plain-cargo equivalents are below.

| Recipe             | What it does                                    | Plain cargo                                                                       |
| ------------------ | ----------------------------------------------- | --------------------------------------------------------------------------------- |
| `just build`       | Build everything                                | `cargo build --workspace --all-targets --all-features`                            |
| `just check`       | Type-check everything                           | `cargo check --workspace --all-targets --all-features`                            |
| `just test`        | Unit, integration and doc tests                 | `cargo test --workspace --all-targets --all-features` plus the same with `--doc`  |
| `just lint`        | Format check, clippy, rustdoc, warnings denied  | `cargo fmt --all -- --check`, `cargo clippy ... -- -D warnings`, `cargo doc ...`  |
| `just fmt`         | Format in place                                 | `cargo fmt --all`                                                                 |
| `just doc`         | Rustdoc, warnings denied                        | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --document-private-items` |
| `just features`    | Check none, `record`, `gallery`, all features   | `cargo check --workspace --all-targets --no-default-features [--features ...]`    |
| `just unused-deps` | Fail on unused dependencies                     | `cargo machete --with-metadata`                                                   |
| `just deny`        | Licences, advisories, bans                      | `cargo deny check`                                                                |
| `just test-gpu`    | Tests needing a (software) GPU                  | `WGPU_BACKEND=vulkan cargo test --workspace --features record gpu -- --include-ignored` |
| `just gallery-test` | Play every gallery scenario in a real window, exit non-zero naming failures (needs a display, not part of `ci`) | `cargo run --example gallery --features gallery -- --run-all` |
| `just ci`          | Everything CI runs                              | all of the above except `test-gpu`                                                |

`just test-gpu` needs a Vulkan driver. On a machine without a GPU, install
lavapipe: `sudo apt-get install -y mesa-vulkan-drivers libvulkan1`. CI does the
same in its `gpu` job. Linux builds also need
`libwayland-dev libx11-dev libxkbcommon-dev`.

`.claude/hooks/session-start.sh` sets all of this up for cloud sessions; it is
registered as a `SessionStart` hook in `.claude/settings.json`. It installs the
pinned toolchain (with `rustfmt` and `clippy`), `just`, `cargo-machete` and
`cargo-deny` (prebuilt via `cargo-binstall`), the lavapipe and Linux build
libraries, and warms the build cache in the background. It is idempotent and
does nothing outside cloud sessions. Run it by hand with
`.claude/hooks/session-start.sh --force` (this also waits for the cache warm-up).
`.claude/settings.json` also sets the auto-compact window to 100k tokens for
Sonnet 5.5 and Opus 5.5 (`modelSettings.<model>.autoCompactWindow`).

## Code practices

The lint regime lives in `[workspace.lints]` in `Cargo.toml` and `clippy.toml`;
every crate inherits it. Clippy `all`, `pedantic`, `nursery` and `cargo` are
denied.

**Forbidden in production code:** `unwrap`, `expect`, `panic!`, `todo!`,
`unimplemented!`, `dbg!`, `print!`/`println!`/`eprintln!`, indexing and
slicing, `unsafe`, `std::` paths that exist in `core`/`alloc`, and the Bevy
methods in `clippy.toml` (`Query::single`, `single_mut`, `World::resource`,
`resource_mut`).

The fallible idiom instead: return `Result` or `Option` and propagate with `?`;
use `.get(i)` rather than `[i]`; use `let Some(x) = ... else { return ... };`;
use `World::get_resource` and `Query::iter().next()` (or `get`) and handle the
`None`; report problems through errors or Bevy's `warn!`/`error!`
logging, not stdout.

- `allow` is permitted only in tests, always with `reason = "..."`.
- Every item is documented, private ones included (`missing_docs` is denied and
  rustdoc runs with `--document-private-items`).
- British spelling (behaviour, colour, initialise). Comments say *why*, not
  what.
- `#[must_use]` on accessors, `const fn` where it compiles, and
  `#[derive(Debug)]` on every type.
- Named constants instead of magic numbers.

## Determinism

A scenario must produce the same frames in every runner. So: fixed 1/60 s
steps, no wall clock, and no `HashMap`/`HashSet` iteration where the order
reaches behaviour (use ordered collections or sort first).

## Testing

- Test names are sentences stating the claim, for example
  `clicking_a_disabled_button_does_nothing`.
- Tests drive a plain Bevy UI, never a game.
- GPU tests have `gpu` in their name, need feature `record`, are `#[ignore]`d,
  and run only via `just test-gpu` (which passes `--include-ignored`; also
  CI's `gpu` job, on lavapipe). `just test` must pass on a machine without a
  GPU.
- Look at what the recorder renders: the PNGs of a GPU test land in
  `target/tmp/` (`recorder-*`). Anything drawn with text should be viewed once;
  a glyph the bundled font lacks renders as an empty box and no assertion
  notices.
- Bevy builds with the dev profile are large (each test binary is over a
  gigabyte with full debug info). On a tight disk set
  `CARGO_PROFILE_DEV_DEBUG=line-tables-only`, as CI does.

## Bevy-side conventions

- Register systems in the plugin, in named system sets.
- Take `Commands` by value.
- Never call the disallowed methods listed above.

## Before you call it done

1. `just fmt`.
2. `just ci`.
3. Update `README.md` if usage changed, and the module table above if modules
   changed.
4. Create or update the PR, and check whether it needs a rebase.

## CI billing failures

A CI job that fails because of billing or quota (for example the account
cannot start runners) is not a failure of the change. Say so in the PR rather
than changing code to chase it.

## Private software

`publish = false` stays. There is no licence file and no open-source framing.

## Dependencies and advisories (`just deny`)

`just deny` runs `cargo deny check` against `deny.toml`: advisories, bans,
licences and sources. It is part of `just ci`.

When `just deny` reports a new advisory:

1. Fix it first: update or replace the dependency (`cargo update -p <crate>`).
2. If it cannot be fixed, add it to `advisories.ignore` in `deny.toml` with the
   advisory ID, the reason, and a date to revisit. No ignore without all three.

A new duplicate crate version is a warning: avoid it, or add it to
`bans.skip` with a reason. A new licence must be checked, then added to the
allow-list explicitly. Only crates.io is allowed as a source.
