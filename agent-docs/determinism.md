# Determinism

A scenario must produce the same frames in every runner. So: fixed 1/60 s
steps, no wall clock, and no `HashMap`/`HashSet` iteration where the order
reaches behavior (use ordered collections or sort first).

Nor may a scenario depend on the order of systems nothing orders. Bevy 0.20
can shuffle those (`ScheduleBuildSettings::shuffle_seed`, behind Bevy's `debug`
feature, enabled for this crate's dev-dependencies only). `tests/determinism.rs`
plays a scenario headless with each of `PreUpdate`, `Update`, `PostUpdate` and
`Last` shuffled in turn, and all at once, under the fixed seeds
`SHUFFLE_SEEDS` (`tests/common`) on the single-threaded executor, and compares
reports and traces with the unshuffled run; the recorder's
`gpu_shuffling_the_order_of_unordered_systems_changes_no_recorded_frame`
compares the recorded frames. A failure names the schedule and the seed:
reproduce it with `common::shuffle(&mut app, schedule, seed)`, and fix it with
an explicit `.before`/`.after`, never by dropping the test. A new schedule the
crate adds to goes in `common::shuffled_schedules`.

