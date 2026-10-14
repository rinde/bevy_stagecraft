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
  a type nobody defined, still fails.

Every check is data: there are no check closures, so every scenario can be
written to a file, recorded over and shown in the gallery with its checks. A
check only your app can make is a variant of its own `AppCheck` enum.

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
- An app check or target that must reach an entity with no `Node` (a selection
  shared between widgets, say) reads it with
  `entity_of(world, &Target::component::<Selection>())`. It finds the entity a
  `Component(..)`, `Name(..)`, `Role(..)`, `Within(..)` or `Nth(..)` target
  names as strictly as a step does; `resolve` works a target out to a pixel and
  refuses such an entity.
- A world saved as a setup for other scenarios is the built-in
  `setup: Scene("start.scene.ron")` (see
  [Saving a checkpoint as a setup](#saving-a-checkpoint-as-a-setup)).

Most questions about plain Bevy data are built in, typed in Rust and found
through Bevy's type registry (`AppTypeRegistry`), so writing app code for a
check is rare:

```rust,ignore
Check::count::<Ghost>().eq(1)                        // entities with Ghost
Check::count::<Ghost>().and::<Selected>().between(1, 3)
Check::resource::<Editor>("tool").eq(Tool::Lay)      // a reflected resource field
Check::component::<Gauge>(Target::text("Speed"), "value").gt(0.5_f32)
Check::in_state(Mode::Editing)                       // a Bevy State<S>
Check::disabled(Target::text("Save"))                // also checked, selected,
                                                     // pressed, focused, visible,
                                                     // exists
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
  `InteractionDisabled`, `Checked`, `Selected`, `Pressed`,
  `InputFocus` (the widget or something below it), `InheritedVisibility` and a
  laid-out size. A text target names the nearest widget at or above the text.
- `TextOf` reads every text node at or below the target's widget, spans
  included, joined by spaces, and matches `Exact`, `Contains` or `StartsWith`.
- `Tree(of: .., matches: "..")` compares the UI at and below the target's node
  with an expected tree, a snapshot in the manner of Playwright's aria
  snapshots (see "Tree snapshots" below).
- `Within(frames, check)` holds if the check holds now or on one of the next
  `frames` frames (Playwright's `expect.poll`), so a step waits for something
  without a guessed `Wait`, and takes only the frames it waited. It works in
  the expect list too.

#### Reading physical pixels as logical

Bevy's UI holds some numbers in **physical** pixels — `ComputedNode.size`,
`ComputedNode.scroll_position` and `UiGlobalTransform` — and some in logical
pixels — `ScrollPosition`. At a window scale factor of 2 every physical value
doubles, so a check written with the number a scale factor 1 window shows
passes only there. A `Resource(..)` or `Component(..)` check, and the matching
`Measure`, reads a physical value as logical with `unit: Logical`: the value is
divided by the primary window's scale factor, so the same number states the
size or position in logical pixels and passes at every scale factor.

```ron
// The popup is 430 logical pixels wide and its centre is 225 logical pixels
// from the left edge, whatever the scale factor.
Check(Component(target: Name("popup"), path: "ComputedNode.size.x", unit: Logical, is: Eq(430.0))),
Check(Component(target: Name("popup"), path: "UiGlobalTransform.0.translation.x", unit: Logical, is: Eq(225.0))),
Capture(name: "x", value: Component(target: Name("popup"), path: "UiGlobalTransform.0.translation.x", unit: Logical)),
```

The default is `unit: Physical`, the field as it is, so an older file reads the
same. A logical value a `Capture` took is stored as the logical number and a
later logical check compares with it, `plus` included; a check and its capture
should name the same unit. `unit: Logical` applies to a number or a vector of
numbers (`f32`, `f64`, `Vec2`, `Vec3`); on anything else it is a load error.
In Rust the builder is `.logical()`:
`Check::component::<ComputedNode>(target, "size.x").logical().eq(430.0_f32)`,
`Measure::component::<ComputedNode>(target, "size.x").logical()`.

#### Tree snapshots

`Tree` checks the shape of a piece of UI in one step: the node a target names
and everything below it, as indented lines of role, label and states.

```ron
Check(Tree(of: Name("Toolbar"), matches: r#"
    - panel:
      - button "Pause"
      - button "Lay" [pressed]
      - slider "Speed" [value=0.5]
"#)),
```

- A line is `- role "label" [state] [state]`, with a colon after it when lines
  follow below. Roles are `panel` (a node that groups others, labelled by its
  `Name` if it has one), `text`, and the widget roles written in kebab case:
  `button`, `checkbox`, `radio`, `slider`, `menu-button`, `menu-item`,
  `list-item`, read from the same components as `Role(..)` targets, not from
  AccessKit. A widget's label is its text, as `TextOf` reads it, and the text
  nodes inside a widget are not lines of their own. Widgets show states:
  `disabled`, `checked`, `selected`, `pressed`, `focused`, and `value=..` on a
  slider.
- The expected tree is partial. A line matches an actual line of the same
  role, with the same label if it gives one, and at least its states (a
  value compares as a number, so `0.50` is `0.5`); the lines below it match
  lines below the actual one as an ordered subsequence, so siblings that do
  not matter are left out, but their order must hold. Parents are not
  skipped: write every level from the root down.
- A snapshot leaves out what is not on screen (`Display::None`, hidden in the
  hierarchy), what is marked `NotUnderTest`, and panels with nothing in them.
- A tree that differs fails with a line diff, `-` for what was expected and `+`
  for what is there:

  ```text
  the tree of Name("Toolbar") differs (- expected, + actual):
    - panel:
      - button "Pause"
  -   - button "Lie"
  +   - button "Lay" [disabled]
      - slider [value=0.5]
  ```

- The tree is read when the script loads, so a misspelt role (`buton`) or a
  line that is not indented properly is a load error naming the line. As
  everywhere, a target that matches several nodes is a mistake in the script
  and fails even inside `Not(..)`.
- `tree_snapshot(world, &target)` writes the current tree of a target, and
  `Check::tree(target, text)` builds the check in Rust. In pick mode the
  checks offered for a node end with `Tree(..)`, the snapshot of that node:
  click it to take the tree as it is now, then trim it to what matters.

A check step takes no frames unless it waits. A failed one fails its step but
does not stop the script. Reports name each check in words, such as `"1" is
shown`.

Targets are `Target::screen`, `Target::text` (any Bevy UI toolkit),
`Target::world` and `Target::app`, one of the app's own targets (see above),
and locators in the manner of Playwright's:

| Rust                                              | RON                                              | Finds                                                     |
| ------------------------------------------------- | ------------------------------------------------ | --------------------------------------------------------- |
| `Target::component::<PauseButton>()`              | `Component("PauseButton")`                       | the entity with this reflected marker component (a test id, Playwright's `getByTestId`) |
| `Target::name("pause")`                           | `Name("pause")`                                  | the entity whose Bevy `Name` is exactly this               |
| `Target::role(WidgetRole::Button, "Pause")`       | `Role(role: Button, label: "Pause")`             | the widget of this role showing exactly this text (`getByRole(.., { name })`) |
| `Target::of_role(WidgetRole::Slider)`             | `Role(role: Slider)`                             | the widget of this role, whatever it shows                 |
| `Target::within(outer, inner)`                    | `Within(Name("second row"), Text("Delete"))`     | the match of `inner` below a match of `outer`             |
| `Target::nth(1, target)`                          | `Nth(1, Text("Delete"))`                         | the match at this index, from 0, in hierarchy order        |

A test id is a unit struct with `#[derive(Component, Reflect)]` and
`#[reflect(Component)]`, written by its short type path (its full one where
two registered types share the short one); a misspelt or non-component type is
a load error naming the step, with close names. Roles come from the
components of `bevy_ui_widgets`, not from AccessKit, so they work headless:
`Button` (`bevy_ui_widgets` or `bevy_ui`'s `Button`), `Checkbox`,
`RadioButton`, `Slider`, `MenuButton`, `MenuItem` and `ListItem`. A label is
every text at or below the widget, joined by spaces, as `TextOf` reads it.
Every target that names a widget is strict, like Playwright's strict mode: if
it matches several, the step or check fails listing each match and where it
is, unless it is wrapped in `Nth(..)`; if it matches none, the failure
suggests close matches (`no widget shows "Pasue"; did you mean "Pause"?`).
An ambiguous target is a mistake in the script, not a state of the app, so a
check over one fails even inside `Not(..)` or `Any(..)` and does not wait
inside `Within(..)`; a target that matches nothing is a state, so
`Not(Exists(Text("Quit")))` holds while nothing shows "Quit". "Hierarchy
order" is depth first, roots by `Entity` and children by their index: stable
from run to run, but not the order on screen. A `Name`, `Component` or `Role`
that matches an entity outside the UI (a named camera, say) is reported as not
a UI node.
The locators work wherever a target does: in steps, in checks and in a
`Target` field of an app check.

### Waiting for a target to be actionable

Scripts need no `Wait` that guesses how long the UI takes to settle. As
Playwright's auto-waiting does, a step that acts on a target that names a
widget (`MoveTo`, `Glide`, `Drag`, and a `Click` or `Press` right after a move
to a widget) first waits, one frame at a time, until the target:

1. exists: it names exactly one widget;
2. is visible: not hidden, with a size, and with its middle neither clipped
   away nor outside the window;
3. is stable: neither its layout nor its transform changed during the last
   frame, so it is where it was two frames running;
4. is enabled, for a step that presses (`Click`, `Press`, the start of a
   `Drag`): neither the widget nor anything above it carries
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
wrap one `MoveTo`, `Glide`, `Drag`, `Click` or `Press`, or a `Type` that has an
`into`; around anything else
the file is refused when it loads.

Frames are fixed at 1/60 s and stability is told by Bevy's change detection
over frames, not by a clock, so the wait takes the same frames on every run
and in every runner. A step waiting for its target is in the middle of its
step, so the gallery's **Step** runs until it has acted. `Check` steps do not
wait unless they say so with `Within(..)`.

Condition 5 follows how Bevy delivers pointer events, so a node with
`Pickable { should_block_lower: false, .. }` (a floating popup or list that
lets the pointer through) never counts as covering what lies below it. To
assert what the person sees instead, check `OnTop(target)`
(`Check::on_top(Target::text("Go"))`): it holds when the widget is painted
topmost at its middle, walking the UI stack in paint order and ignoring
`Pickable`. Only nodes that paint something cover (a background that is not
transparent, an image or text), so an empty full-window container does not,
and the widget's own nodes always count. A failure says what covers it
(`Text("Go") is not on top: it is covered by `Popup``). It is a check, not a
condition of the wait, so `Within(..)` polls it like any other.

After moving onto a widget target the player also checks that the widget, or a
descendant, is under the pointer, and otherwise fails naming what covers it. A
failed step skips the rest of the script.

### Typing text

`Step::type_text("Alpha")` (`Type(text: "Alpha")` in a file) types into
whatever has keyboard focus, like Playwright's `pressSequentially`. For every
character it sends a key press and release with the logical key and the text
the character produces, one character per frame, with Shift pressed and
released in the same frame around the characters that need it (capitals and
the shifted symbols of a US layout), so Bevy's own `EditableText` ends up
holding exactly that text. A character no key of the US layout types is sent
on an unidentified physical key, carrying the text, which text fields read
all the same. `Step::type_into("Alpha", Target::text("Name"))`
(`Type(text: "Alpha", into: Text("Name"))`, Playwright's `fill`) first moves
to the target and clicks it, which focuses a text field, waiting for it to be
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

### Several pointer moves in one frame

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

## Scenarios as RON

`Scenario::to_script()` gives a `Script`, plain serde data (format version,
name, window size, action timeout, setup, view, variants, steps, expect list),
and `Scenario::from_ron`
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
target are refused with a load error saying to write `App(..)`. The script's
`action_timeout` and the `Timeout(..)` and `Force(..)` steps came later in
version 3 and only add forms, as do `Group(..)` and `Capture(..)` (below). Any other older file
is also a newer one.

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
        MoveTo(Component("PauseButton")),        // locators
        MoveTo(Within(Name("second row"), Role(role: Button, label: "Delete"))),
        MoveTo(Nth(1, Text("Delete"))),
        Timeout(120, MoveTo(Text("Loaded"))),     // wait longer for this one
        Force(Click(Left)),                        // click without the checks
        MoveBurst([(200.0, 305.0), (200.0, 310.0)]), // two moves in one frame
        Group(name: "open the menu", steps: [      // named, nestable
            Click(Left),
            Group(name: "wait for it", steps: [Wait(5)]),
        ]),
        Tap(key: KeyP, modifiers: [Ctrl, Shift]),
        Type(text: "Alpha", into: Name("Station name")), // click, then type
        Check(App(ActivatedOnce)),               // the app's own checks
        Check(App(Activated(1))),
        Check(Resource(path: "Activations.0", is: Eq(1))),
        Check(Count(with: ["Ghost"], is: Eq(1))),
        Check(Resource(path: "Editor.tool", is: Eq(Lay))),
        Check(Component(target: Text("Speed"), path: "Gauge.value", is: Gt(0.5))),
        Check(InState(state: "Mode", value: Editing)),
        Check(Disabled(Text("Save"))),
        Check(TextOf(Text("Score"), StartsWith("Score: "))),
        Check(Tree(of: Name("Toolbar"), matches: "- panel:\n  - button \"Pause\"")),
        Check(Within(30, Shows("Saved"))),
        Capture(name: "ghosts", value: Count(with: ["Ghost"])), // keep a value
        Check(Count(with: ["Ghost"], is: Eq(Captured("ghosts")))),
        Check(Resource(path: "Score.0", is: Ge(Captured(name: "ghosts", plus: 1)))),
    ],
    expect: [
        App(ActivatedOnce),
        Shows("Resume"),
        All([Not(Shows("Error")), Exists(Text("Pause"))]),
    ],
)
```

Types are written as short type paths, followed by a reflect path for a field
(`Editor.tool`, `Score.0`, `Board.cells[2]`). Where two registered types share
a short path (Bevy has its own `Label` and `Button`), write the full type path
(`bevy_ui_widgets::button::Button`); the short one is a load error listing the
full paths of every candidate. The typed builders (`Check::count::<C>()`,
`Target::component::<C>()`, ...) write the short path, so for such a type
build the check from the full path (`Check::count_of(["my_game::Label"])`,
`Target::Component("my_game::Label".into())`). A value is plain RON read as the
field's type, so `Eq(Lay)` is the variant `Lay` of whatever enum the field
holds.

A mistake in the form of a check is reported as a syntax error, listing the
forms a check can take.
The check is read whole before it is parsed, so the position given is where
the whole check ends, not where the mistake is: in a check spread over several
lines, look at the lines before the one reported.

```rust,ignore
let scenario = Scenario::from_ron_with(include_str!("x.ron"), &registry)?
    .expect_check(Check::shows("1"));
```

`scenario.to_ron()` / `export_ron(&script)` write pretty RON. A test target
with `harness = false` calling `run_ron_tests(dir, &registry, run)` turns
every `.ron` file in a directory into one test named after the file.
`run_ron_tests_with_variants` plays every file once per variant; see below.

### Named groups of steps

`Group(name: "open the menu", steps: [..])` (`Step::group(name, steps)`)
gives a run of steps a name, and groups nest. It is only for the person
reading: the script plays frame for frame as the same steps without the
groups, and steps are numbered as if they were flat, so `step 4` is the fourth
step that does something, wherever it sits. The name has to say something
(an empty or blank one is a load error), and a group may be empty.

Everything that tells you where a step is shows the path of group names in
front of it, joined with `" > "` (`GROUP_SEPARATOR`; plain ASCII because the
bundled font draws no `›`): a failure reads `open the menu > wait for it >
step 4: ...`, and so do load errors, the gallery's step list and step
captions, the trace and `trace.html`. Each step of a `ScenarioReport` keeps its
path in `StepRecord::group`. `flatten(steps)` gives the leaf steps with their
paths for code that walks a script, and `with_group_path(path, text)` puts a
path in front of a line. A group adds a form to version 3, not a version.

## Window sizes, scale factors and themes: variants

A layout that works at 1280x720 may break at 640x480, on a high-density display
or in the dark theme. Rather than copy every scenario, list **variants** in the
runner (what Playwright calls projects) and each scenario is played once per
variant. A `Variant` has a name and any of a window size (logical pixels), a
scale factor (physical pixels per logical pixel; the window keeps its logical
size, so the layout is the same as at 1.0 and the pictures have that many times
the pixels) and the app's own setup or view (a value of its `#[reflect(AppSetup)]`
or `#[reflect(AppView)]` enum, for example to switch the theme).

```rust,ignore
let variants = [
    Variant::new("wide").window_size(1280, 720),
    Variant::new("narrow").window_size(640, 480),
    Variant::new("retina").window_size(1280, 720).scale_factor(2.0),
    Variant::new("dark").app_setup(MySetup::Dark),
];
```

* `run_ron_tests_with_variants(dir, &registry, &variants, run)` makes one test
  per file and variant, named `file [variant]`, so `cargo test narrow` plays the
  narrow variant of every file. (`run_ron_variants` is the same without the
  `main`.)
* `Recorder::variants(variants)` writes one index entry per scenario and
  variant, named `scenario [variant]`, each with its own pictures and trace. A
  scenario that restricts the variants it runs in (below) is skipped in the
  others: its index card says `SKIPPED` and nothing is recorded for it.
* `Gallery::variants(variants)` adds a **Variant** row of buttons to the
  Scenarios window. Picking one runs the selected scenario in that variant, and
  a run-all plays every entry in every variant. A scenario that restricts the
  variants it runs in (below) is skipped in the others: the row keeps the
  `SKIP` mark and the run report counts it as skipped, not failed.
* By hand, `variant.apply(&scenario)` returns the scenario as that variant runs
  it (named `scenario [variant]`), and `Scenario::scale_factor` and the script's
  `scale_factor: Some(2.0)` set the scale factor of a scenario itself.

The list is exactly what runs: a runner given variants plays only those, so put
`Variant::default()` in the list to play the scenario as written too (its name
has no brackets). A variant replaces the scenario's own window size, scale
factor, app setup or app view where it sets one; a scenario's setup and view
closures still run afterwards.

A step aimed at a fixed screen position (`Screen(..)`, and a `MoveBurst(..)`
of positions) is in **logical** pixels, so it stays valid wherever the
logical window is the one it was written for. A variant that only sets a
scale factor, an app setup or an app view keeps that window, and the step
runs; a variant that sets another window size makes the position ambiguous
and is an error naming the step (`step 3 aims at a fixed screen position
...`): in the test it is a failed test, in the recorder and the gallery a
failed setup. Aim at the widget instead (`Text`, `Role`, `Component`,
`Name`).

A scenario can restrict the variants it runs in, so a file that really needs
a screen position is run only where it is valid and skipped everywhere else
rather than failing. It lists the variant names in the file:

```ron
(
    format_version: 3,
    name: "drag the thumb",
    // The drag starts at a fixed spot, so only the window it was written for
    // is valid.
    variants: ["default"],
    steps: [ /* .. */ ],
)
```

The default variant answers to `default`. A file that names none runs in
every variant. `run_ron_variants` (and `run_ron_tests_with_variants`) reports
a variant a file is thus limited out of as `skipped`, not `failed`, and
`FileResult::skipped` says so.

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

### Comparing with a value captured earlier

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
  logical number; see "Reading physical pixels as logical" above.
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

### The trace: what the world looked like at each step

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
the recorder's tests.

## The gallery (feature `gallery`)

Watch scenarios play in a real app, in a loop. `GalleryPlugin` goes into an
ordinary windowed app (for example `DefaultPlugins`) next to the app's own
plugins. The app under test runs untouched in the primary window; a second OS
window, **Scenarios**, with its own UI camera, holds the catalogue (grouped,
with descriptions and each entry's last result), play and pause, step one
frame, step to the next step, restart, speed (1/4x, 1/2x, 1x, 2x), loop on and
off, **Take over**, **Export RON**, **Save as setup**, **Record**, **New: empty world** / **New: current state**, **Pick**, the live list of
the selected scenario's checks, and its steps (see below). Space plays and pauses, Right steps a frame, Down steps to the next
step and Left restarts, read from the Scenarios window only.

The window's height is shared between the catalogue and the details panel: half each, unless one needs less than that, when the other gets the rest, and neither gets under a fifth.

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

Clicking a step in the selected scenario's list jumps to it: the gallery
restarts the scenario, plays it up to the start of that step with nothing
stored, and pauses, so the live app can be inspected (use **Take over**, or
play on). It plays one script frame per update however the speed is set, and
lands on the same frame and world as a straight run reaches before that step,
which is the point `ScenarioTrace::before` reports. A jump is refused while
recording.

The details panel has two tabs, **Scenario** and **Trace**. Runs in the gallery
are traced (a run-all is not), and the Trace tab shows what the run did step by
step, the same description as `trace.html` in a recording: the failure, what
each target resolved to, each check, what changed in the world's state, and the
UI tree before or after the step (**Before** / **After**). **Follow** keeps it
on the step the script is at, then on the first failing step once the run is
over; clicking a step in its list looks at that one instead, and **Jump to the
start of it** is the jump above. The trace of a finished run is kept across
jumps until another scenario is picked or the scenario restarts.

The trace holds every type, and the Trace tab leaves out of its *changes* what
a `NoiseFilter` does, applied when it draws them. It starts with the filter set
in `SessionHooks::noise_filter` (`GalleryPlugin::with_session_hooks`), by
default `NoiseFilter::default()`. A **Hide** button on a change line leaves out
that type (its full type path), the **Hidden by the filter** count says how
much is left out and **List** shows it, each with a **Show** button;
**App default** and **Everything** switch the whole filter. The line at the end,
**As code**, is the filter in force as Rust (`NoiseFilter::to_code`) to paste
into `SessionHooks::noise_filter`. The override lasts for the gallery session
and every scenario, and changes the Trace tab only, not `trace.html`, the checks
a recording suggests, or pick mode.

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

To run the whole catalogue unattended, add `.with_run_all()` to the plugin
(the example does so with `--run-all`; `just gallery-test` is the one command).
Every entry is reset, set up and played once, in order, with the controls off.
A failing scenario does not stop the rest. At the end the outcome is logged
(a `PASS` or `FAIL` line per scenario, the failed steps and checks under each
failure), kept in the `GalleryRunReport` resource, and the app exits, closing
both windows: `AppExit::Success` if everything passed, else `AppExit::error()`,
which `App::run` returns, so a `main` that returns it gives a non-zero exit code.
The gallery feature turns on the windowing backends and the renderer that draws
UI into a window, which a library test never needs.

## Recording a scenario from a session

Use the app by hand and save what you did as a `.ron` scenario. `SessionPlugin`
(added by `GalleryPlugin` for you) logs the real mouse and keyboard of the
primary window against a frame counter at the fixed 1/60 s, so playback is
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
held keys stay `Key`, `KeyUp` and `Tap` steps. The thresholds are `StepOptions`. The app plugs in through its own reflected
types (see [The app's own setups, views and targets](#the-apps-own-setups-views-and-targets)):

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
- `checkpoint_key` picks the key that takes a checkpoint (below); F8 unless
  set.
- `checkpoint_names` says whether a checkpoint asks for a name (on unless set;
  see "Naming a checkpoint" below). The prompt is drawn by
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
| a labelled widget's state      | `Disabled(Text("Save"))`, `Not(Checked(..))`, also selected, pressed, focused, visible |
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

#### Naming a checkpoint

A checkpoint can take a name, which turns the steps since the checkpoint
before (and its own `Check` steps) into a [`Group`](#named-groups-of-steps)
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

### Saving a checkpoint as a setup

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
