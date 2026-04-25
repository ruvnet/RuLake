#!/usr/bin/env bash
# Build the WASI bundle-toolkit CLI.
#
# Outputs:
#   target/wasm32-wasip1/release/bundle-toolkit.wasm
#
# Requirements:
#   rustup target add wasm32-wasip1   (modern Rust ≥ 1.78)
#   --or-- rustup target add wasm32-wasi  (legacy alias)
set -euo pipefail

cd "$(dirname "$0")"

# The host environment may set RUSTFLAGS for native builds (e.g. mold linker).
# rust-lld for wasm targets does not understand `-fuse-ld=mold`.
unset RUSTFLAGS CARGO_BUILD_RUSTFLAGS

TARGET="wasm32-wasip1"
if ! rustup target list --installed | grep -q "$TARGET"; then
  if rustup target list | grep -q "^${TARGET}\b"; then
    rustup target add "$TARGET"
  else
    # Fall back to the legacy alias on older toolchains.
    TARGET="wasm32-wasi"
    rustup target list --installed | grep -q "$TARGET" || rustup target add "$TARGET"
  fi
fi

cargo build --target "$TARGET" --release

WASM="target/${TARGET}/release/bundle-toolkit.wasm"
if command -v wasm-opt >/dev/null 2>&1; then
  echo "running wasm-opt -Os"
  wasm-opt -Os "$WASM" -o "${WASM}.opt"
  mv "${WASM}.opt" "$WASM"
fi

if [ -f "$WASM" ]; then
  echo
  echo "build complete:"
  ls -lh "$WASM"
  echo
  echo "to run (after installing wasmtime):"
  echo "  wasmtime --dir=/tmp $WASM verify /tmp/rulake-fixture"
fi
