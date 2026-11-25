# Recording a scenario from a session

Use the app by hand and save what you did as a `.ron` scenario. `SessionPlugin`
(added by `GalleryPlugin` for you) logs the real mouse and keyboard of the
primary window against a frame counter at the fixed 1/60 s (touch is not
recorded), so playback is
deterministic whatever frame rate the session ran at. Start and stop with the
gallery's **Record** / **Stop recording** button (**New: empty world** starts a
recording from the app's reset state with nothing selected, **New: current
state** from the app as it is; both are `GalleryAction`s too), with
`SessionRecorder::request_start` / `request_stop` (at the next update), or
straight away with `session::start(world, name)` / `session::stop(world)`; a
command-line flag is a system that calls one of them (see the example's
`--record`).

On stop the log becomes steps: runs of moves become `Glide`s along a path
simplified with Ramer-Douglas-Peucker (3 px by default), a press and release in
place is a `Click`, a straight press, move and release is a `Drag`, a click is
named by the sturdiest target for the widget under it (a test id
`Component(..)`, then `Role(..)` with its label, then `Text(..)`, then
`Name(..)`; the same ranking as pick mode, `target_alternatives`), idle time
over 1.5 s is shortened
to a 30 frame `Wait`, idle time over 1 s before the pointer goes to a widget
(a target such as `Text(..)`) is shortened to a 10 frame `Wait`, as playback
waits for that widget to be actionable anyway, a modifier held around a
key folds into a `Tap`, and consecutive keys that typed text (as the keyboard
reported it, so other layouts record right), each released quickly, merge into
one `Type(text: "..")`; Enter, Tab, Backspace, Control, Alt and Super chords and
held keys stay `Key`, `KeyUp` and `Tap` steps. A step holds at most 65,535
frames, so a longer pause, glide or drag is written as several steps that play
the same total frames and path. The thresholds are `StepOptions`. The app plugs in through its own reflected
types (see [The app's own setups, views and targets](extension-points.md#the-apps-own-setups-views-and-targets)):

- `AppSetup::capture` runs on start and turns the state into one of the app's
  setups, writing what it needs into the output directory (rails writes a save
  file); that is the script's `setup: App(..)`. An error stops the recording
  from starting.
- `AppView::current` names the view, the script's `view: App(..)`.
- `AppTarget::at` is offered every click that hit no widget it can name
  (rails turns ground clicks into lattice points); the rest stay
  `screen(..)`. Only a click on a widget with a role (not a slider, where the
  position matters) or on a text is named; a click on a bare panel keeps its
  position.

and through `SessionHooks`:

- `noise_filter` says which types the snapshots behind the suggested checks
  leave out (below).
- `checkpoint_key` picks the key that takes a checkpoint ([checkpoints.md](checkpoints.md)); F8 unless
  set.
- `checkpoint_names` says whether a checkpoint asks for a name (on unless set;
  see [Naming a checkpoint](checkpoints.md#naming-a-checkpoint)). The prompt is drawn by
  `CursorOverlayPlugin` only: an app with `SessionPlugin` and no overlay turns
  it off.

```ron
// Recorded from a session. Edit freely. ...
(
    format_version: 3,
    name: "Recording",
    window_size: (640, 360),
    setup: App(Counter(count: 0, switch: false)),
    steps: [
        MoveTo(Screen((22.5, 27.8))),
        Glide(target: Role(role: Button, label: "+1"), frames: 12),
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

## Suggested checks

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
| a labeled widget's state      | `Disabled(Text("Save"))`, `Not(Checked(..))`, also selected, pressed, focused, visible |
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
