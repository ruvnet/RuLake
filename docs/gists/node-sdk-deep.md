# ruLake Node.js / TypeScript SDK — A Deep Introduction

## TL;DR

The ruLake Node SDK is a napi-rs binding that exposes the Rust crate's public surface — `RuLake`, `LocalBackend`, `FsBackend`, `Bundle`, `Consistency`, `SearchResult`, `CacheStats` — to a TypeScript audience that lives on the libuv event loop and never wants to see `cargo`. It is async-first (every method that does work returns `Promise<T>`, with the work scheduled on libuv worker threads), zero-copies query vectors through napi-rs `Float32Array`, and ships ESM-first with a `.cjs` shim plus per-platform prebuilt `.node` binaries delivered as `optionalDependencies` so `npm install` never compiles. The crate `rulake-node` lives at `node/` as a sibling of the parent crate per ADR-001; the npm package is `rulake` and the v2.2.0 release shipped in commit `2fb1730 Implement Python (PyO3) and Node.js (napi-rs) SDKs` together with its Python sibling.

## Introduction

Node.js / TypeScript is the second-largest audience for "vector cache in front of my existing data". The dominant deployment patterns are edge-and-serverless RAG (Vercel Edge, Cloudflare Workers, AWS Lambda, where the application talks to a vector DB and an LLM from a TypeScript handler), long-lived Node servers (NestJS, Fastify, plain Express, serving RAG endpoints to web/mobile front-ends), and TypeScript orchestration frameworks (LangChain.js, LlamaIndex.ts, Mastra, Agentic) where the framework is TS and the vector store is one of many pluggable adapters. None of these audiences will tolerate "shell out to a Rust binary" or "stand up a gRPC server in front of ruLake"; they expect `import { RuLake } from "rulake"` and a `Promise`-shaped API.

ADR-003 (`docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md:42`-area) is honest about the constraint set. The SDK has to (1) hide Rust behind an npm package that installs a prebuilt binary, (2) preserve the 1.02× cache-hit tax — the headline claim from `BENCHMARK.md` and ADR-155 — across the FFI hop, with vectors flowing as `Float32Array` and IDs as `bigint`, (3) be ESM-first with a CJS shim because the TS ecosystem has decisively moved to ESM (Node 22+ native, Vite/esbuild/tsx default ESM), (4) run off the Node main thread because RAG handlers are I/O-heavy and cannot afford a 1–10 ms blocking scan on the event loop, (5) provide TypeScript types out of the box generated from the Rust source rather than hand-written, and (6) distribute prebuilt binaries for the platforms Node users actually run on. The constraints are concrete and they shape every load-bearing decision in the ADR.

The reason the SDK is being built *now* rather than as a follow-on is the same as for the Python SDK — the Rust surface is stable enough that the binding is not a moving target. The relevant exports cited in ADR-003 §Context (`RuLake`, `LocalBackend`, `FsBackend`, `Bundle`, `Consistency`, `SearchResult`, `CacheStats`, `PerBackendStats`, the error type) are the same surface ADR-002 wraps for Python. Common decisions — zero-copy buffers, error map shape, the "no abstraction tax" budget — are made jointly across ADR-002 and ADR-003 so the two SDKs do not diverge on shared questions.

The deeper reason this matters specifically for Node is the platform difference from Python. Python users have threadpool ergonomics that release the GIL — a sync API that releases the GIL inside Rust gives them concurrency for free through `concurrent.futures.ThreadPoolExecutor`. Node users have a single event loop and any blocking call destroys that loop's responsiveness. A sync `searchOneSync` in Node would block 1–5 ms per call, which is unacceptable in a Vercel/AWS-Lambda or Express handler. ADR-003 §3 commits to async-only at the JS level for exactly this reason: the platform is different, the foot-gun is different, the right default is different. The Python SDK ships sync-first because that is right for Python; the Node SDK ships async-first because that is right for Node. Common shape, per-platform discipline.

