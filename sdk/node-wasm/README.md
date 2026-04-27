# rulake-wasm — Memory Lake for Agentic AI (WASM, edge)

[![npm version](https://img.shields.io/npm/v/rulake-wasm.svg)](https://www.npmjs.com/package/rulake-wasm)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue.svg)](https://github.com/ruvnet/RuLake#license)

**Witness-anchored vector memory for AI agents — in the browser, in Cloudflare Workers, in Deno, in Bun.**

ruLake's edge build. Same SHAKE-256 witness chain that `rulake` (Rust crate, Python wheel, Node.js binary) uses, compiled to WebAssembly so your edge agent gets the same byte-exact memory guarantees without round-tripping to a server.

**Use cases**: in-browser RAG, Cloudflare Workers AI memory, Deno Deploy semantic search, Bun-based agent runtimes, on-device personal AI assistants, offline-first agentic apps.

## Install

```bash
npm install rulake-wasm
```

Wired as an `optionalDependencies` peer of [`rulake`](https://www.npmjs.com/package/rulake) per [ADR-003 §A](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md): npm consumers on edge platforms get this transparently; consumers on Linux / macOS / Windows get the native binary instead.

## Use

### Browser (ESM)

```html
<script type="module">
  import init, {
    verifyBundleJson,
    computeWitness,
    searchBruteForceL2,
    buildInfo,
  } from "https://unpkg.com/rulake-wasm/pkg-web/rulake_wasm.js";

  await init();
  console.log(buildInfo());

  // Verify a `table.rulake.json` bundle
  const sidecar = await fetch("/snapshots/memories/table.rulake.json").then(r => r.text());
  const r = verifyBundleJson(sidecar);
  console.log(r.ok ? "verified" : "tampered", r.computed);

  // Brute-force top-K over flat Float32Array (exact L2)
  const vectors = new Float32Array(N * 128);   // your embeddings
  const ids = new Float64Array(N);             // optional u64 ids (BigInt unsupported in TS arrays — pass as f64)
  const query = new Float32Array(128);
  const hits = searchBruteForceL2(vectors, ids, 128, query, 10);
  console.log(hits);  // [{idx, id, score}, ...] sorted ascending
</script>
```

### Cloudflare Workers / Deno / Bun

```js
import init, { verifyBundleJson, searchBruteForceL2 } from "rulake-wasm";
await init();
// same API — verifyBundleJson, computeWitness, searchBruteForceL2, formatVersion, buildInfo
```

### Node.js (fallback when the native binary isn't available)

```js
const { verifyBundleJson, searchBruteForceL2 } = require("rulake-wasm/nodejs");
// no init() needed for the nodejs target — synchronous load
```

## What's in this package

| API | Purpose |
|-----|---------|
| `verifyBundleJson(json)` | Parse a `table.rulake.json` and recompute its witness; returns `{ok, computed, stored, fields}`. Enforces 64 KiB / 4 KiB DoS caps. |
| `computeWitness(data_ref, dim, rotation_seed, rerank_factor, generation)` | Build a SHAKE-256(32) witness for a candidate bundle. Useful for client-side bundle building. |
| `searchBruteForceL2(vectors, ids, dim, query, k)` | Exact top-K nearest neighbors by squared L2. Recall = 1.0. Caps `k ≤ min(n, 4096)`. ~10 ms at n=50k, D=128 in browser WASM. |
| `formatVersion()` | Bundle format version this build understands (currently 2). |
| `buildInfo()` | `"rulake-wasm v2.2.0 (witness-format v2, sha3 = 0.10)"`. |

The witness output is **byte-identical** to the host crate's `compute_witness` (`src/bundle.rs`). Cross-runtime check: a sidecar produced by Rust ruLake on a server (witness `dea58c64…07d7e4`) verifies green from this WASM build in the browser.

## What's NOT in this package

These need `std::sync::Mutex` + `rayon` and are intentionally absent from the edge surface:

- The full `RuLake` cache + coherence layer
- Federated `search_federated` parallel fan-out
- Persistence (`save_cache_to_dir` / `warm_from_dir`)
- The `BackendAdapter` trait (you'd implement this in JS for the edge — v2 follow-up)
- AVX-512 popcount kernel (no SIMD beyond wasm32 baseline)

For the full surface, use [`rulake`](https://www.npmjs.com/package/rulake) (Node.js native binary) or the Rust crate.

## Build from source

```bash
git clone --recurse-submodules https://github.com/ruvnet/RuLake
cd RuLake/sdk/node-wasm
rustup target add wasm32-unknown-unknown
cargo install wasm-pack       # if not already installed
./build.sh                     # produces pkg-web/, pkg-nodejs/, pkg-bundler/
```

The build script `unset`s `RUSTFLAGS` because some host setups export
`-C link-arg=-fuse-ld=mold` which the wasm linker rejects.

## License

MIT OR Apache-2.0, matching the parent crate.
