# ruLake — A Memory Lake for Agentic AI

[![Crates.io](https://img.shields.io/crates/v/ruvector-rulake.svg)](https://crates.io/crates/ruvector-rulake)
[![Rust 1.89+](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org)
[![RuVector](https://img.shields.io/badge/part_of-ruvector-purple.svg)](https://github.com/ruvnet/ruvector)
[![ruv.io](https://img.shields.io/badge/ruv.io-website-purple.svg)](https://ruv.io)
[![MIT / Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue.svg)](#license)

### **Give your AI agents fast, trustworthy memory — without standing up a vector database.**

ruLake is the layer between your **agents** and the **data they remember**. Plug in the storage you already have (S3, BigQuery, Snowflake, Parquet, files), expose it through one MCP tool, and every agent on every host gets the same low-latency, content-addressed view of memory.

> Created by [rUv](https://ruv.io). Part of the [RuVector](https://github.com/ruvnet/ruvector) ecosystem alongside [`ruvector-rabitq`](https://github.com/ruvnet/ruvector/tree/main/crates/ruvector-rabitq) (1‑bit compression kernel) and RVF (durable segment format). Designed to be the substrate for the [Cognitum](https://cognitum.one) Agentic Chip's memory hierarchy.

#### What it is, in one paragraph

Agentic systems are built on **contrastive AI** — embeddings that put similar things close together and different things far apart. Every "what does the agent remember about X?" query is, underneath, a contrast: rank the corpus by distance to X. ruLake is the place where those contrasts run. It keeps a compressed copy of your vectors in RAM, serves hits at **≈1.02× raw library speed** (essentially free abstraction), and refreshes cold entries from whatever cloud or file store actually owns the bytes. Each cached entry is anchored by a **cryptographic witness**, so an answer is verifiable across processes, hosts, and time.

#### Why agents in particular

- **One MCP tool, one decision layer.** [`rulake-mcp`](mcp-server/) (ADR-004) speaks the [Model Context Protocol](https://modelcontextprotocol.io). Claude Desktop, Cursor, Cline, Continue, agentic-flow — they all get a single `rulake_query` tool that takes intent (`search` / `verify` / `explain` / `refresh`), risk, freshness budget, and policy, and returns the answer plus a **decision trace** (chosen_action, reason_code, backends_used, refusals). The agent says *what* it wants; ruLake decides *where to look, how strict to be, whether to refuse*.
- **Trust by witness, not vibes.** Every result carries the SHAKE-256 hex of the underlying bundle. Two agents, two hosts, same data → same witness → same answer, byte-exact. No "the model hallucinated again" debates.
- **Honest refusals beat confident lies.** Stale cache + missing remote witness? `WITNESS_MISMATCH_REFUSED`, empty data, agent retries narrower. Better than serving a stale answer with a high score.

#### Performance, cost, footprint

| | What it delivers |
|---|---|
| **Latency** | 1.02× raw RaBitQ ≈ ~1 ms cache-hit at n=100k, D=128. Measured, not promised. |
| **Throughput** | 957 QPS single-thread, **2,854 QPS concurrent** (Arc-drop-lock + AVX-512 VPOPCNTDQ). |
| **Compression** | 1-bit RaBitQ — **32× smaller** than f32 vectors at D=128. RAM footprint stays small even at millions of vectors. |
| **Cost** | **$0.** MIT/Apache-2.0, no service to host, no per-query fee, no metered API. Run it next to your agent. |
| **Backends** | LocalBackend (RAM), FsBackend (disk), GcsParquetBackend (Parquet on GCS). BigQuery, S3, Iceberg, Delta on the M2–M5 roadmap. |
| **Surfaces** | Rust crate · Python wheel (`pip install`) · Node.js (`npm install`) · `rulake-mcp` binary · Docker image. |

#### Edge, browser, and the small-footprint story

ruLake is built to run wherever the agent runs — including small places.

- **Today** — small static binary (the demo + `rulake-mcp` are ~5 MB stripped), distroless Docker, and a Streamable HTTP transport that fits behind any reverse proxy. Runs on a Raspberry Pi or an EC2 t4g.nano, not just a serving cluster.
- **Coming (v0.4 / v0.5)** — `@ruvector/rulake-wasm` for **browsers, Cloudflare Workers, Deno-deploy, Bun**. Same witness-anchored memory model, feature-reduced surface (no AVX-512, no rayon — they don't exist on the edge anyway). The `optionalDependencies` shape ([ADR-003](docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md) §A) is already wired so the WASM package drops in without breaking npm consumers.
- **Why it matters** — agent memory at the edge means the personal AI doesn't round-trip your private context to a far-away cluster. Latency is local; cost is zero per query; the witness story keeps it verifiable.

```bash
# Three install paths, three audiences. Pick one.
cargo add ruvector-rulake                      # Rust
pip   install ruvector-rulake                  # Python (wheels coming to PyPI)
npm   install @ruvector/rulake                 # Node.js / TypeScript
```

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

If you're building an agent that remembers things, your three current options for vector search all hurt:

- **Managed vector DB** (Pinecone, Weaviate) — fast, but it's a whole new service to operate and your private data has to move into it.
- **Lakehouse-native** (BigQuery Vector Search, Snowflake Cortex) — keeps data where it lives, but every query bills the warehouse and round-trips a remote cluster.
- **Local library** (RaBitQ, HNSW, FAISS) — fastest per-process, but every agent spins up its own copy, nothing's verifiable across processes, and there's no governance story.

**ruLake is the missing middle.** Your data stays where it is. The agent gets cache-speed reads (1.02× the raw library cost). One governance and witness story for every backend, instead of N. **No cluster to host, no per-query bill, no separate database.**

For agent platforms that already speak MCP, the integration is one config file:

```json
{
  "mcpServers": {
    "rulake": {
      "command": "rulake-mcp",
      "args": ["stdio", "--config", "/etc/rulake/mcp.toml"]
    }
  }
}
```

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

### MCP server — `rulake-mcp` (agent-callable governed memory)

Rust-native MCP server in [`mcp-server/`](mcp-server/). Lets any MCP-compatible client (Claude Desktop, Cursor, Cline, Continue, agentic-flow) talk to a live ruLake over **stdio** or **Streamable HTTP**, with the planner deciding *where* to search, *how strict* to be, *whether to refuse*, and emitting a decision trace alongside every answer. Implements [ADR-004](docs/adrs/sdk/ADR-004-rulake-mcp-server.md).

```bash
# stdio (default — parent-process trust):
cd mcp-server && cargo build --release
./target/release/rulake-mcp stdio --config tests/fixtures/mcp.toml

# Streamable HTTP on loopback:
./target/release/rulake-mcp http --bind 127.0.0.1:7440 --auth none

# HTTP + bearer + capability tier + audit file:
./target/release/rulake-mcp http \
    --bind 127.0.0.1:7440 \
    --auth bearer --bearer-token-file /etc/rulake/token \
    --capabilities read,publish \
    --audit-file /var/log/rulake-mcp/audit.jsonl

# HTTP + JWT (production — OAuth-style scope→capability mapping):
./target/release/rulake-mcp http \
    --bind 0.0.0.0:7440 \
    --auth jwt \
    --jwt-secret-file /etc/rulake/jwt.secret \
    --jwt-issuer https://idp.example.com \
    --jwt-audience https://rulake.example.com/mcp \
    --capabilities read,publish \
    --audit-file /var/log/rulake-mcp/audit.jsonl
```

**Auth + RBAC (v0.4, production-shaped)**: layered defense across four checks. (1) Connection-level auth — `--auth none|bearer|jwt`. JWT validates JWS signature + iss + aud (RFC 8707 Resource Indicator) + exp; the token's `scope` / `scp` claim maps `mcp:rulake:read|publish|admin` → capabilities per request. (2) **Replay protection** — `MCP-Request-Id` LRU dedup over a 10k-window. (3) **Layered rate limiting** — three governor buckets per ADR-004 §6: per-(transport, principal), per-(principal, backend, collection), per-process. (4) **Per-collection RBAC** via `[[allow]] backend, collection (anchored regex), caps` blocks in `mcp.toml`. Empty allow-list = unrestricted (back-compat). Anchored regex hardens against prefix-match exploits — `docs.*` matches `docs.public` but NOT `secret-docs.public`.

The public tool is `rulake_query` — submit `intent` (`search` | `verify` | `explain` | `refresh`), `target` (collection or routes), `risk`, `freshness`, `budget`, `policy`. The response carries `data` + `provenance` + `trust_level` + `decision` (chosen_action, reason_code from a closed enum, backends_used, refusals).

<details>
<summary>🤖 MCP server — full surface, capabilities, transports</summary>

**Tools by capability tier** (`--capabilities` flag — read is default):

| Tier | Tools exposed |
|---|---|
| `read` (default) | `rulake_query`, `rulake_list_backends` |
| `internal` | + the kernel `rulake_query` composes (operator-only, never OAuth-issued) |
| `publish` | + `rulake_publish_bundle`, `rulake_refresh_from_bundle_dir` (and enables `intent: "refresh"`) |
| `admin` | + `rulake_save_cache_to_dir`, `rulake_warm_from_dir`, `rulake_invalidate_cache` |

`register_backend` is **never** wire-exposed (backends carry credentials; ADR-004 §4 + CVE-2025-53107/53818).

**Resources** (URI-addressable read-only):

- `rulake://stats` — roll-up cache stats (hit_rate, primes, avg_prime_ms)
- `rulake://stats/by-backend` — per-backend stats
- `rulake://bundle/{backend}/{collection}` — v0.4 (witness lookup; backend-implementer contract for cheap `current_bundle()`)

**Transports + auth** (ADR-004 §3 + §5):

| Transport / auth | Notes | Status |
|---|---|---|
| stdio | parent-process trust | ✅ v0.1 |
| Streamable HTTP `--auth none` | loopback only by default; `--insecure-allow-no-auth` to override | ✅ v0.2 |
| Streamable HTTP `--auth bearer` | file token, constant-time compare; dev-only (`--allow-bearer-on-public` for any non-loopback bind) | ✅ v0.2 |
| Streamable HTTP `--auth jwt` | HMAC JWS (HS256/384/512), iss + aud + exp validation, scope→capability mapping via `mcp:rulake:read|publish|admin` | ✅ v0.4 |
| Streamable HTTP `--auth jwt` (RS256/ES256 + JWKS fetch) | public-key signature verification + remote key rotation | v0.5 |
| Streamable HTTP `--auth mtls` | client cert CN as principal, operator-supplied CA | v0.5 |
| **Replay protection** | `MCP-Request-Id` LRU dedup over a 10k-request window | ✅ v0.4 |
| **Layered rate limiting** | 3 governor buckets: (transport, principal), (principal, backend, collection), per-process | ✅ v0.4 |
| **Per-collection RBAC** | `[[allow]] backend, collection (anchored regex), caps` blocks in `mcp.toml` | ✅ v0.4 |
| **`tools/list` capability filter** | agents only see tools they can call — visibility is gated by the same map as call-time `require_cap` | ✅ v0.4 |

DNS-rebinding guard via rmcp's `allowed_hosts` (loopback by default). Bearer-on-public requires `--allow-bearer-on-public` AND emits a `WARN` every 60 s.

**Decision trace** — every response carries the same closed-enum `reason_code`:

```text
CACHE_HIT_FRESH | CACHE_HIT_EVENTUAL | STALE_CACHE_REMOTE_VALID
COLD_PRIME_THEN_SERVE | WITNESS_MISMATCH_REFUSED
BUDGET_EXCEEDED_FALLBACK_CACHE | BUDGET_EXCEEDED_REFUSED
POLICY_REFUSED_RISK | POLICY_REFUSED_ALLOWLIST | POLICY_REFUSED_PATH
PARTIAL_FEDERATION
```

Lets ops alerts filter without regex over `decision.reason` prose.

**JSONL audit file** (`--audit-file PATH`, full ADR-004 §7 schema): every line carries `policy_decision` + `decision` blocks — the "explain itself" gate. Schema is OpenLineage-mappable (ADR-155 §M4 swap-the-sink ready).

**Bounded worker pool** — rayon `cores * 2`, bounded flume submit channel of capacity `--max-inflight 64` (the §6 commitment that rejects unbounded `spawn_blocking`).

Wire to Claude Desktop / Cursor over stdio:

```json
{
  "mcpServers": {
    "rulake": {
      "command": "/path/to/rulake-mcp",
      "args": ["stdio", "--config", "/path/to/mcp.toml"]
    }
  }
}
```

Wire to a remote agent over Streamable HTTP:

```json
{
  "mcpServers": {
    "rulake": {
      "transport": "streamable-http",
      "url": "https://rulake.example.com/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

Build the **distroless Docker image** (`Dockerfile.mcp`):

```bash
docker build -f Dockerfile.mcp -t rulake-mcp .
docker run --rm -p 7440:7440 rulake-mcp http --bind 0.0.0.0:7440 --auth none --insecure-allow-no-auth
```

**21/21 tests pass** (3 audit unit + 11 smoke covering all 4 intents + cap gates + 7 HTTP e2e covering bearer + embarrassing-flag refusals).

</details>

### Cloud backends — `gcs-backend` (Parquet on GCS)

The first cloud backend, in [`gcs-backend/`](gcs-backend/) — reads vector columns from Parquet files on Google Cloud Storage with cache coherence riding GCS's per-object generation token. Implements [ADR-155 §M2](docs/adrs/ADR-155-rulake-datalake-layer.md).

```rust
use std::sync::Arc;
use ruvector_rulake::{cache::Consistency, RuLake, BackendAdapter};
use ruvector_rulake_gcs::{GcsParquetBackend, GcsParquetCollection};

let backend = GcsParquetBackend::open_gcs("gcs-prod", "my-vector-bucket")?;
backend.register(GcsParquetCollection {
    name:    "docs".into(),
    object:  "embeddings/2026-04/docs.parquet".into(),
    dim:     None,    // None → read from Parquet schema on first pull
});

let lake = RuLake::new(20, 42)
    .with_consistency(Consistency::Eventual { ttl_ms: 5_000 });
lake.register_backend(Arc::new(backend) as Arc<dyn BackendAdapter>)?;

let hits = lake.search_one("gcs-prod", "docs", &query, 10)?;
```

<details>
<summary>☁️ GCS backend — schema, auth, deployment</summary>

**Parquet schema contract (v0.1)** — two required columns:

| Column | Type | Notes |
|---|---|---|
| `id` | `INT64` (non-null) | Cast to `u64` by bit pattern. |
| `vector` | `LIST<FLOAT32>` or `FixedSizeList<FLOAT32, N>` (non-null) | Length = collection's `dim`. |

**Auth** — Application Default Credentials. Run `gcloud auth application-default login` once or set `GOOGLE_APPLICATION_CREDENTIALS=/path/to/sa.json`; the `object_store` crate handles the rest.

**Cache coherence** — `Generation = u64` from the GCS object's generation number. Every `gcloud storage cp` of a Parquet file bumps the generation → ruLake's witness picks up the change automatically through the existing `Generation::Num` variant in `src/bundle.rs:55`.

**Cheap `current_bundle()` override** — the default impl in `src/backend.rs:131` does a full `pull_vectors` to learn the dim, which would melt a remote backend at resource-read rates. We override to do a HEAD on the GCS object (~1 RTT) + a Parquet-footer-only schema read (a few KiB). ADR-004's `rulake://bundle/{backend}/{collection}` resource explicitly forbids the default-impl behaviour for any backend it's pointed at.

**Build + test** (4 offline tests against `object_store::memory::InMemory` + 1 live test gated on env var):

```bash
cd gcs-backend && cargo build --release
cargo test --release          # 4/4 offline pass

# Live test — needs gcloud ADC + a real bucket:
RULAKE_GCS_LIVE_TEST=1 \
RULAKE_GCS_BUCKET=your-bucket \
RULAKE_GCS_OBJECT=fixtures/docs-100k.parquet \
cargo test --release -- --ignored gcs_live
```

**Why Parquet first, not RVF** (the better-on-paper format): RVF currently has no public per-vector reader API in `vendor/ruvector/crates/rvf/rvf-runtime` — only `query()` (which returns IDs+distances, not vector content). Documented as an upstream gap in [`docs/research/rvf-backend-blocker.md`](docs/research/rvf-backend-blocker.md); a `rvf-backend/` crate lands when the upstream `read_all_vectors()` ships.

**Coming**: `parquet-on-s3` (same code, different `object_store` factory), `BigQueryBackend` (M3 — push-down via BQ Vector Search), `DeltaBackend` / `IcebergBackend` (M5).

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

**M1 + M1.5 + Audience shells (Python/Node/MCP) + first cloud backend (GCS) shipped** (2026-04-26)

<details>
<summary>✅ What's done — six sibling crates, zero unsafe in ruLake, all suites green</summary>

**Core (M1 + M1.5):**
- `BackendAdapter` trait, `VectorCache`, bundle protocol, 3 consistency modes, LRU
- Reference backends — `LocalBackend` (in-memory), `FsBackend` (file-based with `ruvec1` format)
- Optimizations — adaptive per-shard rerank, Arc-concurrency (13.2× concurrent), parallel prime (11× miss-path), AVX-512 VPOPCNTDQ + AVX2 dispatch, Hadamard rotation (3× build, 32× storage)
- Persist end-to-end — `save_cache_to_dir` / `warm_from_dir` with non-dense external ID preservation
- Observability — hit rate, prime durations, per-backend, per-collection attribution, warm-install counter
- Substrate acceptance test — six-guarantee loop (recall → verify → forget → rehydrate → location-transparency + compact-deferred)
- Security — path-traversal validation, JSON caps, witness verification, atomic writes
- `VectorKernel` trait scaffolding (ADR-157)
- 43 core tests (21 unit + 22 integration)

**Audience shells:**
- **Python SDK** ([`python/`](python/)) — PyO3 + ABI3 wheels, NumPy zero-copy, GIL release on hot paths. **14/14 tests.**
- **Node.js SDK** ([`node/`](node/)) — napi-rs, Float32Array zero-copy, async-only, `bigint` IDs. **10/10 tests.**
- **MCP server** ([`mcp-server/`](mcp-server/)) — `rulake-mcp` binary; stdio + Streamable HTTP; **JWT bearer with OAuth-style scope→capability mapping** + bearer (dev-only embarrassing-flag); 4 intents (search/verify/explain/refresh); 7 tools across read/internal/publish/admin capability tiers; **per-collection RBAC** via `[[allow]]` blocks; **layered rate limiting** (3 governor buckets); **replay protection** (MCP-Request-Id nonce LRU); **`tools/list` filtered by capability** (agents see only what they can call); bounded rayon worker pool; JSONL audit file with §7 schema. **39/39 tests.**

**First cloud backend:**
- **GCS Parquet** ([`gcs-backend/`](gcs-backend/)) — reads `LIST<FLOAT32>` columns from Parquet on GCS, generation = GCS object generation, cheap `current_bundle()` override. **4/4 offline + 1 live (gated) tests.**

**Bundle distribution:**
- **IPFS** ([`ipfs-backend/`](ipfs-backend/)) — publishes `table.rulake.json` bundles to kubo over HTTP RPC, addresses them by CIDv1; bundle-only (vector bodies stay on the body-store backend). Three modes: kubo / gateway-only / kubo + gateway-fallback. Witness ↔ CID via two-anchors-one-cache-key (ADR-005 §3). Per-VM cost ~$20–25/month on Compute Engine `e2-small`. **5/5 offline + 1 live (gated) tests.** ([ADR-005](docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md))

**ADRs:**
- ADR-001 (standalone repo), ADR-155 (cache-first), ADR-156 (substrate), ADR-157 (accelerator plane), ADR-158 (Hadamard + QVCache positioning), [ADR-002](docs/adrs/sdk/ADR-002-python-sdk.md) (Python SDK), [ADR-003](docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md) (Node SDK), [ADR-004](docs/adrs/sdk/ADR-004-rulake-mcp-server.md) (MCP server, 1340 lines), [ADR-005](docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md) (IPFS backend + GCP deploy, 1306 lines)
- Research note: [`docs/research/rvf-backend-blocker.md`](docs/research/rvf-backend-blocker.md) — why RVF-as-backend is upstream-blocked

**CI / release** — six GitHub Actions workflows in [`.github/workflows/`](.github/workflows/):
- `ci.yml` — every push/PR, all 5 sibling crates
- `release-python.yml` — ABI3 wheels (5 platforms) → PyPI on tag `python-v*`
- `release-node.yml` — napi-rs `optionalDependencies` (5 triples) → npm on tag `node-v*`
- `release-rust.yml` — `cargo publish` → crates.io on tag `rust-v*`
- `release-docker.yml` — distroless image → ghcr.io/ruvnet/rulake-mcp on tag `mcp-v*`
- `release-mcp-bin.yml` — prebuilt `rulake-mcp` binaries (5 triples) → GitHub Releases on tag `mcp-v*`

</details>

<details>
<summary>🗺 What's next</summary>

**Sprint-sized:**
- MCP server v0.5 — RS256/ES256 JWT (current v0.4 ships HMAC HS256/384/512) + JWKS fetch loop, mTLS auth mode, session-binding (token-to-(principal, client_id, mTLS-cert) tuple), `rulake://bundle/...` resource (v0.4 ships `rulake://stats` + `rulake://stats/by-backend`)

**Real product work:**
- Persistent disk-backed cache (ADR-155 §M1.5)
- More cloud backends — `BigQueryBackend` (M3 — push-down via BQ Vector Search), `DeltaBackend` / `IcebergBackend` (M5)
- Governance / M4 — RBAC, PII, lineage in OpenLineage. The MCP server's audit schema already maps onto this (swap-the-sink, not redesign-the-event-shape)

**Orthogonal:**
- WASM SDKs (browser / Cloudflare / Deno)
- HTTP wire client (`rulake.client.HttpRuLake` / `@ruvector/rulake/http`)
- Java SDK (both SDK ADRs flag v2)
- rabitq GPU kernel (ADR-157 — scaffolding only today)
- `rvf-backend/` once upstream `rvf-runtime` ships a public per-vector reader (see research note)

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