The economic shape is the same as for any binding wrapping a fast kernel. The per-call FFI cost is one `memcpy` of the query vector — borrowing `query.as_ref()` ties the slice's lifetime to the `Float32Array` parameter, which does not survive across `await`, so napi-rs's `tokio::task::spawn_blocking` pattern needs an owned copy. The copy is one `memcpy` at memory bandwidth — ~3 µs for D = 768 at PC-DDR5 speeds — and the cache scan itself is much larger, so the relative tax stays under 1.05× for the single-query path. For batch search the cost amortises across the batch. The bench gate (`node/test/bench-tax.mjs` in ADR-003 §Verification) keeps the ratio against direct Rust ≤ 1.10×, the same budget as the Python SDK and for the same reason: ~5 µs of FFI per-call overhead on top of the Rust 1.02×.

## The decision in detail

ADR-003 makes six coupled decisions; the load-bearing four are the binding library, the buffer protocol, the async-first API, and the per-platform prebuilt distribution. The other two (ESM-first packaging, single-class error with `code` discriminator) are the ergonomic match for Node's idioms.

The first is **napi-rs** rather than Neon, WebAssembly-only, or a pure-TypeScript rewrite. napi-rs is the only Rust↔Node binding library that targets Node-API (the stable ABI), so a single `.node` binary works across Node 18, 20, 22, 24 without recompile; generates TypeScript types automatically from `#[napi]` annotations (no hand-written `.d.ts` to drift); supports async functions that return `Promise<T>` with the work scheduled on the libuv thread pool; supports TypedArray zero-copy via `Float32Array` parameter types with `.as_ref::<f32>()` returning a borrowed slice; and ships a prebuild + optionalDependencies distribution pattern via `@napi-rs/cli` that the rest of the Node ecosystem (Prisma, next-swc, parcel-css, lightningcss) has battle-tested. Neon is rejected as predating the maturity of napi-rs and lagging on async-fn ergonomics, TypeScript-type generation, and the prebuild pipeline. WebAssembly via `wasm-bindgen` is reserved as an *additive* v2 package (`rulake-wasm`) — WASM SIMD does not reach AVX-512 VPOPCNTDQ, threads are gated behind experimental flags, and the 1.02× tax becomes ~1.5–2×, which eats the differentiation.

The second is the **buffer protocol**. Every API that takes a vector takes `&Float32Array` and calls `.as_ref::<f32>()` (napi-rs), which returns `&[f32]` *without copying* when the underlying ArrayBuffer is contiguous (always true for `new Float32Array(n)`). For top-k ≤ 100 (the common case) the result returns as `Vec<SearchResult>` — napi-rs converts to a JS array of `{ backend, collection, id, score }` objects. The per-object cost is ~100 ns at k = 10. For top-k > ~500 the ADR reserves `searchOneArrays(...)` returning `{ ids: BigUint64Array, scores: Float32Array }` as a v1.5 addition (Open question §4). IDs are `bigint` because Rust `u64` does not fit in a JS Number once the high bit is used; napi-rs serialises `u64` to `bigint` by default. The ergonomic tax is `42n` instead of `42`, paid once per use and type-checked by TypeScript; the ADR is explicit that silent precision loss on the high bit is worse than the typing tax.

The third is **async-first** with the work running on libuv worker threads via `tokio::task::spawn_blocking`. Every method that does work returns `Promise<T>`. Pure-data getters that do not do work (`cacheStats()`, `backendIds()`, `cacheEntryCount()`) stay sync — they take a lock briefly and return. The threshold the ADR commits to (§3): anything that takes longer than ~50 µs in the worst case is async; everything else is sync. Documented per-method in the generated `.d.ts`. The single FFI-boundary copy on the query vector — the ~3 µs `memcpy` — is unavoidable because the `as_ref()` borrow does not survive across the `await` that hands control to the worker pool; ADR-003 §2 calls this out explicitly as the only honest cost, measured at < 5 µs at D = 768.

