#!/usr/bin/env bash
# Build the Node.js-target ruLake bundle witness verifier.
#
# Outputs:
#   pkg/rulake_witness_verifier_nodejs.js       — CommonJS module
#   pkg/rulake_witness_verifier_nodejs_bg.wasm  — the wasm payload
#
# Requirements:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-pack
set -euo pipefail

cd "$(dirname "$0")"

# The host environment may set RUSTFLAGS for native builds (e.g. mold linker).
# rust-lld for wasm targets does not understand `-fuse-ld=mold`, so strip
# RUSTFLAGS for the wasm build.
unset RUSTFLAGS CARGO_BUILD_RUSTFLAGS

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "error: wasm-pack not installed. Run: cargo install wasm-pack" >&2
  exit 1
fi

rustup target list --installed | grep -q wasm32-unknown-unknown || \
  rustup target add wasm32-unknown-unknown

# --target nodejs → CommonJS, ready for `require('./pkg')`.
wasm-pack build --target nodejs --release

WASM="pkg/rulake_witness_verifier_nodejs_bg.wasm"
if command -v wasm-opt >/dev/null 2>&1; then
  echo "running wasm-opt -Oz"
  wasm-opt -Oz "$WASM" -o "${WASM}.opt"
  mv "${WASM}.opt" "$WASM"
fi

if [ -f "$WASM" ]; then
  echo
  echo "build complete:"
  ls -lh "$WASM"
  echo
  echo "next: node verify.js fixtures/known-good-bundle.json"
  echo "tests: npm test"
fi
