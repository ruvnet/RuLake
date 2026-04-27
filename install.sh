#!/usr/bin/env bash
# install.sh — bring a fresh checkout of RuLake to a known-working state.
# Idempotent. Safe to re-run.
#
# Steps:
#   1. Verify a Rust toolchain is available (rustc + cargo). Suggest rustup if not.
#   2. Initialize the vendor/ruvector submodule (sources ruvector-rabitq via path dep).
#   3. cargo build (debug) so dependency resolution surfaces here, not later.
#   4. cargo test --release (the suite exercises the substrate guarantees).
#   5. Smoke-run rulake-demo --fast so the user sees end-to-end output.
#
# Skip individual phases by exporting:
#   RULAKE_SKIP_TEST=1          do not run the test suite
#   RULAKE_SKIP_DEMO=1          do not run rulake-demo
#   RULAKE_SKIP_SUBMODULES=1    do not touch git submodules

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO_DIR"

bold() { printf '\033[1m%s\033[0m\n' "$*"; }
warn() { printf '\033[33m%s\033[0m\n' "$*"; }
err()  { printf '\033[31m%s\033[0m\n' "$*" >&2; }

bold "==> ruLake install"

# 1. Toolchain
if ! command -v cargo >/dev/null 2>&1; then
    err "cargo not found on PATH."
    err "Install Rust via rustup, then re-run:"
    err "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi
bold "rustc:  $(rustc --version)"
bold "cargo:  $(cargo --version)"

# 2. Submodules
if [[ "${RULAKE_SKIP_SUBMODULES:-0}" != "1" ]]; then
    if [[ ! -f vendor/ruvector/Cargo.toml ]]; then
        bold "==> initializing vendor/ruvector submodule"
        git submodule update --init --recursive
    else
        bold "==> vendor/ruvector already populated"
    fi
fi

# 3. Build (the root crate now lives at crates/core/ per the issue #12 reorg).
bold "==> cargo build"
cargo build --manifest-path crates/core/Cargo.toml

# 4. Tests
if [[ "${RULAKE_SKIP_TEST:-0}" != "1" ]]; then
    bold "==> cargo test --release"
    cargo test --release --manifest-path crates/core/Cargo.toml
else
    warn "skipping tests (RULAKE_SKIP_TEST=1)"
fi

# 5. Smoke demo
if [[ "${RULAKE_SKIP_DEMO:-0}" != "1" ]]; then
    bold "==> cargo run --release --bin rulake-demo -- --fast"
    cargo run --release --manifest-path crates/core/Cargo.toml --bin rulake-demo -- --fast
else
    warn "skipping smoke demo (RULAKE_SKIP_DEMO=1)"
fi

bold "==> install complete"
echo "    Try:  cargo run --release --manifest-path crates/core/Cargo.toml --bin rulake-demo   # full ~2 min benchmark"
echo "    Or:   see examples/ (Rust + Node + Python + WASM + GPU samples) at the repo root"
