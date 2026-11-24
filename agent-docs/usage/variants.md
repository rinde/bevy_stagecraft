# Window sizes, scale factors, text sizes and themes: variants

A layout that works at 1280x720 may break at 640x480, on a high-density display
or in the dark theme. Rather than copy every scenario, list **variants** in the
runner (what Playwright calls projects) and each scenario is played once per
variant. A `Variant` has a name and any of a window size (logical pixels), a
scale factor (physical pixels per logical pixel; the window keeps its logical
size, so the layout is the same as at 1.0 and the pictures have that many times
the pixels), a rem size (the root font size, Bevy's `RemSize`, applied before
the setup so `Val::Rem` and `FontSize::Rem` text and layout follow it; the
window keeps its logical size, so `Screen` targets and `MoveBurst` stay valid;
text the app sizes in `Px` does not follow it; the runner puts the size back)
and the app's own setup or view (a value of its `#[reflect(AppSetup)]`
or `#[reflect(AppView)]` enum, for example to switch the theme).

```rust,ignore
let variants = [
    Variant::new("wide").window_size(1280, 720),
    Variant::new("narrow").window_size(640, 480),
    Variant::new("retina").window_size(1280, 720).scale_factor(2.0),
    Variant::new("large text").rem_size(40.0),
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
