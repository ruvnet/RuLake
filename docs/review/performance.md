# ruLake — Performance Review

**Scope.** Static analysis of the ruLake crate's hot paths, lock
discipline, allocation patterns, parallelism, and complexity. Cross-
check against `BENCHMARK.md`'s headline numbers (≈1.02× tax over
direct RaBitQ; 8-12× concurrent QPS lift from the Arc-drop-lock
refactor; 3.86× 4-shard parallel prime). No new benchmarks are run;
the goal is to validate the design choices in code and surface
bottlenecks operators will hit at scale.

---

## 1. The cache-hit hot path — line-by-line

The single most important code path in the entire crate: the
`search_one` cache hit. Mapped from caller down to scan:

```
RuLake::search_one (lake.rs:445)
  → intern_key (cache.rs:189)                        — 2× Arc::from(&str), one alloc each
  → ensure_fresh (lake.rs:638)
      → can_skip_check_interned (cache.rs:862)       — 1× lock acquire, 1 HashMap get
      → mark_hit (cache.rs:682)                      — 1× lock acquire
      → return                                       — fast path exits here under Eventual+TTL
  → cache.search_cached_with_rerank_interned (cache.rs:722)
      → lock cache mutex                             — 3rd lock acquire on the hot path
      → get pointer, get entry, dim check
      → entry.last_used = Instant::now()             — 1× syscall (clock_gettime)
      → Arc::clone(&entry.index), Arc::clone(&entry.pos_to_id)  — 2× refcount bumps
      → drop lock                                    — RELEASED before scan
      → index.search(query, k)                       — pure CPU scan, no shared state
      → map results: pos_to_id[r.id] → external u64
```

**Lock acquisitions per hit-path query: 3** (`can_skip_check_interned`,
`mark_hit`, `search_cached_with_rerank_interned`). Each is a
`Mutex::lock()` on a single `Arc<Mutex<CacheState>>`.

The Arc-drop-lock pattern at `cache.rs:734-762` is the core insight
that makes concurrent QPS scale: the scan itself runs **outside** the
mutex, so N concurrent readers parallelize on the immutable
`RabitqPlusIndex` without contention. This is what `BENCHMARK.md`'s
"8-12× concurrent QPS lift" measures, and the design is correct.

**Bottleneck #1: those 3 mutex acquisitions are sequenced and
contended.** Under N concurrent readers, the critical sections are
short (≈ `Arc::clone` + `HashMap::get`), but every reader still has
to serialize through them. At very high client counts (≥ 32 threads
on a single key) this becomes the next ceiling after the Arc refactor.
Concrete suggestions in §6.

---

## 2. Lock topology — what's protected and at what cost

```
RuLake.backends   : Arc<RwLock<HashMap<BackendId, Arc<dyn BackendAdapter>>>>
RuLake.cache      : Arc<VectorCache>
VectorCache.inner : Arc<Mutex<CacheState>>
LocalBackend.inner: Arc<RwLock<LocalState>>
FsBackend.index   : RwLock<HashMap<CollectionId, String>>
```

Five lock primitives. Notable:

- **`RuLake.backends`** is a `RwLock` (`lake.rs:50`), and every read
  path takes `.read()`. Registration takes `.write()`. **Good.**
- **`VectorCache.inner`** is a plain `Mutex` (`cache.rs:239`), not a
  `RwLock`. Every cache op — including read-only ones like
  `witness_of`, `dim_of`, `has`, `mark_hit`, `mark_miss` — serializes
  through it. The Arc-drop-lock pattern means the heavy work
  (the scan) doesn't hold the lock, but read-mostly bookkeeping
  does. See `cache.rs:330` (`stats()`), `cache.rs:333-363`
  (`stats_by_backend`, `stats_by_collection`).
- **`LocalBackend`** uses a `RwLock` correctly: reads at `pull_vectors`
  (`backend.rs:269`), writes at `put_collection` / `append`
  (`backend.rs:207, 226`).
