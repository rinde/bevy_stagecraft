# Checkpoints

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
    MoveTo(Role(role: Button, label: "+1")),
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

## Naming a checkpoint

A checkpoint can take a name, which turns the steps since the checkpoint
before (and its own `Check` steps) into a [`Group`](ron-format.md#named-groups-of-steps)
called that, so a long recording reads as sections. After F8 the overlay's
caption line becomes a prompt, `Name checkpoint 2 (Enter keeps, Esc skips):
open the menu_`, and typed text builds the name (at most 60 characters,
Backspace edits it). Enter keeps it; Escape skips it, leaving the steps
ungrouped; a mouse press, the next checkpoint or the end of the session keeps
what has been typed. While the prompt is open every key in the app's window
goes to it, and its presses and releases never reach the app or the script,
exactly as with the checkpoint key; keys written in the same update as the
checkpoint key still pass. The prompt is drawn only by `CursorOverlayPlugin`
(the gallery and the recorder add it): an app that adds `SessionPlugin` without
the overlay would lose its keys after F8 with nothing on screen, so it turns
the prompt off, as does an app that needs its keys while recording, with
`SessionHooks::new().checkpoint_names(false)`, and a
checkpoint is then named only from code: `session::checkpoint_named(world,
"open the menu")` takes a checkpoint that opens no prompt. A blank name is no
name. `SavedSession::save_checks` keeps the groups.

An app whose state lives outside the ECS (rails keeps a Bevy-free `Sim`)
publishes what matters of it as a reflected facts resource, updated each
frame; the snapshot reads it like any other, and the suggested
`Resource(..)` checks read it back on playback. This replaces the string
`StateProbe` of earlier versions.

The file, and whatever the app's `AppSetup::capture` wrote, go to the
gallery's export directory (or the `SessionPlugin`'s). Load it like any other
script: its setup is the app's own value, so it plays with nothing else added;
see `tests/session.rs`, `tests/gallery.rs` and the example.

## Saving a checkpoint as a setup

Another scenario often wants to start where a recording got to. While
recording, paused, taken over or after a script has finished (the button is
dimmed while a script plays, and a click then only says so: the world changes
every frame), press the gallery's **Save as setup** (or call
`save_setup(world, dir, stem, &noise)`): the world as it is goes to
`{stem}.scene.ron` in the export directory, next to the scenarios (`stem` is
the selected scenario's file name, else `recording`). A second save uses
`{stem}-2` instead, and so does any save when the directory already holds a
file named `{stem}.<extension>` other than the scenario's own `{stem}.ron`, so
nothing written by a save is overwritten. Any scenario then starts
from it:

```ron
setup: Scene("recording.scene.ron"),
```

or `Scenario::scene_setup("recording.scene.ron")` in Rust. A scenario loaded
from a file (`load_ron_file`, `run_ron_tests`, the gallery example) looks for
the name next to its own file; one built in Rust looks next to
`Scenario::with_base_dir(..)`, else in the working directory.

The file is Bevy's own world format (a `DynamicWorld`, the dynamic scene of
earlier versions) holding the app's reflected resources and entities, and
loading it is built in: resources are replaced, entities are added. What
belongs to the tooling or the UI is left out: every type of Bevy's crates and
of this one (the `NoiseFilter` of the session's hooks decides; by default, all
of them), every UI node, camera and window and what sits below one, and what
is marked `NotUnderTest`. So the scenario brings its own UI, with a setup
closure or the app's own setup, and the data it needs comes from the file.
Types need `#[derive(Reflect, Default)]` with `#[reflect(Resource)]` or
`#[reflect(Component)]`; a component that refers to another entity is saved
with that entity's id, so keep both in the file. The file is the same on every
run: resources, components and entities are written in a fixed order. Inside
one value the order is the value's own, so a `HashMap` or `HashSet` field is
written in hash order; use a `BTreeMap` or a `Vec` for what is saved if the
file must not change between runs.

A file named `*.scene.ron` is a setup, not a scenario, so `ron_files`,
`run_ron_tests` and the gallery example skip it.

An app with a save format of its own does not use the file: its
`AppSetup::capture` writes the save into `dir` (files named
`{stem}.<extension>`, so a second save gets another `stem` and does not
overwrite the first) and returns the `App(..)` setup that loads it, and
**Save as setup** prefers that (the status line then shows the setup to
write).

## Picking a check by clicking a node

Diffs propose what changed; sometimes the check you want is about one widget
("this button is disabled", "this label reads 3"). **Pick** in the Scenarios
window turns on pick mode, while recording or with a scenario selected (which
it pauses). The app gets none of the mouse or keyboard meanwhile: hovering the
app window outlines the node under the pointer (the overlay draws the outline
and a tag naming its best target, both `Pickable::IGNORE`), and a click picks
it instead of reaching the app. Escape in the app window, or **Stop picking**,
ends it.

The Scenarios window then shows, for the picked node:

- its targets, best first: `Component(..)` of each test id only it has,
  `Role(..)` with its label, `Text(..)` of each label that only it shows,
  `Name(..)`, then what the app's `AppTarget::at` makes of the position, then
  `Screen(..)` of where it was clicked. A target that is ambiguous, or leads
  to another widget, is never offered. Clicking another one works the checks
  out again through it;
- the checks that hold for it now: `Shows(..)` or `TextOf(..)`, `Visible`,
  `Disabled` or `Not(Disabled)`, `Checked` or `Not(Checked)`, `Selected` or
  `Not(Selected)`, `Focused`, `Hovered`, each field of its reflected components as
  `Component(target: .., path: "Level.0", is: Eq(3))`, and last
  `Tree(of: .., matches: "..")`, the snapshot of the node and what is below
  it, to take whole;
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
    Check(Disabled(Role(role: Button, label: "Save"))),
],
```

While paused on a played scenario it is added to the scenario's `expect`, in
the catalogue and in the run being played, so it runs at the end; **Export
RON** writes it out. The lists behind the panel are `check::pick`
(`node_at`, `target_alternatives`, `checks_that_hold`, `app_check_forms`,
`Picked`), which read the world only, so picking changes nothing in the app.
Ranking the targets is one function, `target_alternatives`, which the session
recorder uses for clicks too.

Limits: only the primary window is recorded and nothing marked `NotUnderTest`
is ever named as a target; a click on a label replays on the middle of the
labelled node, not the exact pixel; typed text and touch are not recorded (key
presses are, except the checkpoint key's). Selecting, restarting and stepping
are refused in the gallery while recording.
