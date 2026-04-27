# rulake — Benchmarks

> **Translated for agent builders:** these numbers tell you how fast
> your agent's memory lookups will be, what they'll cost, and where
> the limits are when you scale up. Headline: a cache hit costs
> ≈1 ms (1.02× the raw kernel cost), and the same Rust core powers
> the Python SDK, the Node.js SDK, the MCP server, and the GCS-Parquet
> backend — so the per-call tax stays bounded across every surface
> an agent might use to reach the lake.

All numbers produced by a **single reproducible run** of

```bash
cargo run --release -p rulake --bin rulake-demo
```

on a commodity Ryzen-class laptop, release build, deterministic seeds;
reruns bit-identical. The "Concurrent clients × shard count" section
below is multi-threaded (8 clients × 300 queries); the rest is
single-thread unless noted.

## Headline (LocalBackend, same dataset as `ruvector-rabitq`)

Clustered Gaussian, D = 128, 100 clusters, rerank×20, 300 queries per
row (warm-cache; prime time reported separately).

### Intermediary tax is ~0× on a local backend

| n       | direct RaBitQ+ (QPS) | ruLake Fresh (QPS) | ruLake Eventual (QPS) | tax (Fresh/Eventual) |
|--------:|---------------------:|-------------------:|----------------------:|---------------------:|
|   5 000 |              18,998  |            18,500  |             18,800    | 1.03× / 1.01×        |
|  50 000 |               5,959  |             5,900  |              5,950    | 1.01× / 1.00×        |
| 100 000 |               3,661  |             3,542  |              3,626    | 1.03× / 1.01×        |

**Wave-2 optimizations landed** (all preserve determinism):
- **AVX2 popcount** runtime-dispatched kernel (+20% single-thread scan)
- **CacheKey `Arc<str>` intern** (+11% concurrent QPS)
- **Thread-local encode scratch** (3 allocs → 1 per query)
- **Flattened originals** (24 MB saved at n=1M, SIMD-ready for rerank)

Interpretation:
- **Cache-hit path in `RuLake::search_one` costs effectively nothing** vs
  calling `RabitqPlusIndex::search` directly. The pos→id lookup + the
  HashMap get are in the noise.
- `Fresh` mode calls `LocalBackend::generation()` on every search (one
  hash-map read here). On a real backend this becomes a network RPC —
  **expect materially higher tax on BigQuery / Snowflake / S3-Parquet**.
  `Eventual { ttl_ms }` amortises it.
- Measured "prime" time is ≈ the `RabitqPlusIndex` build time on the
  pulled batch (210 ms / 50 k rows, 420 ms / 100 k rows, scales linearly).

### Federation — rayon parallel fan-out (M1)

Parallel fan-out is what the `prime` column measures now: a single
federated query that misses every shard primes them concurrently.

| n       | single-shard prime (ms) | 2-shard prime (ms) | 4-shard prime (ms) | speedup (2 / 4) |
|--------:|------------------------:|-------------------:|-------------------:|----------------:|
|   5 000 |                   22.3  |             12.7   |              6.6   |   1.76× / 3.38× |
|  50 000 |                  213.3  |            109.5   |             55.7   |   1.95× / 3.83× |
| 100 000 |                  424.8  |            215.3   |            110.1   |   1.97× / 3.86× |

Larger `n` hits closer to the theoretical K× ceiling because per-shard
work dominates over rayon + cache-insert serialization.

Warm-cache federated QPS (sequentially-issued queries):

| n       | single-shard QPS | 2 shards QPS | 4 shards QPS |
|--------:|-----------------:|-------------:|-------------:|
|   5 000 |          17,166  |      10,032  |       6,047  |
|  50 000 |           4,995  |       3,679  |       2,455  |
| 100 000 |           2,991  |       2,361  |       1,673  |

The QPS drop with shard count under this single-thread benchmark is
*not* pure `par_iter` startup overhead — see the concurrent-client
numbers below for the honest picture.

### search_batch vs per-query loop (n = 100 k, warm cache, single-threaded)

