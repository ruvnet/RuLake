# ruLake — A Cache-Coherent Vector Execution Fabric

[![Crates.io](https://img.shields.io/crates/v/ruvector-rulake.svg)](https://crates.io/crates/ruvector-rulake)
[![Rust 1.89+](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org)
[![RuVector](https://img.shields.io/badge/part_of-ruvector-purple.svg)](https://github.com/ruvnet/ruvector)
[![ruv.io](https://img.shields.io/badge/ruv.io-website-purple.svg)](https://ruv.io)
[![MIT / Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue.svg)](#license)

### **A cache layer for vector search — sits in front of whatever database, lakehouse, or file store already holds your vectors, and makes every query fast.**

> Created by [rUv](https://ruv.io). Part of the [RuVector](https://github.com/ruvnet/ruvector) ecosystem alongside [`ruvector-rabitq`](https://github.com/ruvnet/ruvector/tree/main/crates/ruvector-rabitq) (1‑bit compression kernel) and RVF (durable segment format). Designed to be the substrate for the [Cognitum](https://cognitum.one) Agentic Chip's memory hierarchy.

```bash
cargo add ruvector-rulake
```

#### You already have vectors somewhere — Parquet on S3, BigQuery rows, an Iceberg table, Snowflake, RVF segments, files on disk. You want fast, consistent semantic search without standing up a separate vector database.

#### **ruLake** is the piece in the middle. An app asks it for the nearest K vectors; it serves hits from a compressed in-memory cache at ≈**1.02× raw library speed**. On miss, it pulls from your backend, compresses with [RaBitQ](https://arxiv.org/abs/2405.12497) 1-bit quantization, and serves. Every entry is anchored by a cryptographic **witness** so two processes pointing at the same bytes share one compressed copy automatically.

Open source. ❤️ Free forever.

```text
            ┌───────────────── RuLake ──────────────────┐
 caller ──▶ │    Consistency: Fresh | Eventual | Frozen │ ──▶ SearchResult
            │                                           │
            │      ┌──── VectorCache (Arc'd) ────┐      │
            │      │   witness → RaBitQ index    │      │
            │      │   pointers: (be, coll) → w  │      │
            │      └──────────────▲───────────────┘     │
            │                     │ prime (on miss)     │
            │      ┌────── BackendAdapter trait ─────┐  │
            │      │  Parquet · BigQuery · Iceberg   │  │
            │      │  Delta · RVF · FsBackend · …    │  │
            │      └──────────────────────────────────┘  │
            └────────────────────────────────────────────┘
```

<details>
<summary>🔍 ruLake vs Typical Vector Databases (15 differences)</summary>

| | ruLake | Typical Vector DB |
|---|---|---|
| **Positioning** | | |
| Owns storage | ❌ — rides your existing lakehouse / filestore | ✅ — you move your data in |
| Intermediary tax | **1.01–1.03× direct RaBitQ** (measured) | n/a (owns the path) |
| Cross-process cache sharing | ✅ — content-addressed via SHAKE‑256 witness | ❌ |
| **Cache & coherence** | | |
| Three consistency modes | ✅ Fresh / Eventual / Frozen — sellable knob | One mode only |
| Witness-authenticated bundles | ✅ SHAKE‑256 over `(data_ref, dim, seed, rerank, gen)` | Mutation-id or snapshot-id only |
| Bundle sidecar protocol | ✅ atomic publish + refresh, 3-state reader | ❌ |
| Warm-restart from disk | ✅ `save_cache_to_dir` / `warm_from_dir`, byte-exact | Re-index on restart |
| **Search** | | |
| Adaptive per-shard rerank | ✅ `k' = max(5, global/K)` | Naive `k` per shard — under-recalls on skew |
| Federated fan-out | ✅ rayon parallel, per-shard over-request `k + √(k·ln S)` | Single-backend or sequential |
| Batch query API | ✅ `search_batch` — one lock + one coherence check per batch | Per-query overhead |
| **Kernels** | | |
| Tiered SIMD dispatch | ✅ scalar → AVX2 → AVX‑512 VPOPCNTDQ runtime-selected | Fixed kernel |
| Optional Hadamard rotation | ✅ `O(D log D)`, 32× smaller storage at D=128 | Dense Haar matrix |
| Parallel cache prime | ✅ rayon `from_vectors_parallel` — 11× at n=100k | Serial build |
| **Security** | | |
| Zero `unsafe` | ✅ (in ruLake crate itself) | Often uses unsafe |
| Path-traversal validated | ✅ FsBackend rejects `..`, `/`, drive letters, control bytes | n/a |
| JSON DoS caps | ✅ 64 KiB bundle, 4 KiB per field, 128-byte witness | n/a |

</details>

<details>
<summary>📋 See Full Capabilities (30+ features across 7 categories)</summary>

**Core cache + coherence**

| # | Capability | What It Does |
|---|------------|--------------|
| 1 | **Cache-first execution** | Hit path is one `Arc<RabitqPlusIndex>::search` — 1.02× direct cost, abstraction is free |
| 2 | **Witness-addressed storage** | SHAKE‑256(32) over the bundle anchors every entry; content-addressed dedup across processes |
| 3 | **Three consistency modes** | `Fresh` per-query check · `Eventual { ttl_ms }` skip-within-TTL · `Frozen` pin until refresh |
| 4 | **LRU eviction** | `with_max_cache_entries(n)` — evicts unpinned entries first; live pointers never evicted |
| 5 | **Arc-drop-lock hot path** | Cache mutex is never held during scan — 8-12× concurrent throughput lift |
| 6 | **Per-backend + per-collection stats** | Attribution at either granularity, hit-rate, prime durations |
| 7 | **Send+Sync everywhere** | Multi-client validated under 8-thread concurrent hammer smoke test |

**Bundle protocol**

| # | Capability | What It Does |
|---|------------|--------------|
| 8 | **`table.rulake.json` sidecar** | 300-byte portable unit; carries data_ref, dim, seed, rerank, generation, witness, pii_policy, lineage_id, memory_class |
| 9 | **`publish_bundle(key, dir)`** | Writer-side primitive — atomic temp+rename+fsync so readers never see torn files |
| 10 | **`refresh_from_bundle_dir(key, dir)`** | Reader-side primitive — three-state: `UpToDate` / `Invalidated` / `BundleMissing` |
| 11 | **Cross-process cache sharing** | Two ruLake instances with the same bundle witness share one compressed entry |
| 12 | **Format version + tag byte** | `format_version: 2` witness includes variant tag on `Generation` to prevent collision |

**Persistence (ADR-155 M1.5)**

| # | Capability | What It Does |
|---|------------|--------------|
| 13 | **`save_cache_to_dir(key, dir)`** | Snapshot a primed cache entry to disk as `index.rbpx` + bundle sidecar |
| 14 | **`warm_from_dir(key, dir)`** | Reload on restart — **byte-exact query results without backend RTT** |
| 15 | **Non-dense external IDs preserved** | `RabitqPlusIndex::ids_u64()` round-trips `[7, 42, 99, 2000, …]` faithfully |
| 16 | **SPIRE-pattern architecture** | Stateless compute + SSD-resident state — compute tier can restart without pull |

**Federation**

| # | Capability | What It Does |
|---|------------|--------------|
| 17 | **Parallel fan-out** | Rayon across all registered backends; first-error short-circuit matches sequential |
| 18 | **Adaptive per-shard rerank** | `max(5, global_rerank / K)` — 4-shard concurrent QPS goes 0.60× → 0.98× |
| 19 | **Per-shard over-request** | `k' = k + ⌈√(k·ln S)⌉` — closes the Weaviate/Elasticsearch data-skew recall gap |
| 20 | **`search_batch` API** | One lock + one coherence check per N queries; plug-point for future GPU kernels |

**Backends** ([`BackendAdapter` trait](src/backend.rs))

| # | Capability | What It Does |
|---|------------|--------------|
| 21 | **`LocalBackend`** | In-memory reference impl; test substrate |
| 22 | **`FsBackend`** | `ruvec1` binary format on disk, mtime-as-generation, atomic writes, path-traversal safe |
| 23 | **Custom backends** | 4-method trait: `id`, `list_collections`, `pull_vectors`, `generation` (+ optional `current_bundle`) |
| 24 | **DoS caps enforced** | `MAX_PULLED_VECTORS=100M`, `MAX_PULLED_DIM=8192`, `MAX_PULLED_BYTES=16 GiB` |

**Kernels** (tiered dispatch in [`ruvector-rabitq::scan`](https://github.com/ruvnet/ruvector/tree/main/crates/ruvector-rabitq))

| # | Capability | What It Does |
|---|------------|--------------|
| 25 | **Scalar popcount** | Portable baseline — always available |
| 26 | **AVX2 + POPCNT** | 4-candidate unrolled loop — +20% single-thread QPS at n=100k |
| 27 | **AVX-512 VPOPCNTDQ** | 8-u64-per-zmm popcount — +10.5% on top of AVX2 where available |
| 28 | **Runtime dispatch** | CPUID-gated `OnceLock<fn>` — laptop / server / edge all run the same source |
| 29 | **`VectorKernel` trait (ADR-157)** | Pluggable accelerator plane — GPU / Metal / WASM kernels as separate crates |
| 30 | **Optional Hadamard rotation (ADR-158)** | HD-HD-HD pattern, FWHT, 43× smaller storage, 3× build speedup at D=128 |

**Security**

| # | Capability | What It Does |
|---|------------|--------------|
| 31 | **Zero `unsafe` in ruLake** | All unsafe confined to two SIMD scan functions in rabitq |
| 32 | **Path-traversal validated** | `FsBackend::register` rejects 12-form attack surface (`..`, separators, control bytes, UNC) |
| 33 | **JSON deserialization caps** | 64 KiB bundle / 4 KiB fields / 128-byte witness — malicious sidecar cannot DoS |
| 34 | **Witness verification** | Every `read_from_dir` re-computes SHAKE‑256 and fails loudly on tamper |
| 35 | **Atomic writes** | Bundle + persist both use temp+rename — concurrent readers never see torn files |

</details>

---

## Why ruLake exists

Today the tradeoff for vector search is ugly:

- **Managed vector DB** (Pinecone, Weaviate) — fast, but a whole new system to operate and your data has to move.
- **Lakehouse-native** (BigQuery Vector Search, Snowflake Cortex) — keeps data in place but queries are expensive, slow, or per-backend.
- **Local library** (RaBitQ, HNSW, FAISS) — fastest per-process but no sharing, no coherence, no governance.

**ruLake is the middle option.** Keep your data where it lives. Get cache-speed reads. Pay governance once instead of per-backend.

<details>
<summary>🧭 The memory-hierarchy framing (ADR-156)</summary>

ruLake is the **substrate** for agent brain memory systems. The brain decides what episodic / semantic / procedural memory *means*; ruLake owns the persistence, coherence, and retrieval primitives. Six guarantees validated mechanically by `brain_substrate_acceptance_recall_verify_forget_rehydrate`:

1. **Recall** — `search_one` / `search_federated` returns top-K
2. **Verify** — `publish_bundle` → `read_from_dir` → `verify_witness`
3. **Forget** — `invalidate_cache` drops the pointer
4. **Rehydrate** — next search re-primes transparently
5. **Location-transparency** — caller only references `(backend, collection)`; never touches `data_ref`
6. **Compact** — explicitly out of scope; belongs to the brain system (RVM / Cognitum)

</details>

---

## Benchmarks

All numbers reproducible via:

```bash
cargo run --release -p ruvector-rulake --bin rulake-demo
```

Commodity Ryzen-class laptop, deterministic seeds, release build.

### Intermediary tax (cache-hit path)

Clustered Gaussian, D=128, rerank×20, 300 warm queries.

| n       | direct RaBitQ+ | ruLake Fresh | ruLake Eventual | tax     |
|--------:|---------------:|-------------:|----------------:|--------:|
|   5 000 |        18,998  |      18,500  |         18,800  | 1.03×   |
|  50 000 |         5,959  |       5,900  |          5,950  | 1.01×   |
| 100 000 |         3,681  |       3,542  |          3,626  | 1.03×   |

**The abstraction layer is not the bottleneck.**

### Concurrent QPS (Arc-drop-lock + AVX-512)

n=100k, 8 clients × 300 queries, adaptive per-shard rerank.

| shards | QPS         | vs original baseline |
|-------:|------------:|---------------------:|
|      1 |     27,814  |                8.3×  |
|      2 |     32,194  |               10.9×  |
|      4 |   **36,715** |              **13.2×** |

### Cold-start prime time (parallel rayon + Hadamard)

| n       | serial   | parallel | +Hadamard | total speedup |
|--------:|---------:|---------:|----------:|--------------:|
|   5 000 |   22 ms  |   4.5 ms |   7.2 ms  |        ~5×    |
|  50 000 |  213 ms  |  19.6 ms |  72.7 ms  |       ~11×    |
| 100 000 |  421 ms  |  37.6 ms | 142.9 ms  |       ~11×    |

### Recall gates

- Single-shard @ D=128 rerank×20 vs brute-force L2²: **≥ 90 %**
- 4-shard adaptive rerank @ D=128: **≥ 85 %**
- Hadamard rotation vs Haar @ D=128: **1.000 / 1.000** (identical)

See [`BENCHMARK.md`](BENCHMARK.md) for the full table.

---

## Quick start

### Build from source (the supported path while the crate is pre-publish)

```bash
git clone --recurse-submodules https://github.com/ruvnet/RuLake.git
cd RuLake
./install.sh                                   # checks rustc, inits submodules, cargo build + test
cargo run --release --bin rulake-demo -- --fast  # smoke-runs the demo in ~5 s
```

If you forgot `--recurse-submodules`, run `git submodule update --init --recursive` to fetch the vendored
`ruvector-rabitq` source under `vendor/ruvector/`. See [ADR-001](docs/adrs/ADR-001-standalone-repo-strategy.md)
for why we vendor instead of taking a `git`/`crates.io` dependency.

### Or run inside Docker (no Rust toolchain required)

```bash
docker build -t rulake .
docker run --rm rulake --fast
```

### As a library dependency

```toml
[dependencies]
ruvector-rulake = "2.2"  # once published; until then use `git = "https://github.com/ruvnet/RuLake"`
```

```rust
use std::sync::Arc;
use ruvector_rulake::{cache::Consistency, LocalBackend, RuLake};

// 1. Point ruLake at a backend.
let backend = Arc::new(LocalBackend::new("my-backend"));
backend.put_collection(
    "memories",
    /* dim    */ 128,
    /* ids    */ vec![1, 2, 3],
    /* vecs   */ vec![vec![0.0; 128]; 3],
)?;

// 2. Configure the cache.
let lake = RuLake::new(20, 42)
    .with_consistency(Consistency::Eventual { ttl_ms: 60_000 });
lake.register_backend(backend)?;

// 3. Query. First call primes; the rest serve from RaBitQ at ~1% over raw.
let hits = lake.search_one("my-backend", "memories", &vec![0.0; 128], 10)?;

// 4. Observe.
println!("hit rate: {:.3}", lake.cache_stats().hit_rate().unwrap_or(0.0));
```

<details>
<summary>💾 Save & warm-restart</summary>

```rust
// Snapshot the primed cache to disk (index + bundle sidecar)
let key = ("my-backend".to_string(), "memories".to_string());
lake.save_cache_to_dir(&key, "/var/rulake/snapshots/memories/")?;

// Later, on restart — spin up a FRESH RuLake with no backend:
let fresh = RuLake::new(20, 42).with_consistency(Consistency::Frozen);
let n = fresh.warm_from_dir(&key, "/var/rulake/snapshots/memories/")?;
println!("warmed {n} vectors without a backend RTT");

// Byte-exact query results vs the original primed cache.
let hits = fresh.search_one("my-backend", "memories", &query, 10)?;
```

</details>

<details>
<summary>🔁 Federated search across clouds</summary>

```rust
let hits = lake.search_federated(
    &[
        ("bigquery",  "events"),
        ("snowflake", "profiles"),
        ("iceberg",   "archive"),
    ],
    &query,
    10,
)?;
// Adaptive per-shard rerank = max(5, 20/3) = 6 per shard.
// Global top-10 merged from all three backends.
```

</details>

<details>
<summary>📦 Cache-sidecar daemon</summary>

Cross-process cache coherence in ~10 lines on top of the bundle protocol:

```rust
loop {
    match lake.refresh_from_bundle_dir(&key, "/mnt/gcs/bundles/")? {
        RefreshResult::Invalidated => metrics.bundle_rotations.inc(),
        _ => {}
    }
    std::thread::sleep(Duration::from_secs(5));
}
```

Full example: [`examples/sidecar_daemon.rs`](examples/sidecar_daemon.rs).

</details>

<details>
<summary>🧱 Writing a custom backend</summary>

```rust
use ruvector_rulake::backend::{BackendAdapter, CollectionId, PulledBatch};

struct ParquetBackend { /* ... */ }

impl BackendAdapter for ParquetBackend {
    fn id(&self) -> &str { "parquet" }
    fn list_collections(&self) -> Result<Vec<CollectionId>> { /* ... */ }
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> { /* ... */ }
    fn generation(&self, collection: &str) -> Result<u64> { /* ... */ }
}
```

See [`src/fs_backend.rs`](src/fs_backend.rs) for a 250-line reference implementation.

</details>

### Python — `pip install ruvector-rulake`

PyO3 bindings live in [`python/`](python/). Wheels (cp39+, manylinux_2_28 / macOS / Windows) per [ADR-002](docs/adrs/sdk/ADR-002-python-sdk.md).

```python
import numpy as np
import rulake

lake = rulake.RuLake(rerank_factor=20, rotation_seed=42) \
    .with_consistency(rulake.Consistency.eventual(ttl_ms=5_000))

be = rulake.LocalBackend("local")
be.put_collection("docs",
                  ids=np.arange(10_000, dtype=np.uint64),
                  vectors=np.random.randn(10_000, 768).astype(np.float32))
lake.register_backend(be)

q = np.random.randn(768).astype(np.float32)
for hit in lake.search_one("local", "docs", q, k=10):
    print(hit.backend, hit.collection, hit.id, hit.score)

print("hit_rate:", lake.cache_stats().hit_rate())
```

<details>
<summary>🐍 Python — full usage, conventions, build</summary>

**Build from source** (until wheels are on PyPI):

```bash
git clone --recurse-submodules https://github.com/ruvnet/RuLake
cd RuLake/python
python -m venv .venv && source .venv/bin/activate
pip install maturin pytest numpy
maturin develop --release      # builds + installs the _rulake extension
pytest tests/ -v               # 14/14 smoke tests
```

**Vector conventions** — vectors are `np.ndarray[float32]`, IDs are `np.ndarray[uint64]`, both C-contiguous. The binding borrows zero-copy via `PyReadonlyArray1<f32>::as_slice()`. Non-contiguous or wrong-dtype arrays raise `ValueError` rather than silently copying — the silent-copy bug is the regression this binding exists to prevent (ADR-002 §2).

**Concurrency** — every search / prime / publish / refresh / save / warm path releases the GIL via `py.allow_threads`. Use `concurrent.futures.ThreadPoolExecutor` for parallel queries; the underlying Rust crate is `Send + Sync` and the RaBitQ scan runs lock-free under contention (8–12× lift on `BENCHMARK.md`'s "concurrent clients" block).

**Error hierarchy** — single base `rulake.RuLakeError`. Typed subclasses discriminate:

```python
try:
    lake.search_one("nope", "docs", q, k=10)
except rulake.BackendNotFoundError as e:
    ...                       # specific
except rulake.RuLakeError as e:
    ...                       # catch-all
```

| Rust variant                     | Python class                          |
|----------------------------------|---------------------------------------|
| `RuLakeError::UnknownBackend`    | `rulake.BackendNotFoundError`         |
| `RuLakeError::UnknownCollection` | `rulake.CollectionNotFoundError`      |
| `RuLakeError::DimensionMismatch` | `rulake.DimensionMismatchError`       |
| `RuLakeError::InvalidParameter`  | `rulake.InvalidParameterError`        |
| `RuLakeError::Backend { .. }`    | `rulake.BackendError`                 |

**Bundle round-trip + warm-restart**:

```python
# Snapshot a primed cache.
lake.save_cache_to_dir("local", "docs", "/var/rulake/snap/")
lake.publish_bundle("local", "docs", "/var/rulake/snap/")

# Reopen elsewhere — no backend RTT, byte-exact results.
fresh = rulake.RuLake(20, 42).with_consistency(rulake.Consistency.frozen())
n = fresh.warm_from_dir("local", "docs", "/var/rulake/snap/")
```

**Type-checked editor support** — `py.typed` + `_rulake.pyi` ship in the wheel. mypy / pyright catch dim/dtype errors at edit time.

**Not in v1** (see ADR-002 §"Open questions"): native async API (use `ThreadPoolExecutor`), Python-implemented `BackendAdapter`, HTTP client variant.

</details>

### Node.js / TypeScript — `npm install @ruvector/rulake`

napi-rs bindings live in [`node/`](node/). Per-platform `.node` binaries via npm `optionalDependencies` (Prisma / next-swc pattern), per [ADR-003](docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md).

```ts
import { RuLake, LocalBackend, Consistency } from "@ruvector/rulake";

const lake = new RuLake(20, 42n)
    .withConsistency(Consistency.eventual(5_000));

const N = 10_000, D = 768;
const ids = new BigInt64Array(N);
for (let i = 0; i < N; i++) ids[i] = BigInt(i);
const vectors = new Float32Array(N * D);   // fill with embeddings...

const be = new LocalBackend("local");
await be.putCollection("docs", ids, vectors, D);
lake.registerLocalBackend(be);

const q = new Float32Array(D);
for (const hit of await lake.searchOne("local", "docs", q, 10)) {
    console.log(hit.backend, hit.collection, hit.id /* bigint */, hit.score);
}
console.log("hitRate:", lake.cacheStats().hitRate);
```

<details>
<summary>🟢 Node.js / TypeScript — full usage, conventions, build</summary>

**Build from source** (until binaries are on npm):

```bash
git clone --recurse-submodules https://github.com/ruvnet/RuLake
cd RuLake/node
cargo build --release
cp target/release/libruvector_rulake_node.so rulake.linux-x64-gnu.node
# (.dylib → rulake.darwin-arm64.node ; .dll → rulake.win32-x64-msvc.node)
node --test __test__/smoke.test.mjs       # 10/10 smoke tests
```

The supported release path uses `@napi-rs/cli` (`npx napi build --platform --release`) which produces the same artifact and regenerates the JS / `.d.ts` shims.

**Vector conventions** — `Float32Array` for vectors, `BigInt64Array` for IDs going in, `bigint` coming out. Rust IDs are `u64`; we don't silently truncate to `Number.MAX_SAFE_INTEGER`. The binding borrows the typed-array buffer and copies *once* at the FFI boundary (~3 µs at D = 768) because the borrow can't cross `await` to a libuv worker thread. The relative tax stays under 1.05× per `BENCHMARK.md`.

**Async-only** — every method that does work returns `Promise<T>` and runs on a libuv worker via `spawn_blocking`. The event loop keeps serving other requests during a scan. Pure getters (`cacheStats()`, `cacheEntryCount()`, `backendIds()`) stay sync.

**ESM-first, CJS shim** — `type: "module"` package, `index.mjs` is the import target, `index.cjs` is the require target, `index.d.ts` is hand-checked against the `napi build`-generated shape.

**Error mapping** — single `RuLakeError` class with a `.code` discriminator (idiomatic Node — matches `SystemError` / AWS SDK):

```ts
try {
  await lake.searchOne("nope", "docs", q, 10);
} catch (e) {
  if (e instanceof RuLakeError && e.code === "RULAKE_BACKEND_NOT_FOUND") {
    // ...
  }
}
```

| Code                              | Meaning                                              |
|-----------------------------------|------------------------------------------------------|
| `RULAKE_BACKEND_NOT_FOUND`        | unknown backend id                                   |
| `RULAKE_COLLECTION_NOT_FOUND`     | backend exists, collection doesn't                   |
| `RULAKE_DIMENSION_MISMATCH`       | query/vectors don't match the collection dim         |
| `RULAKE_INVALID_PARAMETER`        | malformed input (e.g. illegal filename)              |
| `RULAKE_BACKEND`                  | a registered backend reported an internal error      |

**Federated + bundle round-trip**:

```ts
const hits = await lake.searchFederated(
  [["bigquery", "events"], ["snowflake", "profiles"]],
  q, 10,
);

const b = new Bundle("s3://bucket/path", D, 42n, 20, 1n);
await b.writeToDir("/var/rulake/snap/");
const b2 = await Bundle.readFromDir("/var/rulake/snap/");
console.assert(b2.verifyWitness());
```

**Distribution** — npm `optionalDependencies` per platform (`@ruvector/rulake-linux-x64-gnu`, …). On install npm reads `os` / `cpu` / `libc` and pulls only the matching binary. Works in air-gapped envs and behind corporate registries (every binary mirrored), unlike `postinstall`-download patterns.

**Not in v1** (see ADR-003 §"Open questions"): WASM build for browser / Cloudflare Workers / Deno (`@ruvector/rulake-wasm` reserved on npm — loses AVX-512 popcnt + rayon parallel fan-out, so it's a feature-reduced surface), HTTP client variant (`@ruvector/rulake/http`), JS-implemented `BackendAdapter`.

</details>

<details>
<summary>📐 SDK design — why these two languages, why these shapes</summary>

Both SDKs hit the same goals via different platform-shaped means:

| Concern              | Python (PyO3)                              | Node (napi-rs)                                        |
|----------------------|--------------------------------------------|-------------------------------------------------------|
| Hot-path zero-copy   | `PyReadonlyArray1<f32>::as_slice()`        | `Float32Array` borrow + one copy across `await`       |
| Concurrency          | Sync API, GIL released; threadpool friendly | Async-only, libuv `spawn_blocking`, event-loop safe   |
| ID type              | Python `int` (Python ints are arbitrary)   | `bigint` in / out (no `Number.MAX_SAFE_INTEGER` loss) |
| Error model          | `RuLakeError` base + typed subclasses      | Single `RuLakeError`, `.code` discriminator           |
| Distribution         | ABI3 wheels (one per platform per release) | `optionalDependencies` per-platform `.node`           |
| Editor types         | `py.typed` + `.pyi` stubs                  | `index.d.ts` (hand-checked vs `napi build` output)    |
| Tax over Rust        | ≤ 1.05× (release; FFI ~1 µs/call)          | ≤ 1.10× (release; FFI ~5 µs/call from one memcpy)     |

The shared design ground (witness-anchored bundles, RaBitQ kernel, Consistency knob) makes a Python writer and a Node reader interoperable: a Python process can `publish_bundle` a snapshot that a Node process `Bundle.readFromDir`s and verifies byte-exact.

Why these two and not Java / Go / C# in v1: the audiences map directly. Python = ML / RAG / data engineers (the entire `numpy`+`sentence-transformers` stack). Node + TS = edge-RAG, serverless handlers, LangChain.js / LlamaIndex.ts orchestration. Java / Go / C# customers exist but trail by an order of magnitude in the design-partner conversations driving v1 — they're explicitly v2 in both ADRs' "Open questions".

See [ADR-002](docs/adrs/sdk/ADR-002-python-sdk.md) and [ADR-003](docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md) for the rejected alternatives (ctypes/cffi, Neon, WASM-first, sync-Node, async-Python, pure-language rewrites, HTTP-client-first) and the reasoning per axis.

</details>

---

## How it works

<details>
<summary>📐 Data flow diagram</summary>

```
search(backend, collection, query, k)
  │
  ▼
ensure_fresh(key) ─── Consistency mode?
  │                          │
  ├── Frozen   (skip after prime)
  ├── Eventual (skip within TTL)
  └── Fresh    (always check)
         │
         ▼
      ask backend for current witness
         │
    ┌────┴──────────────────┐
  match                  mismatch
  (hit)                    │
                     witness cached elsewhere?
                     │              │
                   yes              no
                     │              │
                move pointer   pull + prime
                (0 work,        (compress into
                 shared_hits++)   RaBitQ codes)
         │                │
         ▼                ▼
  Arc<RabitqPlusIndex>::search  (mutex dropped before scan)
         │
         ▼
     top-K, sorted by L2²
```

</details>

<details>
<summary>🔐 Witness chain</summary>

Every cache entry is anchored by:

```
SHAKE-256(32)(
  "rulake-bundle-witness-v1|" ||
  len(data_ref) || data_ref ||
  "|" || dim || rotation_seed || rerank_factor ||
  "|" || len(generation_bytes) || generation_tag_byte || generation_bytes
)
```

Length-prefixed + domain-separated — two different bundles cannot produce the same witness through field concatenation games. The `generation_tag_byte` (0x00 for `Num`, 0x01 for `Opaque`) closes the `Num(7)` vs `Opaque("\x07\0…")` collision the 2026-04-23 security audit surfaced.

</details>

<details>
<summary>🎛 Adaptive per-shard rerank (ADR-155 §6)</summary>

Under federation, RaBitQ would run its `rerank_factor × k` rerank once per shard, costing K× more work as shard count grows. ruLake divides the budget:

```
per_shard_rerank = max(MIN_PER_SHARD_RERANK, global_rerank / K)
```

K=4 at rerank×20 → 5 per shard. Measured recall@10 stays above 85 % (gate test). Callers needing byte-exact single-shard parity use `search_federated_with_rerank(.., Some(global_rerank))`.

</details>

<details>
<summary>⚡ Arc-based concurrency (the 13.2× win)</summary>

`CacheEntry::index` is `Arc<RabitqPlusIndex>`. Readers:

1. Lock cache mutex
2. Clone the Arc (refcount bump — cycles, not milliseconds)
3. **Drop the lock**
4. Scan without holding anything shared

Index is immutable after build, so concurrent scans are a pure data race against nothing. This is the single biggest performance win on the branch — **8-12× concurrent QPS**.

</details>

---

## User guide

<details>
<summary>🎚 Choose a consistency mode</summary>

| Symptom / requirement | Mode |
|---|---|
| Legal / compliance — can't serve stale data, ever | `Fresh` |
| Search, RAG, recommendation, agent retrieval | `Eventual { ttl_ms: 60_000 }` |
| Audit snapshot — data is cryptographically pinned | `Frozen` |

</details>

<details>
<summary>📏 Size the cache</summary>

```rust
// Unbounded — fine for small collections
let lake = RuLake::new(20, 42);

// LRU-capped for memory-bounded serving processes
let lake = RuLake::new(20, 42).with_max_cache_entries(100);
```

Only unpinned entries (refcount == 0) are evicted; active `(backend, collection)` pointers keep their entry alive.

</details>

<details>
<summary>📊 Operational metrics</summary>

| Metric | Signal | Action |
|---|---|---|
| `hit_rate` | < 0.95 | Grow cache or warm aggressively |
| `last_prime_ms` | spiking | Backend RTT changed or collection grew |
| `primes` | growing unexpectedly | Check for witness churn |
| `shared_hits` | > 0 | Cross-backend sharing is working |
| `invalidations` | climbing | Coherence protocol firing — inspect |
| `warm_installs` | > 0 | `warm_from_dir` is being used |

Per-backend and per-collection views via `cache_stats_by_backend()` / `cache_stats_by_collection()`.

</details>

<details>
<summary>🚀 Run the examples</summary>

```bash
# End-to-end sidecar daemon (publisher + reader + coherence loop)
cargo run --release -p ruvector-rulake --example sidecar_daemon

# Save → ship → warm-restart cycle
cargo run --release -p ruvector-rulake --example warm_restart

# Benchmark harness (~2 minutes)
cargo run --release -p ruvector-rulake --bin rulake-demo

# Fast mode (~5 seconds, just n=5k)
cargo run --release -p ruvector-rulake --bin rulake-demo -- --fast
```

</details>

---

## Examples

Cross-language examples live under [`examples/`](examples/) — each with its own
README, install instructions, tests, and a runnable smoke flow. The bundle
protocol (`table.rulake.json` + SHAKE-256 witness) is the language-portable
primitive; clients in any language verify witnesses against Rust ruLake
byte-exactly.

| Stack | Path | Highlights |
|-------|------|------------|
| Rust  | [`examples/sidecar_daemon.rs`](examples/sidecar_daemon.rs) · [`warm_restart.rs`](examples/warm_restart.rs) | Publisher + reader coherence loop, save → ship → warm-restart |
| Python | [`examples/python/`](examples/python/) | verify-witness · bundle-server (FastAPI) · subprocess-wrapper · [rag-grounded](examples/python/04-rag-grounded/) |
| Node + TypeScript | [`examples/nodejs/`](examples/nodejs/) | verify-witness · bundle-publisher (Express) · subprocess-wrapper · [MCP tool](examples/nodejs/04-mcp-tool/) |
| Rust → WASM | [`examples/wasm/`](examples/wasm/) | [browser verifier](examples/wasm/01-witness-verifier-browser/) (~127 KB) · [Node verifier](examples/wasm/02-witness-verifier-nodejs/) · [WASI CLI](examples/wasm/03-bundle-toolkit-cli/) |
| GPU | [`examples/gpu/`](examples/gpu/) | [CUDA brute-force](examples/gpu/01-cuda-brute-force/) (38× kernel) · [wgpu portable](examples/gpu/02-wgpu-portable/) (Vulkan/Metal/DX12) · [rabitq-GPU design note](examples/gpu/03-rabitq-gpu-design-note/) |

All cross-language verifiers reproduce the canonical Rust witness
`dea58c64adb1eb4109438f0353a2b1749d4dc29ed7266e9236720ab6cf07d7e4` byte-exactly.
Per-language details and the cross-cutting story are in the index at
[`examples/README.md`](examples/README.md).

---

## Status

**M1 + M1.5 shipped and measured** (2026-04-24)

<details>
<summary>✅ What's done (43 tests passing — 21 unit + 22 integration — zero unsafe in ruLake)</summary>

- Core abstraction — `BackendAdapter` trait, `VectorCache`, bundle protocol, 3 consistency modes, LRU
- Two reference backends — `LocalBackend` (in-memory), `FsBackend` (file-based with `ruvec1` format)
- Optimizations — adaptive per-shard rerank, Arc-concurrency (13.2× concurrent), parallel prime (11× miss-path), AVX-512 VPOPCNTDQ + AVX2 dispatch, Hadamard rotation (3× build, 32× storage)
- **Persist end-to-end** — `save_cache_to_dir` / `warm_from_dir` with non-dense external ID preservation
- Observability — hit rate, prime durations, per-backend, per-collection attribution, warm-install counter
- Substrate acceptance test — six-guarantee loop (recall → verify → forget → rehydrate → location-transparency + compact-deferred)
- Security — path-traversal validation, JSON caps, witness verification, atomic writes
- `VectorKernel` trait scaffolding (ADR-157)
- Per-shard over-request — `k' = k + ⌈√(k·ln S)⌉`
- 4 ADRs — 155 cache-first, 156 substrate, 157 accelerator plane, 158 Hadamard + QVCache positioning

</details>

<details>
<summary>🗺 M2+ roadmap</summary>

- **Backends** — `ParquetBackend` (`arrow` crate), `BigQueryBackend` (storage-read API), `IcebergBackend` (Nessie / Polaris catalog), `DeltaBackend` (CDF coherence)
- **Wire** — HTTP / gRPC protocol layer with OpenAPI schema
- **Governance** — RBAC via OIDC/JWT, PII passthrough (reusing `rvf-federation::pii`), OpenLineage emission with witness as lineage-id
- **Kernels** — GPU in separate crates (`ruvector-rabitq-cuda`, `-rocm`, `-metal`), turbovec-style FastScan 4-bit LUT, WASM SIMD
- **Acceleration** — mmap'd index persistence via `memmap2`, HNSW layer on top of RabitQ via `hnsw_rs::datamap`
- **SOTA integrations** — QVCache-style adaptive per-region rerank, SPIRE-style 8B-vector federation

</details>

---

## Comparison with other systems

| System           | Intermediary tax | Cross-backend federation | Witness-authenticated | Cross-process cache sharing | CPU-first / GPU-optional | `unsafe` count |
|------------------|-----------------:|-------------------------:|----------------------:|----------------------------:|-------------------------:|---------------:|
| **ruLake**       | **1.02×**        | ✅ (rayon fan-out)       | ✅ (SHAKE‑256)         | ✅ (content-addressed)      | ✅                       | **0**          |
| Pinecone         | n/a (hosted)     | ❌                        | ❌                    | ❌                          | n/a                      | n/a            |
| Weaviate         | n/a (hosted)     | ❌                        | ❌                    | ❌                          | ✅                       | n/a            |
| Milvus           | ~1.5–2×          | partial                   | ❌                    | ❌                          | ✅                       | many           |
| LanceDB          | ~1.1–1.3×        | ❌                        | ❌                    | ❌                          | ✅                       | some           |
| BQ Vector Search | n/a (hosted)     | ❌ (BQ-only)              | ❌                    | ❌                          | n/a                      | n/a            |
| QVCache (2026)   | region-adaptive  | ❌                        | ❌                    | ❌                          | ✅                       | unknown        |

ruLake is explicitly **not** a vector database — it doesn't own storage. It's the substrate that lets you query whichever vector DB or lakehouse you already have, with a coherent compression + governance story across all of them.

---

## RuVector ecosystem

| Crate                  | Role                                               |
|------------------------|----------------------------------------------------|
| [`ruvector-rvf`](https://github.com/ruvnet/ruvector/tree/main/crates/rvf) | Durable segment format — appendable, witness-signed vector storage |
| [`ruvector-rabitq`](https://github.com/ruvnet/ruvector/tree/main/crates/ruvector-rabitq) | Rotation-based 1-bit quantization kernel |
| [`ruvector-rulake`](https://github.com/ruvnet/ruvector/tree/main/crates/ruvector-rulake) | **this crate** — cache, coherence, federation, governance |

RVF is your durable truth. rabitq is your compressor. ruLake is the execution layer.

---

## License

Licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

---

## Links

- Main development: [ruvnet/ruvector — `crates/ruvector-rulake`](https://github.com/ruvnet/ruvector/tree/main/crates/ruvector-rulake)
- ADRs: [ADR-155](docs/adrs/ADR-155-rulake-datalake-layer.md) · [ADR-156](docs/adrs/ADR-156-rulake-as-memory-substrate.md) · [ADR-157](docs/adrs/ADR-157-optional-accelerator-plane.md) · [ADR-158](docs/adrs/ADR-158-optional-rotation-and-qvcache-positioning.md) · [ADR-001 (this repo's standalone strategy)](docs/adrs/ADR-001-standalone-repo-strategy.md)
- Research: [`docs/research/ruLake/`](https://github.com/ruvnet/ruvector/tree/main/docs/research/ruLake) (lives in upstream RuVector — not vendored)
- Benchmarks: [`BENCHMARK.md`](BENCHMARK.md)
- Capability / performance / security review: [`docs/review/`](docs/review/)
- Powered by Cognitum: [cognitum.one](https://cognitum.one)
- Website: [ruv.io](https://ruv.io)