The fourth is **per-platform prebuilt binaries via `optionalDependencies`**. The umbrella package `rulake` ships with five `optionalDependencies` (`rulake-linux-x64-gnu`, `rulake-linux-arm64-gnu`, `rulake-darwin-x64`, `rulake-darwin-arm64`, `rulake-win32-x64-msvc` — see `node/package.json:101`-area). Each platform-specific package contains a single `.node` binary plus a `package.json` with `os` / `cpu` / `libc` constraints, so npm only downloads the right binary per host. The ADR's §5 walks through why this beats `postinstall` / `prebuild-install`: it works in air-gapped environments, behind corporate npm registries, with `npm ci` lockfile guarantees, and inside Lambda layers / Vercel build cache / Docker layer cache — none of which work cleanly with a postinstall download.

| trade-off | what got picked | what got rejected | why |
|---|---|---|---|
| binding library | napi-rs (Node-API stable ABI) | Neon, hand-rolled JSON-RPC | Neon's async story is callback-heavier; napi-rs has the macro surface and the prebuild pipeline. |
| API style | async-only | sync-first (mirroring ADR-002 Python) | Node has a single event loop; a sync API is a foot-gun in a way that the Python sync API is not. |
| packaging | ESM-first with `.cjs` shim | CJS-first with ESM shim, ESM-only | Vercel / Cloudflare / Deno / Bun default to ESM; CJS users still get a working module. |
| binary distribution | per-platform `optionalDependencies` | fat tarball with five `.node` files | Fat would weigh ~25 MB and cost every `npm install` even on CI that does not need the binary. |
| browser support | not in v1; `rulake-wasm` reserved for v2 | WASM as the v1 default | WASM raises tax to ~1.5–2× and loses AVX-512; "native by default, WASM additive" is the right shape. |
| errors | single class with `code` discriminator | multi-class hierarchy (Python-style) | JS exceptions do not multi-inherit cleanly; `code` matches `SystemError`/AWS SDK idiom. |

## Capabilities

The binding's capability surface mirrors ADR-002 in shape and the Rust crate in semantics, with the Node-idiomatic adjustments. `new RuLake(rerankFactor, rotationSeed)` constructs a lake — `rotationSeed` is `bigint` because it is a Rust `u64`. `withConsistency(c)` and `withMaxCacheEntries(n)` swap configuration. `registerLocalBackend(b)` and `registerFsBackend(b)` add backends. `searchOne(backend, collection, query, k)` returns a `Promise<SearchResult[]>` where `query: Float32Array` is the zero-copy input. `searchFederated(targets, query, k)` takes `targets: string[][]` (an array of `[backend, collection]` pairs) and runs the parallel fan-out. `searchBatch(backend, collection, queries, dim, k)` amortises FFI cost over a batch where `queries: Float32Array` is row-major of length `n*dim`. `publishBundle(backend, collection, dir)`, `refreshFromBundleDir(...)` (returning `"up_to_date" | "invalidated" | "bundle_missing"`), `saveCacheToDir(...)`, and `warmFromDir(...)` cover the bundle and lifecycle methods. The full TypeScript shape lives at `node/index.d.ts`; the ESM entry at `node/index.mjs` re-exports from the CJS shim at `node/index.cjs`.

`Bundle` (`node/index.d.ts:58`-area) takes `(dataRef, dim, rotationSeed: bigint, rerankFactor, generation: bigint | string)` and exposes `rvfWitness` (the SHAKE-256(32) hex, 64 chars), `formatVersion`, `piiPolicy`, `lineageId`, `memoryClass`, plus `verifyWitness()`, `toJson()`, `Bundle.fromJson(s)`, `writeToDir(dir): Promise<string>`, `Bundle.readFromDir(dir): Promise<Bundle>`, and the chained tag methods `withPiiPolicy(p)` / `withLineageId(id)` / `withMemoryClass(klass)`. The `generation: bigint | string` discriminated parameter mirrors the Python SDK's `int | str` polymorphism and corresponds to the Rust `Generation::Num(u64) | Generation::Opaque(String)` enum at `src/bundle.rs:82`.