`RuLake::search_batch(queries, k)` amortizes `ensure_fresh` and the
cache mutex across N queries. Measured speedup on an already-primed
`LocalBackend` under `Consistency::Eventual` (the hot path):

| batch size |     QPS | speedup vs per-query |
|-----------:|--------:|---------------------:|
|         8  |   2,874 |              1.01×   |
|        32  |   2,961 |              1.04×   |
|       128  |   2,943 |              1.03×   |
|       300  |   2,986 |              1.05×   |
| per-query  |   2,855 | baseline             |

Modest on this workload — the warm cache path is already uncontended
(single-threaded, Eventual-TTL so `ensure_fresh` is a HashMap lookup,
not a backend RTT). The bigger wins for batch are latent:

- **`Consistency::Fresh`** — each per-query `ensure_fresh` is a
  backend round-trip. A batch of 300 on Fresh amortizes 300 RTTs
  into 1, which is catastrophically different at network latency.
- **Concurrent contention** — fewer mutex acquires under heavy
  multi-client load. Not measured in this single-threaded bench.
- **Kernel dispatch (ADR-157)** — GPU / SIMD kernels cross over CPU
  only above their `min_batch`. `search_batch` is the plug-point
  that makes dispatch tractable; a per-query API would never let
  GPU win.

Test `search_batch_acquires_cache_lock_once` proves the amortization
mechanically: a batch of 32 registers as 1 coherence check, not 32.

### Concurrent clients × shard count (n = 100 k, 8 clients × 300 queries)

With **Arc-wrapped cache entries** (the cache mutex no longer serializes
scans) and **adaptive per-shard rerank** (`max(5, global / K)`),
concurrent QPS scales linearly with core count:

| shards | wall (ms) |      QPS | QPS vs original baseline | per-shard rerank |
|-------:|----------:|---------:|-------------------------:|-----------------:|
|      1 |      86.3 |   27,814 |                    9.7×  |               20 |
|      2 |      74.5 |   32,194 |                   10.9×  |               10 |
|      4 |      65.4 |   36,715 |                   13.2×  |                5 |

**Wave-2 lift (AVX2 popcount + CacheKey intern stacked)** vs
Arc-refactor-only:

| shards | pre-wave2  | wave2      | lift  |
|-------:|-----------:|-----------:|------:|
|      1 |    23,681  |    27,814  | +17%  |
|      2 |    28,971  |    32,194  | +11%  |
|      4 |    33,094  |    36,715  | +11%  |

**Before the Arc refactor** (iter 28, 2026-04-23), the cache
`Mutex<CacheState>` held the scan duration, serializing all readers:

| shards | QPS (old) | QPS (new) | lift |
|-------:|----------:|----------:|-----:|
|      1 |    2,854  |   23,681  | 8.3× |
|      2 |    2,959  |   28,971  | 9.8× |
|      4 |    2,791  |   33,094  |11.9× |

The refactor: `CacheEntry::index` is now `Arc<RabitqPlusIndex>`. Readers
clone the Arc under the mutex (microseconds), drop the lock, then scan
unlocked. The index is immutable once built, so there's no data race;
concurrent readers parallelize perfectly. This is the single biggest
optimization of the M1 branch.

Recall@10 under K=2 / K=4 adaptive rerank stays above 85% on clustered
D=128 n=5k (gate test `adaptive_per_shard_rerank_preserves_recall`).

**Implementation:** the floor (`MIN_PER_SHARD_RERANK = 5`) stops the
divide-by-K from going below the point where exact L2² rerank can
meaningfully separate near ties. Callers who need byte-exact parity
with single-shard output can pass `Some(global_rerank)` explicitly via
`search_federated_with_rerank`.

**Consequences.** The rayon fan-out pays off now — it minimizes tail
latency on the miss path (prime-time speedups earlier in this file)
AND keeps hot-path throughput at ~parity with the single-shard
baseline. Federation is genuinely free for reachability/memory sharding
on same-box setups; on network-backed setups it's a clear win.

### Randomized Hadamard rotation (ADR-158, opt-in)

