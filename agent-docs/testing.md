# Testing

- Test names are sentences stating the claim, for example
  `clicking_a_disabled_button_does_nothing`.
- Tests drive a plain Bevy UI, never a game.
- GPU tests have `gpu` in their name, need feature `record`, are `#[ignore]`d,
  and run only via `just test-gpu` or `just test-gpu-headless` (which pass
  `--include-ignored`; CI's `gpu` job runs the latter, on lavapipe). `just test` must pass on a machine without a
  GPU.
- Look at what the recorder renders: the PNGs of a GPU test land in
  `target/tmp/` (`recorder-*`). Anything drawn with text should be viewed once;
  a glyph the bundled font lacks renders as an empty box and no assertion
  notices.
- Bevy test binaries are large; see "Disk use" in
  [running.md](running.md) for how the profiles keep them small.