The exception model is a single `RuLakeError extends Error` class with a `code: string` discriminator. The mapping (ADR-003 §6, reflected at `node/index.d.ts:8`-area):

| Rust variant | `error.code` |
|---|---|
| `RuLakeError::BackendNotFound` | `RULAKE_BACKEND_NOT_FOUND` |
| `RuLakeError::CollectionNotFound` | `RULAKE_COLLECTION_NOT_FOUND` |
| `RuLakeError::DimensionMismatch` | `RULAKE_DIMENSION_MISMATCH` |
| `RuLakeError::InvalidParameter` | `RULAKE_INVALID_PARAMETER` |
| `RuLakeError::Backend(s)` | `RULAKE_BACKEND` |
| `RuLakeError::Rabitq(_)` | `RULAKE_RABITQ` |

This idiom matches Node's `SystemError`, AWS SDK errors, and similar — it avoids the multi-inheritance contortions ADR-002 documents for Python because JS exceptions do not multi-inherit cleanly.

A worked example. Suppose you have 10,000 documents whose embeddings live in a `Float32Array` of length `10000 * 768`, and a query vector `q` of length 768. The flow is:

```js
import { RuLake, LocalBackend, Consistency } from "rulake";
const lake = new RuLake(20, 42n).withConsistency(Consistency.eventual(5000));
const be = new LocalBackend("local");
const ids = new BigInt64Array(10_000); for (let i = 0; i < 10_000; i++) ids[i] = BigInt(i);
const vs  = new Float32Array(10_000 * 768);  // populated however you embed
await be.putCollection("docs", ids, vs, 768);
lake.registerLocalBackend(be);
const hits = await lake.searchOne("local", "docs", q, 10);
for (const h of hits) console.log(h.backend, h.collection, h.id, h.score);  // h.id is bigint
```

Behind the scenes, `putCollection` zero-copies `vs` through napi-rs's `Float32Array.as_ref()`. `searchOne` takes one `memcpy` of `q` at the FFI boundary (the only honest cost ADR-003 §2 acknowledges, since the borrow does not survive across `await`), schedules the search on a libuv worker thread via `tokio::task::spawn_blocking`, and resolves the `Promise` with a `SearchResult[]` whose `id` field is `bigint`. A RAG handler can fan out 100 concurrent `searchOne` calls and the event loop keeps serving other connections — that is the point of the async-first commitment.

The HTTP entry-point shape — `import { ... } from "rulake/http"` — is exposed in `node/package.json:15` (`./http`) and the `node/http.d.ts` / `node/http.mjs` files. ADR-003 §Open question §2 reserves the actual wire client until a customer asks for it; v2.2.0 ships the import path as a documented surface for forward compatibility.

## Trust & correctness contract — no abstraction tax

The trust contract for ADR-003 is "no abstraction tax + zero-copy where it matters", same shape as ADR-002 and same enforcement discipline.

The first mechanism is the **buffer-protocol borrow**. Every vector parameter takes a `Float32Array` and calls `.as_ref::<f32>()` to obtain `&[f32]`. The borrow is contiguous because `new Float32Array(n)` always produces a contiguous ArrayBuffer. The single honest copy is the one `memcpy` from `Float32Array` to an owned `Vec<f32>` before crossing the `await` boundary into `tokio::task::spawn_blocking` — ADR-003 §2 documents this as the unavoidable cost of async-on-worker-thread semantics, measured at < 5 µs at D = 768. Batch search amortises the copy across the batch; the relative cost is < 1%.

