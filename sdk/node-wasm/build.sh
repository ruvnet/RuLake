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
# Out-dir matches the package.json exports map ("./web/...") so Vite
# and other resolvers find the entry without a separate copy step.
# Pre-clean the dir because wasm-pack refuses to write into a non-
# wasm-pack-managed directory.
rm -rf web && wasm-pack build --release --target web --out-dir web --out-name rulake_wasm

echo "==> wasm-pack build --target nodejs"
rm -rf nodejs && wasm-pack build --release --target nodejs --out-dir nodejs --out-name rulake_wasm

echo "==> wasm-pack build --target bundler (root files for fallback exports)"
# The bundler target's output is what the package.json's `main` and
# `module` fields point to (rulake_wasm.js at root). pkg-bundler is a
# scratch dir; we copy the artifacts up to root after wasm-pack.
rm -rf pkg-bundler && wasm-pack build --release --target bundler --out-dir pkg-bundler --out-name rulake_wasm
cp pkg-bundler/rulake_wasm.js \
   pkg-bundler/rulake_wasm.d.ts \
   pkg-bundler/rulake_wasm_bg.js \
   pkg-bundler/rulake_wasm_bg.wasm \
   pkg-bundler/rulake_wasm_bg.wasm.d.ts \
   .

echo "==> sizes"
for d in web nodejs pkg-bundler; do
    f=$(find "$d" -name '*.wasm' -print -quit)
    if [[ -n "$f" ]]; then
        size=$(stat -c%s "$f")
        echo "  $d: $f → $((size / 1024)) KB"
    fi
done

if command -v wasm-opt >/dev/null 2>&1; then
    echo "==> wasm-opt -Oz pass"
    for d in web nodejs pkg-bundler; do
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