- **`FsBackend.index`** uses an `RwLock` for the collection-name
  table (`fs_backend.rs:58`); the actual file I/O is unsynchronized
  per-file, relying on POSIX rename atomicity for write coherence.

**Observation: every cache read takes a global mutex, not a sharded
one.** Two queries against two completely different
`(backend, collection)` pairs still contend on the same
`VectorCache.inner` mutex for the ≈1-microsecond bookkeeping. At very
high QPS this becomes visible.

---

## 3. Allocation patterns

### The `intern_key` fix (memory-audit finding #1)

`cache.rs:189`:

```rust
pub(crate) fn intern_key(backend: &str, collection: &str) -> InternedKey {
    (Arc::from(backend), Arc::from(collection))
}
```

Called once per `search_one` / `search_federated` shard / `search_batch`.
Allocates **two** `Arc<str>`s on entry. Every downstream cache op
(`mark_hit`, `mark_miss`, `per_backend_mut`, pointer lookups) takes
`Arc::clone` refcount bumps — **zero further allocations**.

This is the right pattern for the hot path. Quantified at
`cache.rs:29-37` as "≈96 B/query at federated fan-out with 3 K calls
per query".

**Mild concern:** `intern_key` is called per-query, not per-Lake.
For a stable `(backend, collection)` pair queried at >100K QPS, you
allocate 2 × N Arcs across N queries, then drop them all when the
top-level call returns. A per-Lake intern table
(`HashMap<(&str,&str), InternedKey>`) would amortize this to one
allocation per pair across the lifetime of the process. Probably not
worth the complexity until profiling shows allocator pressure.

### Per-query allocations downstream

- `search_cached_with_rerank_interned` returns `Vec<(u64, f32)>`
  (`cache.rs:722`). At k=10 this is 240 bytes — fine.
- `lake.rs:460-468` then **re-allocates** that into
  `Vec<SearchResult>` with `backend.to_string()` and
  `collection.to_string()` per result. **At k=10, this is 10 ×
  (sizeof(SearchResult) + 2 × String) ≈ 1 KB plus 20 short-string
  allocations per query**.
- `search_federated` collects all shard results, sorts (in-place
  `merged.sort_by` at `lake.rs:540`), truncates. Sort is `O(K·k·log(K·k))`
  — irrelevant for typical k = 10, K ≤ 8.

**Bottleneck #2: per-result `String` allocation in `SearchResult`
construction.** The interned `Arc<str>` for `(backend, collection)` is
already in scope at this point but is converted back to owned `String`
for the public type. A small refactor —
`SearchResult { backend: Arc<str>, collection: Arc<str>, ... }` —
would eliminate ≈20 short-string allocs per query at k=10. Public
API change, semver-minor.

### `pos_to_id` duplication (memory-audit finding #2, HIGH)

`cache.rs:223-232` documents that `pos_to_id: Arc<Vec<u64>>` inside
`CacheEntry` duplicates `RabitqPlusIndex.ids: Vec<u32>` inside
`ruvector-rabitq`. At n=1M with u64 ids that's 8 MB per entry. Reason
documented inline: cross-crate widening would lose cache-line density.
**Verdict:** the trade-off is reasonable for now; revisit if
`max_entries` × 8 MB starts hurting. Operators with very large entry
pools should size `with_max_cache_entries` to bound it.

### Prime-path allocations

`cache.rs:368-447` `prime_interned`:

1. Validate (no allocation).
2. Acquire lock; check fast path (no allocation if shared).
3. Drop lock.
4. **Heavy work outside the lock:**
   - `pos_to_id: Vec<u64> = batch.ids.clone()` — `O(n)` u64s.
   - Either serial `RabitqPlusIndex::new + add` loop (n < 1024) or
     `from_vectors_parallel` (n ≥ 1024).
5. Re-acquire lock, race-check, insert.

