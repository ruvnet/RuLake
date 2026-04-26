# ruLake — Deep Review (M1 + M1.5)

Three-part code-level review of the standalone `rulake` crate
as imported into this repo. Reviewers sourced everything from `src/`,
`tests/`, `examples/`, `Cargo.toml`, `BENCHMARK.md`, and the four ADRs
under `docs/adrs/` — no external benchmarks were run; the goal was
honest static analysis of what the code does today vs what the README
and ADRs say it does.

## Documents

- [`capabilities.md`](./capabilities.md) — every public API mapped to
  a real implementation site, full feature-claim matrix vs README, M1
  vs M2+ split, ADR conformance.
- [`performance.md`](./performance.md) — hot-path walk-through, lock
  topology, allocation patterns, parallelism, complexity, ranked
  bottlenecks, prioritized optimization opportunities.
- [`security.md`](./security.md) — threat model, severity-rated
  findings (Critical / High / Medium / Low / Info), witness/digest
  scheme audit, path-traversal review, dependency surface, full
  remediation table.

---

## Executive summary — capabilities

ruLake's M1 + M1.5 + persistence claims are **real and reproducible
from the code**. The 20-method `RuLake` surface, the
`BackendAdapter` trait, the witness-anchored bundle protocol,
`save_cache_to_dir` / `warm_from_dir`, the three consistency modes,
adaptive per-shard rerank, batch API, LRU, `LocalBackend` and
`FsBackend` are all implemented and exercised by 37 tests in this
crate (19 in `tests/federation_smoke.rs`, plus inline tests in
`bundle.rs`, `backend.rs`, `fs_backend.rs`). The README's headline
"83 tests" is a workspace-wide count, not this crate alone — minor
clarity issue. The README oversells a few things — the
`VectorKernel` trait, AVX2/AVX-512 kernels, and Hadamard rotation —
which actually live in the sibling crate `ruvector-rabitq` and are
**not directly callable from `RuLake`'s API**; ADR-157's accelerator
plane is "Proposed — scaffolding-only" on both sides. Every M2+
backend (Parquet, BigQuery, Iceberg, Delta) and all of M4 governance
(RBAC, PII enforcement, OpenLineage emission) are roadmap, not code.

---

## Executive summary — performance

The performance design is **honest and well-engineered**. The single
biggest architectural win — `Arc<RabitqPlusIndex>` cloned under the
mutex, mutex dropped before scan (`cache.rs:734-762`) — eliminates
scan serialization and is what `BENCHMARK.md` measures as 8-13×
concurrent QPS lift. Per-query allocation pressure is low (the
`intern_key` Arc-pattern at `cache.rs:189` was a deliberate fix from
a memory audit). Rayon fan-out in `search_federated`
(`lake.rs:527-538`) hits 1.97× / 3.86× speedups at 2 / 4 shards on
the prime path. The remaining ceiling is **lock-state contention**
on the global `Arc<Mutex<CacheState>>` — the 3 mutex acquisitions per
hit-path query become the next bottleneck above ~50K QPS or under
high shard × client counts. Top three ranked optimization
opportunities: (1) atomicize the hit-rate counters so `mark_hit` /
`mark_miss` doesn't need the mutex, (2) shard the `CacheState` per
witness-bucket, (3) add `par_iter` inside `search_batch` for the
CPU-only path. None of these are blockers for the M1 claims.

---

## Executive summary — security

**Critical: 0. High: 0. Medium: 6. Low: 6. Info: 5.** Posture is
substantially better than typical for a v1 Rust crate at this scope.
**Zero `unsafe`** in `src/`. Witness scheme is correctly
domain-separated and length-prefixed (SHAKE-256(32) over five
fields), with both regression tests against the historic
`Num` vs `Opaque` collision and the "a|b" vs "ab|" concatenation
collision. JSON sidecars are length-capped (64 KiB body, 4 KiB per
field, 128-byte witness). The `FsBackend` filename validator covers
12 attack inputs (POSIX, Windows, control bytes, UNC, drive
letters). Atomic temp+rename for every on-disk write. The medium-
severity findings are **operator footguns**, not directly
exploitable vulnerabilities: M1 (`Mutex` poisoning bricks the Lake),
M2 (default `current_bundle` does a full pull → catastrophic on
`Fresh`), M3 (`FsBackend` symlink/TOCTOU on multi-tenant hosts), M4
(`Generation::Opaque(String)` unbounded inside `PulledBatch`), M5
(`LocalBackend::append` no growth cap), M6 (`Cargo.toml` workspace
inheritance + path dep blocks standalone builds and audit). All M
findings have low-to-medium remediation effort; addressing them plus
adding fuzz harnesses for the two parsers (I5) is roughly one week
of work.

---

## Overall verdict

For a v1 cache-first vector execution fabric, ruLake **delivers what
the ADRs say it delivers and tests what it ships**. The M1 + M1.5
+ persistence story is mechanical, not aspirational — the
`brain_substrate_acceptance_recall_verify_forget_rehydrate` test
(`tests/federation_smoke.rs:766`) is the proof. The architecture
choices that hold up under static review:

1. **Witness-anchored content-addressed cache** — correctly
   designed, correctly implemented, regression-tested against the
   audit-driven collision findings.
2. **Arc-drop-lock concurrency** — the single most consequential
   correctness × performance × safety triple-win in the crate.
3. **Defense in depth on every external parser** — `validate_pulled_batch`,
   filename whitelist, JSON caps, header bounds, witness verify on
   read.
4. **Honest scope.** ADR-155 is explicit about what M1 ships and
   what M2-M5 doesn't; the code matches the ADR exactly. The
   README is the only place that oversells.

What needs work before a production-grade tag:

- **README clean-up** to move kernel claims into "via
  ruvector-rabitq" framing and to disambiguate M1-shipped vs
  M2+-roadmap. (Capabilities §6 and §10.)
- **Standalone packaging** — drop the path dep on
  `ruvector-rabitq`, pin every `workspace = true` dep to a concrete
  version, ship a `Cargo.lock`, add `cargo audit`/`cargo deny` to
  CI. (Security M6, Capabilities §5.)
- **Address M1-M5 security findings** — they're hardening, not
  fixes for active exploits, but the FsBackend symlink case (M3)
  matters for any multi-tenant deployment.
- **Lift the next perf ceiling** — atomicize hit-rate counters and
  move toward a sharded `CacheState` once benchmarks show the
  global mutex is the bottleneck. (Performance B1, B4, O3, O6.)

The crate is a credible substrate for the cache + coherence + warm-
restart + federation primitives it claims, with clear-eyed
acknowledgment of where its scope ends.

---

**Reviewed:** 2026-04-25 against commit `1d0cc35` ("Initial commit:
ruLake — a cache-coherent vector execution fabric").
