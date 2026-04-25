#!/usr/bin/env bash
# Build the browser-target ruLake bundle witness verifier.
#
# Outputs:
#   pkg/rulake_witness_verifier_browser_bg.wasm  — the wasm module
#   pkg/rulake_witness_verifier_browser.js       — ES-module glue
#
# Requirements:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-pack
# Optional (for size optimization):
#   binaryen / wasm-opt
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

# --target web → ES module loadable via <script type=module> in modern browsers.
# --release → opt-level=s, lto=true (see Cargo.toml [profile.release]).
wasm-pack build --target web --release

# Optional second pass with wasm-opt -Oz for the smallest possible binary.
WASM="pkg/rulake_witness_verifier_browser_bg.wasm"
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
  echo "next: cd $(pwd) && python3 -m http.server 8000"
  echo "then open http://localhost:8000/index.html"
fi