**The race-check is correct** (`cache.rs:429-431`): if another thread
beat us to the insert, we drop our work and take the shared entry.
**Memory implication:** under heavy first-prime contention on the
same witness, multiple threads will simultaneously do the
`O(n·D)` compression work, all but one dropping it on the floor. For
a 100K-vector × D=128 prime that's hundreds of MBs of wasted compute
per loser.

A pre-lock "are we already building this witness?" futures-style
deduplication would prevent redundant work on contended primes. Not
present today. Probably not worth it on cold-start traffic patterns
where the first query primes once and the rest are hits — but worth
flagging for "thundering herd" workloads (e.g. cache restart with N
parallel callers).

---

## 4. Parallelism — rayon usage

### Federated fan-out (`lake.rs:527-538`)

```rust
let shard_hits: Result<Vec<Vec<SearchResult>>> = targets
    .par_iter()
    .map(|(backend, collection)| {
        self.search_one_with_rerank(backend, collection, query, k_per_shard, rerank_override)
    })
    .collect();
```

Standard rayon. First-error short-circuit via `Result` collect.
`BENCHMARK.md` reports 1.97× / 3.86× speedups at 2 / 4 shards on the
prime path.

**Performance characteristic:** each shard does its own
`ensure_fresh` → `cache.search_cached_with_rerank_interned` which
in turn takes `cache.inner.lock()` three times. So 4-shard
federation does **12 mutex acquisitions** on the global cache state
across rayon worker threads — these are all on `Arc<Mutex<CacheState>>`,
serialized. Under high concurrency the rayon win on the scan is
partly offset by lock contention on the cache map.

This is the same root cause as Bottleneck #1 above: a sharded cache
state (one `Mutex` per witness, or per witness-bucket) would let
shards proceed without lock crosstalk. **The Arc-drop-lock refactor
removed scan serialization; lock-state serialization is the next
ceiling.**

### Parallel prime (`cache.rs:398-414`)

```rust
const PARALLEL_PRIME_THRESHOLD: usize = 1024;
let idx = if batch.vectors.len() >= PARALLEL_PRIME_THRESHOLD {
    let items: Vec<(usize, Vec<f32>)> = batch.vectors.into_iter().enumerate().collect();
    RabitqPlusIndex::from_vectors_parallel(dim, ..., items)?
} else {
    let mut idx = RabitqPlusIndex::new(dim, ..., ...);
    for (pos, v) in batch.vectors.into_iter().enumerate() {
        idx.add(pos, v)?;
    }
    idx
};
```

Two issues:

1. **Threshold is hard-coded at 1024.** Documented as "picked from a
   sweep on D=128". For D=768 (OpenAI embeddings) or D=1024 the
   crossover is at lower n; for tiny D the crossover is higher. A
   `D × n`-aware threshold (e.g. `total_flops > 1<<20`) would adapt
   better. **Suggestion:** make threshold configurable, default to
   `n × D > 100_000` or similar.

2. **`batch.vectors.into_iter().enumerate().collect::<Vec<_>>` is an
   extra `O(n·D)` allocation+copy** just to materialize positions.
   `from_vectors_parallel` could plausibly take an iterator + a
   `start_position` to avoid this — cross-crate change required.

### `search_batch` (`lake.rs:600`, `cache.rs:795`)

The doc comment at `cache.rs:783-786` advertises "lock-once" — true:
the cache mutex is held for one acquisition across all N queries, the
Arcs are cloned, lock dropped, and the loop runs unlocked
(`cache.rs:805-840`). **However, the loop is sequential** —
`for q in queries { ... }` — so a single batch call does not
parallelize across queries even when N is large.

`BENCHMARK.md` reports ~1.05× speedup of `search_batch` over
per-query loop on the warm path (single-threaded, Eventual TTL),
which matches: the win is amortizing 3 mutex acquires across N
queries, not parallel scan.

