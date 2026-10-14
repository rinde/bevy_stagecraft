set shell := ["bash", "-uc"]

# List the available recipes.
_default:
    @just --list

# Build every target with every feature.
build:
    cargo build --workspace --all-targets --all-features

# Type-check every target with every feature.
check:
    cargo check --workspace --all-targets --all-features

# Run unit, integration and doc tests.
test:
    cargo test --quiet --workspace --all-targets --all-features
    cargo test --quiet --workspace --all-features --doc

# Format, clippy and rustdoc, all with warnings denied.
lint: fmt-check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --document-private-items

# Format the code in place.
fmt:
    cargo fmt --all

# Fail if the code is not formatted.
fmt-check:
    cargo fmt --all -- --check

# Build the documentation, warnings denied.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --document-private-items

# Remove build artefacts.
clean:
    cargo clean

# Fail on dependencies that are declared but unused.
unused-deps:
    cargo machete --with-metadata

# Check licences, advisories and bans (see deny.toml).
deny:
    cargo deny check

# Build each feature combination that matters: none, record, gallery, all.
features:
    cargo check --workspace --all-targets --no-default-features
    cargo check --workspace --all-targets --no-default-features --features record
    cargo check --workspace --all-targets --no-default-features --features gallery
    cargo check --workspace --all-targets --all-features

# Run the tests that need a GPU (the recorder's) on whatever backend wgpu picks
# (Metal on macOS). Needs a display.
test-gpu:
    cargo test --quiet --workspace --features record gpu -- --include-ignored

# The same on a software GPU (lavapipe), forcing Vulkan: for CI and machines
# without a GPU.
test-gpu-headless:
    WGPU_BACKEND=vulkan cargo test --quiet --workspace --features record gpu -- --include-ignored

# Play every gallery scenario in a real window, one by one, and close the
# window when done. Exits non-zero, naming what failed, if any scenario failed.
# Needs a display.
gallery-test:
    cargo run --quiet --example gallery --features gallery -- --run-all

# Everything CI runs.
ci: lint test features unused-deps deny