The second is **async work runs off the main thread**. Every method that does work returns `Promise<T>` with the body inside `spawn_blocking`. The ADR's threshold (§3): anything that takes longer than ~50 µs in the worst case is async; everything else is sync. This is the contract that lets a Node handler fan out 100 concurrent searches without single-threading the event loop. A regression here — a search method that ran on the main thread or a sync method that exceeded 50 µs — would surface in the bench gate and in production tail-latency telemetry.

The third is **TypeScript types generated from the Rust source**. napi-rs's `define_package_json` pipeline regenerates `node/index.d.ts` from the `#[napi]` annotations on every release; the hand-written version checked in (`node/index.d.ts:1`-area) is the authoritative shape until the generator runs in CI, at which point the two are diff-checked. The ADR's §1 ("Bindings via napi-rs") makes this a stated property: there is no hand-maintained `.d.ts` to drift.

The fourth is **the bundle contract preservation**. `Bundle(...)` constructs a `RuLakeBundle` via the same `RuLakeBundle::new` (`src/bundle.rs:166`) the Python SDK reaches; the witness recipe at `src/bundle.rs:362` is unchanged across language boundaries. A Node process and a Python process constructing the same bundle (same `dataRef`, dim, seed, rerank, generation) produce byte-identical witnesses — the witness recipe is purely a function of bytes. `Bundle.fromJson(s)` and `Bundle.readFromDir(dir)` propagate the witness-fail-closed posture from `src/bundle.rs:340`-area; a Node caller reading a tampered bundle gets an `RuLakeError` with `code: RULAKE_WITNESS_*`, never silently bad data.

The fifth is **the per-platform binary integrity** through npm's `optionalDependencies`. Each platform package contains exactly one `.node` binary plus a constrained `package.json`; the umbrella package resolves the right one via npm's `os` / `cpu` / `libc` matching. The `prepublishOnly` script at `node/package.json:86` enforces the submodule-present check (per ADR-001) before publish: if `vendor/ruvector/Cargo.toml` is missing, the publish fails with the instructional message `vendor/ruvector submodule missing — run git submodule update --init --recursive (ADR-001)`. The check is one `fs.existsSync` call and runs on every publish, blocking the regression at the source.

The bench gate (`node/test/bench-tax.mjs`, ADR-003 §Verification) measures async `searchOne` QPS against a direct Rust binary at n = 100 k, D = 128, k = 10 and asserts the ratio stays ≤ 1.10× — same budget as ADR-002, same reasoning (allow ~5 µs of FFI per-call overhead on top of the Rust 1.02×). A regression past 1.10× blocks a release.

## Reference implementation status

The crate `rulake-node` v2.2.0 lives at `node/`. The npm package is `rulake`; the cdylib is the per-platform `.node` binary (`rulake.linux-x64-gnu.node`, `rulake.linux-arm64-gnu.node`, `rulake.darwin-x64.node`, `rulake.darwin-arm64.node`, `rulake.win32-x64-msvc.node`). As of v2.2.0 (commit `2fb1730 Implement Python (PyO3) and Node.js (napi-rs) SDKs`, called out in ADR-003 §Status), the surface that is shipping:

- `RuLake` constructor `(rerankFactor: number, rotationSeed: bigint)`, with `withConsistency`, `withMaxCacheEntries`, `registerLocalBackend`, `registerFsBackend`, `backendIds`, `cacheStats`, `cacheEntryCount`, `cacheWitnessOf`, `invalidateCache`, `searchOne`, `searchFederated`, `searchBatch`, `publishBundle`, `refreshFromBundleDir`, `saveCacheToDir`, `warmFromDir` (per `node/index.d.ts:131`-area).
- `LocalBackend(id)` with `putCollection`, `append`.
- `FsBackend(id, root)` with `register`, `write`.
- `Bundle(dataRef, dim, rotationSeed, rerankFactor, generation)` with the full method surface — `verifyWitness`, `toJson`, `fromJson`, `writeToDir`, `readFromDir`, plus the chained tag methods (`node/index.d.ts:58`-area).
- `Consistency.fresh()`, `Consistency.eventual(ttlMs)`, `Consistency.frozen()`.
- `SearchResult` interface, `CacheStats` interface with `hitRate` and `avgPrimeMs` rollups.
- `RuLakeError extends Error` with `code: string` discriminator.
- ESM-first packaging: `"type": "module"`, `"main": "index.cjs"`, `"module": "index.mjs"`, `"types": "index.d.ts"`, with the `exports` map at `node/package.json:9`-area routing `./` to all three shapes and `./http` to the (reserved) HTTP entrypoint.
- Per-platform `optionalDependencies` for the prebuilt binaries (`node/package.json:101`-area).
- napi configuration: `napi8` (Node-API ≥ Node 12.22 / 14.17 / 16.0) per `node/Cargo.toml:25`. `engines.node >= 18` per `node/package.json:34`.

What v2.2.0 does *not* ship, per ADR-003 §Open questions:

- WASM build (`rulake-wasm`, v2 — browser/Workers/Deno/Bun; reduced surface, no rayon, no AVX-512). The umbrella `package.json:107` reserves `rulake-wasm = "2.2.1"` so v2 does not fight for the name.
- HTTP client (`rulake/http`, v2 — same shape as Python `rulake.client`); export path reserved at `node/package.json:15`.
- JS-side `BackendAdapter` (v2 — same problem as Python ADR-002 §F: JS `pull_vectors` re-enters V8 from a worker thread via `ThreadsafeFunction` at ~10 µs per call).
- High-k `searchOneArrays` (v1.5, same k > ~500 trigger as Python).
- Sharing one `RuLake` across multiple `worker_threads` (v2; structured cloning vs shared references is complex).
- Streaming results via `AsyncIterable` (v2 if a customer asks).

## Composition with the rest of ruLake

The Node SDK is a thin shim over the same Rust surface ADR-002 wraps for Python; the consequences for composition are the same shape.

**The cache and federation primitives are reachable.** `lake.searchFederated(targets, query, k)` from Node reaches `RuLake::search_federated` at `src/lake.rs:521`, which fans out across registered backends in parallel via rayon. A Node caller assembling a 10-shard federated query pays one `memcpy` on the query vector at the FFI boundary and one JS-array allocation on the result; the parallel scan happens in pure Rust on libuv worker threads. The bench gate confirms this stays inside the 1.10× tax budget.

**The bundle and witness contract is reachable.** `new Bundle(...)` from Node constructs a `RuLakeBundle` via `RuLakeBundle::new` (`src/bundle.rs:166`), which calls `compute_witness` (`src/bundle.rs:362`) to produce the SHAKE-256(32) hex witness. A Node process and a Python process and a Rust process constructing the same bundle produce byte-identical witnesses, because the witness recipe is purely a function of bytes. `Bundle.fromJson` and `Bundle.readFromDir` propagate the witness-fail-closed posture; a Node caller reading a tampered bundle gets a `RuLakeError`, never silently bad data. The DoS hardening (64 KiB body cap, 4 KiB per-field cap, 128-char witness cap from `src/bundle.rs:218`-area) is inherited verbatim.

**The MCP server and substrate scaffolds compose underneath.** ADR-004's MCP server consumes the same `RuLake` instance the Node SDK constructs. A Node process can construct a `RuLake`, register backends, and let the in-tree MCP server expose those backends to a Claude Desktop / Cursor / Cline client over stdio — same `RuLake`, two language entry-points, one cache. ADR-005's IPFS backend produces bundles that `Bundle.fromJson` will accept; cross-deployment cache via CID works the same way regardless of which language constructed the bundle.

