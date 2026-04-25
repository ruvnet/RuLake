# ruLake — Rust WASM examples

Cross-language interop examples that compile ruLake's bundle witness
verification logic to WebAssembly. Each example is a standalone Cargo
package (`[workspace]` empty) so the wasm32 builds don't pull the host
ruLake crate's `std::sync::Mutex` / `rayon` dependencies.

| # | dir                              | target                           | runtime              |
|---|----------------------------------|----------------------------------|----------------------|
| 1 | `01-witness-verifier-browser/`   | `wasm32-unknown-unknown` (web)   | any modern browser   |
| 2 | `02-witness-verifier-nodejs/`    | `wasm32-unknown-unknown` (node)  | Node.js ≥ 18         |
| 3 | `03-bundle-toolkit-cli/`         | `wasm32-wasip1`                  | wasmtime / wasmer    |

Examples 1 and 2 share the same Rust source (`src/lib.rs`) but build for
different wasm-bindgen targets. Example 3 is pure WASI, no
wasm-bindgen, suitable for sandboxed pipelines.

## Why WASM?

The full ruLake crate uses `std::sync::Mutex` and `rayon`, neither of
which compiles cleanly to `wasm32-unknown-unknown`. The bundle witness
module is the carve-out: pure data + SHA3, zero unsafe, zero
non-portable deps. This makes it the natural surface for cross-language
verification of ruLake bundles produced by Rust ruLake servers.

A browser app or a Node tool can verify ruLake bundles end-to-end with
no backend round-trip — the witness is the cryptographic anchor that
makes this work.

## Witness algorithm

```
SHAKE-256(32) of:
    "rulake-bundle-witness-v1|"
    || u64_le(len(data_ref)) || data_ref
    || "|"
    || u64_le(dim) || u64_le(rotation_seed) || u64_le(rerank_factor)
    || "|"
    || u64_le(len(generation_bytes)) || generation_bytes
```

`generation_bytes`:

- `Num(n)`: `0x00 || u64_le(n)` — 9 bytes
- `Opaque(s)`: `0x01 || utf8_bytes(s)`

The variant tag byte (`0x00` / `0x01`) was added by the 2026-04-23
audit to prevent `Num(7)` colliding with
`Opaque("\x07\0\0\0\0\0\0\0")`. Each WASM example carries the same tag
byte; the fixtures are byte-identical to ones produced by the host
crate's `RuLakeBundle::new`.

## Quickstart

```bash
# Browser demo
cd 01-witness-verifier-browser && ./build.sh && python3 -m http.server 8000
# → open http://localhost:8000/index.html

# Node verifier + tests
cd 02-witness-verifier-nodejs && ./build.sh && npm test

# WASI CLI
cd 03-bundle-toolkit-cli && ./build.sh
# wasmtime --dir=/tmp target/wasm32-wasip1/release/bundle-toolkit.wasm verify /tmp/rulake-fixture
```

## Toolchain assumptions

- Rust ≥ 1.85 (uses 2021 edition; the host crate sets MSRV 1.89)
- `rustup target add wasm32-unknown-unknown wasm32-wasip1`
- `cargo install wasm-pack` (for examples 1 and 2)
- `wasmtime` or `wasmer` (for example 3, optional but recommended)
- `wasm-opt` / `binaryen` — optional, all build scripts use it if
  present and skip the optimization pass otherwise

## Skipped optional tooling

The verifier swarm flagged that some sizing/CVE numbers depend on
tools that may not be installed on every host. Each `build.sh`
gracefully skips them when absent:

- **`wasm-opt`** — when present, applies `-Os` post-pass; shrinks
  examples 1/2 from ~127 KB to ~80–95 KB and example 3 from ~315 KB to
  ~200–220 KB. Without it, the wasm-pack output ships as-built.
- **`cargo audit`** — when present, runs against each example's lockfile
  for known CVEs. Without it, no CVE scan is performed and the deps
  are trusted as-pinned in `Cargo.toml`.
- **`wasmtime` / `wasmer`** — only example 3 needs a WASI runtime to
  actually execute. Without one, the wasm builds successfully but is
  not exercised end-to-end. Install via
  `curl https://wasmtime.dev/install.sh -sSf | bash`.

Each per-example README repeats the relevant skip-conditions in
context. None of these tools are required for the build to succeed —
they only affect output size, supply-chain assurance, and runtime
verification respectively.

## Producing a real fixture

```bash
cd /home/ruvultra/projects/RuLake
cargo run --release --example sidecar_daemon
# bundle is at /tmp/rulake-sidecar-demo-<pid>/table.rulake.json until cleanup
```

The captured `01-witness-verifier-browser/fixtures/known-good-bundle.json`
is exactly such a sidecar, frozen so the demo works offline.