**Suggestion:** add an internal `par_iter` over the batch when
`queries.len() ≥ N_threshold` AND a kernel doesn't itself batch (
ADR-157 GPU kernels would prefer to receive the whole batch). Today
this is a missed opportunity for CPU `search_batch` to scale.

---

## 5. Complexity of search

| Operation | Complexity | Reference |
|---|---|---|
| `intern_key` | `O(|backend| + |collection|)` (Arc::from(&str)) | `cache.rs:189` |
| `ensure_fresh` (Eventual+TTL hit) | `O(1)` (mutex+HashMap lookup) | `lake.rs:638-645` |
| `ensure_fresh` (witness-already-cached) | `O(1)` cache work + `O(backend.current_bundle)` | `lake.rs:647-660` |
| `ensure_fresh` (miss → prime) | `O(backend.pull) + O(n·D log D)` rabitq build | `lake.rs:662-672` |
| `RabitqPlusIndex::search` (cache-hit scan) | `O(n × D / 64)` popcount + `O(rerank_factor × k × D)` exact rerank | inherited from rabitq |
| `search_federated` | `O(K × per_shard_search) / par_workers + O(K·k log(K·k))` merge | `lake.rs:527-541` |
| `search_batch` (sequential) | `O(N × per_query_search)` — mutex amortized once | `cache.rs:805-840` |
| `save_cache_to_dir` | `O(n × D)` serialize + atomic rename | `lake.rs:263-334` |
| `warm_from_dir` | `O(n × D)` deserialize + `O(1)` cache install | `lake.rs:378-441` |
| `evict_lru_if_over` | `O(entries) per eviction` (linear scan to find min `last_used`) | `cache.rs:548-565` |

**Cost concern: LRU eviction is O(entries) per eviction step.** A
binary-heap or `BTreeMap<Instant, Witness>` would make this
`O(log entries)`. Today that's fine because `entries` is bounded by
`max_entries` (typically tens to hundreds) — at 1000 entries with
frequent evictions this becomes a real cost. **Suggestion:** if
operators ever set `max_entries` ≥ 1024 in production, switch to a
heap-based LRU. For the M1 default (unbounded) it doesn't matter.

---

## 6. Identified bottlenecks (ranked)

### B1 — Global cache mutex serializes per-query bookkeeping

**Location:** `cache.rs:239` `Arc<Mutex<CacheState>>`.

**Symptom:** at very high QPS (≥ 50K/s) on a single Lake instance,
the 3 mutex acquisitions per query dominate scan cost. `BENCHMARK.md`
shows linear scaling to 4 shards × 8 clients (36,715 QPS); above that,
mutex contention will plateau the curve.

**Fix paths:**
1. Shard the cache state per witness or per witness-bucket
   (`Vec<Mutex<Bucket>>` with hash-based shard selection). High effort,
   high payoff at scale.
2. Replace `Mutex` with `RwLock` and use `read()` for
   `can_skip_check`, `witness_of`, `mark_hit`. The stats counters
   would need atomic conversion (`AtomicU64` instead of plain `u64`).
   Medium effort, partial payoff (writes still serialize).
3. Use `parking_lot::Mutex` instead of `std::sync::Mutex` — typically
   10-50% faster on contention. Trivial drop-in.

### B2 — Per-result `String` allocation in `SearchResult`

**Location:** `lake.rs:460-468`, also in `search_one_with_rerank`,
`search_federated`, `search_batch`.

**Symptom:** at k=10 across 4 shards, a `search_federated` call
allocates ≈40 short `String`s for the (backend, collection) labels
on returned hits. That's 40 allocations per federated query.

**Fix:** change `SearchResult.backend: String → Arc<str>` (and
similarly for `collection`). Public API change, semver-minor. Bonus:
`Arc<str>` clones are refcount bumps, not heap allocs.

### B3 — Sequential loop inside `search_batch`

**Location:** `cache.rs:822-840`.

**Symptom:** `search_batch(N=1000)` doesn't parallelize across
queries on CPU. The mutex amortization is the only win.

