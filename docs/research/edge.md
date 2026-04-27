# ruLake on the Edge — Vertical Applications for Embedded, Mobile, and On-Device Vector Workloads

**Scope.** This document examines where ruLake — the cache-coherent vector
execution fabric under `/home/ruvultra/projects/RuLake/` — fits in
edge, embedded, and on-device deployments. It is one of four vertical
research notes; the others cover datacenter, agentic, and lakehouse
verticals. Edge here means anything from a Raspberry Pi 5 in a smart
camera, through Android / iOS phones running on-device LLMs, into the
browser via WebAssembly, out to industrial PLCs and SCADA gateways,
and as far as we can honestly take the substrate before microcontrollers
without an OS become the wrong tool.

**Method.** Every claim about ruLake's behaviour is grounded in a
specific source file or ADR. Sizing tables are derived from the
RaBitQ 1-bit storage model (`D/8` bytes per vector, plus rotation
state and rerank buffers), the measured benchmarks in
`/home/ruvultra/projects/RuLake/BENCHMARK.md`, and the cache geometry
documented at `crates/core/src/cache.rs`. Latency budgets are calibrated against
`BENCHMARK.md`'s headline numbers (1.02× direct RaBitQ on the hit path;
~3,500 QPS at n=100k D=128 single-thread on a Ryzen-class laptop)
and scaled for the relevant edge silicon. Where ruLake will not work
today, that is stated explicitly rather than waved past.

**Style.** No padding. No marketing copy. Where the substrate is a
poor fit for an edge constraint — hard real-time, microcontrollers,
sub-millisecond determinism — that is called out and the alternative
substrate is named.

---

## Contents

1. Why edge needs cache-coherent vectors
2. Vertical 1 — On-device LLM RAG (mobile / desktop)
3. Vertical 2 — Browser / WASM deployment
4. Vertical 3 — Robotics and autonomous vehicles
5. Vertical 4 — Industrial IoT (factory floor, OT networks)
6. Vertical 5 — Edge inference cache for similarity search
7. Vertical 6 — Federated edge with cloud truth
8. Vertical 7 — Mobile agent (offline-first)
9. Vertical 8 — Embedded Linux: Pi 5 / Jetson class
10. Vertical 9 — Microcontroller adjacency (ESP32-class and below)
11. Vertical 10 — In-vehicle infotainment and cockpit AI
12. Vertical 11 — Drones and UAVs
13. Power / thermal envelope
14. Reality check — where ruLake will not work
15. Open questions and recommended extensions

---

## 1. Why edge needs cache-coherent vectors

The defining edge tradeoff is the triangle of **freshness, latency,
and bandwidth**, with the further constraint that compute and memory
are bounded in ways a datacenter never sees. A device that wants
semantic retrieval — find the nearest five "known faces", retrieve
the three most similar past sensor readings, look up the closest match
to a partial command — is forced to choose:

- Round-trip every query to the cloud. Adds 50–500 ms of network
  latency per query, requires the link to be up, and burns radio
  power that on battery-powered devices dwarfs the cost of doing the
  work locally.
- Hold the entire vector store on-device, statically. Solves latency
  and bandwidth but freezes the index — no way to integrate new data
  from the cloud without redeploying the binary.
- Run a vector DB on the device. SQLite-VSS, USearch, LanceDB, even
  an in-process FAISS — all work, but none of them speak a coherence
  protocol against an upstream truth, so any update story is bespoke.

**ruLake sits in the middle of this triangle**: it owns the
in-memory compressed cache (RaBitQ 1-bit codes; ~D/8 bytes per
vector), it has a swappable `BackendAdapter` that can be a local
filesystem, a memory-mapped file, or a cloud bucket polled at a
human-paced interval, and it has a witness-anchored bundle protocol
(`crates/core/src/bundle.rs`, `RuLakeBundle`) that lets you say "the cloud just
republished its truth; refresh your cache" in ~10 lines of code
(`crates/core/src/lake.rs:200`, `refresh_from_bundle_dir`).

The three consistency modes (`crates/core/src/cache.rs:55` `Consistency` enum)
map cleanly onto the edge requirement spectrum:

- `Frozen` — burned-in mission data. The drone has a target list for
  the duration of the flight; the audit system has a snapshot for
  the trip. Consistency check is skipped after the first prime; the
  witness pins the bytes cryptographically.
- `Eventual { ttl_ms }` — the working steady state for edge
  inference. Refresh the witness against the backend at most every
  N milliseconds; serve everything else out of compressed memory.
