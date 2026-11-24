# Checks

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
  `Visible` is about the named node itself, not the widget above it, so a
  hidden list inside a visible selectable row is not `Visible`. `OnTop` reads
  the named node too, so a plain popup root inside a widget is judged by its own
  paint; only a text label stands for the widget it sits in. The other states
  read the nearest widget at or above the node.
- `TextOf` reads every text node at or below the target's widget, spans
  included, joined by spaces, and matches `Exact`, `Contains` or `StartsWith`.
  For a text input (`TextInput`) it reads the value the field holds, and
  `Shows(..)` finds a field by its value too.
- `Tree(of: .., matches: "..")` compares the UI at and below the target's node
  with an expected tree, a snapshot in the manner of Playwright's aria
  snapshots (see [Tree snapshots](#tree-snapshots) below).
- `Within(frames, check)` holds if the check holds now or on one of the next
  `frames` frames (Playwright's `expect.poll`), so a step waits for something
  without a guessed `Wait`, and takes only the frames it waited. It works in
  the expect list too.
- `Screenshot(name: "..", of: .., tolerance: (..))` compares the frame, or the
  rectangle of the target's node, with a stored picture (see "Screenshot
  checks" in [recording.md](recording.md#screenshot-checks)).

### Reading physical pixels as logical

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

### Tree snapshots

`Tree` checks the shape of a piece of UI in one step: the node a target names
and everything below it, as indented lines of role, label and states.

```ron
Check(Tree(of: Name("Toolbar"), matches: r#"
    - panel:
      - button "Pause"
      - button "Lay" [pressed]
      - slider "Speed" [value=0.5]
      - text-input "Name" = "Alice"
"#)),
```

- A line is `- role "label" [state] [state]`, with a colon after it when lines
  follow below. Roles are `panel` (a node that groups others, labelled by its
  `Name` if it has one), `text`, and the widget roles written in kebab case:
  `button`, `checkbox`, `radio`, `slider`, `menu-button`, `menu-item`,
  `list-item`, `text-input`, `tab`, `tab-list`, read from the same components as `Role(..)` targets, not from
  AccessKit. A widget's label is its text, as `TextOf` reads it, and the text
  nodes inside a widget are not lines of their own. Widgets show states:
  `disabled`, `checked`, `selected`, `pressed`, `focused`, and `value=..` on a
  slider. A text input also shows the text it holds, after its states, as
  `= "Alice"` (quoted like a label; leave it out to match any value).
- An image or box set inline in text (`InlineImage`, `InlineBox`) is a line of
  its own under its text, `image` or `inline-box`, so a snapshot can say an
  icon is there. The label of a widget with one is its text alone, with no
  gap or stray character, and nothing inside an inline box is read as text.
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
`Target::world` and `Target::app`, one of the app's own targets (see [extension-points.md](extension-points.md)),
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
`RadioButton`, `Slider`, `MenuButton`, `MenuItem`, `ListItem`, `TextInput`, `Tab` and `TabList`. A label is
every text at or below the widget, joined by spaces, as `TextOf` reads it, except
that a `TextInput`'s label is the text below it and never the value it holds:
`Role(role: TextInput, label: "Name")` names a field, and `TextOf` of it (or
`Shows("Alice")`) reads what it holds.
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
a UI node. A UI node is an entity with a `Node`, or with both a `ComputedNode`
and a `UiGlobalTransform`: `bevy_ui_widgets`' `ScrollbarThumb` has no `Node`
(its `Scrollbar` places it after the layout), and a step such as
`Drag(from: Name("ScrollThumb"), ..)` still finds it.
The locators work wherever a target does: in steps, in checks and in a
`Target` field of an app check.
