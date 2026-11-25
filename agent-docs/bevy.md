# Bevy

## Bevy version policy

Bevy is pinned exactly (`=0.20.1`) in the workspace `Cargo.toml`.

The Bevy feature list in `crates/bevy_stagecraft/Cargo.toml` is curated by
hand, with `default-features = false` in the workspace. Do not "simplify" it to
a bundle such as `default` or `ui`: a library must not force audio, gamepad,
dynamic linking or a windowing backend on whoever depends on it. Windowing and
rendering sit behind the `gallery` and `record` features.

## Bevy-side conventions

- Register systems in the plugin, in named system sets.
- Take `Commands` by value.
- Never call the disallowed methods listed in [code-practices.md](code-practices.md).
- Order a system against every Bevy set it depends on, each with its own
  `.before`/`.after`. Since Bevy 0.20 Bevy's own set chains (`UiSystems` in
  `PostUpdate`, the UI extract sets, the render sets) are weak: they order
  only systems whose tracked data access conflicts, so being before
  `UiSystems::Prepare` no longer implies being before `UiSystems::Layout`.
  A dependency through anything the scheduler cannot see (a channel, a
  global, an asset read by id) needs an explicit edge.
  `tests/determinism.rs` and the recorder's
  `gpu_a_recorded_scenario_gives_identical_frames_on_every_run` play the same
  scenario twice and compare the results.

