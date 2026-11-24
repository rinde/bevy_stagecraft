# Tags, skip, fixme and should-fail

A scenario can carry tags and say it should not run, as Playwright's
annotations do:

```ron
(
    format_version: 3,
    name: "bridge builds",
    tags: ["bridge", "slow"],
    // skip: Some("needs a display"),        not run, listed as ignored
    // fixme: Some("crashes, see #12"),      not run, listed apart from skip
    should_fail: Some("rails overlap, #34"), // passes only by failing
    steps: [ /* .. */ ],
)
```

In code: `Scenario::tags([..])`, `skip(reason)`, `fixme(reason)`,
`should_fail(reason)`.

* A `skip`ped or `fixme`d scenario is not run, in any variant. `run_ron_tests`
  prints it as `ignored, skip: reason` / `ignored, fixme: reason`
  (`FileOutcome::Disabled`) and counts it apart from `skipped`, which stays for
  variants.
* A `should_fail` scenario passes only if it fails (`ok (failed as expected)`).
  When it passes, the test fails saying the bug is fixed and `should_fail`
  should go, so a fixed bug does not go unnoticed.
* `run_ron_tests` filters by tag through `RIN_SCENARIO_TAGS`:
  `RIN_SCENARIO_TAGS=bridge,-slow cargo test` plays the scenarios tagged
  `bridge` except those tagged `slow`. A scenario outside the filter is not
  listed. In code, `run_ron_tagged(dir, &registry, filter, &TagFilter::parse(..),
  &variants, run)`.
* The gallery shows tags beside each entry, dims skipped and fixme entries with
  their reason, and has a **Tag** row of buttons that shows only the entries
  with that tag (a run of the whole catalogue plays only those too;
  `GalleryPlugin::with_tag("bridge")` starts filtered).
* The recorder's `index.html` shows tags on the cards, lists skipped and fixme
  scenarios last, dimmed, with their reason, and flips `should_fail`
  scenarios as above. `Recorder::with_tag_filter(TagFilter)` limits what is
  recorded.
