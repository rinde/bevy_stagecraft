# Scenarios as RON

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
`run_ron_tests_with_variants` plays every file once per variant; see [variants.md](variants.md).

## Named groups of steps

`Group(name: "open the menu", steps: [..])` (`Step::group(name, steps)`)
gives a run of steps a name, and groups nest. It is only for the person
reading: the script plays frame for frame as the same steps without the
groups, and steps are numbered as if they were flat, so `step 4` is the fourth
step that does something, wherever it sits. The name has to say something
(an empty or blank one is a load error), and a group may be empty.

Everything that tells you where a step is shows the path of group names in
front of it, joined with `" > "` (`GROUP_SEPARATOR`): a failure reads
`open the menu > wait for it > step 4: ...`, and so do load errors, the
gallery's step list and captions, and the trace. `flatten(steps)` gives the
leaf steps with their paths for code that walks a script. A group adds a form
to version 3, not a version.
