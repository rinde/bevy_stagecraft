# The gallery (feature `gallery`)

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
the window back and lifts any finger the script still holds down (as a
cancel); **Resume script** carries on. Scripted touch plays in the primary
window the same way: real touch to it is removed, the step list, the Trace tab
and run-all list the touch steps, and the overlay shows the fingers. Touch input
to the Scenarios window does not work during playback.

Try it with `cargo run --example gallery --features gallery` (needs a display).

## Editing scenario files while the gallery runs

`Gallery::directory(name, dir, &registry)` adds a group of the `.ron` scenarios
in a directory (not `*.scene.ron`) and watches it, as Playwright's UI mode
does. When a file is saved the gallery reads it again, replaces its entry, and
restarts the scenario if it is the selected one, so an edited check shows its
new result without restarting the app. A new file is added to the group. A
file that fails to load (a syntax error, an unknown type) shows its
`LoadError` under its catalogue entry and in the status line, and the last
version that loaded keeps working; a file that never loaded has an entry that
cannot be played (and fails **Run all**). A deleted file keeps its entry,
marked "The file is gone". Use `Gallery::prepare_files(|scenario| ..)`, set
before the directory, for what the gallery does to every scenario it reads,
such as `window_size`, so a reload gives the same result as the first load.

Files are looked at every `WATCH_INTERVAL` (30) updates by modification time
and size, with no extra dependency; this is part of feature `gallery` only.
Changes made in the gallery (a check added by **Pick**) live in the catalogue
until the file is read again, so **Export RON** them first.

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
