# Headless app and synthetic input

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
            // `CounterCheck` is the app's own `#[reflect(AppCheck)]` enum (see extension-points.md)
            Step::check(Check::app(CounterCheck::Is(1))),
        ])
        .expect_check(!Check::shows("Error"))   // at the end
        .run_headless();
    assert!(report.passed(), "{:?}", report.failures());
}
```

A `Check` is one of:

- `Check::app(value)`: one of the app's own checks (see [extension-points.md](extension-points.md));
- `Check::shows(text)`: some UI node of the app shows exactly this text;
- `Check::hovered(target)`: the target's widget, or something below it, is
  under the pointer;
- the built-in checks on plain Bevy data ([checks.md](checks.md)), such as
  `Check::resource::<Score>(".0").eq(3)`;
- `!check`: the check does not hold. A check that cannot run at all, such as
  a type nobody defined, still fails.

Every check is data: there are no check closures, so every scenario can be
written to a file, recorded over and shown in the gallery with its checks. A
check only your app can make is a variant of its own `AppCheck` enum.

## Waiting for a target to be actionable

Scripts need no `Wait` that guesses how long the UI takes to settle. As
Playwright's auto-waiting does, a step that acts on a target that names a
widget (`MoveTo`, `Glide`, `Drag`, the touch steps that put a finger on a target, and
a `Click` or `Press` right after a move to a widget) first waits, one frame at a time, until the target:

1. exists: it names exactly one widget;
2. is visible: not hidden, with a size, and with its middle neither clipped
   away nor outside the window;
3. is stable: neither its layout nor its transform changed during the last
   frame, so it is where it was two frames running;
4. is enabled, for a step that presses (`Click`, `Press`, the start of a
   `Drag`, `TouchDown`, `TouchTap` and `TouchDrag`): neither the widget nor anything above it carries
   `InteractionDisabled`;
5. receives the pointer: at the point the step aims at (the middle of the
   widget, or the pointer for a press), the topmost UI node that takes the
   pointer is the widget or lies below it, not something covering it.

A move only points, so it does not need the widget to be enabled: hovering a
disabled button is fine, clicking it is not. A press checks the widget the move
before it went to: still there, enabled, and still receiving the pointer.

The wait lasts at most 60 frames (`DEFAULT_ACTION_TIMEOUT`), or what the
scenario says with `Scenario::action_timeout(frames)` (`action_timeout: 120`
in a file), or what one step says with `step.with_timeout(frames)`
(`Timeout(120, MoveTo(Text("Go")))`). When it runs out the step fails, naming
the first condition that did not hold and how long it waited:

```text
no widget shows "Goo"; did you mean "Go"? (still after 60 frames)
Text("Go") is not visible: it is hidden (still after 60 frames)
Text("Go") is not stable: it moved or changed size during the last frame (still after 60 frames)
Text("Save") is disabled (still after 60 frames)
Text("Go") does not receive the pointer: it is covered by `Curtain` (still after 60 frames)
```

A target that matches several widgets, or names something that is not a UI
node, fails at once: waiting would not fix the script. Positions (`Screen`,
`World`, and an app target that reports no widget) name no widget and never
wait. `step.forced()` (`Force(Click(Left))`, Playwright's `force`) acts at
once, with none of these checks and no hover check after it, for a script that
means to click a disabled or covered widget. `Timeout(..)` and `Force(..)`
wrap one `MoveTo`, `Glide`, `Drag`, `Click` or `Press`, a `Type` that has an
`into`, or a `TouchDown`, `TouchMove`, `TouchTap` or `TouchDrag`; around
anything else
the file is refused when it loads.

Frames are fixed at 1/60 s and stability is told by Bevy's change detection
over frames, not by a clock, so the wait takes the same frames on every run
and in every runner. A step waiting for its target is in the middle of its
step, so the gallery's **Step** runs until it has acted. `Check` steps do not
wait unless they say so with `Within(..)`.

A setup that spawns a BSN scene with assets (`World::queue_spawn_scene`, whose
dependencies may still be loading) needs no `Within` either: before the first
step the player waits, a frame at a time, until every scene the app holds
(`Assets<ScenePatch>` and scene lists) has loaded with its dependencies and
been applied, for at most the scenario's action timeout. This is always on,
with no script option, and costs nothing where no scene is queued: a setup that
spawns nothing, `Setup::Scene` and `World::spawn_scene` do not wait. When the
time runs out the report has a `setup_failure` naming what was not ready (for
example `scene .. waits for inner.bsn`) and no step plays.

Condition 5 follows how Bevy delivers pointer events, so a node with
`Pickable { should_block_lower: false, .. }` (a floating popup or list that
lets the pointer through) never counts as covering what lies below it. To
assert what the person sees instead, check `OnTop(target)`
(`Check::on_top(Target::text("Go"))`): it holds when the named node (a text
label stands for its widget) is painted topmost at its middle, walking the UI
stack in paint order and ignoring `Pickable`. Only nodes that paint something
cover (a background that is not transparent, an image or text), so an empty
full-window container does not, and the widget's own nodes always count. A
failure says what covers it (`Text("Go") is not on top: it is covered by
`Popup``). It is a check, not a condition of the wait, so `Within(..)` polls it
like any other.

After moving onto a widget target the player also checks that the widget, or a
descendant, is under the pointer, and otherwise fails naming what covers it. A
failed step skips the rest of the script.

## Typing text

`Step::type_text("Alpha")` (`Type(text: "Alpha")` in a file) types into
whatever has keyboard focus, like Playwright's `pressSequentially`. For every
character it sends a key press and release with the logical key and the text
the character produces, one character per frame, with Shift pressed and
released in the same frame around the characters that need it (capitals and
the shifted symbols of a US layout), so a Bevy text field (`TextInput`, with
its `EditableText`) ends up holding exactly that text. A character no key of
the US layout types is sent on an unidentified physical key, carrying the
text, which text fields read all the same. `Step::type_into("Alpha", Target::text("Name"))`
(`Type(text: "Alpha", into: Text("Name"))`, Playwright's `fill`) first moves
to the target and clicks it, which focuses a text field (Bevy focuses a
clicked field only if it has a `TabIndex`), waiting for it to be
actionable as a click does (so `Timeout(..)` and `Force(..)` may wrap a `Type`
that has an `into`, and no other).

```ron
steps: [
    Type(text: "Alpha", into: Text("Name")),   // click the field, then type
    Type(text: "Smith"),                       // type into the focused widget
    Tap(key: Enter, modifiers: []),            // keys that are not text stay taps
],
```

The file syntax is the struct form `Type(text: "..")`; a bare `Type("..")` is
not read. Enter, Tab and Backspace are not text: use `Tap`. Typing with no
focused widget does nothing and does not fail. `Type` only adds a form to
format version 3, so every older file still loads.

## A panic in the app

Since Bevy 0.20 a panic in a system, command or observer reaches the world's
`FallbackErrorHandler`. While a scenario plays (`run_headless`, `run_in`, so
`run_ron_tests` and its variants, the recorder and the gallery) the player
installs a handler that keeps such errors, and puts the app's own back
afterwards. The step during which the app panicked fails, naming the system,
command or observer and the message; the rest of the steps are skipped, and
the runner goes on with the next scenario. Errors of lower severity go to
Bevy's default handler as before. The errors are kept per thread, so this
holds where the app updates on the thread that plays it (the default here).

## Several pointer moves in one frame

A real mouse can report several moves before the app reads them. A handler
that starts each move's delta from a value refreshed only by layout (a drag
that scrolls, say) then counts only the last one, a regression a one-move-per-
frame scenario cannot show. `Step::move_burst(positions)`
(`MoveBurst([(x, y), ..])` in a file) sends every position in one frame, in
order, and leaves the pointer at the last one. It takes one frame whatever
the number of positions, names no widget, and never waits for a target.

```ron
steps: [
    MoveTo(Name("thumb")),           // put the pointer on the widget
    Press(Left),                       // and hold it, as a drag does
    MoveBurst([(200.0, 305.0), (200.0, 310.0), (200.0, 315.0)]),
    Release(Left),
],
```

Like `Screen(..)`, a burst names fixed positions of the window (logical
pixels), so a variant that runs the scenario in another window size refuses a
script that holds one; a scale factor or a theme change does not. It only
adds a form to format version 3, so every older file still loads.

## Touch

A finger is not the mouse. The touch steps write Bevy `TouchInput` messages
for the primary window (logical positions), so `Touches` works, and a
`PointerInput` for `PointerId::Touch(finger)` so picking and the widgets see a
touch pointer, which exists from the finger going down until it lifts. The
cursor and `Hovered` never move. `finger` is 0 unless a step says, and is left
out of a file when it is 0.

| Step                                  | What it does                                              | Frames     |
| ------------------------------------- | --------------------------------------------------------- | ---------- |
| `TouchDown(at: T, finger: n)`         | put a finger down at the target                           | 1          |
| `TouchMove(to: T, finger: n)`         | move a finger that is down                                | 1          |
| `TouchUp(finger: n)`                  | lift a finger, where it is                                | 1          |
| `TouchCancel(finger: n)`              | the system takes the finger away: no click, `Pointer<Out>` only (below) | 1          |
| `TouchTap(T)`                         | finger 0 down, then up on the next frame, as `Click`      | 2          |
| `TouchDrag(from: T, to: T, frames: n)` | down, along `Glide`'s eased path, up                     | `n` + 2    |

`TouchTap` takes two frames because Bevy clicks only an entity the pointer
hovered the frame before. A step that needs a finger down (`TouchMove`,
`TouchUp`, `TouchCancel`) on one that is not, or a `TouchDown` on one that is,
fails at once. The steps that aim at a target wait for it to be actionable
like `Click` and `Drag`, and `Timeout(..)` and `Force(..)` wrap them (not
`TouchUp` or `TouchCancel`).

On Bevy 0.20 a cancelled finger reaches widgets as `Pointer<Out>` and never as
`Pointer<Cancel>`: `bevy_picking` drops the hits of a pointer that has a cancel
input, so there is nobody to send it to. A real touch screen does the same, and
`TouchCancel` sends no `Pointer<Cancel>` of its own, so a scenario cannot pass
where the gesture fails on a device. An app that must react reads `Touches`
(`just_canceled`, `any_just_canceled`), which `TouchCancel` updates that frame.

```ron
steps: [
    TouchTap(Text("Pause")),
    TouchDown(at: Name("a"), finger: 1),
    TouchDown(at: Name("b"), finger: 2),
    TouchUp(finger: 1),
    TouchCancel(finger: 2),
],
```

They only add forms to format version 3, so every older file still loads.
