# `@ruvector/rulake` — Node.js SDK

napi-rs bindings for [`ruvector-rulake`](https://github.com/ruvnet/RuLake) —
a vector cache + federation intermediary that sits in front of whatever
data lake already holds your vectors.

Implements [ADR-003](../docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md).

## Install

```bash
npm install @ruvector/rulake     # once published — prebuilt binaries
```

`npm install` resolves the right per-platform `@ruvector/rulake-<triple>`
package via `optionalDependencies` and pulls only the matching `.node`
binary.

## Build from source

This repo uses a Git submodule for the upstream RaBitQ kernel
([ADR-001](../docs/adrs/ADR-001-standalone-repo-strategy.md)). Clone
with submodules, then build:

```bash
git clone --recurse-submodules https://github.com/ruvnet/RuLake
cd RuLake/node

# Local dev loop — cargo build + copy the .so/.dylib into place.
cargo build --release
cp target/release/libruvector_rulake_node.so rulake.linux-x64-gnu.node   # Linux x86_64
# or:  cp target/release/libruvector_rulake_node.dylib rulake.darwin-arm64.node
# or:  cp target/release/ruvector_rulake_node.dll rulake.win32-x64-msvc.node

# Run smoke tests.
node --test __test__/smoke.test.mjs
```

The supported release path uses `@napi-rs/cli`:

```bash
npx napi build --platform --release
```

— which produces the `.node` binary with the right filename and
generates `index.js` / `index.d.ts` / `binding.js` directly. The
hand-written equivalents in this directory match what the CLI would
produce, so switching to it is a no-op.

## Usage

```ts
import { RuLake, LocalBackend, Bundle, Consistency, RuLakeError } from "@ruvector/rulake";

const lake = new RuLake(20, 42n)
  .withConsistency(Consistency.eventual(5_000))
  .withMaxCacheEntries(1024);

const be = new LocalBackend("local");
const N = 10_000, D = 768;
const ids = new BigInt64Array(N);
for (let i = 0; i < N; i++) ids[i] = BigInt(i);
const vectors = new Float32Array(N * D);  // fill with embeddings...
await be.putCollection("docs", ids, vectors, D);
lake.registerLocalBackend(be);

const q = new Float32Array(D);  // your query embedding
for (const hit of await lake.searchOne("local", "docs", q, 10)) {
  console.log(hit.backend, hit.collection, hit.id /* bigint */, hit.score);
}

// Federate across backends:
const hits = await lake.searchFederated([["local", "docs"]], q, 5);

// Bundles — language-portable witness-anchored sidecars:
const b = new Bundle("s3://bucket/path", D, 42n, 20, 1n);
await b.writeToDir("/tmp/snapshot");
const b2 = await Bundle.readFromDir("/tmp/snapshot");
console.assert(b2.verifyWitness());

// Cache-first KPI (ADR-155 §M1.5 — target ≥ 0.95):
console.log(lake.cacheStats().hitRate);
```

## Conventions

- **Vectors are `Float32Array`.** The binding borrows them and copies
  *once* at the FFI boundary (the borrow can't cross `await` to a libuv
  worker thread). One memcpy at memory bandwidth, ~3 µs at D=768.
- **IDs are `BigInt64Array` going in, `bigint` coming out.** Rust ids
  are `u64`; we don't silently truncate to `Number.MAX_SAFE_INTEGER`.
- **Async-only API at the JS level** (ADR-003 §3). Every method that
  does work runs on a libuv worker thread and returns `Promise<T>`.
  Pure getters (`cacheStats()`, `cacheEntryCount()`) are sync.
- **Errors are `RuLakeError` with a `.code` discriminator** (ADR-003
  §6). The native binding throws plain `Error` with the code as a
  prefix; the JS wrapper rewraps into a typed class.

## What's not (yet) here

See ADR-003 §"Open questions":

- **WASM build (`@ruvector/rulake-wasm`)** — v2. Browser, Cloudflare
  Workers, Deno-deploy, Bun. Loses AVX-512 popcnt and rayon parallel
  fan-out, so it's a feature-reduced surface.
- **HTTP client variant (`@ruvector/rulake/http`)** — v2.
- **JS-implemented `BackendAdapter`** — v2.

## License

MIT OR Apache-2.0, matching the parent crate.
