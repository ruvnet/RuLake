#!/usr/bin/env bash
# Build the rulake-wasm package for browsers, Cloudflare Workers,
# Deno, Bun, and Node.js fallback.
#
# Produces three sibling `pkg/` dirs (web / nodejs / bundler) so npm
# consumers can pick the one their bundler / runtime expects.
#
# Prereqs:
#   - rustup target add wasm32-unknown-unknown
#   - cargo install wasm-pack
# Optional:
#   - wasm-opt (binaryen) for an extra ~30% size win
set -euo pipefail

# Some hosts export RUSTFLAGS=-C link-arg=-fuse-ld=mold which the
# wasm linker rejects. Strip it for the build.
unset RUSTFLAGS CARGO_BUILD_RUSTFLAGS

cd "$(dirname "${BASH_SOURCE[0]}")"

echo "==> wasm-pack build --target web"
wasm-pack build --release --target web --out-dir pkg-web --out-name rulake_wasm

echo "==> wasm-pack build --target nodejs"
wasm-pack build --release --target nodejs --out-dir pkg-nodejs --out-name rulake_wasm

echo "==> wasm-pack build --target bundler"
wasm-pack build --release --target bundler --out-dir pkg-bundler --out-name rulake_wasm

echo "==> sizes"
for d in pkg-web pkg-nodejs pkg-bundler; do
    f=$(find "$d" -name '*.wasm' -print -quit)
    if [[ -n "$f" ]]; then
        size=$(stat -c%s "$f")
        echo "  $d: $f → $((size / 1024)) KB"
    fi
done

if command -v wasm-opt >/dev/null 2>&1; then
    echo "==> wasm-opt -Oz pass"
    for d in pkg-web pkg-nodejs pkg-bundler; do
        f=$(find "$d" -name '*.wasm' -print -quit)
        if [[ -n "$f" ]]; then
            wasm-opt -Oz "$f" -o "$f.opt" && mv "$f.opt" "$f"
            size=$(stat -c%s "$f")
            echo "  $d: $((size / 1024)) KB after wasm-opt"
        fi
    done
else
    echo "==> wasm-opt not installed — skipping (install binaryen for ~30% size win)"
fi