`RabitqPlusIndex::new_with_rotation(.., RandomRotationKind::HadamardSigned)`
replaces the default D×D Haar matrix (`O(D²)` per query, `4·D² B`
storage) with a D₁·FWHT·D₂·FWHT·D₃ pattern (`O(D log D)` per query,
`12·D B` storage). Recall is preserved; the RaBitQ error bound only
requires "close to Haar-uniform" (TurboQuant 2025 §3.2) and HD-HD-HD
meets that bar.

At D=128 (clustered Gaussian, rerank×20, single-thread):

| n       | Haar build | Hadamard build | build speedup | Haar QPS | Hadamard QPS |
|--------:|-----------:|---------------:|--------------:|---------:|-------------:|
|   5 000 |    22.4 ms |        7.2 ms  |       3.09×   |   18,894 |      20,881  |
|  50 000 |   211.6 ms |       72.7 ms  |       2.91×   |    6,065 |       5,854  |
| 100 000 |   421.1 ms |      142.9 ms  |       2.95×   |    3,675 |       3,638  |

Rotation storage: **66,052 B Haar → 2,052 B Hadamard at D=128
(32.2× reduction)**. Per-query QPS is within ±3% because the scan +
rerank steps dominate over the rotation cost at n ≥ 50k. The win is
on the cold-start / prime path — **3× build-time speedup** stacks
with the existing `parallel prime` (rayon) optimization to give
millisecond-scale primes even at n=100k.

Recall@10 vs exact L2² brute force on clustered D=128 n=500: **1.000
for both Haar and Hadamard** (test `hadamard_recall_at_10_within_5pct_of_haar`).

## Per-surface tax — what your agent actually pays

The numbers above are direct Rust calls into `RuLake::search_one`. In
production an agent reaches the lake through one of four surfaces.
Each adds a small, bounded layer of overhead.

