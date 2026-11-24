# Architecture

How the modules fit together. What each module does is in its rustdoc
(`just doc`); this file holds only the design facts that span modules. Start
from [`player`](../crates/bevy_stagecraft/src/player.rs) and
[`script`](../crates/bevy_stagecraft/src/script.rs).

## Data, not closures

A scenario is a [`Script`](../crates/bevy_stagecraft/src/script.rs) in
memory: steps, checks, targets and the app's setup and view are all plain
data. A Rust scenario and a `.ron` file are two spellings of one thing, so
every scenario exports (`Scenario::to_script`). The only closures are the
setup and view closures a Rust author adds, which run after the data ones and
never reach a file.

The app's own parts are values of one reflected enum per extension point (see
`extension` and `check::app`). Bevy's automatic type registration finds them;
nothing is registered or looked up by name. A file keeps such a value as raw
RON (`AppValue`) until it loads, then reads it with
`TypedReflectDeserializer` against the app's type. A wrong variant or field
is a load error naming where, never a failure mid-run.

## Load time and run time

The same lookups serve two moments (`check::reflect`). At load, only type
information is at hand (the `Registry`), and `Scenario::from_script` rejects
whatever cannot be read: unknown types, fields, values, captured names used
before they are taken, wrappers around steps that cannot wait, variants that
change a window under a step in logical pixels. At run time the world
supplies the values and the same lookups read them. Anything a load check can
catch must not wait until a run.

## One frame at a time

The player (`player`) advances a scenario one frame per `App::update`. It
plays the flattened steps (groups are labels only), so numbering, reports and
playback never see a group. Several things make it wait, none of which count
as a step:

- Before the first step, until every scene the setup queued is loaded, at
  most the scenario's action timeout (`await_scenes`). Failing that, the
  report carries a `setup_failure` and no step plays. A setup that returns an
  error does the same with every step skipped.
- Before an action on a widget, until the target is actionable
  (`targets::actionable`), up to the step's timeout; `Force` skips it.
- While a `Check::Within` has not yet held, polled once per frame.
- Before polling a check that holds a screenshot, for the picture (see
  below).

Time is fixed at 1/60 s per update (`headless`), and nothing reads the wall
clock, so a frame count means the same everywhere (see `determinism.md`).

## Errors from the app

A panic in a system, command or observer reaches Bevy's
`FallbackErrorHandler`, whose default panics again and would end the run.
While a scenario is active the player installs its own handler
(`player::catch`) in a `First` system and puts the app's back afterwards. The
handler keeps errors of severity `Panic` in a thread local, so tests running
in parallel threads stay apart; every other error, and any error on a thread
that is not catching, goes to Bevy's default. A `Last` system turns what was
kept into a failed step: the step last started, which aborts the script like
any failed step. `run_in` syncs the handler around its own loop too.

## How a check ends

`Check::poll` gives `Option<Outcome>`: `None` while a `Within` still waits.
Underneath, one frame yields a `Verdict`: holds, fails, pending, broken or
unavailable. Broken is a check that cannot run at all (an ambiguous target)
and fails even inside `Not` or `Any`. Unavailable is a check that cannot run
here (a screenshot without a renderer): wrappers pass it through and it ends
as `Outcome::Skipped`. `ScenarioReport::passed` fails on a failed check only,
while a skipped step still fails the scenario.

## Captured values

A `Capture` step reads the world in the player (`Running::captures`) and
keeps the RON text and type name under a name, taking no frame. The store
(`CapturedValues`) is the player's per-run state, passed to every
`Check::poll`, expected checks included, and is never a world resource. A
`Value::captured(name)` operand in a later check is resolved to a literal
just before it is compared, so a failure reads `expected Captured("x") = 2`.
The names are known at load time (`Check::captured_names`), which is how a
check using a name no step took is refused before the run.

## Screenshots

The player cannot take a picture and the recorder can, so they meet in the
crate-private `Pictures` resource (`check::screenshot`). Before polling a
check that holds a screenshot, the player asks for a picture and waits; the
recorder answers it in `Last` (reusing the recording's own frame where it has
one), hides the overlay for it and puts the overlay back. A world without
`Pictures` (headless, gallery) never gets an answer, so those checks end as
skipped. The pure compare, crop and diff code is ungated; PNG encoding needs
feature `record`.

## Traces

With tracing on, the player takes a `TracePoint` (a world snapshot and UI
tree) at each step boundary where a later step became due, and at the end. It
also keeps each step's targets, checks and captured values. `StepView`
(`trace`) joins one step with its points and is the single description that
`index::trace_html` and the gallery's Trace tab both render. `pause_before`
lets the gallery jump to a step by halting before that step's operations are
expanded and taking the frame back, so the world equals the trace's
`before(k)`.

## Variants

A `Variant` is the runner's environment for a scenario, not part of it. It
sets the window size, scale factor, rem size, and may replace the setup or
view. The runners (`files`, `recorder`, `gallery`) each play every scenario
once per variant and skip a scenario that names other variants. A scale factor
or rem size leaves the logical window alone, so a step in logical pixels
(`Target::Screen`, `MoveBurst`) stays valid; only a window-size change is a
load error for such a step.

## Reading the world

Checks read the world through `Probe`; targets resolve through `targets`;
neither writes. Gallery and overlay UI is marked `NotUnderTest`, which hides
it from lookups. A target names a UI node only where it has a `Node` (or a
`ComputedNode` and `UiGlobalTransform`); `entity_of` is the one lookup that
may name an entity that is not a UI node.