**Fix:** rayon `par_iter` over `queries` when `queries.len() ≥ THRESH`,
gated on the kernel not preferring whole-batch dispatch (ADR-157
forward-compat). For today's CPU-only world this is pure upside.

### B4 — Federated fan-out's lock crosstalk

**Location:** `lake.rs:527-538` × `cache.rs:734-762`.

**Symptom:** rayon shards each do 3 mutex acquisitions on the **same**
global cache mutex. At 4 shards × 8 concurrent clients that's 96
acquisitions per round, all serialized.

**Fix:** same as B1 — sharded cache state. Once B1 is fixed, B4 falls
out for free.

### B5 — Thundering-herd prime work

**Location:** `cache.rs:395-432`.

**Symptom:** N threads simultaneously cache-missing the same
`(backend, collection)` will all do the full `O(n·D log D)` rabitq
build; only one wins the post-build race. Wasted CPU on cold restart
or after explicit `invalidate_cache`.

**Fix:** "in-flight builds" map: `HashMap<WitnessKey,
Arc<oneshot::Receiver>>` so the first miss to start a build registers
intent, and subsequent misses await the receiver instead of
duplicating work. Higher complexity; only matters for cold-start
patterns. Defer until measured.

### B6 — `evict_lru_if_over` is O(entries)

**Location:** `cache.rs:548-565`.

**Symptom:** linear scan to find the minimum `last_used` per eviction.
Fine for small caps (≤ 100); painful at ≥ 1000.

**Fix:** `BTreeMap<Instant, WitnessKey>` index of unpinned entries by
`last_used`; or a doubly-linked-list LRU. Standard.

### B7 — `intern_key` per-query allocation

**Location:** `cache.rs:189`, `lake.rs:455`.

**Symptom:** 2 `Arc<str>` allocations per `search_one`. At 100K QPS
that's 200K allocations/sec just for keys.

**Fix:** Lake-level intern table for stable keys. Defer until
allocator pressure shows up.

---

## 7. Rayon thread pool usage

The crate uses the **global rayon pool** (`rayon::prelude::*` at
`lake.rs:510`). Two operational notes:

1. **No way to opt out of parallelism** — a caller embedding ruLake
   in a single-threaded context (e.g. inside a hot async event loop)
   has no `with_thread_pool(...)` knob. `par_iter` will spawn work
   on the global pool.

2. **No interaction with `with_max_cache_entries`** for prime-time
   memory pressure: `from_vectors_parallel` may use all rayon
   threads, each holding intermediate state. For very large N, this
   is N × (per-thread scratch) memory beyond the final index size.

**Suggestion:** expose a `RuLake::with_thread_pool(Arc<rayon::ThreadPool>)`
builder for embedded users. Low effort.

---

## 8. Cross-checks against `BENCHMARK.md`

The benchmark numbers are reproducible from `src/bin/rulake-demo.rs`.
Checked claim-by-claim against the code:

| BENCHMARK.md claim | Code evidence | Verdict |
|---|---|---|
| 1.02× tax over direct rabitq, hit path | `lake.rs:445` calls `cache.rs:722` which is one `Arc::clone` + scan; the only added work is the `intern_key` (2 allocs) and the `mark_hit` HashMap update | **Plausible, matches design.** Direct rabitq scan vs lake hit path differs only by a few microseconds of bookkeeping. |
| 8.3-13.2× concurrent QPS lift from Arc refactor | Arc-drop-lock at `cache.rs:734-762` releases the mutex before the CPU-bound scan | **Backed.** This is the central architectural win. |
| 1.97× / 3.86× prime-time speedup at 2 / 4 shards | rayon `par_iter` at `lake.rs:528` | **Backed.** Less-than-perfect K× ceiling explained by per-shard cache-mutex contention on insert. |
| Adaptive per-shard rerank `max(5, global/K)` keeps recall ≥ 0.85 | `lake.rs:474, 512-519`; tested at `tests:1066` | **Backed.** |
| `search_batch` ≈ 1.05× per-query loop on warm Eventual | `cache.rs:795` lock-once, then sequential per-query scan; CPU work dominates | **Matches** — the win is the mutex amortization, which is small at warm Eventual. |