- `Fresh` — only when the law (HIPAA, GDPR, automotive functional
  safety) forbids serving stale data. Almost never the right edge
  default; on a Parquet-on-S3 backend `Fresh` adds a backend RTT to
  every query (`docs/review/performance.md` §2 calls this out as
  "self-inflicted DoS" if `current_bundle` isn't overridden).

### Comparison to the obvious alternatives

| System | Where it lives | Cache coherence | Cross-process sharing | On-device compress | Edge fit |
|---|---|---|---|---|---|
| **ruLake** | Rust crate, in-process | Witness + 3 modes | Content-addressed via SHAKE-256 | RaBitQ 1-bit (~32×) | Strong on Pi/Jetson/phone, conditional on browser |
| SQLite-VSS | SQLite extension | Manual, app-driven | None | Optional float16 | Strong, but no coherence story |
| USearch | C++ / Rust / Py | None | None | Quantized index | Strong as a kernel; you build coherence |
| FAISS | C++ heavyweight | None | None | Various | Bad — too large, no edge-friendly persist |
| LanceDB | Rust embedded | Snapshot id | Per-process | IVF_PQ, etc. | Strong, but heavier than ruLake |
| SurrealDB Vector | Embeddable DB | DB-level | DB-level | Optional | Heavy — full DB engine |
| Pinecone Edge | Hosted SaaS | Server-side | Server-side | Server-side | Bad — assumes always-on link |

ruLake's distinguishing edge property is that the **bundle sidecar is
the unit of sync**, not the whole index. A ~300-byte JSON file
(`crates/core/src/bundle.rs:113-155`) carries the witness, dim, seed, rerank, and
generation; as long as the witness matches what the cache holds, the
edge device serves out of memory and never round-trips. When the
witness changes — the cloud rotated its data — the cache invalidates
and the next query primes from whatever the local backend is, which
on edge is usually the FsBackend over a directory the cloud has
already filled with the new `ruvec1` binary
(`crates/core/src/fs_backend.rs:14-22`).

This split — small sidecar over the slow link, big binary already
local — is the architectural shape that makes ruLake a real edge
substrate rather than just a vector library that happens to be small.

---

## 2. Vertical 1 — On-device LLM RAG (mobile / desktop)

The first vertical where ruLake earns its keep on edge is on-device
retrieval-augmented generation. The model lives on the device — a
quantized 1B–8B parameter LLM via `llama.cpp`, MLX, or a Rust runtime
like `mistral.rs` — and the embedding store needs to be accessed at
generation time without involving the cloud.

### Why this is hard without a substrate

A naive "load all my notes into FAISS at app start" approach fails
the moment the corpus crosses a few thousand documents:

- Cold-start latency. A 50k-document IVF_PQ build takes seconds; the
  user sees the spinner.
- Memory cost. f32 vectors at D=768 are 3 KB each; 50k of them is
  150 MB before the index overhead.
- No coherence with the cloud. If the user adds a note on the
  desktop, the phone's index is stale until the next manual rebuild.
- No warm restart. App relaunch re-pays the build cost.

ruLake addresses three of those four directly, and the fourth
(embedding compute itself) is somebody else's problem.

### Sizing tables

RaBitQ stores vectors as 1 bit per dimension after rotation, so the
compressed footprint per vector is `ceil(D/8)` bytes for the codes
plus a small constant for rotation state and rerank buffer. The
rotation matrix is per-index, not per-vector, so it disappears in the
amortization at typical n. ruLake's `pos_to_id` map adds 8 bytes per
vector (`crates/core/src/cache.rs:223-232`) — the documented memory-audit
finding #2 — so the per-vector cost in the cache is `D/8 + 8` bytes.

**Table 2.1 — Vectors per memory budget at common embedding dims**

| Memory budget | D=384 (e.g. all-MiniLM) | D=768 (BERT-base) | D=1024 (text-embedding-3-small) | D=1536 (text-embedding-3-large) |
|---:|---:|---:|---:|---:|
|  10 MB |    178 k |     104 k |     78 k |     54 k |
|  50 MB |    892 k |     521 k |    390 k |    270 k |
| 100 MB |  1.78 M  |    1.04 M |    781 k |    540 k |
| 250 MB |  4.46 M  |    2.61 M |   1.95 M |    1.35 M |
| 500 MB |  8.92 M  |    5.21 M |   3.91 M |    2.70 M |

(Computed: `vectors = budget / (D/8 + 8)`; rotation overhead
≤ 16 KB per index per
`docs/adrs/ADR-158-optional-rotation-and-qvcache-positioning.md`,
absorbed into rounding.)

For mobile-LLM RAG at D=768 (the modal embedding dim for sub-3B
on-device models in early 2026), **a 100 MB budget holds about 1
million vectors** in the compressed cache. That is enough corpus
for a year of personal notes, every email, every code file in a
typical developer's workspace, or a moderate-sized PDF library.

A rough corpus calibration:

- Personal notes (5,000 docs × ~5 chunks each) → 25k vectors → 2.5 MB.
- Codebase (200k LOC, chunked at function granularity) → ~50k vectors
  → 5 MB.
- Email (10 years × ~50 emails/day) → ~180k vectors → 18 MB.
- All of the above plus PDF library plus voice transcripts → still
  comfortably under 100 MB at D=768 for a power user.

The 32× compression vs raw f32 (`docs/adrs/ADR-158` §2) is what makes
the budget realistic on a phone. Without RaBitQ, the same 1M-vector
corpus at D=768 would cost 3 GB — out of bounds for any phone.

### Latency budgets

`BENCHMARK.md` reports **3,681 QPS direct RaBitQ at n=100k D=128**
on a Ryzen-class laptop, single-threaded. Scaling to phones requires
two adjustments:

1. **Dimensionality.** Scan cost is `O(n × D / 64)` for the popcount
   pre-filter plus `O(rerank × k × D)` for the exact L2² rerank
   (`docs/review/performance.md` §5). Going from D=128 to D=768 is
   6× more work in both phases; QPS drops to ~600 on the same silicon.
2. **CPU class.** A 2025-era Apple A18 or Snapdragon 8 Gen 4 has
   per-core integer popcount throughput within 2× of a Ryzen laptop
   core. AVX2 / AVX-512 are unavailable; the scalar popcount path
   in rabitq runs (`README.md` §"Kernels" #25). Realistic derating:
   2–3× slower per query than the benchmark.

Putting it together for an on-phone agent:

**Table 2.2 — Estimated p50 query latency on a 2025 flagship phone, D=768**

| n (vectors in cache) | p50 latency | p99 latency | Use case |
|---:|---:|---:|---|
|     1 k |    < 0.1 ms |    < 0.5 ms | Contact list, small caches |
|    10 k |    ~ 0.5 ms |    ~ 2 ms   | Per-app personalization |
|   100 k |    ~ 5 ms   |    ~ 20 ms  | Full notes / code corpus |
|     1 M |    ~ 50 ms  |    ~ 200 ms | Power-user life corpus |

The p99 is conservative — it folds in cache-mutex contention
(`docs/review/performance.md` B1) under any concurrent load, even
though phones are typically single-tenant for the agent process.

For an LLM RAG loop where the model itself takes 200–500 ms per
generation step (1B-class models on a phone NPU at int4), retrieval
at 5 ms per query at n=100k is **invisible in the response budget**.
That is the headline result: ruLake disappears into the noise of the
LLM serve loop, even at a respectable corpus size.

### Coherence in practice

The phone runs a `Consistency::Eventual { ttl_ms: 60_000 }` ruLake
against an `FsBackend` rooted at the app's sandbox directory. A
background sync service polls the user's cloud (iCloud, Google Drive,
Dropbox) for a `table.rulake.json` rotation:

```rust
// background sync loop (runs every 5s, or on FCM/APNs push)
match lake.refresh_from_bundle_dir(&key, "/var/mobile/Containers/.../bundles/")? {
    RefreshResult::Invalidated => {
        // Cloud rotated. Pull the new ruvec1 file before the next query
        // primes against an empty backend.
        download_and_replace("notes.bin").await?;
    }
    RefreshResult::UpToDate => {} // 99% of the time
    RefreshResult::BundleMissing => log::info!("no cloud sync yet"),
}
```

The witness check costs one ~300-byte JSON parse plus a SHAKE-256
recompute (microseconds). If the witness matches, the cache continues
to serve from compressed memory; if it doesn't, the next query primes
transparently from the new file.

Critically, `Eventual` mode means the user's queries do not block on
the cloud poll. Stale results for up to 60 s after a cloud update
is the explicit tradeoff documented in `crates/core/src/cache.rs:62-65`.

### Where it stops being a fit

- **Embedding compute itself.** ruLake assumes you have already turned
  text into f32 vectors. On-device embedding models (e.g. a quantized
  all-MiniLM at int8) cost 5–50 ms per chunk — that is your dominant
  latency, not retrieval. ruLake is the layer below the embedder.
- **Stream-on-write workloads.** A user typing a query character by
  character should not re-embed and re-search 200 times. That is a
  product debouncing decision, not a substrate property.
- **Cross-app sharing.** iOS app sandboxing forbids two apps from
  sharing the same on-device file. ruLake's content-addressed cache
  sharing is irrelevant when the OS wall is in the way; each app
  owns its own ruLake instance with its own backend.

---

## 3. Vertical 2 — Browser / WASM deployment

ruLake is currently a Rust 1.89+ crate with no FFI and zero `unsafe`
code in its own surface (`docs/review/security.md` §6, verified by
`grep -rn "unsafe " src/`). `ruvector-rabitq` — the kernel — has two
SIMD scan functions that are AVX2 / AVX-512 specific
(`README.md` table cells 26, 27), but the scalar popcount baseline
(table cell 25) is portable and is what runs on any non-x86 target.

That makes a WASM build of ruLake **architecturally feasible
today**, with caveats listed below. Nobody has shipped one yet —
neither the README nor any ADR claims a WASM build exists — so the
honest framing is: this is the first realistic edge target where the
work has not been done, but the substrate is designed to allow it
(ADR-157 §"Targets" explicitly lists "Browser / edge — WASM SIMD
(no GPU)" as a deployment target).

### What a WASM build would look like

`Cargo.toml` (`/home/ruvultra/projects/RuLake/crates/core/Cargo.toml`) declares
nine direct dependencies. Of those:

- `serde`, `serde_json`, `thiserror`, `sha3`, `hex`, `rand`,
  `rand_distr` — all `no_std`-friendly or trivially WASM-portable.
- `rayon` — does **not** work in WASM (single-threaded by default;
  `rayon` requires real OS threads). The federated fan-out and
  parallel prime paths would need a `#[cfg(not(target_arch =
  "wasm32"))]` shim that falls back to serial execution. For a
  browser deployment this is acceptable: typical browser caches
  hold one or two collections, not federated sets.
- `ruvector-rabitq` — vendored under `vendor/ruvector/`. The scan
  function is `cfg`-gated for AVX2 / AVX-512 (`README.md` table cell
  28 documents the runtime CPUID dispatch); the scalar fallback is
  what WASM would land on.

What would compile out of the box if you ran `cargo build
--target=wasm32-unknown-unknown`:

- `RuLake::new`, `with_consistency`, `with_max_cache_entries`.
- `register_backend` against a custom `WasmBackend` (since
  `LocalBackend` is in-memory and works fine; `FsBackend` does not,
  because the browser has no POSIX filesystem).
- `search_one` (would compile but the rayon-using
  `search_federated` would need a fallback).
- The bundle protocol — `publish_bundle`, `refresh_from_bundle_dir`
  would need to swap filesystem ops for IndexedDB or
  `localStorage`.
- `save_cache_to_dir` / `warm_from_dir` would need similar WASM-side
  storage primitives.

### Latency expectations in the browser

Browsers do not give you AVX-512. A WASM build hits the scalar
popcount path. Empirically, WASM scalar code runs at ~50–70% of
native scalar speed on the same CPU (V8 / SpiderMonkey overhead).
WASM SIMD (the `wasm32-unknown-unknown` target with `+simd128`)
narrows that to ~80–90% but does not get back the AVX2 win.

Calibrating against `BENCHMARK.md`'s 3,681 QPS at n=100k D=128 native
single-thread on a Ryzen-class laptop:

**Table 3.1 — Estimated browser-side semantic search QPS, D=128**

| n (vectors) | Native scalar | WASM SIMD128 | WASM scalar |
|---:|---:|---:|---:|
|   1 k |   100 k+ |    80 k |    50 k |
|  10 k |    37 k |    30 k |    19 k |
|  50 k |    7.4 k |    6 k  |    3.7 k |
| 100 k |    3.7 k |    3 k  |    1.9 k |

For a browser-side semantic search over **10k embeddings at
D=128** — the realistic size for a Chrome extension that caches
your tab history, or a static-site search index — you get ~30k QPS
under WASM SIMD128. That is **comfortably interactive**: even at
the worst case of 10 queries per keystroke, a typing user with
50 ms keypress intervals never exhausts the budget.

For **100k embeddings at D=384** (a hypothetical browser-side full
documentation search), the linear scaling implies ~1k QPS in WASM
SIMD128. Still interactive at one-query-per-search-submit, but
keystroke-debouncing is now necessary.

### Memory in the browser

Browsers cap WASM linear memory to 4 GB total (32-bit), and in
practice a tab is healthier under 1 GB. Re-applying Table 2.1 with
that cap:

- 100 MB cache budget, D=384 → 1.78 M vectors.
- 100 MB cache budget, D=768 → 1.04 M vectors.

The constraints map directly: a browser tab's "100 MB available for
embeddings" is the same shape as a phone app's, and ruLake's
compression makes both viable.

### What blocks a browser build today

Three concrete pieces of work are missing:

1. **A `WasmBackend` impl of `BackendAdapter`** (`crates/core/src/backend.rs:110`).
   The trait is 4 methods (`id`, `list_collections`, `pull_vectors`,
   `generation`) plus `current_bundle` override. A browser-side
   implementation pulls vectors from IndexedDB or fetches from a
   pre-published bundle directory served as static assets.
2. **`#[cfg(not(target_arch = "wasm32"))]` gates around the rayon
   `par_iter` in `lake.rs:528` and `cache.rs:402`.** Falls back to
   serial. No correctness change; only the federation parallelism is
   lost.
3. **A WASM-friendly persistence layer.** `save_cache_to_dir` /
   `warm_from_dir` use `std::fs`. A browser equivalent writes to
   IndexedDB (binary blob for the `.rbpx`, JSON for the sidecar).

None of these are large changes; the WASM-readiness is mostly a
matter of writing the adapters, not refactoring the core. Anyone
building a browser-side semantic-search extension in 2026 would find
the substrate ergonomic.

### What you do not get in WASM

- AVX-512 VPOPCNTDQ (the +10.5% kernel win, `README.md` cell 27).
- AVX2 + POPCNT (the +20% single-thread win, `README.md` cell 26).
- Real parallelism in `search_federated` and parallel prime — single
  thread is the ceiling without `wasm-bindgen-rayon` and SharedArrayBuffer
  support, which adds significant deployment complexity (cross-origin
  isolation headers, etc.).
- mmap'd persistence (the M2+ roadmap item) — IndexedDB does not
  expose the mmap primitive.

---

## 4. Vertical 3 — Robotics and autonomous vehicles

Robotics is where the consistency-mode story becomes load-bearing.
A robot has at least two distinct retrieval workloads with opposite
constraints:

- **Perception pipeline** — "is this object I'm seeing in my
  catalog of known parts?" Soft real-time, refresh-tolerant, runs
  many times per second.
- **Safety system** — "is this scenario in my approved-behaviour
  envelope?" Hard correctness, must run against an audit-pinned
  snapshot, never against a transient cloud update.

Both can sit on the same ruLake instance, on different collections,
with different consistency modes. The substrate already supports
this pattern; no work needed.

### Per-collection isolation for sensor modalities

A single robot fuses vision, lidar, and radar. Each modality
produces embeddings at different dimensionalities — vision via a
ResNet variant at D=2048, lidar via a PointNet at D=512, radar via
a custom model at D=128. ruLake's `(backend_id, collection_id)`
addressing (`crates/core/src/cache.rs:166`) lets each modality be a separate
collection within a single backend, with independent witness chains
and independent generation tokens:

```rust
let lake = RuLake::new(20, 42)
    .with_consistency(Consistency::Eventual { ttl_ms: 1000 })
    .with_max_cache_entries(8);

// One backend per sensor modality, distinct dim each.
let cam_backend = Arc::new(FsBackend::new("vision", "/var/robot/vision/")?);
let lid_backend = Arc::new(FsBackend::new("lidar", "/var/robot/lidar/")?);
let rad_backend = Arc::new(FsBackend::new("radar", "/var/robot/radar/")?);
lake.register_backend(cam_backend)?;
lake.register_backend(lid_backend)?;
lake.register_backend(rad_backend)?;

// Catalog of known objects per modality, primed at startup.
let _ = lake.search_one("vision", "known_objects", &cam_embedding, 5)?;
let _ = lake.search_one("lidar",  "known_objects", &lid_embedding, 5)?;
let _ = lake.search_one("radar",  "known_objects", &rad_embedding, 5)?;
```

The dim mismatch between collections is enforced at search time
(`crates/core/src/cache.rs:750`, `DimensionMismatch` error), so a vision-dim
query against the lidar collection fails loud rather than silently
returning garbage. That is the right failure mode for a safety
system.

### Frozen mode for the safety snapshot

The safety system runs against a pre-validated, witness-pinned
catalog of scenarios. `Consistency::Frozen` (`crates/core/src/cache.rs:67-77`)
asserts that the catalog is immutable for the cache's lifetime; the
witness is computed once at boot, the cache primes, and from then
on no automatic coherence check ever runs.

```rust
let safety_lake = RuLake::new(20, 42)
    .with_consistency(Consistency::Frozen);
let safety_backend = Arc::new(FsBackend::new("safety", "/var/robot/safety/")?);
safety_lake.register_backend(safety_backend)?;
// Prime explicitly at boot.
safety_lake.search_one("safety", "approved_envelopes", &probe, 1)?;
// From here on, no backend RTT for any query against this collection.
```

The witness is recoverable — `safety_lake.cache_witness_of(&key)`
returns the SHAKE-256 hex (`crates/core/src/lake.rs:134`), which the system can
log, sign, and submit as evidence in a post-incident review. Because
the witness is anchored on the bundle (`data_ref`, `dim`,
`rotation_seed`, `rerank`, `generation`), it cryptographically pins
**which bytes the safety decisions were made against**.

This is the property that makes ruLake interesting for safety-critical
workloads: not the throughput, not the compression, but the ability
to point at a 32-byte hex string and say "every retrieval the safety
system did during this trip resolved against this exact snapshot,
provably."

### Hard real-time — the honest take

ruLake **is not a hard real-time system**. The hot path uses
`std::sync::Mutex` (`crates/core/src/cache.rs:239` — `Arc<Mutex<CacheState>>`),
and the documented review (`docs/review/performance.md` §2) calls
out that every cache operation acquires a global mutex even for
read-mostly bookkeeping. The mutex is short-held — the heavy scan
runs unlocked via the Arc-drop-lock pattern (`crates/core/src/cache.rs:734-762`)
— but it is still a lock, with all the WCET (worst-case execution
time) ambiguity that implies.

For a system that needs strict deadline guarantees:

- **Avoid concurrent access.** Single-threaded use of a `Frozen`
  ruLake against a primed cache has predictable latency: one
  `Mutex::lock()` (microseconds, uncontended), one `Arc::clone`,
  one drop, one scan. Deterministic up to scheduler jitter.
- **Pre-prime everything.** Frozen mode after a successful prime
  has no allocation in the hot path beyond the result vector. No
  `pull_vectors`, no rabitq build.
- **Pin the thread.** The per-thread cost of priority inversion
  on the cache mutex disappears if only one thread ever touches the
  cache.

With those caveats, **soft real-time use up to ~10 kHz query rate
on a single-threaded loop is realistic** at small n (~10k vectors,
D=128). Above that, the lock contention bottleneck
(`docs/review/performance.md` B1) starts to matter, and at very high
QPS the Arc-drop-lock window itself is a source of variance.

ruLake is **not** the right substrate for:

- Sub-millisecond deadline retrieval (e.g. an inner ABS loop).
- Lock-free memory-bounded queues (use `crossbeam` channels and a
  pre-built static index).
- Anything that requires statically-bounded heap allocation
  (RaBitQ's prime path allocates on miss; you must avoid the miss
  path under deadline by staying in `Frozen` mode after warm-up).

For a perception-loop workload, ruLake fits inside the soft real-time
budget. For the safety system, Frozen + pre-primed gives a static
hot path. For an inner control loop, use a different layer of the
stack.

### Witness-pinned audit replay

A useful corollary: because the witness is content-addressed, the
on-vehicle log can record "(timestamp, query, top-k results,
witness_hex)" and a post-trip analysis can rehydrate the **exact**
cache state that produced the result by warm-loading from the
matching bundle bytes. `RuLake::warm_from_dir` (`crates/core/src/lake.rs:378`)
plus the recorded witness gives byte-exact replay
(`crates/core/src/lake.rs:378` doc says "byte-exact query results without
backend RTT"), which is the audit replay primitive a safety
investigator actually wants.

---

## 5. Vertical 4 — Industrial IoT (factory floor, OT networks)

Industrial IoT (IIoT) is the vertical where ruLake's federation
story stops being theoretical. A factory floor has tens to hundreds
of edge gateways: PLCs, SCADA boxes, vibration analyzers, vision
inspection stations. Each runs a small Linux box (often i.MX,
Atmel, or Intel Atom-class) with intermittent backhaul to a central
plant server, and increasingly intermittent backhaul to a cloud
truth.

### The connectivity profile

OT networks are not the public internet. Typical assumptions:

- Gateways are wired but on a segregated VLAN with one chokepoint
  to IT, often a Purdue Level 3.5 firewall.
- Cloud access is rate-limited or windowed — "you may push to S3
  during the maintenance window from 02:00 to 04:00".
- Power cycles are routine; UPS coverage varies.
- Devices live for 10–20 years; firmware updates are a quarterly
  event at most.

ruLake's two relevant primitives for this:

1. **Bundle sidecar over MQTT or LoRaWAN.** The sidecar is ~300
   bytes (`README.md` cell 8). MQTT topic `factory/line3/witness`
   carries the bundle JSON; subscribers parse and call
   `refresh_from_bundle_dir` against a local mount. LoRaWAN
   payloads max at ~242 bytes per uplink — the bundle just barely
   fits, depending on how long the `data_ref` URI is. For LoRaWAN
   you would more realistically broadcast just the witness hex
   (64 bytes) and have the receiver re-fetch the full sidecar over
   a higher-bandwidth secondary link when the witness changes.

2. **`save_cache_to_dir` / `warm_from_dir` for power-cycle
   survival.** A vibration analyzer that builds an embedding catalog
   of "known healthy bearing signatures" runs its initial prime on
   first deployment, persists via `save_cache_to_dir`
   (`crates/core/src/lake.rs:263`), and on every reboot calls `warm_from_dir`
   (`crates/core/src/lake.rs:378`). The warm-load takes ~5 ms for n=5000 D=128
   per ADR-155 §"Status" — fast enough that the gateway is back in
   anomaly-detection service before the SCADA poll cycle notices the
   reboot.

### Federated cache across PLCs

The factory has 50 vibration analyzers. Each holds its own catalog
of known signatures for its own machine. A central plant server
runs ruLake federated across all 50, allowing engineers to ask
"have we seen this signature anywhere on the floor?":

```rust
let plant = RuLake::new(20, 42)
    .with_consistency(Consistency::Eventual { ttl_ms: 30_000 });
// Each analyzer is a backend; plant has read-only access.
for analyzer_id in 0..50 {
    let backend = Arc::new(FsBackend::new(
        format!("anlz-{analyzer_id}"),
        format!("/mnt/factory/anlz-{analyzer_id}/")
    )?);
    plant.register_backend(backend)?;
}
let targets: Vec<_> = (0..50)
    .map(|i| (format!("anlz-{i}"), "signatures".to_string()))
    .collect();
let target_refs: Vec<(&str, &str)> = targets.iter()
    .map(|(b, c)| (b.as_str(), c.as_str()))
    .collect();
let hits = plant.search_federated(&target_refs, &query_signature, 10)?;
```

The federated path runs in parallel via rayon (`crates/core/src/lake.rs:528`)
with the adaptive per-shard rerank
(`crates/core/src/lake.rs:474`, `MIN_PER_SHARD_RERANK = 5`) keeping recall
above 0.85 even at K=50 shards. The over-request formula
`k' = k + ceil(sqrt(k * ln(S)))` (`crates/core/src/lake.rs:553-560`) compensates
for skewed distributions where one analyzer holds disproportionately
more matches.

### Witness-anchored content-addressed sharing

A subtle point that pays off in IIoT: if two analyzers happen to
have the **same** library of healthy signatures (because they're
the same model machine, vibrating the same way), their bundles
produce identical witnesses (data_ref, dim, seed, rerank, generation
all match), and ruLake's content-addressed cache **shares the
compressed entry between them**
(`crates/core/src/cache.rs:378-383`, the witness-already-cached fast path;
`README.md` cell 11 advertises this as "Cross-process cache
sharing").

In a 50-analyzer fleet where 30 are the same model, the plant
server caches 21 distinct signature catalogs instead of 50 — a
~40% memory saving with no extra code. The `cache_refcount_of`
diagnostic (`crates/core/src/lake.rs:147`) lets operators see the share factor
explicitly.

### Bundle protocol over MQTT — concrete shape

A realistic publisher / subscriber loop:

```rust
// Publisher (analyzer side)
let bundle_path = analyzer_lake.publish_bundle(&key, "/var/anlz/sidecar/")?;
let body = std::fs::read_to_string(&bundle_path)?;
mqtt.publish("factory/line3/anlz-7/witness", &body, QoS::AtLeastOnce)?;

// Subscriber (plant side)
mqtt.on_message("factory/line3/anlz-+/witness", |topic, payload| {
    let analyzer_id = parse_analyzer_id(topic);
    let key = (format!("anlz-{analyzer_id}"), "signatures".to_string());
    // Drop the bundle into the local mirror so refresh sees it.
    std::fs::write(
        format!("/mnt/factory/anlz-{analyzer_id}/sidecar/table.rulake.json"),
        payload
    )?;
    match plant.refresh_from_bundle_dir(&key, &mirror_path)? {
        RefreshResult::Invalidated => {
            metrics.bundle_rotations.with_label("anlz", &analyzer_id.to_string()).inc();
        }
        _ => {}
    }
    Ok(())
});
```

The 10-line sidecar daemon pattern documented in `README.md` cell
"Cache-sidecar daemon" plus `examples/sidecar_daemon.rs` is exactly
this shape.

### What you give up on the factory floor

- **Sub-millisecond determinism** — already discussed in §4. A PLC
  inner control loop should not call ruLake.
- **Hot updates without re-prime.** A signature catalog rotation
  invalidates the cache; the next query primes (`O(n × D log D)`).
  At the typical analyzer scale of n=5000 vectors at D=128, prime
  is ~20 ms (`BENCHMARK.md` "Cold-start prime time"). Below the
  noise floor for vibration analysis. Worse for high-D image-based
  inspection — n=10k at D=2048 is ~700 ms, audible.
- **Wire encryption.** ruLake does not encrypt bundles or `ruvec1`
  files at rest or on the wire. OT networks need TLS at the MQTT
  layer (which they already use) and disk-level encryption (LUKS
  or platform equivalent) for the FsBackend root.

---

## 6. Vertical 5 — Edge inference cache for similarity search

A camera at the edge — an entryway, a parking-lot meter, a quality
inspection station — runs a perception model and produces an
embedding per frame. The application question is: "is this similar
to anything in my known-good / known-bad catalog?"

This is the textbook ruLake workload at the edge. The catalog is
small (hundreds to tens of thousands of vectors), the query rate is
moderate (10–60 Hz), the freshness window is hours-to-days (we update
the "known faces" list when HR adds an employee, not when the
camera sees one), and survival across power cycles is critical.

### Sizing

A typical entryway-camera deployment:

- Catalog: 500 employee face embeddings at D=512
  (FaceNet-class).
- Cache footprint: 500 × (512/8 + 8) = 500 × 72 = 36 KB.
- Query rate: 30 Hz when motion detected, idle otherwise.

This entire workload comfortably fits in L1 cache — the index is
~32 KB, smaller than a typical CPU L1d. Per-query latency at this
scale is bounded by the rotation step (D log D for Hadamard at
D=512, that's ~4500 flops per query) plus the 1-bit popcount scan
across 500 vectors plus the rerank — call it 50 microseconds on
modest edge silicon.

For a parking-lot LPR (license-plate recognition) deployment with
n=10,000 plates at D=256:

- Cache footprint: 10,000 × (256/8 + 8) = 10,000 × 40 = 400 KB.
- Per-query latency at ~3,000 QPS at n=10k D=128 native baseline,
  derated for D=256 (2× scan work) and edge silicon (~3× slower)
  = ~500 QPS on something like a Pi 4 / industrial Atom. Per-query
  latency ~2 ms.

Both fit inside any realistic frame budget, with massive headroom.

### TTL-based freshness against the cloud

The edge camera runs `Consistency::Eventual { ttl_ms: 3_600_000 }`
(1 hour). A central HR system republishes the bundle when the
employee list changes; the camera notices on the next refresh poll.
Stale results (an ex-employee still flagged as "known-good") for
up to one hour is the explicit security/operations tradeoff.

For higher-stakes deployments (banking, prison entry control), drop
the TTL to 60 seconds and accept the proportional increase in
backend load. For lower-stakes (warehouse pallet identification),
24-hour TTL is fine and the cloud-side bundle rebuild cost
amortizes to near-zero.

### `FsBackend` snapshots survive power cycle

Edge cameras lose power. Industrial buildings brown out. The
camera reboots, and the application wants to be back in service in
seconds, not minutes.

`FsBackend` writes via temp+fsync+rename (`crates/core/src/fs_backend.rs:166-202`),
so a power loss during a write either leaves the previous version
intact or leaves a `.tmp` file that is ignored on next read. The
on-disk format (`ruvec1`, magic-byte-checked at
`crates/core/src/fs_backend.rs:247-252`) is fixed-stride and bounds-checked
before allocation (`crates/core/src/fs_backend.rs:259-281`), so a corrupt
truncation on power loss surfaces as `InvalidParameter` rather
than a parser crash.

For warm-restart, the `save_cache_to_dir` / `warm_from_dir` pair
gives a tighter loop:

- Boot.
- `RuLake::new(20, 42).with_consistency(Frozen)`.
- `lake.warm_from_dir(&key, "/var/cam/snapshot/")?` — 5 ms at
  n=5000.
- Camera back in service.

The Frozen mode here means "we trust the snapshot we wrote at
shutdown and we will not re-check the cloud until something explicit
changes." For a camera that boots and immediately needs to identify
the next person walking through, that 5 ms warm-load is the
load-bearing primitive.

### Where the edge-inference model itself lives

ruLake does not host the inference model. The camera runs a
quantized FaceNet via ONNX Runtime (or a similar lightweight runtime)
and produces an f32 embedding per detected face. ruLake takes that
embedding and does the nearest-neighbour lookup. The split is
clean — and important: the inference model is what eats the
gigabyte of flash and the watt of power on the camera. The ruLake
cache is the small, fast, eventually-coherent layer on top that
makes the catalog lookup not an embarrassment.

---

## 7. Vertical 6 — Federated edge with cloud truth

The general pattern under the IIoT and edge-camera verticals
generalizes: **cloud is the durable backend, each edge runs ruLake
against a local FsBackend that mirrors a relevant slice.** This
section makes the architecture explicit because it shows up
repeatedly across the edge verticals.

### The architecture

```
                ┌──────────────────────────────────────┐
                │             Cloud (RVF / S3 / GCS)    │
                │                                       │
                │   bundles/   ← truth (witness-signed)│
                │   shards/    ← ruvec1 binaries       │
                └──────────────┬───────────────────────┘
                               │
                  bundle (~300 B) over the slow link
                               │
        ┌──────────────────────┼──────────────────────┐
        │                      │                      │
        ▼                      ▼                      ▼
   ┌─────────┐            ┌─────────┐            ┌─────────┐
   │ edge 1  │            │ edge 2  │            │ edge N  │
   │         │            │         │            │         │
   │ ruLake  │            │ ruLake  │            │ ruLake  │
   │ FsBack  │            │ FsBack  │            │ FsBack  │
   │ /var/.. │            │ /var/.. │            │ /var/.. │
   └─────────┘            └─────────┘            └─────────┘
```

The cloud writes new bundles. Each edge polls (or receives push
notification of) bundle rotation. When the local witness disagrees
with the cloud's witness, the edge pulls the new `ruvec1` binary,
drops it into its FsBackend root, and the next ruLake query primes
against it.

This pattern decouples three things that traditional
hub-and-spoke architectures conflate:

- **Truth ownership** — cloud.
- **Latency-bounded serving** — edge.
- **Coherence enforcement** — bundle witness + ruLake's `Eventual`
  TTL.

### SPIRE-pattern tier separation

ADR-155 §"Persistence" (capability #16) calls out the
"SPIRE-pattern" framing: "Stateless compute + SSD-resident state —
compute tier can restart without pull". On the edge, this maps to:

- The ruLake process is stateless beyond its in-memory cache.
- The state lives on local SSD as `ruvec1` files plus
  `table.rulake.json` sidecars.
- Restart: `warm_from_dir` skips the backend RTT entirely
  (cloud-side may not even be reachable at the moment) and serves
  from the SSD-resident state.

A cellular IoT device that has not had cloud connectivity for 6 hours
still serves queries against the last-known-good catalog, with a
clear audit trail (the witness in the cache stats matches the
bundle that was on disk at boot).

### Bandwidth budget

A bundle rotation on the cloud side propagates to N edges. For
each edge, the steady state is:

- **Sidecar download**: ~300 bytes per rotation event. At one
  rotation per hour and 10,000 edges, that is 8.6 MB/day across the
  fleet — irrelevant.
- **Binary download** when invalidated: full `ruvec1` file size,
  which is `n × (D × 4 + 8)` bytes plus a 24-byte header
  (`crates/core/src/fs_backend.rs:14-22`). At n=10k D=128 that is ~5.3 MB per
  edge per rotation. At n=100k D=768 it is ~310 MB — significant
  on a cellular link, fine on a wired LAN.

For very large catalogs on bandwidth-constrained edges, the right
extension is **delta sync** at the binary level — out of scope for
ruLake today (the bundle is whole-collection-atomic), but a natural
M2+ enhancement.

### Frozen mode for offline operation

If the edge is fully disconnected — a research vessel between
Wi-Fi windows, a remote agricultural sensor — switch to
`Consistency::Frozen` after the last successful sync:

```rust
// During sync window:
lake.refresh_from_bundle_dir(&key, "/mnt/sync/bundles/")?;
// Going offline; pin the cache:
let lake = lake.with_consistency(Consistency::Frozen);
// All subsequent queries serve from the pinned witness, no backend
// RTT, no automatic invalidation.
```

The pinned witness is recoverable for audit, and on the next sync
window the system can compare the pinned witness against the cloud's
current witness to compute "how stale was each query during the
offline window."

---

## 8. Vertical 7 — Mobile agent (offline-first)

A mobile personal AI assistant — the specific shape that 2025–2026
on-device LLM products are converging on — combines several
embedding stores:

- **Episodic memory.** Embeddings of the user's past conversations,
  app interactions, location traces. High-cardinality, append-mostly,
  privacy-sensitive.
- **Contact embeddings.** "Who is this person?" lookups. Low
  cardinality, low-update-frequency, high-importance.
- **App-action embeddings.** "What in-app action does the user
  want?" — embeddings of available app intents, used for routing
  natural-language commands to the right system.
- **Document embeddings.** PDFs, photos with OCR text, voice
  transcripts. Mid-cardinality, append-mostly.

ruLake's `(backend, collection)` namespace fits this pattern with no
extension: each store is a collection, all live in a single
`FsBackend` rooted at the app's sandbox.

### Memory-class tags

`RuLakeBundle.memory_class: Option<String>` (`crates/core/src/bundle.rs:144-155`)
is the substrate hook for exactly this pattern. The agent layer
tags each bundle with `"episodic"`, `"semantic"`, `"procedural"`,
`"identity"` — opaque to ruLake, meaningful to the agent. The tag
does **not** affect the witness (`bundle.rs:152` doc:
"Two bundles with identical data but different classes share the
cache"), so the same vectors retagged for a different cognitive
purpose still share their compressed entry — a memory-saving
property that matters on a phone.

### Cloud sync via bundle rotation when on Wi-Fi

The phone is the hot edge; a desktop or cloud agent is the warm
synchronizer. The pattern:

- Phone runs ruLake against its local FsBackend.
- Phone is offline most of the time (user is moving, on cellular,
  battery-conscious).
- When on Wi-Fi and charging, a background sync uploads the
  episodic embeddings to the cloud, downloads any updates from
  other devices, and rotates the local bundles.

`Consistency::Eventual { ttl_ms: 5 * 60_000 }` (5 minutes) is
roughly the right knob for a phone — long enough to amortize the
Wi-Fi sync, short enough that a same-device update is reflected
quickly.

### Privacy properties

ruLake on the phone is a **read-side** substrate. The actual
write path — turning text into embeddings — happens above ruLake,
in the agent layer. ruLake's bundle witness is computed over the
embeddings (`data_ref`, `dim`, `rotation_seed`, `rerank`,
`generation`) and **does not include the embedding contents
themselves** beyond the data_ref pointer.

For a phone deployment, two implications:

1. The witness is not a privacy leak — knowing the witness does not
   reveal embedding contents. It reveals which `data_ref` (typically
   a local file path or content hash) was used, and which generation.
2. The bundle's `pii_policy` field (`crates/core/src/bundle.rs:138`) is
   passthrough opaque on the substrate side. The agent layer
   populates it with whatever PII classification the application
   demands; ruLake does not interpret or enforce it. M4 in the
   ADR-155 roadmap is when enforcement shows up — for v1, the field
   is documentation, not gate.

For an on-device agent that wants strong privacy guarantees, the
relevant property is that **all embedding data lives on-device and
the cloud sync is opt-in**. ruLake's substrate makes that possible —
nothing in the crate phones home — but the privacy story is owned
by the application above, not the substrate below.

### Sizing for a 1-year personal assistant

A heavy user generating 100 turns/day across all chat surfaces, with
3 chunks of context per turn, is 110k vectors/year. At D=768 (the
typical on-device embedding model in early 2026), that is:

- Cache footprint: 110k × (768/8 + 8) = 110k × 104 = 11.5 MB.
- One year of episodic memory in 11.5 MB. The phone barely notices.

Add the static stores (50k document embeddings, 5k contact
embeddings, 500 app-action embeddings) and the total agent state on
the phone is comfortably under 50 MB even for a power user.

### Thrash-avoidance via LRU cap

If the agent has many active sessions and the cache grows beyond
budget, `with_max_cache_entries(n)` (`crates/core/src/lake.rs:78`) caps the
distinct compressed entries; LRU evicts the least-recently-used
**unpinned** entry (`crates/core/src/cache.rs:548-565`). Two notes for mobile:

- "Unpinned" means refcount-zero. As long as there is an active
  pointer for a `(backend, collection)`, that entry is never evicted
  even if the cap is reached (`docs/review/capabilities.md` G5 calls
  this out as a "soft cap"). For mobile, this is the right
  behavior: the agent's "current session" pointer keeps the active
  entry pinned regardless of memory pressure.
- The LRU eviction is `O(entries)` per eviction
  (`docs/review/performance.md` B6). At small caps (≤ 100 entries),
  this is fine. At the personal-agent scale of 4–8 active
  collections, it is invisible.

---

## 9. Vertical 8 — Embedded Linux: Pi 5 / Jetson class

The Raspberry Pi 5 (8 GB), Jetson Orin Nano (8 GB), and similar
embedded Linux boxes are the comfortable middle ground for ruLake.
They run Linux, they have several GB of RAM, they have an SSD or
high-quality SD card, and they have CPU instruction sets ruLake's
kernel can use (NEON via the rabitq portable path; AVX2 if x86_64
embedded like an Atom).

### Concrete sizing for Pi 5 (8 GB)

- Total RAM: 8 GB. Allocate 1 GB to the ruLake cache; leave the
  rest for the OS, the application, and the inference model.
- 1 GB cache = 1,073,741,824 bytes / (D/8 + 8) per vector.

**Table 9.1 — Vectors at 1 GB cache budget on Pi 5**

| D     | Vectors per GB |
|------:|---------------:|
|   128 |     44.7 M     |
|   384 |     17.9 M     |
|   768 |     10.4 M     |
|  1024 |      7.81 M    |
|  1536 |      5.40 M    |
|  2048 |      4.04 M    |

**44 million 128-dim vectors fit in 1 GB on a Pi 5.** That is a
genuinely large corpus for an edge box — every Wikipedia summary
embedding, every product in a mid-size catalog, every line of code
in a moderate enterprise's repository.

### Latency budget on Pi 5

The Pi 5 has 4× Cortex-A76 cores at 2.4 GHz. Per-core integer
popcount throughput is roughly 1/3 to 1/4 of a Ryzen-laptop core,
and there is no AVX. NEON SIMD is available but not currently used
by `ruvector-rabitq`'s scan path (the AVX2 / AVX-512 paths are the
only SIMD scaffolds; the scalar path is what runs on ARM).

Calibrating against `BENCHMARK.md`:

- Native Ryzen single-thread, n=100k D=128: 3,681 QPS.
- Pi 5 single-thread, n=100k D=128, scalar popcount: ~1,000 QPS
  (3.5× derate).
- Pi 5 single-thread, n=1M D=128, scalar popcount: ~100 QPS.
- Pi 5 single-thread, n=100k D=768 (6× scan work): ~170 QPS.

All four numbers are interactive for a single-user agent. For a
multi-user system on a Pi 5 (e.g. a small office assistant), the
4-core concurrent path (`BENCHMARK.md` reports 13.2× concurrent
QPS at 4 shards on Ryzen) extrapolates to ~4× on the Pi's 4 cores,
giving ~4,000 QPS at n=100k D=128 — comfortable for a 5-user
shared instance.

### SD card vs SSD for the FsBackend

The Pi 5 has an NVMe SSD HAT option but ships defaults to SD card.
For ruLake's `save_cache_to_dir` / `warm_from_dir` loop:

- Warm-load from SD card: I/O-bound at SD card sequential read
  speeds (~100 MB/s for a quality card). A 100 MB compressed
  cache loads in 1 second.
- Warm-load from NVMe: I/O-bound at NVMe sequential read speeds
  (~3 GB/s on modern controllers). The same 100 MB cache loads in
  ~30 ms.
- Steady state is RAM, so storage speed only matters at boot.

For an edge box that boots once a week, the SD card is fine. For
an edge box that reboots hourly (some industrial controllers),
NVMe pays off.

### NEON SIMD — the missing optimization

The single biggest unlocked performance for ARM-Linux edge is a
NEON popcount scan in `ruvector-rabitq`. ARMv8 has `vcnt`
(byte-level popcount) and `addv` (horizontal sum) — together they
implement a 16-byte-at-a-time popcount that should match the
expected ~2× speedup the AVX2 path delivers on x86. This is
out-of-scope for ruLake itself (it lives in the kernel crate per
ADR-157) but it is the missing piece for ARM edge.

### Jetson Orin Nano

The Orin Nano adds a 1024-core CUDA GPU on top of an ARM
Cortex-A78AE 6-core CPU. ADR-157 already scaffolds the GPU
dispatch path:

- `VectorKernel` trait in `ruvector-rabitq` (per ADR-157), with
  `KernelCaps { min_batch, max_dim, deterministic, accelerator }`.
- A `ruvector-rabitq-cuda` crate could ship a CUDA scan kernel for
  the Orin's GPU.

For a Jetson workload, the GPU is interesting at **batch sizes
≥ 64 queries** (the typical CPU/GPU crossover documented in ADR-157
§"Trait shape (normative)"). Per-frame retrieval at 30 Hz with one
query per frame is **CPU-friendly**, not GPU-friendly. Batch
retrieval in a planner step (e.g. evaluating 200 candidate actions
against a catalog) is GPU-friendly.

The honest take: **on a Jetson Orin, the CPU path is sufficient
for most edge-perception workloads, and the GPU path is interesting
only for batched planner-style queries.** The substrate ships the
trait scaffold; the kernel crate is the work that hasn't happened.

---

## 10. Vertical 9 — Microcontroller adjacency (ESP32-class and below)

ruLake **will not run on a Cortex-M class microcontroller**. Three
hard reasons:

1. **`std`-only.** The crate uses `std::sync::Mutex`, `std::fs`,
   `std::collections::HashMap`, and `Box<dyn Trait>` extensively.
   No `no_std` mode is documented or feasible without a major
   refactor.
2. **`alloc`-heavy.** Every prime allocates an `Arc<RabitqPlusIndex>`
   and a `Vec<Vec<f32>>` for the pulled batch (`crates/core/src/cache.rs:399`).
   Cortex-M parts have 64–512 KB of RAM total; allocating per-query
   is not viable.
3. **rayon.** The federated path uses `rayon::par_iter`
   (`crates/core/src/lake.rs:528`), which requires real OS threads. Cortex-M
   typically has zero or one thread of execution under an RTOS.

So the question is not "can ruLake run on an ESP32?" — it cannot —
but **"where does ruLake fit in an architecture that includes
ESP32-class sensors?"**

### The gateway pattern

A typical industrial-IoT mesh has dozens to hundreds of MCU sensors
(ESP32-class — 240 MHz Xtensa, 520 KB RAM, Wi-Fi, CAN/Modbus/etc.)
reporting to one or a few gateway boxes (Pi-class ARM Linux). The
sensors are dumb: they sample, they preprocess, they ship raw or
lightly-summarized data over MQTT or LoRaWAN.

ruLake lives on the gateway. The sensors produce:

- Raw sensor readings (vibration spectra, current draw,
  temperature) — already too narrow to be a vector.
- Or, if the sensor has a tiny embedding model (e.g. a TinyML
  microcontroller-class model producing an 8–32-dim embedding per
  reading), a low-dim embedding stream.

The gateway:

- Aggregates embeddings from N sensors.
- Optionally re-embeds via a higher-quality model (if the gateway
  is Jetson-class).
- Stores the catalog in a ruLake instance with one collection per
  sensor, or one collection per equipment group.
- Serves "is this similar to anything we've seen before?" queries
  to the SCADA / MES system.

### What the MCU side does

The MCU runs a TinyML embedding model in a few hundred KB of flash.
The model produces an 8–32-dim feature per reading. The MCU sends
the feature plus a timestamp over MQTT to the gateway. The MCU
itself does no vector search.

For this pattern, ruLake on the gateway is the natural substrate: it
ingests the MCU stream into a collection, builds the compressed
catalog, and serves nearest-neighbour queries from any client that
can reach the gateway over IP.

### What the MCU side cannot do

- Run nearest-neighbour search against more than ~100 vectors
  in-place (and that only with a custom scalar implementation, not
  ruLake).
- Store a witness chain in flash and serve it back to a verifier —
  the SHA3 / SHAKE-256 implementation fits in flash (~5 KB) but
  the bundle JSON parser does not.
- Participate in cache-coherent federation — the substrate is too
  heavy.

### What is feasible in adjacent ecosystems

For genuinely on-MCU vector search, the relevant projects are:

- `arduino_LearningKit_Sample` and similar — small fixed-size
  catalogs (≤ 100 vectors) with float-bit comparison, no
  compression.
- TensorFlow Lite Micro with embedding-table lookups — works for
  dictionary-style retrieval at very small scale.
- A custom hand-rolled 1-bit Hamming distance over a static table
  in flash — the most ruLake-aligned approach, but bespoke.

The cleanest architectural separation is to treat ruLake as the
**gateway-side substrate** and let the MCU mesh do raw data
acquisition. A Cortex-M MCU sensor that produces an embedding stream
becomes a write-side data producer; the gateway-side ruLake is the
read-side substrate. Trying to push ruLake further down the stack
than that is fighting the substrate's basic assumptions.

---

## 11. Vertical 10 — In-vehicle infotainment and cockpit AI

Automotive cockpit AI is the edge vertical with the most
constrained power, thermal, and certification requirements, but
also the most stable hardware target — a vehicle ships with a
known SoC and known firmware for its 10–15-year service life.

### The deployment shape

A modern vehicle has multiple displays (cluster, head unit, rear
seats, head-up display), each rendering personalized content based
on the current driver and passenger profiles. The AI workload
includes:

- Voice command routing — "play the next song" needs to find the
  right intent embedding.
- Personalization — recently played media, navigation favorites,
  climate preferences, all retrieved by similarity to the current
  context.
- Diagnostic recall — when something goes wrong, replay the
  vehicle's recent sensor state for technician analysis.

ruLake fits all three with the consistency-mode toolkit:

- **Voice command routing** — a small static catalog of intents
  (~500 vectors at D=384). `Consistency::Frozen` after warm-up.
  Per-query latency ~50 µs even on automotive-grade silicon.
- **Personalization** — per-profile embeddings of preferences and
  history. `Consistency::Eventual { ttl_ms: 60_000 }` against a
  local profile store that the OS updates on profile change.
- **Diagnostic recall** — long-window storage of sensor embeddings
  with `Frozen` snapshots taken at fault events for technician
  replay.

### Coherent personalization across screens

The vehicle has 4 screens. Each runs a UI process. All four UI
processes need to see the same personalization vectors —
"the current driver likes hip-hop and dislikes jazz, derived from
embedding the listening history."

ruLake's content-addressed cache sharing (`README.md` cell 11) is
the right primitive: each UI process has its own ruLake instance,
all four point at the same FsBackend root, all four compute the
same witness from the same bundle, and the OS-level shared memory
or a single file-backed mmap shares the underlying compressed bytes.

In a single-process design (the four UIs are threads in one process),
this is just the regular ruLake hot path with one cache. In a
multi-process design (the four UIs are separate processes for fault
isolation), the witness-anchored sharing means the disk cache hit
is at the OS page-cache level; ruLake itself doesn't share state
across processes (each process has its own `Arc<RabitqPlusIndex>`)
but the underlying file is mmapped once by the OS.

### Frozen mode for diagnostic recreation

When the vehicle logs a fault, the diagnostic system captures:

- The current cache witness (one 64-char hex string).
- The recent query history (timestamps, query embeddings, top-k
  results).
- The current `ruvec1` files at the FsBackend root (atomic
  snapshot via fs hardlinks).

In the dealer's service bay six weeks later, the technician
loads the snapshot directory, runs ruLake in `Frozen` mode against
that snapshot, replays the queries, and gets **byte-exact** results
— because the `Frozen` + `warm_from_dir` path skips any backend
RTT and reproduces the exact compressed cache state
(`crates/core/src/lake.rs:378`, doc: "byte-exact query results without backend
RTT").

This is the property automotive functional-safety engineers actually
care about: the ability to reproduce a fault not "approximately"
but "exactly", with cryptographic provenance via the witness.

### Power and thermal

A typical automotive cockpit SoC (Snapdragon 8295, Renesas R-Car
V4H, or similar) has 4–8 ARM cores, modest GPU, and a power
envelope of 5–15 W for the entire infotainment stack. ruLake's
hot-path cost — one Arc clone, one RaBitQ scan — is in the
millions of ops per second range, well under 1 W of CPU draw at
the typical query rates (10–100 Hz).

In low-power steady state (parked, screen dimmed), the application
typically suspends queries entirely. ruLake's cache sits idle in
memory; no background work, no polling. Wake-on-touch resumes; the
warm cache serves the first query at full speed without re-prime.

### Where automotive certification gets hard

Functional safety standards (ISO 26262, ASPICE) require:

- WCET analysis for all software in the safety-critical path.
- Verification that allocation behaviour is statically bounded.
- Code coverage and toolchain qualification.

ruLake **does not have ISO 26262 certification** and the use of
`std::sync::Mutex` makes WCET analysis difficult (the lock is
mutually-exclusive but the critical section is not statically
bounded against scheduler behaviour). For QM (quality-managed)
code paths in the infotainment stack — voice routing,
personalization, diagnostic — ruLake is in scope. For ASIL-rated
safety paths (e.g. driver-attention monitoring with a safety
fallback), it is not.

The realistic positioning: ruLake handles the QM cockpit-AI
workload. The safety system has its own dedicated path that does
not call into ruLake. The diagnostic recall system uses ruLake's
Frozen + warm-restart pair to give technicians forensic-grade
replay.

---

## 12. Vertical 11 — Drones and UAVs

Drones combine the constraints of mobile, robotic, and
intermittent-connectivity verticals. A typical mid-size commercial
drone (DJI Matrice-class or military equivalent) has:

- An onboard SoC (Snapdragon Flight, Ambarella, or NXP) with a few
  GB of RAM.
- Limited link budget — typically 1–10 Mbps to ground at short
  range, dropping to kbps at extended range.
- Mission durations of 20 minutes to several hours.
- Hard return-to-base time bounds — battery is the deadline.

### Mission-window Frozen consistency

A typical inspection mission has a pre-flight planning phase, a
flight phase, and a post-flight analysis phase. ruLake's
consistency modes map cleanly:

- **Pre-flight**: `Eventual` against the cloud's latest mission
  catalog (target list, known-obstacle list, known-friend list).
  Warm the cache, persist via `save_cache_to_dir`, take off.
- **Flight**: switch to `Frozen` once the link to ground may go
  intermittent. The witness pins the catalog cryptographically;
  no automatic invalidation can happen mid-mission. The drone's
  perception loop queries against this pinned catalog at perception
  rates (10–60 Hz).
- **Post-flight**: warm-load the recorded snapshot for analysis. The
  witness recorded in flight matches the witness on the analyst's
  desk; replay is byte-exact.

```rust
// Pre-flight (on the ground, with ground-link)
let lake = RuLake::new(20, 42)
    .with_consistency(Consistency::Eventual { ttl_ms: 1000 });
let cloud = Arc::new(FsBackend::new("mission", "/mnt/ground/mission/")?);
lake.register_backend(cloud)?;
lake.search_one("mission", "targets", &probe, 1)?;  // prime
lake.save_cache_to_dir(&("mission".into(), "targets".into()),
                       "/var/drone/snapshot/")?;

// In flight (ground-link unreliable)
let mission_lake = RuLake::new(20, 42)
    .with_consistency(Consistency::Frozen);
mission_lake.warm_from_dir(&("mission".into(), "targets".into()),
                          "/var/drone/snapshot/")?;
// Perception loop runs against mission_lake; never touches the cloud.

// Post-flight
let analyst_lake = RuLake::new(20, 42)
    .with_consistency(Consistency::Frozen);
analyst_lake.warm_from_dir(&recorded_key, "/recordings/flight-3247/snapshot/")?;
// Recorded queries replay byte-exactly.
```

### Bandwidth-budget aware bundle rotation

When the link to ground does come back briefly, the drone must
decide whether to spend bytes refreshing the bundle. The bundle
sidecar is ~300 bytes; the full `ruvec1` binary is `n × (D × 4 +
8)` bytes. A rotation that requires the binary to refresh might
be 10 MB; the bundle alone is trivial.

A pragmatic policy:

- Always download the bundle on link-up.
- If the witness changed, decide based on link-quality estimate
  whether to download the binary now or defer to next link
  opportunity.
- During the deferred period, log every query as "served against
  pre-rotation witness" so the post-flight analyst can identify
  which queries used stale data.

ruLake's `cache_witness_of` (`crates/core/src/lake.rs:134`) gives the witness
that resolved each query for free; no extra logging hook needed.

### Per-mission warm-restart

Drones go down and come back. Failures are routine — battery
swap, software watchdog, temporary GPS loss. Every restart should
be back in service in under a second.

`warm_from_dir` at n=5000 D=128 takes ~5 ms per ADR-155 §"Status".
At n=10000 D=512 (a realistic catalog of known objects for an
inspection drone), the warm-load is ~50 ms (linear in `n × D`
for the deserialize cost). Even at n=100k D=512 it is ~500 ms —
still under a second.

The drone OS:

- Detects watchdog reset.
- Spawns the perception process.
- Perception process calls `warm_from_dir` for each mission
  collection (typically 2–4 collections).
- Total warm-up: ~50–200 ms.
- Drone is back in mission state within the watchdog grace period.

### What drones do not get

- **Sub-millisecond control loops.** As repeatedly noted, ruLake
  is not the substrate for the inner attitude controller. That is
  a hard real-time loop running on dedicated RTOS-level
  infrastructure.
- **Cross-drone federation in flight.** Two drones in formation
  cannot easily share their caches over the air during the mission
  — the bandwidth and latency budget make it impractical, and
  ruLake has no peer-to-peer sync primitive (it is read-only on
  the consumer side; writes happen at ingest, not at query). For
  cross-drone shared situational awareness, the right pattern is
  centralized: one ground station holds the current state, both
  drones read from it.

---

## 13. Power and thermal envelope

The hot path through ruLake — `RuLake::search_one` against a
warm `Eventual`-cached entry — is documented at
`docs/review/performance.md` §1 as:

- 3 mutex acquisitions (`can_skip_check_interned`, `mark_hit`,
  `search_cached_with_rerank_interned`).
- 1 `Arc::clone` per Arc (cache index + pos_to_id, 2 total).
- 1 `clock_gettime` syscall (touching `last_used`).
- 1 unlocked RaBitQ scan.

The dominant cost is the scan: `O(n × D / 64)` integer popcount
plus `O(rerank × k × D)` exact L2² rerank.

### Translating to mW / mJ

Edge silicon power numbers are dominated by core type and clock,
not by instruction mix at this scale. Order-of-magnitude estimates
for a single per-query cost on common edge silicon, at n=10k
D=128:

| Silicon class            | Per-query CPU cycles | Per-query latency | Avg power during query | Energy per query |
|--------------------------|---------------------:|------------------:|-----------------------:|-----------------:|
| Pi 5 (A76 @ 2.4 GHz)     | ~2.4 M               | ~1 ms             | ~3 W                   | ~3 mJ            |
| Snapdragon 8 Gen 4 (1 P-core) | ~2.0 M           | ~0.6 ms           | ~5 W                   | ~3 mJ            |
| Apple A18 (1 P-core)     | ~1.8 M               | ~0.5 ms           | ~5 W                   | ~2.5 mJ          |
| Cortex-A53 (Pi 4 @ 1.5 GHz) | ~3.6 M           | ~2.4 ms           | ~2 W                   | ~5 mJ            |
| Snapdragon 8295 (auto)   | ~2.0 M               | ~0.8 ms           | ~6 W                   | ~5 mJ            |
| Jetson Orin Nano CPU     | ~2.2 M               | ~1 ms             | ~5 W                   | ~5 mJ            |

(Cycles estimated from `BENCHMARK.md`'s 3,681 QPS × 2.4 GHz
on the Ryzen baseline, scaled by IPC ratios; energy = power ×
latency.)

For a query rate of 10 Hz (a typical edge perception loop), the
sustained power draw from ruLake is well under 100 mW even on
modest silicon — a small fraction of the inference model's
typical 1–3 W draw. **ruLake is not the bottleneck on edge
power budgets.** The model and the radio are.

### Low-power steady state

When no queries are in flight, ruLake's CPU cost is zero — there
is no background thread. Memory cost is the cache's RAM footprint,
which the CPU's DRAM controller has to refresh. At 1 GB cache,
that is ~50 mW of refresh power on typical mobile DDR. The
substrate adds nothing to idle power beyond the memory it
occupies.

### Coherence-poll cost

The `Eventual` mode's TTL is the right knob for trading freshness
against power. At 60-second TTL, the cost of polling the backend
for the current witness is amortized to <1% of query cost on any
realistic query rate. At 1-second TTL on a sub-second query rate,
the witness check cost dominates — and that is the regime where
`current_bundle` overrides become critical (`docs/review/security.md`
M2, the "self-inflicted DoS" footgun).

### Thermal

For battery-powered devices in thermal-limited form factors
(phones, drones, smartwatches), sustained ruLake load at the rates
above is **invisible in the thermal budget**. The thermal headroom
is consumed by the model and the display, not by the substrate.

---

## 14. Reality check — where ruLake will not work today

For anyone evaluating ruLake on the edge, here is the honest list
of what does not work, what cannot be made to work, and what
might work with extension.

### Will not work, no extension can help

- **Microcontrollers without an OS.** Cortex-M class. No `std`,
  no `alloc` budget, no rayon. Use a custom 1-bit Hamming kernel
  or the TFLite Micro embedding-table approach.
- **Hard real-time systems with sub-millisecond deadlines.** The
  `std::sync::Mutex` precludes WCET guarantees. Use a static index
  with no shared state.
- **Cryptographic forget / GDPR shred at the substrate level.**
  The cache can drop pointers (`invalidate_cache`,
  `crates/core/src/lake.rs:154`), but the underlying bytes (in the FsBackend
  or in another process's cache) are not crypto-shredded. That is
  the M2+ RVF responsibility.
- **End-to-end network encryption.** ruLake does not encrypt
  bundles or `ruvec1` files. Wire encryption is the transport
  layer's job (TLS over MQTT, etc.); at-rest encryption is the
  filesystem layer's job (LUKS, dm-crypt).

### Will not work today, extension feasible

- **WASM in the browser.** Code is `no_unsafe`, deps are mostly
  portable. Need: `WasmBackend` impl, `#[cfg]` gates around rayon,
  IndexedDB persistence layer. Work: ~1–2 weeks for a competent
  Rust-WASM developer. Not a research problem.
- **NEON SIMD on ARM.** Currently only AVX2 / AVX-512 SIMD paths
  exist in `ruvector-rabitq`. NEON popcount is well-understood;
  adding it would close the 2× scan-speed gap on Pi-class hardware.
  Work: ~1 week in the kernel crate, behind a feature flag.
- **mmap'd persistence.** Currently `warm_from_dir` deserializes
  the full `ruvec1` into RAM. With `memmap2`, the index could be
  served directly from the file's mapped pages, dropping warm-load
  to ~1 ms regardless of n (only the metadata pages need to be
  read). Listed as M2+ in `README.md` "M2+ roadmap". Work:
  significant — requires a new `ruvec1` variant that is
  memory-layout-stable.
- **Android NDK build.** No platform-specific blockers; the crate
  is pure Rust 1.89+. The work is packaging (cross-compile, JNI
  binding for the Android-side caller, AAR distribution). Not
  research.

### Will work with caveats

- **Embedded Linux, Pi-class, Jetson-class.** Works today. The
  unlocked NEON SIMD is the only meaningful perf gap.
- **Phones (iOS, Android).** Works today via cross-compile. The
  storage layer needs platform-appropriate paths (sandbox
  directories on iOS, `getFilesDir()` on Android), but the
  substrate is fine.
- **In-vehicle infotainment.** Works today on QM-rated paths.
  Not for ASIL-rated safety paths without a separate certification
  story.
- **Industrial gateways.** Works today, with the bundle protocol as
  the natural sync primitive over MQTT.

### Will not work — but is the wrong tool, not a missing feature

- **Vector store with mutation.** ruLake is read-side. Writes go
  through the backend (RVF, Parquet, FsBackend's `write` method).
  An "edit" workflow looks like: edit, write to backend, rotate
  the bundle, let ruLake invalidate and re-prime. There is no
  "update vector in place" API and there will not be one — that
  conflates ownership of the bytes with the cache layer.
- **Multi-writer mesh.** ruLake's writers are append-mostly via the
  backend. Multi-writer reconciliation is the storage layer's job
  (RVF, S3 versioning, etc.); ruLake reads whatever the bundle
  points at. CRDT-style multi-writer is interesting (see Open
  Questions §15) but explicitly not in scope today.
- **Per-vector ACLs.** The `pii_policy` field on the bundle is
  passthrough; enforcement is M4 governance. For per-vector
  access control today, use distinct collections and back them
  with distinct FsBackend roots that have OS-level ACLs.

---

## 15. Open questions and recommended extensions

These are the questions an edge-focused ruLake roadmap would need
to answer. Some are blocking specific verticals; some are
incremental quality-of-life improvements.

### 15.1 Should ruLake have a `no_std`-friendly subset?

The current crate uses `std` extensively. A `no_std + alloc`
subset that skipped the `BackendAdapter` (which inherently needs
I/O) and offered just the `VectorCache` against a caller-supplied
`PulledBatch` would unlock embedded deployments where the host
runtime owns the I/O (e.g. an RTOS-side application that pulls
from flash and primes ruLake's cache with the bytes).

The work:

- Replace `std::sync::Mutex` with `spin::Mutex` (or a feature flag
  to choose).
- Replace `std::collections::HashMap` with `hashbrown`.
- Strip `rayon` from the no_std build via cfg gates.
- Strip `std::time::Instant` (used for LRU `last_used` and
  `Eventual` TTL); replace with a caller-supplied "now" function.
- Strip `std::fs` from `bundle.rs` (the in-memory bundle parsing
  works; only the `read_from_dir` / `write_to_dir` use fs).

Rough effort: 2–4 weeks for a Rust developer comfortable with
`no_std`. The result is a substrate that runs on Cortex-A class
parts under an RTOS without dragging Linux in.

### 15.2 Should there be a WASM-SIMD path?

WASM SIMD128 is stable in major browsers as of 2025. A WASM build
of `ruvector-rabitq` with `+simd128` would close most of the
performance gap to native scalar (Table 3.1 row 2 vs row 3).
The work belongs in `ruvector-rabitq`, not ruLake; ruLake just
needs to compile on `wasm32-unknown-unknown`.

A natural milestone: ship a demo of browser-side ruLake with
~100k vectors at D=384 doing live semantic search at typing
speed. That is a believable proof point for the WASM story.

### 15.3 Should ruLake have a CRDT-style multi-writer mode for edge meshes?

The current architecture is single-writer-per-collection: the
backend owns the bytes, the bundle is rotated atomically, readers
invalidate on witness change. For an edge mesh where multiple
devices want to contribute new vectors and converge to a shared
state without a central authority, this falls down.

CRDT-style approaches (G-Set for append-only embeddings, with
witness chains as causal markers) would add convergence guarantees
without breaking the cache-first model. The bundle could carry
CRDT metadata (vector clock, last-writer-wins per-id) without
affecting the witness for the underlying bytes.

This is a real research problem, not a packaging one. Worth
exploring if the IIoT or drone-mesh verticals demand it; not
worth pre-building.

### 15.4 Memory-mapped `ruvec1` files

Already on the M2+ roadmap (`README.md` "Acceleration —
mmap'd index persistence via memmap2"). The blocker is making
`ruvec1` memory-layout-stable across endianness and CPU word size.
The current format is little-endian f32 with explicit byte
manipulation in `fs_backend.rs`; an mmap variant would need
explicit alignment + zero-copy guarantees, which is a real format
revision (`ruvec2`?), not just an mmap call.

For edge, the win is significant: warm-load drops from O(n × D)
to O(metadata pages), which is interesting at the n=1M scale.
Smaller caches don't benefit much (they fit in the kernel page
cache anyway after first read).

### 15.5 Bundle delta sync

Currently a bundle rotation requires re-downloading the entire
`ruvec1` binary. For bandwidth-constrained edges (cellular IoT,
satellite IoT), a delta-sync protocol — "the bundle now points at
data version 7; you have version 5; here are the appended chunks"
— would compress the wire cost by orders of magnitude in
append-mostly workloads.

The substrate's bundle protocol does not preclude this; it just
doesn't implement it. A `data_ref` could point at a manifest of
chunks rather than a single file, and the FsBackend equivalent
could track which chunks it has and request only the deltas. The
witness would still cover the manifest-as-of-now, preserving
correctness.

### 15.6 Cross-platform packaging

For ruLake to be a frictionless edge substrate, the packaging
needs to keep up:

- **Android AAR** with JNI bindings.
- **iOS XCFramework** with Swift bindings.
- **Yocto / Buildroot recipes** for embedded Linux distros.
- **A pre-built `wasm32-unknown-unknown` artifact** with the
  WASM-side adapters bundled.

None of this is research; all of it is the unglamorous engineering
work that determines whether the substrate is actually used on
the platforms it could run on.

### 15.7 `memory_class` vs operational class

ADR-156 introduces `memory_class` as a tag for cognitive type
(episodic, semantic, procedural). For edge, an analogous tag for
**operational class** (mission-critical, best-effort,
diagnostic-only) would let the substrate make different
prioritization decisions in resource-constrained scenarios:

- LRU eviction respects operational class — never evict
  mission-critical entries even if they are unpinned and oldest.
- Cache pressure metrics can be emitted per operational class.
- Power-management hooks can be triggered by class transitions.

This is small — a similar opaque-tag pattern to `memory_class` —
and would matter for safety-critical edge deployments.

### 15.8 Power-aware coherence

`Consistency::Eventual { ttl_ms }` is a static knob. For
battery-powered edges, dynamic TTL adjustment based on battery
state would extend mission duration:

- Battery > 50%: TTL = 60 s, fresher results.
- Battery 20–50%: TTL = 600 s, fewer cloud polls.
- Battery < 20%: TTL = ∞ (effectively `Frozen`), no cloud polls
  until link is also free.

This is application-layer logic today; the substrate already
exposes `with_consistency` as a builder that could be re-applied,
but `with_consistency` is a builder method that creates a new
`RuLake` (`crates/core/src/lake.rs:70`), not a runtime mutator. A
`set_consistency(&mut self, c)` method would be the substrate hook;
trivial to add, semver-minor.

### 15.9 Lock-free read path

`docs/review/performance.md` §B1 calls out the global cache mutex
as the next ceiling after the Arc-drop-lock refactor. For very
high-QPS edge deployments — concurrent multi-camera systems, busy
gateway boxes — sharding the cache state per witness bucket would
unlock another 4–8× concurrent throughput.

The work is substantial (sharded HashMap, atomic counters for the
stats, careful invariant maintenance) but the design is
well-understood. For most edge workloads, the single-mutex design
is sufficient; this matters at the high end.

### 15.10 Witness in a 32-byte raw form for radio links

The witness is currently 64 hex chars (`crates/core/src/bundle.rs:386`,
`hex::encode(out)`). For radio-budget-constrained edges
(LoRaWAN), a 32-byte raw form would halve the wire cost. This is
a serialization detail, not a substrate change — the bundle's
`rvf_witness` field could optionally serialize as base64 or raw
bytes for transport purposes, decoded back to hex on the receiving
side for storage.

---

## Summary

ruLake is a real and useful substrate for the edge. The core
properties — ~32× compression via RaBitQ, ~1.02× tax over direct
library use, three consistency modes, witness-anchored sharing,
warm-restart from disk in milliseconds, and a small bundle
protocol that fits in MQTT messages or static-asset HTTP responses
— line up with the constraints that edge deployments live with:
limited memory, intermittent backhaul, frequent power cycles, and
a need for cryptographic provenance over the served data.

The strongest fits today, in priority order:

1. **On-device LLM RAG (mobile / desktop, Vertical 2).** Concrete
   sizing is comfortable; consistency-mode story maps cleanly to
   personal / cloud sync. This is the highest-impact, lowest-risk
   edge deployment; it works on hardware shipping today.
2. **Federated edge with cloud truth (Vertical 6).** The pattern
   underlying IIoT, edge cameras, and drone fleets. Bundle
   protocol over the slow link, full state on the local SSD, cache
   in RAM. Already deployable.
3. **Embedded Linux gateway / Pi 5 / Jetson class (Vertical 8).**
   The substrate is sized for this hardware comfortably; the only
   gap is the missing NEON SIMD path in the kernel crate, which is
   a single-week piece of work in `ruvector-rabitq`.

Conditional fits:

- **Browser / WASM (Vertical 3).** Architecturally feasible, not
  yet built. Would unlock a meaningful new deployment vector.
- **Robotics (Vertical 4).** Soft real-time fits; hard real-time
  does not. Frozen + warm-restart give the audit-clean primitive
  safety systems want.
- **Drones (Vertical 11).** Mission-window Frozen mode is the
  natural shape; bundle protocol over intermittent ground link
  works.
- **Automotive cockpit (Vertical 10).** QM workloads fit;
  ASIL-rated paths do not.

Misfits:

- **Microcontrollers without an OS (Vertical 9).** Cannot run
  ruLake. Use as the gateway-side substrate that the MCU mesh
  writes into.
- **Hard real-time inner control loops.** Use a different layer
  of the stack.

The work that would most expand the edge footprint, if anyone is
asking what to build next:

- WASM port (architecture is ready, work is mostly adapters).
- NEON SIMD in `ruvector-rabitq` (one-week kernel work; doubles
  ARM performance).
- Memory-mapped `ruvec1` persistence (M2+ roadmap; matters at
  large n).
- `no_std` subset (opens RTOS-class deployments).

None of those are research-grade unknowns; they are engineering
that follows from the design choices already made. The substrate
is in good shape for the edge.

---

## File and ADR references

- `/home/ruvultra/projects/RuLake/crates/core/Cargo.toml` — dependency
  surface (9 direct, no async runtime, no FFI).
- `/home/ruvultra/projects/RuLake/crates/core/src/lib.rs` — public re-exports
  (six modules).
- `/home/ruvultra/projects/RuLake/crates/core/src/lake.rs` — `RuLake` entry
  point, search APIs, persistence, federation.
- `/home/ruvultra/projects/RuLake/crates/core/src/cache.rs` — `VectorCache`,
  `Consistency` enum, Arc-drop-lock hot path.
- `/home/ruvultra/projects/RuLake/crates/core/src/backend.rs` —
  `BackendAdapter` trait (4 methods + bundle override),
  `LocalBackend`, DoS caps.
- `/home/ruvultra/projects/RuLake/crates/core/src/fs_backend.rs` — `FsBackend`,
  `ruvec1` format, atomic writes, path-traversal validation.
- `/home/ruvultra/projects/RuLake/crates/core/src/bundle.rs` —
  `RuLakeBundle`, witness scheme, sidecar I/O, `memory_class`.
- `/home/ruvultra/projects/RuLake/BENCHMARK.md` — measured
  numbers; the calibration source for all latency / QPS tables in
  this document.
- `/home/ruvultra/projects/RuLake/docs/adrs/ADR-001-standalone-repo-strategy.md`
  — vendored submodule layout, packaging.
- `/home/ruvultra/projects/RuLake/docs/adrs/ADR-155-rulake-datalake-layer.md`
  — cache-first decision, M1 acceptance, roadmap.
- `/home/ruvultra/projects/RuLake/docs/adrs/ADR-156-rulake-as-memory-substrate.md`
  — substrate framing, `memory_class`, six-guarantee acceptance.
- `/home/ruvultra/projects/RuLake/docs/adrs/ADR-157-optional-accelerator-plane.md`
  — `VectorKernel` trait scaffolding, GPU dispatch policy, edge
  targets explicitly listed (browser / WASM SIMD).
- `/home/ruvultra/projects/RuLake/docs/adrs/ADR-158-optional-rotation-and-qvcache-positioning.md`
  — Hadamard rotation, QVCache positioning, rotation kind in
  witness (open question).
- `/home/ruvultra/projects/RuLake/docs/review/capabilities.md` —
  capability-vs-claim matrix; the source for several "this is
  scaffolding only" notes.
- `/home/ruvultra/projects/RuLake/docs/review/performance.md` —
  hot-path analysis, lock topology, identified bottlenecks
  (B1–B7).
- `/home/ruvultra/projects/RuLake/docs/review/security.md` —
  threat model, findings (M1–M6, L1–L7), input-validation map.