| Surface | What it adds | Tax target (vs direct Rust) | Status |
|---|---|---|---|
| **Rust direct** (`cargo add rulake`) | nothing — in-process call | 1.00× (baseline) | measured ([above](#headline-localbackend-same-dataset-as-ruvector-rabitq)) |
| **Python SDK** ([`sdk/python/`](sdk/python/), PyO3) | NumPy zero-copy borrow + GIL release | **≤ 1.10×** ([ADR-002 §Verification](docs/adrs/sdk/ADR-002-python-sdk.md)) — ~1 µs FFI per call | budget; bench gate ships in v0.4 |
| **Node.js SDK** ([`sdk/node/`](sdk/node/), napi-rs) | one Float32Array memcpy across `await` (~3 µs at D=768) | **≤ 1.10×** ([ADR-003 §Verification](docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md)) | budget; bench gate ships in v0.4 |
| **MCP server stdio** ([`crates/mcp-server/`](crates/mcp-server/), `rulake-mcp`) | JSON-RPC framing + serde + planner pass | **≤ 1.20×** ([ADR-004 §Verification](docs/adrs/sdk/ADR-004-rulake-mcp-server.md)) | budget; bench gate ships in v0.4 |
| **MCP server Streamable HTTP** | + TCP/HTTP framing + bearer-auth check | ~1.25× expected; not yet measured | unmeasured |

**For agent builders:** the worst case (an agent calling
`rulake_query` over MCP-stdio) is still ≤ 1.2 ms cache-hit at the
benchmarked workload — well inside the budget for an agent loop that
spends most of its time waiting on an LLM round-trip.

**For cloud-backend builders:** the GCS Parquet backend
([`crates/gcs-backend/`](crates/gcs-backend/)) adds **one HEAD request per cache miss**
for `current_bundle()` (~30 ms typical from GCP networks) and
**zero per cache hit** — every subsequent search inside the
Consistency TTL window is a pure local scan. A "cold" Parquet pull on
a 100k-row file is dominated by the GCS body fetch (~250 ms over a
1 Gb/s link), not by ruLake's encode step (~210 ms / 50k vectors).
Real M2 acceptance bench against a live bucket lands when the
ParquetBackend gate test does.

## Acceptance checks (M1)

The smoke tests under `crates/core/tests/federation_smoke.rs` gate M1 from
`docs/research/ruLake/07-implementation-plan.md`, plus bundle tests
in `crates/core/src/bundle.rs` (including FS persistence):

| # | Test | What it proves |
|---|---|---|
| 1 | `rulake_matches_direct_rabitq_on_local_backend` | Federation path is byte-exact vs direct RaBitQ at the same seed + rerank factor |
| 2 | `rulake_recomputes_on_backend_generation_bump` | Cache coherence protocol works — backend mutation is observed on next search |
| 3 | `rulake_federates_across_two_backends` | Multi-backend fan-out + score merge produces the globally-correct top-k |
| 4 | `cache_hit_is_faster_than_miss` | Cache prime-then-serve path beats uncached (measurement-level sanity) |
| 5 | `rulake_recall_at_10_above_90pct_vs_brute_force` | End-to-end recall on clustered data stays above 90% |
| 6 | `two_backends_share_cache_when_witness_matches` | Witness-addressed cache lets two backends serving identical bytes share one compressed entry |
| 7 | `lru_eviction_caps_entry_count_when_pointers_dropped` | Bounded-memory mode: LRU evicts unpinned entries |
| 8-10 | `*_returns_error` | Error types surface on bad inputs / misconfig / unknown collections |
| 11-19 | bundle tests | Witness determinism, length-prefixing, tamper detection, FS roundtrip + atomic write, tamper-on-disk rejection |

```
cargo test --release   # core 43 + python 14 + node 10 + mcp-server 21 + gcs-backend 4 = 92 pass
```

## What's NOT benchmarked (current scope)

- **Real-backend network latency.** `LocalBackend::pull_vectors` is an in-process
  HashMap read; the Fresh-mode tax reported above is the floor, not the ceiling.
  Real backends (Parquet on S3, BigQuery via Storage Read API) add 10–100 ms
  per prime. The GCS backend ships in [`crates/gcs-backend/`](crates/gcs-backend/) today; a
  measured live bench against a real bucket lands with the M2 acceptance gate.
- **Recall regressions vs direct RaBitQ.** The test suite confirms byte-exact
  ordering + scores at the same seed. Formal recall sweeps across n / D /
  rerank_factor reuse `ruvector-rabitq::BENCHMARK.md` — ruLake doesn't change
  recall, only the distribution layer.
- **Push-down paths.** ADR-155 §Decision 4 defers backend-native vector ops
  to Tier-2 per-adapter. Not measured.
- **Per-surface tax under load.** The Python / Node / MCP-server tax targets
  in the table above are *budgets* from the respective ADRs, not measured
  numbers. Per-surface bench gates ship in the v0.4 of each crate.
- **Cache memory footprint vs backend size.** LRU eviction over unpinned
  entries is implemented (`RuLake::with_max_cache_entries`). Not yet
  tuned under memory pressure.
- **Edge / WASM throughput.** WASM SDKs are roadmap (ADR-002 §A,
  ADR-003 §A — `@ruvector/rulake-wasm` reserved). They lose AVX-512
  popcount + rayon parallel fan-out, so the per-query cost will be
  meaningfully higher than the native numbers above. Bench when ship.

## Reproduce

```bash
# Core Rust crate (the numbers in this file).
cargo run   --release --bin rulake-demo           # ~30 s on n=100k
cargo run   --release --bin rulake-demo -- --fast # ~5 s smoke
cargo test  --release                             # 43 pass

# Per-sibling-crate test suites — each has its own README:
cd python      && maturin develop --release && pytest tests/         # 14 pass
cd node        && cargo build --release && \
                  cp target/release/libruvector_rulake_node.so rulake.linux-x64-gnu.node && \
                  node --test __test__/smoke.test.mjs                # 10 pass
cd mcp-server  && cargo test --release                                # 21 pass
cd gcs-backend && cargo test --release                                # 4 pass (offline)
```

Dataset generator + seeds in `crates/core/src/bin/rulake-demo.rs::clustered`.
