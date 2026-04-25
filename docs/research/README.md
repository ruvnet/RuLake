# Vertical applications research — ruLake

Four white-paper-voice notes exploring where ruLake's substrate primitives
(witness-anchored coherence, three-mode consistency, atomic bundle protocol,
warm-restart persistence, federated rerank) earn their keep across vertical
domains. Produced in parallel by a research swarm grounded in the deep
review under [`../review/`](../review/) so claims stay sourced.

## Notes in this directory

| Note | Lines | Scope |
|------|-------|-------|
| [`agentic.md`](agentic.md) | 2,073 | Agent memory hierarchies, multi-agent swarms, tool-using retrieval, long-running autonomous agents, ReasoningBank-style replay, agentic payments |
| [`ai-ml.md`](ai-ml.md) | 2,111 | Production RAG with witness-pinned provenance, multi-tenant SaaS RAG, lakehouse cache, embedding-pipeline cache, vector-aware feature store, recsys, training-data dedup, A/B + counterfactual eval |
| [`edge.md`](edge.md) | 1,848 | On-device LLM RAG, browser/WASM, robotics, IIoT, edge inference cache, federated edge with cloud truth, mobile offline-first, embedded Linux (Pi/Jetson), in-vehicle, drones |
| [`exotic.md`](exotic.md) | 1,039 | Healthcare CDS audit, financial fraud + regulatory lineage, climate / earth observation, genomics, robotics fleet learning, scientific reproducibility, smart cities, spaceflight |

Total ~7,000 lines of substantive research. Each note ends with an honest
gap inventory and (where applicable) a ranking of verticals by leverage ×
feasibility on today's M1+M1.5 surface.

## What the swarm actually agreed on

Picking the same vertical from independent research is the strongest
positive signal. Three patterns showed up across notes:

**The witness chain is the differentiator, not the cache itself.**
Every note converged on the same observation: faster vector retrieval is
a commodity. What's *not* commodity is a cryptographically-anchored,
content-addressed, cross-process-shared, snapshot-replayable,
publication-portable provenance unit. ruLake is the only vector substrate
that ships this as a first-class primitive.

**`Consistency::Frozen` + `warm_from_dir` is the regulated-vertical
killer feature.** Healthcare audit, financial regulatory replay, scientific
reproducibility, spaceflight reference data — all four pick this exact
pair as the load-bearing primitive. No managed vector DB exposes a
"freeze the bit-state, archive it, replay it years later, prove byte-exact
reproduction" pathway.

**Cross-process witness sharing matters at swarm and fleet scale.** Agent
swarms (claude-flow-style hive-minds), robot fleets, multi-process
inference servers, researcher-laptop ↔ cloud-worker setups — all benefit
from the content-addressed dedup the witness gives for free. Verified by
the existing `two_backends_share_cache_when_witness_matches` test in
`tests/federation_smoke.rs`.

## Top-3 verticals per note (their picks, not ours)

### From [`agentic.md`](agentic.md)
1. Agent memory hierarchies (per ADR-156)
2. Multi-agent swarms with shared world model
3. Tool-using agent retrieval (MCP routing)

### From [`ai-ml.md`](ai-ml.md)
1. Production RAG with witness-pinned provenance
2. Vector-aware feature store
3. Embedding-pipeline cache

### From [`edge.md`](edge.md)
1. On-device LLM RAG (mobile / desktop)
2. Federated edge with cloud truth
3. Embedded Linux gateway (Pi 5 / Jetson class)

### From [`exotic.md`](exotic.md)
1. Scientific reproducibility (portable provenance unit)
2. Financial fraud + regulatory lineage
3. Healthcare CDS audit

## Cross-cutting gaps the swarm called out

- **No production cloud backends ship today.** `LocalBackend` (in-memory)
  and `FsBackend` (disk) are the M1 set. Parquet, BigQuery, Iceberg,
  Delta, Snowflake, S3-native are M2+ roadmap (ADR-155 §M2). Verticals
  that depend on these say so explicitly per-note rather than treating
  them as if shipped.
- **Governance plane is unimplemented.** RBAC via OIDC/JWT, PII
  passthrough via `rvf-federation::pii`, OpenLineage emission with
  witness-as-lineage-id — all M4 roadmap. Healthcare and financial
  verticals require these to deploy in production.
- **Single-writer per backend.** Bundle protocol assumes one publisher,
  many readers. Multi-region trading, multi-coordinator robotics fleets,
  and multi-spacecraft formation flying all stress this.
- **`Consistency` is per-`RuLake`, not per-collection.** Workloads that
  want different freshness budgets per collection (live vs archived,
  hot vs cold tier) instantiate two `RuLake`s today.
- **Sharded `CacheState` is the next concurrent-QPS ceiling.** The
  performance review's B1 finding starts to bite at K=300 shards × 10s
  of analyst workstations. Earth-observation and consortium-genomics
  hit this first.
- **No NEON / WASM kernels in `ruvector-rabitq`.** ARM laptops, Jetson
  edge, and browser deployments run the scalar fallback. ADR-157 is the
  scaffolding; per-arch crates are the work.

A thing the swarm flagged that has since been **closed**: the
standalone-build gap (`Cargo.toml` workspace inheritance + path-dep that
walked out of the repo) was the single biggest evaluator-blocker the
review surfaced. It's fixed in commit `a71f99d` per
[ADR-001](../adrs/ADR-001-standalone-repo-strategy.md) — a fresh
`git clone --recurse-submodules && cargo test --release` now passes
43/43 on a vanilla machine. Verticals that depend on a clean evaluator
path (regulatory submissions, scientific reproducibility, defense
procurement) are first-order beneficiaries.

## How to use these notes

- **Picking a first vertical to pursue?** Read each note's "Top 3" and
  the rankings. Highest-leverage verticals on today's M1+M1.5 surface
  (no roadmap dependencies): scientific reproducibility, agent memory
  hierarchies, fraud + lineage hot path, federated edge.
- **Sizing a custom backend?** The `BackendAdapter` trait is 4 methods
  (`id`, `list_collections`, `pull_vectors`, `generation`); see
  `src/backend.rs` for the contract and `src/fs_backend.rs` for a
  ~250-line reference. Several of the verticals (each cloud backend,
  each fleet aggregator, each EHR adapter) are described to roughly
  this level of detail in the notes.
- **Building a regulated deployment?** The PII / RBAC / OpenLineage gaps
  are real and called out per-vertical. The substrate gives you the
  audit anchor; the surrounding governance plane is application work
  on M1+M1.5.
- **Stress-testing a claim?** The notes cite file paths and ADRs
  consistently; cross-referencing into [`../review/`](../review/) and
  `BENCHMARK.md` is the fastest way to validate.

## Generation methodology

This research was produced by a four-agent swarm spawned in parallel via
the Claude Code Agent tool. Each agent received an identical "what ruLake
actually is" capability summary derived from the deep review, plus a
narrow domain prompt (agentic / AI-ML / edge / exotic). Agents wrote
directly to their assigned file in this directory. The exotic agent's
first run stalled mid-research; the second attempt also stalled before
writing, so `exotic.md` was authored directly against the same context
brief in the orchestrator turn — same source, same ground rules,
shorter (~1,000 lines vs the 1,800–2,100 the parallel agents produced).