The shape ADR-003 commits to is the same shape ADR-002 commits to for Python, with the Node-idiomatic adjustments (async-only, `bigint` IDs, single-class error with `code`, ESM-first). Common decisions are made jointly across the two ADRs; per-language decisions are made per-language. The result is two SDKs that feel native in their respective ecosystems while wrapping the same Rust kernel, both subject to the same 1.10× tax budget.

## Open questions

Six honest unknowns track in the ADR. The WASM build (`rulake-wasm`) is the most-asked but the most-uncertain — if the tax stays under ~2× direct Rust the package ships, but if it explodes past 5× the right answer is to reconsider whether browser-side ruLake is the right shape at all. The HTTP client (`rulake/http`) waits for a customer ask; the export path is reserved so the introduction is non-breaking. JS-side `BackendAdapter` is the same problem as Python's ADR-002 §F and waits for the same trigger (a real JS-only backend). High-k `searchOneArrays` waits for a benchmark filing past k > ~500. Sharing one `RuLake` across multiple Node `worker_threads` is complex (structured-cloning vs shared references) and v2 if it lands at all. Streaming results via `AsyncIterable` is useful for time-to-first-hit UIs but speculative without a consumer. None blocks v2.2.0; each is honest about being unresolved.

## References

- ADR-003: `/home/ruvultra/projects/RuLake/docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md`
- Crate manifest: `/home/ruvultra/projects/RuLake/node/Cargo.toml`
- npm package manifest: `/home/ruvultra/projects/RuLake/node/package.json`
  - exports map (ESM-first + CJS shim + `./http`): `node/package.json:9`
  - per-platform binary list: `node/package.json:21`
  - `optionalDependencies` for prebuilt binaries: `node/package.json:101`
  - `prepublishOnly` submodule guard (per ADR-001): `node/package.json:86`
- TypeScript declarations: `/home/ruvultra/projects/RuLake/node/index.d.ts`
  - `RuLakeError extends Error` with `code` discriminator: `node/index.d.ts:8`
  - `Consistency` factory: `node/index.d.ts:25`
  - `SearchResult` interface: `node/index.d.ts:35`
  - `Bundle` (witness-anchored, with `bigint | string` generation): `node/index.d.ts:58`
  - `RuLake` async-first method surface: `node/index.d.ts:131`
- ESM entrypoint (re-exports from CJS shim): `/home/ruvultra/projects/RuLake/node/index.mjs`
- Tests: `/home/ruvultra/projects/RuLake/node/__test__/`
- Sibling-crate discipline (no workspace, prebuilt-binary distribution): ADR-001 (`docs/adrs/ADR-001-standalone-repo-strategy.md`)
- Companion SDK ADR for Python (joint decisions): `docs/adrs/sdk/ADR-002-python-sdk.md`
- Companion ADR for the MCP server (consumes the same `RuLake`): `docs/adrs/sdk/ADR-004-rulake-mcp-server.md`
- Public Rust surface the binding wraps: `src/lib.rs:53`-area
- Bundle witness recipe (preserved across the FFI): `src/bundle.rs:166` (`RuLakeBundle::new`), `src/bundle.rs:362` (`compute_witness`), `src/bundle.rs:340`-area (witness-fail-closed in `read_from_dir`)
- DoS-hardening size caps the binding inherits: `src/bundle.rs:218`-area, `src/backend.rs:60`-area
- Federation primitive reachable from Node: `src/lake.rs:521` (`search_federated`)
- napi-rs / Node-API references — the ADR cites these as opaque versioned dependencies (`napi = "2.16"`, `@napi-rs/cli ^2.18.4`, Node-API version `napi8`). The napi-rs project README and the Node-API documentation at the Node.js docs site are the upstream documentation; ADR-003 does not pin URLs, treating them as opaque versioned identifiers.
- Prior-art prebuilt-distribution pattern (cited in ADR-003 §5): Prisma, next-swc, parcel-css, lightningcss, datadog-lambda-extension — all use `@napi-rs/cli`'s prebuild + optionalDependencies pattern.
