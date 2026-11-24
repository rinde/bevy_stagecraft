# The app's own checks

An app declares its checks as one enum that derives `Reflect` and carries
`#[reflect(AppCheck)]`, and implements `AppCheck` for it. Bevy's type registry
finds the enum by itself: there is no registration call and no generic
parameter.

```rust,ignore
use bevy_stagecraft::{AppCheck, CheckResult, Probe, ReflectAppCheck, Target};

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
its start (see [Suggested checks](session-recording.md#suggested-checks)). It has no receiver and
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

## The app's own setups, views and targets

Setups, views and targets follow the same pattern: one enum per extension
point, deriving `Reflect` and carrying the attribute, found by Bevy with no
registration call.

```rust,ignore
use bevy_stagecraft::{
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
  [Saving a checkpoint as a setup](checkpoints.md#saving-a-checkpoint-as-a-setup)).
