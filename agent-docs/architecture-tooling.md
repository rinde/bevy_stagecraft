# Architecture of the tooling modules

How the tooling modules work together. What one module or type does is in its
rustdoc (`cargo doc --document-private-items`); this file is only the design
across modules, with the items to read for each part. Paths are under
`crates/bevy_stagecraft/src/`.

## One scenario, three ways

A [`Scenario`](../crates/bevy_stagecraft/src/scenario.rs) is played by the
[`player`](../crates/bevy_stagecraft/src/player.rs) in a world. The same player
runs headless in `cargo test` (`headless`), offscreen with the real renderer
(`recorder`, feature `record`) and in the gallery window (`gallery`, feature
`gallery`). The recorder and the gallery add the cursor `overlay`; headless
tests never do. What differs between the three is only what is added around the
player, so a scenario behaves the same in each.

## Screenshot checks: the `Pictures` hand-over

A screenshot check needs pixels, and only the recorder has a renderer. The
`Pictures` resource in `check/screenshot.rs` is the hand-over: the player asks
for the picture of the frame it just played, the recorder takes it, and the
player's check then compares it with the baseline. A world without `Pictures`
(headless, gallery) skips screenshot checks, which is recorded as skipped and
does not fail the scenario.

The steps, all in `recorder.rs` (`RecorderSupport`):

1. the player marks a picture as wanted in `Pictures`;
2. `hide_overlay_for_picture` (`PostUpdate`, before visibility propagates)
   sets every `OverlayRoot` hidden and remembers each one's visibility in
   `Pictures`, so a baseline holds the app and not the cursor or caption;
3. `request_check_picture` (`Last`, after `RecorderSystems::Capture`) spawns
   the `Screenshot`, or, as Bevy keeps one screenshot per target per frame,
   reuses the recording's own screenshot of that frame where `request_frames`
   took one;
4. `store_check_picture` (an observer of `ScreenshotCaptured`) delivers it to
   `Pictures`, with a token so a picture that comes back late from a stopped
   scenario is not taken for the next one's;
5. `show_overlay_again` (`First`) puts the visibilities back.

Read: [`recorder`](../crates/bevy_stagecraft/src/recorder.rs),
[`check::screenshot`](../crates/bevy_stagecraft/src/check/screenshot.rs),
[`overlay::OverlayRoot`](../crates/bevy_stagecraft/src/overlay.rs).

## One trace, two renderers

A `ScenarioTrace` (`trace.rs`) is taken in `Last` after each played frame,
which is also when the recorder asks for that frame's picture, so a trace point
and its picture show the same moment. `StepView` is the one plain-data
description of a step (outcome, targets, captured values, checks, snapshot
diff, the points before and after) and `step_title` the one wording. The
recorder's `index::trace_html` and the gallery's Trace tab both render
`StepView`s and add only their own markup, so the page and the tab read the
same.

The recorder traces with its own noise filter and writes what it leaves out
nowhere. The gallery traces with `NoiseFilter::nothing()` and filters the
changes when the tab draws (`StepView::new` takes the filter), so the person
can hide and show types without playing the scenario again. That filter lives
in `GalleryState` and changes the Trace tab only.

Read: [`trace`](../crates/bevy_stagecraft/src/trace.rs),
[`index`](../crates/bevy_stagecraft/src/index.rs),
[`gallery::state`](../crates/bevy_stagecraft/src/gallery/state.rs).

## One ranking of targets

`check::pick::target_alternatives` is the only place targets are ranked (test
id, role, text, name, the app's own target, screen position). Pick mode in the
gallery lists the whole ranking; the session recorder (`session/target.rs`)
writes the first for a click. A change to the ranking therefore changes what a
recording writes and what pick mode offers, together.

Read: [`check::pick`](../crates/bevy_stagecraft/src/check/pick.rs),
[`session::target`](../crates/bevy_stagecraft/src/session/target.rs).

## How a recording becomes a scenario

`SessionPlugin` (`session/mod.rs`) logs the primary window's real mouse, keyboard
and touch input as `RecordedEvent`s stamped with a frame counter at the fixed
1/60 s, so playback does not depend on the frame rate of the session. On stop,
the pure `session::log` (`steps_from_events`, `read_log`) turns the log into
steps and says where each checkpoint falls; it needs no window, so it is tested
without one. Clicks are named through `target_alternatives`, and the app's
own setup, view and targets come from its reflected types
(`AppSetup::capture`, `AppView::current`, `AppTarget::at`), not from hooks.

Checks come from snapshots. `WorldSnapshot` (`check/suggest.rs`) is taken on
start, at each checkpoint and on stop. The diff against the previous
checkpoint, or the start, becomes `Check` steps at that checkpoint; the last
diff becomes the script's `expect`. Every suggestion is kept only if it loads
and holds in the world it came from. A named checkpoint turns the steps since
the one before into a `Step::Group`. The checkpoint key is taken out of the
primary window's keyboard messages before `InputSystems` while recording
(`SessionSystems::Checkpoint`), so the app never sees it.

The gallery's **Record** button drives the session; its ticking of suggested
checks rewrites the file through `SavedSession::save_checks` and `save_expect`.
Pick mode adds a check chosen by hand with `session::add_check`. `save`
turns a checkpoint into a setup other scenarios load.

Read: [`session`](../crates/bevy_stagecraft/src/session/mod.rs),
[`session::log`](../crates/bevy_stagecraft/src/session/log.rs),
[`check::suggest`](../crates/bevy_stagecraft/src/check/suggest.rs),
[`save`](../crates/bevy_stagecraft/src/save.rs).

## The gallery owns the primary window while a script plays

While a script plays, `gallery::input` removes real mouse, keyboard and touch
messages aimed at the primary window before `InputSystems` reads them and turns
Bevy's own mouse picking off; the Scenarios window gets a pointer of its own.
Pick mode keeps the app's input away the same way whether or not a script
plays. Everything the gallery and the overlay spawn is `NotUnderTest`, so
target lookups, picking and recording never see it.

Read: [`gallery`](../crates/bevy_stagecraft/src/gallery/mod.rs),
[`gallery::input`](../crates/bevy_stagecraft/src/gallery/input.rs).
