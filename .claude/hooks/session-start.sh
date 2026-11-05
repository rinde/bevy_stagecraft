#!/usr/bin/env bash
# Brings a fresh Claude Code cloud session up able to run `just ci` and
# `just test-gpu-headless`. Idempotent: every step is skipped when already done.
#
# Runs as the SessionStart hook in cloud sessions only. Run it by hand with
# `.claude/hooks/session-start.sh --force`.
set -euo pipefail

force=0
[[ "${1:-}" == "--force" ]] && force=1

if [[ "${CLAUDE_CODE_REMOTE:-}" != "true" && $force -eq 0 ]]; then
    exit 0
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

export PATH="$HOME/.cargo/bin:$PATH"
cache_log="${TMPDIR:-/tmp}/bevy_stagecraft-warm-cache.log"

# sudo is absent when already root (typical in containers).
as_root() {
    if [[ $EUID -eq 0 ]]; then "$@"; else sudo "$@"; fi
}

log() { echo "session-start: $*" >&2; }

# 1. Toolchain. rust-toolchain.toml pins the channel and components, so asking
#    rustup to install what the file says is enough.
#    The rustup bootstrap is a pipe-to-sh from sh.rustup.rs. That is the
#    official install route and it only runs when rustup is absent; the
#    trade-off is that it is not pinned or checksummed.
if ! command -v rustup >/dev/null; then
    log "installing rustup"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
        sh -s -- -y --default-toolchain none --profile minimal
    export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! (cargo fmt --version && cargo clippy --version) >/dev/null 2>&1; then
    log "installing the pinned toolchain with rustfmt and clippy"
    rustup toolchain install --profile minimal --component rustfmt --component clippy
fi

# Installs a pinned cargo-binstall release, verifying its SHA-256 before it is
# unpacked. To bump: change the version and refresh both checksums from the
# release assets (cargo-binstall-<arch>-unknown-linux-musl.tgz).
binstall_version="1.24.0"
install_binstall() (
    local arch sha
    case "$(uname -m)" in
        x86_64) arch=x86_64
            sha=7ffe1832c4bc3148d00f02d51c1d96c9cccdec76a0fa20e6e5692201d78bcc0b ;;
        aarch64 | arm64) arch=aarch64
            sha=c2c44aba65d402ac43ee821387c49246bc574bde65d3d7f0f7a9e666c5a69f7f ;;
        *) log "no pinned cargo-binstall for $(uname -m)"; return 1 ;;
    esac
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    curl --proto '=https' --tlsv1.2 -sSfL -o "$tmp/binstall.tgz" \
        "https://github.com/cargo-bins/cargo-binstall/releases/download/v${binstall_version}/cargo-binstall-${arch}-unknown-linux-musl.tgz"
    echo "$sha  $tmp/binstall.tgz" | sha256sum -c --quiet - || {
        log "cargo-binstall checksum mismatch"
        return 1
    }
    tar -xzf "$tmp/binstall.tgz" -C "$tmp" cargo-binstall
    mkdir -p "$HOME/.cargo/bin"
    install -m 0755 "$tmp/cargo-binstall" "$HOME/.cargo/bin/cargo-binstall"
)

# 2. Cargo tools. Prefer prebuilt binaries via cargo-binstall, since compiling
#    them costs minutes; fall back to `cargo install --locked` if that fails.
missing=()
for tool in just cargo-machete cargo-deny; do
    command -v "$tool" >/dev/null || missing+=("$tool")
done
if ((${#missing[@]})); then
    log "installing ${missing[*]}"
    if ! command -v cargo-binstall >/dev/null; then
        install_binstall || log "cargo-binstall unavailable, compiling instead"
    fi
    if command -v cargo-binstall >/dev/null; then
        cargo binstall --no-confirm --locked "${missing[@]}" ||
            cargo install --locked "${missing[@]}"
    else
        cargo install --locked "${missing[@]}"
    fi
fi

# 3. System packages: lavapipe for the GPU tests, plus the libraries Bevy
#    needs to build on Linux.
packages=(mesa-vulkan-drivers libvulkan1 libwayland-dev libx11-dev libxkbcommon-dev)
if command -v apt-get >/dev/null; then
    to_install=()
    for pkg in "${packages[@]}"; do
        dpkg -s "$pkg" >/dev/null 2>&1 || to_install+=("$pkg")
    done
    if ((${#to_install[@]})); then
        log "installing ${to_install[*]}"
        as_root apt-get update -qq
        as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${to_install[@]}"
    fi
else
    log "apt-get not found: install ${packages[*]} yourself"
fi

# 4. Warm the build cache so the first `just test` is quick. Backgrounded so
#    the session does not wait; `--force` runs it in the foreground.
warm() {
    cargo build --workspace --all-targets --all-features &&
        cargo test --workspace --all-targets --all-features --no-run
}
if [[ $force -eq 1 ]]; then
    warm
    log "ready: toolchain, just, cargo-machete, cargo-deny, lavapipe, build cache warm"
else
    nohup bash -c "$(declare -f warm); warm" >"$cache_log" 2>&1 &
    log "ready: toolchain, just, cargo-machete, cargo-deny, lavapipe; build cache warming in background (log: $cache_log)"
fi