`BENCHMARK.md`'s explicit caveat: "Bench is single-thread … cache
memory footprint vs backend size … not yet tuned" — both honest. Real
remote-backend Fresh-mode tax is **not** measured anywhere; documented
as M2 work.

---

## 9. Concrete optimization opportunities (priority-ranked)

Listed by `(impact × ease)`. Highest first.

| # | Change | Impact | Ease | Notes |
|---|---|---|---|---|
| O1 | Swap `std::sync::Mutex` → `parking_lot::Mutex` for `CacheState.inner` | Medium (10-50% on contention) | Trivial | One line + dep. Probably already considered; trade-off is one more crate dep. |
| O2 | Add `par_iter` to `search_batch` when `queries.len() ≥ THRESH` | High at large batches | Easy | Pure upside on CPU; gate behind a const for now. |
| O3 | Convert hit-rate stats counters to `AtomicU64` so `mark_hit` doesn't need the mutex | High (eliminates 1 of 3 acquisitions per hit) | Medium | Need to be careful about the `per_backend` / `per_collection` HashMap inserts on first-touch — those still need the lock. Could use `DashMap` or upgrade-on-insert pattern. |
| O4 | Make `PARALLEL_PRIME_THRESHOLD` a `D × n` heuristic | Medium (low-D / high-D primes) | Easy | Replace constant with `(n.saturating_mul(dim)) >= 100_000`. |
| O5 | Replace `SearchResult.backend: String` with `Arc<str>` | Medium (40 allocs/query at K=4, k=10) | Medium (semver-minor public API change) | Caller-side `as_ref()` reads as `&str` for free. |
| O6 | Sharded `CacheState` (one `Mutex` per witness-hash bucket) | Very high at high QPS | Hard | The "next ceiling" after Arc refactor. Real engineering. |
| O7 | Heap-based LRU index | Medium at large `max_entries` | Medium | `BTreeMap<Instant, WitnessKey>` for unpinned entries. |
| O8 | In-flight prime dedup | Medium under cold-start herds | Hard | New mechanism (oneshot/await). Defer until measured. |
| O9 | Expose `with_thread_pool(Arc<ThreadPool>)` builder | Low (operational) | Easy | Helps embedded users. |

---

## 10. Verdict

The performance story in this crate is honest and well-engineered:

- **The hot path is correct by construction.** Arc-drop-lock at the
  cache scan means N concurrent readers on the same key parallelize
  perfectly on CPU. This is the single biggest design win and it
  shows up in `BENCHMARK.md`'s 8-13× concurrent QPS lift.
- **The miss path has a meaningful parallel knob** for n ≥ 1024 via
  `from_vectors_parallel`, with a documented threshold and a measured
  speedup at 4 shards.
- **Safety knobs are everywhere:** `validate_pulled_batch` rejects
  hostile inputs before any allocation; LRU eviction is documented
  to never evict pinned entries; the witness fast-path makes shared
  primes free.
- **Allocation discipline is good** in the steady-state hot path
  (refcount bumps, not clones) and the documented "memory-audit
  finding #1" fix is real and effective.

The remaining ceiling is **lock-state contention on the global
`CacheState` mutex** at ≥ 50K QPS or under high shard × client
counts (≥ 4 shards × 16 clients). The Arc refactor moved scan out
of the lock; the next refactor needs to move per-key bookkeeping out
of the lock too — either by sharding or by atomicizing the stats
counters. That's the only meaningful structural improvement left
before per-backend network latency (M2) becomes the dominant factor
anyway.

For the M1+M1.5 surface this crate ships, the perf design is solid
and the headline benchmark claims (≈1.02× tax, 13.2× concurrent QPS
at 4 shards) are credible from the code.
