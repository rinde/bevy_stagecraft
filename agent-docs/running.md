# Running things

`just` is the entry point; the plain-cargo equivalents are below.

| Recipe             | What it does                                    | Plain cargo                                                                       |
| ------------------ | ----------------------------------------------- | --------------------------------------------------------------------------------- |
| `just build`       | Build everything                                | `cargo build --workspace --all-targets --all-features`                            |
| `just check`       | Type-check everything                           | `cargo check --workspace --all-targets --all-features`                            |
| `just test`        | Unit, integration and doc tests                 | `cargo test --workspace --all-targets --all-features` plus the same with `--doc`  |
| `just lint`        | Format check, clippy, rustdoc, warnings denied  | `cargo fmt --all -- --check`, `cargo clippy ... -- -D warnings`, `cargo doc ...`  |
| `just fmt`         | Format in place                                 | `cargo fmt --all`                                                                 |
| `just doc`         | Rustdoc, warnings denied                        | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --document-private-items` |
| `just features`    | Check none, `record`, `gallery`, all features   | `cargo check --workspace --all-targets --no-default-features [--features ...]`    |
| `just unused-deps` | Fail on unused dependencies                     | `cargo machete --with-metadata`                                                   |
| `just deny`        | Licenses, advisories, bans                      | `cargo deny check`                                                                |
| `just test-gpu`    | Tests needing a GPU, on the backend wgpu picks (needs a display) | `cargo test --workspace --features record gpu -- --include-ignored` |
| `just test-gpu-headless` | The same on a software GPU (lavapipe), forcing Vulkan | `WGPU_BACKEND=vulkan cargo test --workspace --features record gpu -- --include-ignored` |
| `just gallery-test` | Play every gallery scenario in a window; non-zero on failure (needs a display, not in `ci`) | `cargo run --example gallery --features gallery -- --run-all` |
| `just docs`        | Docs size budgets and links (part of `test`)    | `cargo test -p bevy_stagecraft --test docs`                                       |
| `just ci`          | Everything CI runs                              | all of the above except `test-gpu` and `test-gpu-headless`                                                |

`just test-gpu-headless` needs a Vulkan driver. On a machine without a GPU, install
lavapipe: `sudo apt-get install -y mesa-vulkan-drivers libvulkan1`. CI does the
same in its `gpu` job. Linux builds also need
`libwayland-dev libx11-dev libxkbcommon-dev`.

`.claude/hooks/session-start.sh` sets all of this up for cloud sessions; it is
registered as a `SessionStart` hook in `.claude/settings.json`. It installs the
pinned toolchain (with `rustfmt` and `clippy`), `just`, `cargo-machete` and
`cargo-deny` (prebuilt via `cargo-binstall`), the lavapipe and Linux build
libraries, and warms the build cache in the background. It is idempotent and
does nothing outside cloud sessions. Run it by hand with
`.claude/hooks/session-start.sh --force` (this also waits for the cache warm-up).

## Disk use

`just ci` builds about 90 test binaries. With full debuginfo each was roughly
1.5 GB, which filled a 250 GB volume ("No space left on device").
The workspace `Cargo.toml` therefore sets, for the dev profile (which the
test profile inherits):

- `debug = "line-tables-only"` for our own crates: panics and backtraces keep
  file and line, but a debugger cannot inspect variables;
- `debug = false` for dependencies: their frames show symbol names only.

No environment variable is needed locally or in CI. To debug with variables,
override per run: `CARGO_PROFILE_DEV_DEBUG=2 cargo test -p bevy_stagecraft
--test <name>`, preferably for one test binary. Merging the integration tests
into fewer binaries was not done: it would save little once debuginfo is
trimmed and would slow incremental builds. `cargo clean` frees the rest.
