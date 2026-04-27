# ruLake for Agentic AI

**A research paper on the cache-coherent vector execution fabric as
substrate for autonomous agents, multi-agent swarms, and long-running
agentic systems.**

---

**Status:** research / vertical exploration · 2026-04-25
**Author:** agentic-AI vertical agent (4-agent research swarm)
**Audience:** agent-framework authors, multi-agent platform engineers,
research operators considering ruLake as the substrate beneath an
agent brain or orchestration layer
**Scope:** describe how the *measured* M1 + M1.5 surface of ruLake
(`crates/core/src/lake.rs`, `crates/core/src/cache.rs`, `crates/core/src/bundle.rs`, `crates/core/src/backend.rs`,
`crates/core/src/fs_backend.rs`) maps to the recurring memory + retrieval needs
of agent systems. Cite code; cite ADRs; mark every gap honestly.
**Non-scope:** building the brain, defining cognitive semantics,
proposing reward models, ranking fits relative to vector databases on
benchmarks ruLake does not run.

---

## Table of contents

1. Why ruLake matters for agents
2. Vertical 1 — Agent memory hierarchies (per ADR-156)
3. Vertical 2 — Multi-agent swarms with a shared world model
4. Vertical 3 — Long-running autonomous agents
5. Vertical 4 — Tool-using agent retrieval
6. Vertical 5 — Agentic payments / ledger access
7. Vertical 6 — Federated LLM RAG with provenance
8. Vertical 7 — Reflection / experience replay (ReasoningBank-style)
9. Reality check — what ruLake explicitly does *not* do for agents
10. Open research questions
11. Appendix A — API reference cheat-sheet (the agent-relevant subset)
12. Appendix B — Operating-mode decision matrix per agent class
13. Appendix C — Gaps inherited from the deep review
14. Appendix D — Worked memory-class taxonomy for the bundle tag

---

## 1. Why ruLake matters for agents

### 1.1 The problem agents have with vector storage today

Agentic systems retrieve vectors a lot. Every agent loop that touches
RAG, every tool-router that picks the right MCP server, every
reflection module that consults its own past trajectories, every
multi-agent shared scratchpad — these are vector queries, mostly
nearest-K, mostly read-heavy, almost always against a corpus the agent
does not own. The substrate options today are:

1. **In-process FAISS / hnswlib / `ruvector-rabitq`.** Lowest latency,
   no coherence story across processes, no story for "the source data
   changed and I need to re-prime", no story for shared caching across
   agents in the same swarm. Each agent owns its own copy of the codes;
   when the dataset rotates, every process re-pays the compression
   cost.
2. **Hosted vector DBs (Pinecone, Weaviate, Qdrant, Milvus,
   pgvector).** Network hop on every query (5–50 ms even regional).
   Schema lives in the DB, not next to the data. Coherence with a
   data-lake source-of-truth (Parquet on S3, Iceberg, BigQuery) is the
   operator's problem. Multi-tenant rate-limits, vendor-specific
   filters, and you cannot snapshot the warm index byte-for-byte to
   feed a fresh agent process.
3. **App-managed embedded indexes plus polling.** What most production
   agent stacks do today — load Parquet into `RabitqPlusIndex` or
   `Annoy`, poll the source file, rebuild on diff. Works for one
   agent; falls over at swarm scale because every agent re-builds.

None of these provide all of: (a) microsecond per-query overhead, (b)
content-addressed cache sharing across processes, (c) backend-driven
coherence with a verifiable witness, (d) a snapshot-and-rehydrate
primitive for warm-restart, (e) federation across heterogeneous
backends, (f) per-class consistency knobs (Fresh / Eventual / Frozen),
(g) zero-network operation when serving from cache.

ruLake provides all seven by design (`crates/core/src/lake.rs:445-625` and
`crates/core/src/cache.rs:294-885`), at a measured 1.00–1.03× overhead over the
underlying RaBitQ scan on the cache-hit path
(`docs/review/performance.md` §1; cross-checked at §8). It does so
deliberately as **substrate**, not as a brain — ADR-156 records that
positioning explicitly:

> **ruLake stays as substrate. It is the memory hierarchy, not the
> brain.** (ADR-156 §Decision)

That distinction is load-bearing for agentic AI. Brain semantics
(episodic vs semantic, contradiction scoring, reward-weighted recall)
move fast and are domain-specific; the substrate's six guarantees
(recall / verify / forget / rehydrate / location-transparency /
compact-deferred — see ADR-156 §"The substrate acceptance test") are
stable and shared.

### 1.2 What an agent actually needs from a vector store

If you list the operations an autonomous agent does against a vector
store across one of its loops, you get:

| Op | Frequency | Notes |
|---|---|---|
| Top-K nearest-neighbour fetch | 100–10000× per task | Memory recall, RAG, tool selection. |
| Verify the entry hasn't been mutated under us | Every fetch in compliance modes | Audit trail; tamper detection. |
| Drop a memory the agent has decided is stale | 10–100× per session | "I learned X is wrong"; pruning. |
| Re-fetch from source when invalidated | On generation bump | Source-of-truth changed. |
| Fan out across collections | 1–10× per query | Tool router; cross-domain RAG. |
| Hand the warm index to a new process | On restart, on hand-off | Daemon survives reboot. |
| Snapshot for forensics / debugging | Per session | "What memories did the agent see?" |

Now compare to the ruLake API surface (the 20 methods on `RuLake`
listed in `docs/review/capabilities.md` §1):

| Agent need | ruLake primitive | Source |
|---|---|---|
| Top-K nearest-neighbour fetch | `RuLake::search_one`, `search_batch` | `lake.rs:445`, `:600` |
| Verify entry hasn't been mutated | `RuLakeBundle::verify_witness`, automatic on read | `bundle.rs:191`, `:340` |
| Drop a memory | `RuLake::invalidate_cache` | `lake.rs:154` |
| Re-fetch from source on bump | `RuLake::refresh_from_bundle_dir` (returns `Invalidated`) | `lake.rs:200` |
| Fan out across collections | `RuLake::search_federated` (rayon parallel) | `lake.rs:491` |
| Hand the warm index to a new process | `RuLake::save_cache_to_dir` / `warm_from_dir` | `lake.rs:263`, `:378` |
| Snapshot for forensics | The `index.rbpx` + `table.rulake.json` pair on disk | `lake.rs:263` |

Every operation an agent does has a primitive, and the primitive is
not buried under three layers of abstraction. That is what "substrate"
means — small, focused, no hidden cost.

### 1.3 The witness chain is the fit

The single most agent-relevant property of ruLake is that **every
cache entry is content-addressed by a SHAKE-256 witness over
`(data_ref, dim, rotation_seed, rerank_factor, generation)`**
(`bundle.rs:359-390`). For an autonomous agent, this is the missing
link between "I retrieved memory X" and "I can prove I retrieved
exactly the right memory X, and not a tampered, stale, or rebuilt-
under-different-codes copy".

Two consequences flow from that one design choice:

1. **Cross-process / cross-agent cache sharing is automatic.** Two
   agents pointing at the same `data_ref`, same seed, same
   `rerank_factor`, same generation produce the same witness, and the
   cache deduplicates them into one compressed entry with `refcount =
   2` (`cache.rs:202-205`, `:378-383`; verified by
   `two_backends_share_cache_when_witness_matches` in
   `crates/core/tests/federation_smoke.rs:242` per
   `docs/review/capabilities.md` §2/#11). For a swarm of N agents on a
   single host, this is N× memory reduction at zero coordination cost.
2. **Forensic chain-of-custody is a one-line check.** A regulator or
   debugging operator asks "what memory did agent A act on at time
   T?" — the answer is the witness recorded in the agent's audit log,
   recomputable byte-for-byte against the on-disk bundle. If the
   re-computation matches, the agent acted on un-tampered data. If it
   doesn't, the bundle was rotated under the agent's feet (or the log
   was edited).

Both properties are purely structural — they fall out of the
substrate's design without an "audit mode" flag or a separate logging
plane. You get them by using `search_one`.

### 1.4 The three-mode consistency knob is agent-shaped

Most vector systems pick one consistency story and force every
workload into it. ruLake exposes three (`cache.rs:53-77`):

```rust
pub enum Consistency {
    /// Consult the backend's current bundle on every search.
    Fresh,
    /// Trust the cache for up to ttl_ms milliseconds between checks.
    Eventual { ttl_ms: u64 },
    /// Caller asserts the bundle is immutable for the cache's lifetime.
    Frozen,
}
```

Maps cleanly to three agent-runtime profiles:

| Mode | Agent profile | Why |
|---|---|---|
| `Fresh` | Compliance / payments / agents acting on regulated facts | Every recall is verified against backend; no stale answer is acceptable. The 1.02× hit-path tax (`docs/review/performance.md` §8) means you pay almost nothing for the safety. |
| `Eventual { ttl_ms: 60_000 }` | Conversational / RAG / tool-routing agents | A 60-second freshness window is invisible to the user, and the witness check is skipped (`cache.rs:867-874`) for that whole window. This is the profile that gets the real QPS lift from cache. |
| `Frozen` | Replay / forensics / agents auditing a sealed snapshot | First query primes; every subsequent query serves out of the installed entry without ever touching the backend (`cache.rs:880-883`). The ideal mode for a "what would agent X have done given memory snapshot S" replay. |

This is unusual for a substrate. Most "you choose your consistency"
systems offer two modes (read-your-writes vs eventual). The third
mode — Frozen-for-audit — is specifically valuable to agentic AI
because **agents need replayability**, and replay is impossible if the
substrate keeps refreshing under you.

### 1.5 Federation for "knowledge spread across N silos"

Real agents do not retrieve from one corpus. A coding agent searches
its codebase, the language documentation, the team's design docs, and
its own prior trajectories. A customer-support agent searches the
product knowledge base, the user's history, the policy manuals, and
the live ticket queue. Each of these is a different vector store with
a different freshness story.

`RuLake::search_federated` (`lake.rs:491-543`) takes a slice of
`(backend, collection)` targets, runs rayon-parallel fan-out, and
merges by score with adaptive per-shard rerank
(`max(MIN_PER_SHARD_RERANK, global / K)` per `lake.rs:474, :512-519`)
plus per-shard over-request `k' = k + ⌈√(k·ln S)⌉` (`lake.rs:553-560`).
The recall floor is gated by
`adaptive_per_shard_rerank_preserves_recall` at recall ≥ 0.85 across
K∈{2,4} (per `docs/review/capabilities.md` §2/#18).

For agentic AI this is the difference between "I have to write a
custom router that fans out to four vector services and merges
heuristically" and "I write `lake.search_federated(&[...], q, k)` and
get correct top-K with documented recall properties." The router lives
in the substrate, not in every agent's prompt.

### 1.6 The one-line summary

ruLake matters to agents because it is the only published vector-
substrate (as of 2026-04) that simultaneously offers:

- 1.00–1.03× hit-path overhead vs raw library (`BENCHMARK.md`,
  cross-checked in `docs/review/performance.md` §8);
- content-addressed dedup across processes (`cache.rs:378-383`);
- three explicit consistency modes (`cache.rs:53-77`);
- federation with adaptive rerank (`lake.rs:491-560`);
- snapshot-and-warm-restart (`lake.rs:263-441`);
- zero `unsafe` (`docs/review/security.md` §6);
- bundle-protocol publish/refresh as a 300-byte sidecar
  (`bundle.rs:113-156`).

Every other property an agent operator wants — embeddings, scoring,
reward modeling, contradiction detection, summarization — is
explicitly **not in ruLake** (ADR-156 §Decision §A; §9 below). Brain
authors get a stable substrate they can build on.

---

## 2. Vertical 1 — Agent memory hierarchies (per ADR-156)

### 2.1 What ADR-156 actually claims

ADR-156 is the document that names ruLake as a *substrate* for agent
brain memory. It does **not** claim ruLake is the brain — explicitly:

> An agent brain system (the hypothetical consumer) owns:
>
> - Memory *type* semantics — episodic, semantic, procedural, identity,
>   policy, observation. These are cognitive labels; the substrate stores
>   them as opaque strings if asked, but never interprets them.
> - Recall *policy* — which candidates matter, how to combine vector
>   similarity with graph neighborhood / recency / trust / contradiction
>   / mincut boundary.
> - Mutation *policy* — when to write, merge, delete, compact, rehydrate.
>
> ruLake owns (and already ships as of ADR-155 M1):
>
> | Brain-system concern      | ruLake primitive                                  |
> |---------------------------|---------------------------------------------------|
> | Hot memory                | `VectorCache` + RaBitQ codes (1.02× tax)          |
> | Warm memory               | `BackendAdapter::pull_vectors` + `RuLakeBundle`   |
> | Cold memory               | Backend-adapter contract (Parquet/GCS/BQ/...)     |
> | Freshness contract        | `Consistency::{Fresh, Eventual, Frozen}`          |
> | Witnessed state           | SHAKE-256 witness over the bundle                 |
> | Cross-process handoff     | Sidecar protocol: `publish_bundle` / `refresh_*`  |
> | Observability             | `CacheStats::{hit_rate, avg_prime_ms, last_prime_ms}` |
> | Multi-tier eviction       | LRU cap over unpinned entries                     |

The six guarantees ADR-156 derives from this — recall, verify, forget,
compact (deferred), rehydrate, location-transparency — are the
substrate acceptance test. Five of six are shipped and tested:
`brain_substrate_acceptance_recall_verify_forget_rehydrate` in
`crates/core/tests/federation_smoke.rs:766` (cited in `docs/review/capabilities.md`
§3 ADR-156). Compact is explicitly out of scope per ADR-156.

### 2.2 Mapping the six guarantees to a real agent loop

A working cognitive architecture (RVM, LangGraph, LlamaIndex Agents,
or claude-flow's hive-mind) typically structures memory as:

- **Episodic** — what happened in this session / trajectory. High
  write rate, low recall rate, short half-life. Each entry is one
  step in a trajectory.
- **Semantic** — facts the agent has learned and consolidated.
  Mid-write, high-recall, long half-life. The "knowledge graph" of
  the agent.
- **Procedural** — skills, tool-use patterns, prompt templates that
  worked. Low-write (only consolidated patterns get in), high-recall,
  effectively immutable once consolidated.
- **Identity / policy** — who the agent is, what it cannot do, what
  its long-running goals are. Effectively immutable; the few mutations
  go through proof-gated review.
- **Observation** — sensor / tool-output stream, the rawest layer
  before episodic consolidation. Highest write, lowest recall.

Map each to ruLake:

| Memory type | Backend | Consistency | Bundle `memory_class` | Notes |
|---|---|---|---|---|
| Episodic | `LocalBackend` (in-process) or per-session `FsBackend` | `Eventual { ttl_ms: 1_000 }` | `"episodic"` | High write rate; short TTL because new episodes replace old quickly. The brain owns the consolidation policy. |
| Semantic | `FsBackend` (per-agent) or shared backend (per-tenant) | `Eventual { ttl_ms: 60_000 }` | `"semantic"` | Mid TTL; published bundles let cross-agent sharing happen. |
| Procedural | Read-only `FsBackend` | `Frozen` | `"procedural"` | Skills are immutable once consolidated. Frozen means no backend round-trip ever — pure cache serve. |
| Identity / policy | Read-only `FsBackend` with operator-published bundle | `Frozen` | `"identity"` / `"policy"` | Same shape as procedural; rotation requires operator signature out-of-band. |
| Observation | High-throughput `LocalBackend` with eager LRU eviction | `Eventual { ttl_ms: 100 }` | `"observation"` | LRU cap is non-trivial here; brain consolidates into episodic before eviction. |

The `memory_class` field is shipped on `RuLakeBundle`
(`bundle.rs:144-155`); the substrate stores the string and surfaces it
through stats and bundle inspection but never interprets it
(`memory_class_roundtrips_and_does_not_affect_witness` in
`bundle.rs:571`). Two bundles with identical underlying vectors but
different memory-class tags share the same witness and the same cache
entry — the brain can re-classify a memory without invalidating the
substrate.

### 2.3 Code-level pseudocode — a brain on top of ruLake

Below is a sketch of how a hypothetical brain layer (call it
`AgentBrain`) consumes ruLake. The API calls are real — every
`lake.search_one`, `lake.invalidate_cache`, etc. is exactly as defined
in `crates/core/src/lake.rs`. The "brain decisions" — when to consolidate, what's
a contradiction, when to rehydrate — are pseudo-code stand-ins for
whatever cognitive policy the consumer defines.

```rust
use std::sync::Arc;
use rulake::{
    LocalBackend, FsBackend, RuLake,
    cache::Consistency,
    RuLakeBundle, Generation,
};

/// One ruLake instance per agent. Cheap to clone (everything is
/// behind Arc — see lake.rs:48).
struct AgentBrain {
    lake: RuLake,
    agent_id: String,
}

impl AgentBrain {
    fn new(agent_id: String, snapshot_dir: Option<&std::path::Path>) -> Self {
        // rerank_factor=20 hits 100% recall@10 on D=128 clustered data
        // per ruvector-rabitq's BENCHMARK.md (cited in lake.rs:57-60).
        // rotation_seed is shared across all collections so witnesses
        // are deterministic across processes.
        let lake = RuLake::new(20, 0xA6E47_B7A1)
            .with_consistency(Consistency::Eventual { ttl_ms: 60_000 })
            // 256 distinct compressed entries — bounded RAM.
            .with_max_cache_entries(256);

        // Per-agent backends — one each for episodic, semantic,
        // procedural. Identity/policy lives in a shared org-wide
        // backend mounted read-only.
        let episodic = Arc::new(LocalBackend::new(format!("ep-{agent_id}")));
        let semantic = Arc::new(FsBackend::new(
            format!("sem-{agent_id}"),
            format!("/var/agent-mem/{agent_id}/sem"),
        ).expect("fs backend"));
        let procedural = Arc::new(FsBackend::new(
            "procedural",
            "/usr/share/agent-skills",  // org-wide read-only mount
        ).expect("fs backend"));
        let identity = Arc::new(FsBackend::new(
            "identity",
            "/etc/agent-identity",
        ).expect("fs backend"));

        for b in [&episodic as &Arc<dyn _>, &semantic, &procedural, &identity] {
            lake.register_backend(Arc::clone(b)).expect("register");
        }

        // Optional warm-restart: if a snapshot from a prior process
        // exists, install it without backend round-trip.
        if let Some(dir) = snapshot_dir {
            for class in &["semantic", "procedural", "identity"] {
                let key = (class.to_string(), agent_id.clone());
                let class_dir = dir.join(class);
                if class_dir.exists() {
                    // warm_from_dir does NOT require the backend to
                    // be registered — see lake.rs:368-371. Cold-start
                    // becomes O(file-read), not O(pull + RaBitQ
                    // compress).
                    let n = lake.warm_from_dir(&key, &class_dir)
                        .expect("warm_from_dir");
                    println!("warmed {class}: {n} memories");
                }
            }
        }

        Self { lake, agent_id }
    }

    /// Recall: search across all memory types, ranked by L2² score.
    /// The brain's consolidation step decides which class to weight
    /// higher; the substrate just returns ranked candidates.
    fn recall(&self, query: &[f32], k: usize) -> Vec<MemoryHit> {
        // Federated fan-out across the four memory classes.
        // search_federated runs rayon-parallel (lake.rs:527-538) and
        // merges by score; the adaptive per-shard rerank
        // (max(5, global/K) per lake.rs:474+) keeps total rerank
        // budget roughly constant.
        let targets: Vec<(&str, &str)> = vec![
            (&format!("ep-{}", self.agent_id),  &self.agent_id),
            (&format!("sem-{}", self.agent_id), &self.agent_id),
            ("procedural",                       &self.agent_id),
            ("identity",                         &self.agent_id),
        ];
        let hits = self.lake.search_federated(&targets, query, k * 4)
            .expect("search_federated");

        // Brain step (NOT in ruLake): re-rank by recency + trust +
        // contradiction. The substrate handed us 4k candidates with
        // their (backend, collection, id, score). The brain decides
        // what wins.
        self.brain_rerank(hits, k)
    }

    fn brain_rerank(&self, hits: Vec<SearchResult>, k: usize) -> Vec<MemoryHit> {
        unimplemented!("This is the brain's job — substrate-out-of-scope.")
    }

    /// Forget: drop a memory pointer (substrate-level invalidation,
    /// per ADR-156 footnote — NOT cryptographic shred).
    fn forget(&self, class: &str, id: u64) {
        let key = self.key_for(class);
        // invalidate_cache drops the pointer; the underlying entry
        // is GC'd when refcount hits 0 (lake.rs:151-156, cache.rs:611).
        self.lake.invalidate_cache(&key);
        // If hard delete is required, the brain must also tell the
        // backend to delete the underlying data (substrate doesn't
        // own backend mutations).
    }

    /// Rehydrate: explicit pre-warm of a class. After this, the next
    /// query against that class is a hit, not a prime.
    fn rehydrate(&self, class: &str, q_seed: &[f32]) {
        let key = self.key_for(class);
        // search_one transparently primes on miss (lake.rs:445-469
        // → ensure_fresh → prime_interned). We throw away the result;
        // the side effect is the cache prime.
        let _ = self.lake.search_one(&key.0, &key.1, q_seed, 1);
    }

    /// Snapshot: persist the warm cache for warm-restart.
    fn snapshot(&self, dir: &std::path::Path) {
        for class in &["semantic", "procedural", "identity"] {
            // Episodic is intentionally NOT snapshotted — it's
            // session-local and gets consolidated into semantic
            // before the agent dies.
            let key = self.key_for(class);
            let class_dir = dir.join(class);
            std::fs::create_dir_all(&class_dir).unwrap();
            let _ = self.lake.save_cache_to_dir(&key, &class_dir);
            // Snapshot is two files: index.rbpx + table.rulake.json
            // (lake.rs:21-22, :291). ~5 ms for 5k vectors per the
            // warm_restart example.
        }
    }

    fn key_for(&self, class: &str) -> (String, String) {
        match class {
            "episodic"   => (format!("ep-{}", self.agent_id), self.agent_id.clone()),
            "semantic"   => (format!("sem-{}", self.agent_id), self.agent_id.clone()),
            "procedural" => ("procedural".to_string(), self.agent_id.clone()),
            "identity"   => ("identity".to_string(),   self.agent_id.clone()),
            _ => unreachable!()
        }
    }
}

struct SearchResult { /* per lake.rs:26-32 */ }
struct MemoryHit    { /* brain's decorated result */ }
```

Three things to notice in the sketch above:

1. **Every method on `AgentBrain` maps 1:1 to one of ruLake's
   primitives.** No glue layer hides in a thousand lines of brain
   code; the brain is literally five wrapper methods.
2. **`recall` is a federated query, not four sequential ones.** The
   substrate does the rayon fan-out; the brain only sees a ranked
   merged list. Recall floor (≥ 0.85) is enforced by ruLake's tests,
   not the brain's.
3. **`snapshot` and the warm-restart on `new`** mean a freshly-spawned
   replacement agent process is serving in milliseconds, not seconds.
   For agent platforms that auto-restart on crash, this is the
   difference between "noticeable downtime per restart" and
   "imperceptible".

### 2.4 Where the substrate ends and the brain begins

The "brain decisions" the pseudocode delegates to brain code, all of
which ADR-156 explicitly leaves out of substrate scope:

- **Consolidation:** when does an episodic event become semantic? The
  brain decides; the substrate stores whatever the brain primes into
  the semantic backend.
- **Contradiction scoring:** when two memories disagree, which wins?
  The brain decides. The substrate returns both ranked by L2² score.
- **Trust weighting:** memory from a high-trust source vs low-trust?
  The brain decides; the substrate doesn't know what "trust" means.
- **Compaction:** when do you merge near-duplicates and re-prime? Per
  ADR-156 §Decision §3, "Compaction belongs to RVM / Cognitum, not
  ruLake." The brain owns the schedule; the substrate's primitives are
  `invalidate_cache` and `prime` (transparent on next miss).
- **Mincut / graph-walk recall:** vector similarity is one signal
  among many. The brain weaves them; the substrate returns the
  ranked vector candidates only.

ADR-156 §Decision Alternative-A explicitly rejects absorbing these
into ruLake:

> Make ruLake own memory classification, recall policy, contradiction
> scoring, mincut-based routing. Rejected: violates the substrate
> separation. A cache-first execution fabric does not know what
> "episodic" means; if it does, it has stopped being a substrate.

The discipline matters because brain semantics evolve fast (see how
much the agent-research literature has moved 2023→2026); the
substrate's six guarantees do not. By keeping them separate, the
brain can be rewritten without touching the cache.

### 2.5 The "forget" gap

ADR-156 is honest about one limitation: substrate-level forget is a
pointer drop, not a crypto-shred:

> "Forget" in the full brain sense (crypto-shred the underlying bytes)
> stays as the ADR-155 GDPR follow-up; substrate-level forget is the
> cache pointer drop + invalidation, which is sufficient for the
> agent's recall semantics.

For an agent operator with strict GDPR / right-to-be-forgotten
requirements, this means: when a user requests deletion, the agent
must (a) call `lake.invalidate_cache(key)` to drop the substrate
pointer, AND (b) trigger backend-level deletion (a distinct concern
the substrate does not own). The combination is sufficient; either
alone is not.

This is consistent with ADR-155's roadmap (M4 governance) but worth
restating in any real production deployment plan.

### 2.6 Why this is the highest-leverage vertical

Of the seven verticals in this paper, agent memory hierarchies is the
one ruLake was *literally designed for* — ADR-156 exists precisely to
record this fit. The 1.02× tax means the brain pays effectively
nothing for the substrate; the witness chain means cross-agent sharing
is automatic; the warm-restart loop means agent-process churn is
cheap; the federated fan-out means cross-class recall is one call.

If you are building an agent framework today and you do not already
have a working memory substrate, this is the place to start.

---

## 3. Vertical 2 — Multi-agent swarms with shared world model

### 3.1 The shared-world-model problem

A swarm of N agents working on the same task — claude-flow's
hive-mind, AutoGen's group-chat, CrewAI's crew, Microsoft's Magnetic-
One — typically share read-mostly context: a project codebase, a
knowledge base, a shared scratchpad of "what we've decided so far".
Without a coherence story, each of N agents loads its own copy.
Memory cost is N× the corpus size; coherence is "we tell each agent to
re-load" (which is brittle).

ruLake's content-addressed cache (`cache.rs:202-205`,
`cache.rs:378-383`) makes this trivial. Two backends pointing at the
same `data_ref` produce the same witness; the cache deduplicates them
into one compressed entry. For N agents on the same host, the
underlying RaBitQ codes are stored once.

### 3.2 The witness equation

`bundle.rs:362-390` defines the witness as SHAKE-256(32) over
`(data_ref, dim, rotation_seed, rerank_factor, generation)`. The
rotation_seed is set once per `RuLake` instance (`lake.rs:61-67`); the
data_ref / dim / generation are properties of the source data; the
rerank_factor is a deployment-wide setting.

So for a swarm on a single host, if all N agents construct
`RuLake::new(20, SAME_SEED)` with the same `rerank_factor`, and all
register a backend whose `current_bundle` returns the same `data_ref`
and `dim`, then they all compute the same witness, and the cache
deduplicates.

The "different backend object, same data" case is exactly what
`two_backends_share_cache_when_witness_matches` tests (cited
`docs/review/capabilities.md` §2/#11): two `LocalBackend`s with
identical data but different `id`s that both override `current_bundle`
to emit the same `data_ref` share one `CacheEntry` with `refcount =
2`.

### 3.3 Worked example — 8-agent claude-flow swarm

Consider a claude-flow-style swarm: 8 agents (1 coordinator + 7
workers) on a single host, working on a software-engineering task. All
8 share access to:

- The repository codebase (≈ 50 K vectorized chunks, D=768)
- The test corpus (≈ 5 K vectorized failing-test snapshots, D=768)
- The team's design docs (≈ 2 K vectorized markdown chunks, D=768)
- The agent-pattern library (≈ 1 K vectorized prior-trajectory
  embeddings, D=768)

Each agent also has its own private scratchpad of the current
trajectory — that one is per-agent, no sharing.

Without ruLake: each of 8 agents loads its own RaBitQ index over each
shared corpus. At 1-bit codes that's `(50 + 5 + 2 + 1) * 1024 / 8
bytes/code = ~7.4 KB per vector × 58 K vectors = ~430 MB per agent ×
8 = 3.4 GB total** for the shared part alone, plus rerank vectors
(`rerank_factor` × n × D × 4 bytes ≈ a lot more).

With ruLake: each agent's RuLake instance points at the same
`(data_ref, seed, rerank_factor)` for each shared corpus. The cache
sees the same witness from all 8 agents and stores one compressed
entry per corpus. The shared-cache memory cost is ~430 MB total —
**8× reduction**.

Code:

```rust
use std::sync::Arc;
use rulake::{LocalBackend, RuLake, cache::Consistency};

/// One ruLake per agent. The shared backends are constructed once
/// at the swarm level and Arc-shared into each agent's lake.
struct Swarm {
    /// Shared, read-mostly. One Arc per backend, cloned into each
    /// agent's RuLake.
    code_backend:    Arc<dyn rulake::BackendAdapter>,
    test_backend:    Arc<dyn rulake::BackendAdapter>,
    design_backend:  Arc<dyn rulake::BackendAdapter>,
    pattern_backend: Arc<dyn rulake::BackendAdapter>,
    /// One per agent. Each gets its own scratchpad backend.
    agents: Vec<AgentLake>,
}

struct AgentLake {
    id: String,
    lake: RuLake,
    /// Private to this agent.
    scratchpad: Arc<dyn rulake::BackendAdapter>,
}

impl Swarm {
    fn new(n_agents: usize) -> Self {
        // Build the shared backends once. In a real deployment these
        // would be FsBackend over an mmap'd index file or a real
        // ParquetBackend (M2 roadmap, not in ruLake yet).
        let code    = Arc::new(LocalBackend::new("code"))    as Arc<dyn _>;
        let test    = Arc::new(LocalBackend::new("test"))    as Arc<dyn _>;
        let design  = Arc::new(LocalBackend::new("design"))  as Arc<dyn _>;
        let pattern = Arc::new(LocalBackend::new("pattern")) as Arc<dyn _>;

        // ... (load corpora into each via put_collection — out of scope) ...

        let agents = (0..n_agents).map(|i| {
            let id = format!("agent-{i}");
            // CRITICAL: same rerank_factor + same rotation_seed
            // across every agent. This is what makes the shared
            // backends produce the same witness across all 8 lakes.
            let lake = RuLake::new(20, 0xA6E47_B7A1)
                // Eventual mode — TTL chosen so the swarm sees code
                // edits within 5 seconds without paying a coherence
                // check on every query.
                .with_consistency(Consistency::Eventual { ttl_ms: 5_000 });

            // Register the shared backends. They are Arc<dyn>, so
            // each lake just gets a refcount bump.
            for b in [&code, &test, &design, &pattern] {
                lake.register_backend(Arc::clone(b)).expect("register");
            }

            // The per-agent scratchpad is unique per agent.
            let scratchpad = Arc::new(LocalBackend::new(&id)) as Arc<dyn _>;
            lake.register_backend(Arc::clone(&scratchpad)).expect("register");

            AgentLake { id, lake, scratchpad }
        }).collect();

        Self { code_backend: code, test_backend: test, design_backend: design,
               pattern_backend: pattern, agents }
    }

    /// Diagnostic: prove the cache is shared. After all 8 agents have
    /// queried each shared backend at least once, every shared
    /// witness should have refcount = 8 in the cache.
    fn assert_shared(&self) {
        // Pick any agent's lake; the witness is the same across all 8.
        let any_lake = &self.agents[0].lake;

        for collection in &["main", "main", "main", "main"] {
            for backend_id in &["code", "test", "design", "pattern"] {
                let key = (backend_id.to_string(), collection.to_string());
                if let Some(witness) = any_lake.cache_witness_of(&key) {
                    let rc = any_lake.cache_refcount_of(&witness);
                    // 8 agents × 1 pointer per agent per collection
                    assert!(rc >= 1, "shared witness must be installed");
                    println!("witness {witness} refcount = {rc}");
                }
            }
        }

        // Number of distinct compressed entries should equal:
        //   shared backends (4) + per-agent scratchpad (8) = 12
        // not 8 lakes × 5 backends = 40.
        let n_entries = any_lake.cache_entry_count();
        println!("distinct compressed entries: {n_entries}");
        assert!(n_entries <= 12 + 4, "expected ≤16 distinct entries");
    }
}
```

The `cache_witness_of`, `cache_refcount_of`, `cache_entry_count`
methods are real (`lake.rs:134-149`); operators can verify cache
sharing in production by polling these.

### 3.4 What this enables that other approaches don't

1. **Linear-scale memory cost.** The shared portion is constant in N;
   only the per-agent scratchpads scale. For a 64-agent swarm sharing
   10 GB of corpora, this is the difference between 640 GB
   (impossible) and ~10 GB + small per-agent overhead (fits on one
   box).

2. **Coherence is automatic.** When the source data changes (a code
   commit lands, a test corpus is updated), the publisher emits a new
   `table.rulake.json` (`lake.rs:167-179`); each agent's daemon polls
   and calls `refresh_from_bundle_dir` (`lake.rs:200-228`). The next
   query re-primes once and is shared again. No "tell every agent" RPC.

3. **Cross-agent provenance.** When agent-3 says "I retrieved memory
   X", any other agent can verify by recomputing the witness from the
   same `(data_ref, seed, rerank_factor, generation)` and checking
   that agent-3's recorded witness matches. This gives the swarm a
   primitive for "proof of consistent retrieval".

4. **Free hand-off.** When agent-A finishes a sub-task and hands off
   to agent-B, B does not need to re-load A's working memory — they
   were already sharing the substrate. A passes B the witness; B
   queries the same cache entry.

### 3.5 The thundering-herd consideration

`docs/review/performance.md` §6 B5 flags one scenario where the swarm
case hits a soft limitation:

> N threads simultaneously cache-missing the same `(backend,
> collection)` will all do the full `O(n·D log D)` rabitq build; only
> one wins the post-build race. Wasted CPU on cold restart or after
> explicit `invalidate_cache`.

For an 8-agent swarm starting all at once against a cold cache, all 8
will start the prime work, 1 will finish first, and the other 7 will
discard their work and take the shared entry. The wasted work is
bounded by `O(N × n · D log D)` once at swarm startup. For an 8-agent
swarm primering a 50K × D=768 corpus once at startup, this is
measurable but typically tolerable (tens of seconds wasted CPU at
launch, never again).

The deep review notes (B5) that an "in-flight builds" map would
deduplicate, but it is deferred until measured. Swarm operators with
many agents and frequent cache invalidation should be aware of this;
others can ignore it.

### 3.6 Practical operating profile

For an 8-agent claude-flow swarm:

| Setting | Value | Why |
|---|---|---|
| `RuLake::new(rerank_factor, rotation_seed)` | `(20, FIXED)` per agent | 100% recall@10; same seed → shared witness |
| `with_consistency` | `Eventual { ttl_ms: 5_000 }` | Code edits propagate within 5 s |
| `with_max_cache_entries` | `64` | Bounded RAM, plenty of headroom |
| Backends registered | 4 shared (Arc-cloned) + 1 private | Sharing happens at the witness level, not the backend object level |
| Snapshot dir | per-swarm `/var/swarm/{id}/snapshot/` | Warm-restart on swarm relaunch |
| Sidecar publisher | external process or coordinator agent | Re-publishes when source data rotates |

### 3.7 Why this is the second-highest-leverage vertical

Multi-agent swarms are eating the agentic-AI space (see the
proliferation of frameworks 2024–2026). Every framework needs a
shared-memory story; most have a bespoke one. ruLake offers a
substrate that *is* the shared-memory story by design — no
coordination needed beyond "use the same seed and rerank_factor."

If you build an agent framework with shared world model, this is the
substrate to plug in.

---

## 4. Vertical 3 — Long-running autonomous agents

### 4.1 The crash-survival problem

Long-running autonomous agents — CronCreate-style schedulers,
daemon-mode pair programmers, monitoring agents that run for weeks —
must survive process restarts. The vector store they query is
typically large enough that re-priming from cold on restart costs
seconds per collection. For an agent that touches 10 collections, the
cold-start window is "tens of seconds where the agent is unresponsive
after restart".

Hosted vector DBs have no story for this — they don't ship the warm
index to your process; you re-pay the network round-trip.
In-process libraries (FAISS, hnswlib) have no save-and-warm story
that includes a witness.

ruLake has both, shipped at M1.5
(`docs/review/capabilities.md` §2 claims #13–#16):

- `save_cache_to_dir(key, dir)` (`lake.rs:263-334`): atomic-write
  `index.rbpx` + `table.rulake.json` to a directory.
- `warm_from_dir(key, dir)` (`lake.rs:378-441`): read both files,
  verify the witness, install the prebuilt index. **No backend
  round-trip required** (`lake.rs:368-371`).

Measured: the `examples/warm_restart.rs` demo loads 5K vectors at
D=128 in approximately 5 ms (the demo's published numbers). The
speedup over cold-prime-from-backend depends on backend cost; for an
in-memory `LocalBackend` it's typically 10–50×, for a
real network-backed source it would be ~500–5000×.

### 4.2 The TTL freshness window

For a long-running agent, the trade-off between freshness and QPS is
genuine. A monitoring agent that re-checks every 60 seconds wastes
backend bandwidth; one that re-checks every hour misses incidents.
`Consistency::Eventual { ttl_ms }` (`cache.rs:64`) is the knob.

Suggested TTLs by agent class:

| Agent class | Suggested TTL | Reasoning |
|---|---|---|
| CronCreate-style scheduler (1 trigger/min) | 30 s | TTL << job period; coherence on every job. |
| Daemon-mode pair programmer (interactive) | 5 s | Code edits propagate within 5 s; user-perceived as fresh. |
| Monitoring agent (continuous) | 1 min | Backend update rate << 1 min for most monitored corpora. |
| Knowledge-base agent (RAG) | 5 min | KB churn measured in days; 5-minute window is generous. |
| Replay agent (auditing) | `Frozen` | No coherence ever — explicit refresh only. |

The cache's `can_skip_check_interned` (`cache.rs:862-885`) implements
this: under `Eventual { ttl_ms }`, if `last_checked + ttl > now`, the
backend round-trip is skipped entirely. The witness check is the only
cost being skipped — the cached search itself is unchanged.

### 4.3 Pseudocode — daemon-mode agent with warm-restart

```rust
use std::sync::Arc;
use std::time::Duration;
use rulake::{
    LocalBackend, FsBackend, RuLake,
    cache::Consistency,
    RefreshResult,
};

const SNAPSHOT_DIR: &str = "/var/agent/snapshots";

struct LongRunningAgent {
    lake: RuLake,
    /// Where we save snapshots. On restart, we warm from here.
    snapshot_root: std::path::PathBuf,
}

impl LongRunningAgent {
    /// Cold start: warm from any existing snapshot, register backends.
    /// Hot start (after a crash): same code path; the snapshot dir
    /// either has files or doesn't.
    fn boot(snapshot_root: &std::path::Path) -> Self {
        let lake = RuLake::new(20, 0xDEADBEEF)
            .with_consistency(Consistency::Eventual { ttl_ms: 30_000 })
            .with_max_cache_entries(64);

        let backend = Arc::new(FsBackend::new(
            "kb",
            "/var/data/kb",
        ).expect("fs backend"));
        lake.register_backend(backend).expect("register");

        // Warm from snapshot if present. warm_from_dir does NOT
        // require the backend to be registered (lake.rs:368-371) —
        // we register it anyway because we'll need it for coherence
        // checks once Eventual TTL expires.
        let kb_snapshot = snapshot_root.join("kb");
        let key = ("kb".to_string(), "main".to_string());
        if kb_snapshot.join("index.rbpx").exists() {
            match lake.warm_from_dir(&key, &kb_snapshot) {
                Ok(n) => println!("warm restart: {n} vectors loaded"),
                Err(e) => eprintln!("warm restart failed: {e}; will cold-prime"),
            }
        }

        Self { lake, snapshot_root: snapshot_root.to_path_buf() }
    }

    /// Periodic snapshot loop. Runs in a background thread.
    /// Cheap (~5 ms for 5k vectors per the warm_restart example),
    /// so a 5-minute period is over-engineered safety.
    fn snapshot_loop(self: Arc<Self>) {
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(300));
                let key = ("kb".to_string(), "main".to_string());
                let kb_snapshot = self.snapshot_root.join("kb");
                std::fs::create_dir_all(&kb_snapshot).unwrap();
                match self.lake.save_cache_to_dir(&key, &kb_snapshot) {
                    Ok(p) => println!("snapshot OK: {}", p.display()),
                    Err(e) => eprintln!("snapshot failed: {e}"),
                }
            }
        });
    }

    /// Sidecar daemon: watch for upstream rotations.
    /// Pattern from examples/sidecar_daemon.rs.
    fn coherence_loop(self: Arc<Self>, watch_dir: std::path::PathBuf) {
        std::thread::spawn(move || {
            let key = ("kb".to_string(), "main".to_string());
            loop {
                std::thread::sleep(Duration::from_secs(10));
                match self.lake.refresh_from_bundle_dir(&key, &watch_dir) {
                    Ok(RefreshResult::Invalidated) => {
                        // Cache pointer was dropped; next query
                        // primes (per lake.rs:215-219).
                        println!("upstream rotated; cache invalidated");
                    }
                    Ok(RefreshResult::UpToDate)     => {} // quiet
                    Ok(RefreshResult::BundleMissing) => {
                        // Could be normal at startup before publisher
                        // has emitted; could be an outage.
                        eprintln!("watch dir has no sidecar");
                    }
                    Err(e) => eprintln!("refresh error: {e}"),
                }
            }
        });
    }

    fn query(&self, q: &[f32], k: usize) -> rulake::Result<Vec<_>> {
        // Hot path: 1.02× direct rabitq cost
        // (docs/review/performance.md §1).
        self.lake.search_one("kb", "main", q, k)
    }
}
```

Three operational patterns visible in this sketch:

1. **`boot()` is symmetric for cold vs hot start.** Snapshot dir
   either has files (warm) or doesn't (cold). No special "am I a
   restart?" logic.
2. **The snapshot loop is fire-and-forget.** Cheap enough that
   over-snapshotting is a non-issue; cheap snapshots mean small
   crash-recovery RTO.
3. **The coherence daemon is independent of the snapshot loop.** The
   former handles "upstream changed"; the latter handles "I might
   crash". They don't interact. Both are 10-line patterns from the
   shipped `examples/`.

### 4.4 The `Frozen` mode for forensics

A long-running agent that needs to support "show me what you saw at
13:42 yesterday" can keep historical snapshots. Each snapshot is a
directory pair `(index.rbpx, table.rulake.json)`; mounting one in
`Frozen` mode (`cache.rs:77`) gives byte-exact replay:

```rust
let replay_lake = RuLake::new(20, ORIG_SEED)
    .with_consistency(Consistency::Frozen);
// No backend registered. Frozen never asks the backend.
replay_lake.warm_from_dir(
    &("kb".to_string(), "main".to_string()),
    "/var/agent/snapshots/2026-04-24T13:42:00/kb",
).expect("warm");

// Every query against this lake is byte-exact what the live agent
// saw at that time. Forensic replay.
let hits = replay_lake.search_one("kb", "main", &replay_query, 10)?;
```

This is the audit story without writing audit code. The snapshot pair
+ `Frozen` mode is the forensic primitive.

### 4.5 Operating-profile recommendations

| Knob | Daemon recommendation | Reasoning |
|---|---|---|
| `Consistency` | `Eventual { ttl_ms: 30_000 }` for live, `Frozen` for replay | Trade freshness for QPS in steady state; replay is read-only. |
| `with_max_cache_entries` | 32–256 per lake | Bounded RAM; depends on collection count. |
| Snapshot interval | 5 min | Cheap enough to over-snapshot; small RTO. |
| Coherence-daemon poll period | 10 s | Latency floor for catching upstream rotations. |
| Snapshot retention | 30 days, daily granular | Forensic replay budget. |

### 4.6 What the substrate doesn't give you

A long-running agent operator should still build:

- A snapshot rotation policy (substrate writes the file; you decide
  when to delete old ones).
- A health endpoint exposing `lake.cache_stats().hit_rate()` and
  `last_prime_ms()` for operator alerting.
- A backend-side coherence story — the substrate trusts whatever the
  backend says about generation. A backend with a buggy `generation()`
  silently breaks the freshness story (the witness is correct but
  doesn't update).

These are operator concerns, not substrate concerns, and ADR-155 is
honest about them.

---

## 5. Vertical 4 — Tool-using agent retrieval

### 5.1 The tool-routing problem

Modern agentic systems route across many tools. Anthropic's MCP
ecosystem already has many hundreds of registered servers; an agent
that has access to all of them needs to *select* the right tool for a
query. The selection is itself a vector retrieval problem: each tool
has a description; the query is matched against the descriptions.

At small N (≤ 50 tools), naive embedding-and-rank is fine. At large N
(hundreds–thousands), routing latency dominates. At very large N
(envision a marketplace of 10K MCP servers), you need:

- **Per-tool collection caches.** One tool's description set is one
  collection; the cache primes once and serves forever.
- **Federation across tool collections.** "Which tool can answer this
  question?" is a federated search across all registered tool
  collections.
- **Adaptive freshness.** Tool descriptions rotate slowly (a few times
  per week); `Eventual { ttl_ms: 3_600_000 }` (1 hour) is appropriate.
- **Sharing across agent processes.** A whole MCP-server gateway
  hosting many agents shares the same tool catalog.

### 5.2 Mapping to ruLake

One backend per tool catalog (think: one per registered MCP server),
one collection per tool variant (or one collection per server with one
vector per tool). Then a federated search at query time gives the top
candidates across all servers.

```rust
use std::sync::Arc;
use rulake::{LocalBackend, FsBackend, RuLake, cache::Consistency};

struct ToolRouter {
    lake: RuLake,
    /// All registered MCP servers' tool descriptions.
    servers: Vec<String>,
}

impl ToolRouter {
    fn new() -> Self {
        // Long TTL — tool catalogs rotate slowly.
        let lake = RuLake::new(20, 0xC4FE_BABE)
            .with_consistency(Consistency::Eventual { ttl_ms: 3_600_000 })
            // Bound the cache; an LLM-host serving 1000 tool catalogs
            // doesn't need to keep all of them hot.
            .with_max_cache_entries(512);

        Self { lake, servers: Vec::new() }
    }

    /// Register a tool catalog as a backend. The backend pulls tool
    /// descriptions from wherever they live (a registry, an FS dir,
    /// a remote endpoint).
    fn register_server(&mut self, server_id: String, backend: Arc<dyn _>) {
        self.lake.register_backend(backend).expect("register");
        self.servers.push(server_id);
    }

    /// Route a query: which tools across which servers can answer it?
    /// Federated search returns top-k across every (server, "tools")
    /// pair, ranked by vector similarity.
    fn route(&self, query: &[f32], k: usize) -> Vec<RoutedTool> {
        // Build the federation target list. For 1000 servers this is
        // 1000 (backend, "tools") pairs. The search_federated path
        // (lake.rs:491-543) runs rayon-parallel fan-out; the
        // adaptive per-shard rerank (max(5, global/K) per
        // lake.rs:474+) keeps the rerank budget roughly constant in
        // K. Per-shard over-request k' = k + ceil(sqrt(k * ln(S)))
        // (lake.rs:553-560) adds insurance against data-skew across
        // shards.
        let targets: Vec<(&str, &str)> = self.servers.iter()
            .map(|s| (s.as_str(), "tools"))
            .collect();

        let hits = self.lake.search_federated(&targets, query, k)
            .expect("search_federated");

        hits.into_iter().map(|h| RoutedTool {
            server: h.backend,       // SearchResult.backend (lake.rs:29)
            tool_id: h.id,           // SearchResult.id (lake.rs:31)
            score: h.score,
        }).collect()
    }
}

struct RoutedTool {
    server: String,
    tool_id: u64,
    score: f32,
}
```

### 5.3 The federation cost at scale

The deep review (`docs/review/performance.md` §4 + §6 B4) calls out
that federated fan-out at large K hits two ceilings:

1. Each shard does 3 mutex acquisitions on the global cache state
   (`docs/review/performance.md` §1). At K=1000 shards, that's 3000
   mutex acquisitions per query, all on the same `Arc<Mutex<CacheState>>`.
2. The merge step is `O(K·k log(K·k))` — at K=1000, k=10 that's
   `O(10000 log 10000) ≈ 132K` comparisons. Cheap per query but
   non-trivial.

The first is the next ceiling listed in `docs/review/performance.md`
§6 B1 (sharded cache state); the second is irrelevant until k or K is
much larger than realistic.

**For tool routing at K ≤ 100 servers, ruLake works without
modification.** Beyond that, the operator should:

- Pre-filter the federation target list ("which servers have a tool
  in this category?") so the final fan-out is over ≤ 100 candidates.
- Or wait for the M2+ sharded `CacheState` (§B1 in the perf review).

### 5.4 Sharing tool catalogs across processes

If you run an MCP gateway that hosts many agent processes, each agent
process gets its own `RuLake`. With the same `(rerank_factor, seed)`,
tool catalog backends produce the same witness across all agent
processes — the cache deduplicates.

For a 10-agent gateway with 100 tool catalogs, cache memory is 100
distinct entries (not 10×100=1000), assuming all agents share the
same `RuLake::new` parameters.

The witness chain also gives the gateway a primitive for "all agents
agree on the same tool catalog version." The gateway publishes
`table.rulake.json` for each catalog; agents poll and refresh; every
agent ends up at the same witness or the operator can detect drift.

### 5.5 The bundle as a tool-catalog manifest

The bundle's `lineage_id` (`bundle.rs:140`) is exactly what an MCP
operator wants for tool-catalog provenance:

```json
{
  "format_version": 2,
  "data_ref": "mcp-registry://tools/v3",
  "dim": 768,
  "rotation_seed": 3299402686,
  "rerank_factor": 20,
  "generation": 142,
  "rvf_witness": "9e3a...c104",
  "pii_policy": null,
  "lineage_id": "ol://catalog-build/2026-04-24T07:00",
  "memory_class": "tool-catalog"
}
```

The `data_ref` points at the authoritative catalog; the
`lineage_id` ties the bundle to the catalog-build job that produced
it; the witness anchors all of it. Any agent that retrieved a tool
from this catalog can record `(server_id, tool_id, witness)` as the
audit trail.

### 5.6 Where this falls short

ruLake doesn't help with:

- **Tool descriptions themselves** — you need an embedder upstream
  (sentence-transformers, OpenAI text-embedding-3, voyage-large-2,
  etc.). Substrate doesn't embed.
- **Tool-call serialization / RPC** — substrate just routes to "which
  tool"; the actual call is the agent's job.
- **Tool-output caching** — substrate is for embeddings; caching tool
  outputs is a separate concern.

But for the routing-by-similarity step, this is the cleanest
substrate I've seen.

### 5.7 Why this is the third-highest-leverage vertical

The MCP ecosystem will continue to grow — measured in hundreds of
thousands of tool definitions within a few years if current trends
hold. Existing approaches (load all descriptions into a single FAISS
index per agent) collapse at that scale; ruLake's federated fan-out
with shared witnesses is the only obvious path to "thousands of tool
catalogs, served from one cache, with per-tool freshness."

---

## 6. Vertical 5 — Agentic payments / ledger access

### 6.1 The provenance problem in agentic finance

When an agent acts on a fact — "current account balance is $X",
"counterparty is Y", "exchange rate is Z" — the audit trail must
prove **which version of the fact** the agent saw. Compliance teams
want to be able to ask, three years later, "what data was the agent
acting on at that decision point?"

The witness chain gives an exact answer. Every time the agent calls
`lake.search_one(...)`, the underlying entry has a witness; the
backend's bundle records `(data_ref, dim, rotation_seed,
rerank_factor, generation, rvf_witness, lineage_id)`. The agent can
record the witness alongside the decision in its audit log:

```rust
let hits = lake.search_one("ledger", "balances", &q, 3)?;
let key = ("ledger".to_string(), "balances".to_string());
let witness = lake.cache_witness_of(&key)
    .expect("cache must be primed if we got hits");

audit_log.append(AuditEntry {
    timestamp: now(),
    agent_id: self.id.clone(),
    decision: "transfer-approved",
    retrieved_witness: witness,
    retrieved_ids: hits.iter().map(|h| h.id).collect(),
});
```

Three years later, a compliance officer:

1. Reads `retrieved_witness` from the audit log.
2. Goes to the ledger backend's archive; finds the bundle that has
   matching witness.
3. Loads the corresponding `index.rbpx` snapshot (if the deployment
   archived these alongside the bundle), or re-ingests the underlying
   data and recomputes the witness.
4. Confirms it matches.

If it matches, the agent acted on the data the audit log says it did.
If it doesn't match, either the audit log was edited or the data was
rotated under the agent's feet without re-witnessing. The witness is
the bridge.

### 6.2 The `Fresh` consistency mode

For payments, `Consistency::Fresh` (`cache.rs:60`) is the
right default:

```rust
let lake = RuLake::new(20, 0x1337_C4FE)
    .with_consistency(Consistency::Fresh);
```

Every search consults the backend's `current_bundle` (`lake.rs:638-672`
`ensure_fresh`); the witness is checked against the cached pointer;
if they differ, the cache invalidates and re-primes. The hit-path tax
is still 1.02× because the bookkeeping is cheap; the "stale answer"
window is essentially zero.

The cost: every query incurs a backend round-trip for the coherence
check. For a real Parquet-on-GCS or BigQuery backend, this is 10–100
ms per query (per ADR-155 §"Strict freshness, or 10× throughput").
For an in-process or local backend it's microseconds.

For payments, this is the correct trade. **You do not want to
approve a $1M transfer based on a 5-minute-old balance.**

### 6.3 Caveat: substrate-level forget is not crypto-shred

Repeating from §2.5: when a customer requests deletion, the agent
must (a) call `lake.invalidate_cache(key)` to drop the substrate
pointer, AND (b) trigger backend deletion. The substrate pointer drop
removes the cached compressed codes; the backend bytes are still on
disk until the backend deletes them. For full GDPR-style forget, both
steps are required.

This is consistent with ADR-156 §Consequences ("Negative") and
ADR-155 §"Non-goals" ("Not GDPR-compliant out of the box. v1
supports phase-1 logical delete...").

### 6.4 What the `lineage_id` field gives the auditor

The bundle field `lineage_id: Option<String>` (`bundle.rs:140`) is
specifically intended to map to OpenLineage / W3C PROV (per
`bundle.rs:34-46` and §7 below). For a payments deployment:

```json
{
  "data_ref": "iceberg://lake/finance/balances",
  "dim": 384,
  "rotation_seed": 0xCAFE,
  "rerank_factor": 20,
  "generation": "01JCX7NK6G5R9G1YZ7QH",
  "rvf_witness": "f1c2...3e7a",
  "pii_policy": "pii://policies/finance/v3",
  "lineage_id": "ol://jobs/balance-snapshot/2026-04-25T08:00:00Z",
  "memory_class": "ledger"
}
```

The compliance officer joins the audit log to OpenLineage at
`lineage_id`, traces the upstream data flow, sees which source system
contributed which row, etc. The witness is the cryptographic anchor
between "what the agent saw" and "what the data pipeline produced".

### 6.5 What ruLake doesn't help with

For agentic payments, ruLake gives provenance but does *not* give:

- Authentication of the agent itself ("did agent-A really make this
  call?"). The substrate trusts the caller; agent-identity is the
  caller's concern.
- Authorization of the action ("was agent-A allowed to transfer
  $1M?"). RBAC is M4 roadmap (`docs/review/capabilities.md` §3 ADR-155).
- Idempotency ("did agent-A retry and double-debit?"). Idempotency is
  the application's concern.
- Cryptographic attestation that the agent's *inference* over the
  retrieved data was correct. Substrate proves the retrieval; not the
  reasoning.

These are all real concerns for agentic payments, none of which
ruLake claims to solve. The substrate's contribution is the
"retrieval is provable" piece.

### 6.6 An honest verdict on the leverage

Of all seven verticals, this one is the most speculative. Agentic
payments are still a small market, and the provenance story ruLake
provides is one of several you'd need (alongside attestation, RBAC,
idempotency). The witness chain is *necessary* but not *sufficient*.

I'd recommend this vertical only for deployments where (a) the
provenance gap is identified as a blocker and (b) the operator is
willing to build the rest of the compliance stack on top.

---

## 7. Vertical 6 — Federated LLM RAG with provenance

### 7.1 The cross-silo RAG problem

Most production RAG pipelines today are single-corpus: one vector DB,
one embedding model, one retrieval call. Real organizations have
their knowledge spread across silos — wikis, document stores, ticket
systems, code repositories, design docs in different formats. Each
silo has its own freshness story; combining them is hand-rolled.

`RuLake::search_federated` (`lake.rs:491-560`) gives you cross-silo
RAG as a single primitive. Each silo is one `BackendAdapter`; the
fan-out is rayon-parallel; the per-shard rerank is adaptive; the
per-shard over-request handles data skew across silos.

### 7.2 Adaptive per-shard rerank — why it matters

The mathematical insight at `lake.rs:474, :512-519`:

> A K-shard federated search paid K× the rerank cost because each
> shard reranked its own `rerank_factor × k` candidates. The adaptive
> default keeps the total pre-merge rerank budget roughly constant in
> K while relying on the merge step to produce the globally correct
> top-k.

Concretely, for a 4-shard federated search with `rerank_factor=20`
globally, each shard reranks `max(5, 20/4) = 5` candidates locally.
The merge step (`lake.rs:539-541`) then resorts the 4 × k results
and truncates to global top-k.

The recall floor under this scheme is enforced by
`adaptive_per_shard_rerank_preserves_recall` at recall ≥ 0.85 across
K∈{2,4} (per `docs/review/capabilities.md` §2/#18). For RAG, this is
acceptable — recall@10 ≥ 0.85 across federated silos is far better
than typical "round-robin across N retrievers" baselines.

### 7.3 Per-shard over-request — the SOTA touch

Added 2026-04-23: `lake.rs:553-560` implements
`k' = k + ⌈√(k · ln S)⌉`, clamped to `[k, 4k]`. The 2024–2025
federated-ANN literature (SPIRE, HARMONY, OpenSearch's recall guide)
established this as the folklore rule for handling data skew across
shards — one shard might disproportionately hold true top-K hits, so
asking each shard for slightly more than `k/S` covers the case
without inflating rerank cost.

For K=4, k=10, this gives k' = 13 per shard. For K=16, k=10, k' = 16
per shard. A free recall lift over naive "k per shard" fan-out.

### 7.4 The `lineage_id` / OpenLineage handoff

For RAG with provenance, the per-bundle `lineage_id` field
(`bundle.rs:140`) is the join key into OpenLineage. Each retrieval the
agent makes records:

```rust
let hits = lake.search_federated(&[
    ("wiki",     "main"),
    ("docs",     "main"),
    ("tickets",  "main"),
    ("code",     "main"),
], &query, 10)?;

// Per-hit provenance: the bundle's witness + lineage_id.
for hit in &hits {
    let key = (hit.backend.clone(), hit.collection.clone());
    let witness = lake.cache_witness_of(&key);
    // The lineage_id lives on the bundle, accessed via
    // backend.current_bundle(...). Caching it per-key in the
    // application is the typical pattern.
    audit_log.record(RagHit {
        agent_id: self.id.clone(),
        query_hash: hash(&query),
        retrieved_id: hit.id,
        backend: hit.backend.clone(),
        score: hit.score,
        witness,
    });
}
```

Three years later, the auditor joins the audit log to OpenLineage at
`lineage_id` and traces the full upstream data flow per retrieval.

### 7.5 Per-silo memory_class — the cognitive overlay

For an enterprise RAG deployment, each silo has a different
trust/freshness profile. The `memory_class` tag (`bundle.rs:144-155`)
lets the application categorize:

| Silo | `memory_class` | Trust | Freshness |
|---|---|---|---|
| Wiki | `"semantic-edited"` | High (curated) | Mid (edits propagate hourly) |
| Docs | `"semantic-source"` | High (versioned releases) | Low (changes per release) |
| Tickets | `"observation"` | Variable (user-generated) | High (real-time) |
| Code | `"semantic-source"` | High (commits versioned) | Mid (commits propagate per-commit) |

The substrate stores the tag and surfaces it; the application's
re-ranker uses it to weight hits. ruLake doesn't interpret it (per
ADR-156).

### 7.6 What about rate-limits and per-silo quotas?

Out of scope for ruLake. The substrate gives you parallel fan-out;
rate-limiting is the backend's concern (and historically the
application's). For a federated RAG deployment, the operator should:

- Implement rate-limiting in the `BackendAdapter::pull_vectors` impl
  for each silo.
- Use `with_max_cache_entries` to bound memory across all silos.
- Monitor `cache_stats_by_backend()` (`lake.rs:117-120`) to identify
  hot / cold silos.

### 7.7 The recall-stability story

Most "federate across N retrievers" approaches lose recall as N grows
(the merge step misses globally-best hits because each retriever was
only asked for k/N). ruLake's adaptive per-shard rerank +
over-request keeps recall ≥ 0.85 at K ≤ 4 (tested) and theoretically
at higher K (the formula extends). This is the substrate-level
guarantee that lets you confidently federate RAG across many silos.

### 7.8 Why this matters for agents specifically

Agent-driven RAG is the dominant production use case for vector
search today. Every agent framework that ships a "search the
knowledge base" tool is doing this. ruLake gives operators a substrate
that handles the cross-silo coordination problem natively — instead of
each framework re-implementing federation badly, they can mount
ruLake under the existing tool definition.

For an agent-platform vendor, this is an obvious component to adopt.

---

## 8. Vertical 7 — Reflection / experience replay (ReasoningBank-style)

### 8.1 The trajectory-storage problem

ReasoningBank (and similar systems: AgentDB's pattern store, OpenAI's
Operator memory, etc.) records agent trajectories as vectors and
retrieves them for reflection / replay / experience-driven learning.
The substrate needs:

- High write rate during a session (every step is a write).
- Low write rate after consolidation (one merged trajectory after
  reflection).
- Recall by "similar past situation" (vector query).
- Controlled forgetting — old trajectories age out.
- Cross-session sharing (other agents learn from my trajectories).

ruLake's `Eventual { ttl_ms }` mode + bounded LRU (`with_max_cache_entries`)
+ federated cross-session search are the substrate primitives for
this.

### 8.2 The mapping

| ReasoningBank op | ruLake primitive | Source |
|---|---|---|
| Append a trajectory step (vector) | Backend-level write + `lake.search_one` to trigger re-prime | Backend `pull_vectors` is the substrate hook; the substrate doesn't write to backends. |
| Recall similar past situations | `lake.search_one` or `lake.search_federated` across session collections | `lake.rs:445`, `:491` |
| Forget old trajectories | `lake.invalidate_cache(key)` + LRU on cache + backend deletion | `lake.rs:154`, `cache.rs:548-565` |
| Cross-session search | `search_federated` across all session-collections | `lake.rs:491-543` |
| Snapshot a session for replay | `save_cache_to_dir` + `Frozen` mode for replay | `lake.rs:263`, `cache.rs:77` |

Note: ruLake **does not write to backends**. ReasoningBank's append
path goes through a backend that owns the storage; the substrate
caches the read side. This is consistent with ADR-156 §Decision §3:

> ruLake v1 is read-optimized and append-only; writes go through RVF
> ingest, not through ruLake.

For ReasoningBank specifically, the application owns "write a
trajectory step to the backend"; ruLake handles "the next read sees
the new step after coherence check". The bundle's `generation` field
bumps on every backend write; the cache's `ensure_fresh`
(`lake.rs:638-672`) detects the bump and re-primes.

### 8.3 The TTL trade-off for trajectories

For an active session, recent trajectories should be visible quickly
(low TTL); for the historical archive, slower freshness is fine
(higher TTL). Two operating modes:

```rust
// Active-session lake: recent trajectories visible within 1 second.
let active_lake = RuLake::new(20, SEED)
    .with_consistency(Consistency::Eventual { ttl_ms: 1_000 })
    .with_max_cache_entries(64);

// Archive lake: historical trajectories, hourly freshness.
let archive_lake = RuLake::new(20, SEED)
    .with_consistency(Consistency::Eventual { ttl_ms: 3_600_000 })
    .with_max_cache_entries(256);
```

These can be the same `RuLake` instance with different `Consistency`
settings per backend — except that `Consistency` is a `RuLake`-wide
setting today (`lake.rs:69-73`). For now, use two `RuLake` instances.
A future per-collection consistency override is in ADR-155 "Open
Questions" §"Consistency SLA" (deferred; no customer has asked).

### 8.4 The `Frozen` mode for replay

For a "replay this session" workflow:

```rust
// Snapshot the session at end-of-session.
active_lake.save_cache_to_dir(
    &("sessions".to_string(), session_id.clone()),
    &snapshot_dir.join(&session_id),
).expect("save");

// Later: rehydrate for replay.
let replay_lake = RuLake::new(20, SEED)
    .with_consistency(Consistency::Frozen);
replay_lake.warm_from_dir(
    &("sessions".to_string(), session_id.clone()),
    &snapshot_dir.join(&session_id),
).expect("warm");

// Every replay query is byte-exact what the original session saw.
```

This is particularly valuable for ReasoningBank-style systems because
the *learning signal* often comes from comparing what the agent saw
vs what it should have done in retrospect. Byte-exact replay is the
only way to make that comparison fair.

### 8.5 Cross-session learning

For an agent that learns from its own past sessions, the federated
search across many session collections is the substrate primitive:

```rust
let session_ids = list_recent_sessions(); // e.g. last 30 days
let targets: Vec<(&str, &str)> = session_ids.iter()
    .map(|id| ("sessions", id.as_str()))
    .collect();

let hits = lake.search_federated(&targets, &current_situation, 10)?;
// `hits` is the top-10 most-similar past situations across all
// recent sessions. The brain decides what to do with them.
```

Adaptive per-shard rerank (per `lake.rs:474+`) keeps the rerank cost
bounded as session count grows; the per-shard over-request
(`lake.rs:553-560`) handles the data skew where one session
disproportionately holds similar situations.

### 8.6 Bounded forgetting via LRU

For an archive of N sessions where N grows unbounded, the `with_max_cache_entries(M)`
cap (`lake.rs:78-87`) enforces "at most M distinct compressed entries
in the cache." Sessions that haven't been queried recently get evicted
(LRU on `last_used`, `cache.rs:218`); the underlying backend bytes
remain (substrate doesn't own backend deletion).

This is "soft forgetting" — the substrate forgets the cached codes,
the backend retains the source. A subsequent query against an evicted
session re-primes from the backend. For "hard forgetting", the
operator deletes from the backend after `lake.invalidate_cache(key)`.

### 8.7 The missing pieces ruLake doesn't provide

For a complete ReasoningBank-style system, the application still
owns:

- **Trajectory schema** — what's a "step"? What's a "trajectory"?
  Substrate sees vectors only.
- **Reward / verdict** — was a trajectory good or bad? Substrate
  doesn't score.
- **Distillation** — how do you merge similar trajectories into a
  pattern? Substrate stores; the brain consolidates.
- **Pattern storage vs trajectory storage** — substrate is one cache;
  the brain partitions.
- **EWC++ / continual learning regularization** — substrate doesn't
  know what "important to remember" means.

These are exactly the brain-vs-substrate boundary ADR-156 draws.

### 8.8 Why this is a moderate-leverage vertical

ReasoningBank-style systems are still niche (AgentDB, claude-flow's
ReasoningBank, a few research prototypes). The fit with ruLake is
clean — the substrate's primitives match the application's needs —
but the market is smaller than e.g. tool routing or memory
hierarchies. I'd rate this leverage 4/5: high technical fit, smaller
addressable surface.

---

## 9. Reality check — what ruLake explicitly does *not* do for agents

This section is the honest counterweight to the seven verticals
above. ruLake is a substrate; the README has at points oversold
features that actually live in `ruvector-rabitq` or are M2+ roadmap
(`docs/review/capabilities.md` §6). For agentic AI specifically, here
is what ruLake does **not** give you:

### 9.1 No embedding generation

ruLake takes vectors as input and gives vectors back. It does not
embed text → vector. Operators bring their own embedder
(sentence-transformers, OpenAI text-embedding-3, voyage-large-2,
ONNX-via-`ruv-embedder`, whatever). The cache is downstream of the
embedder.

### 9.2 No summarization or compaction

ADR-156 §Decision §3 is explicit: "Compaction belongs to RVM /
Cognitum, not ruLake." If the agent has 10,000 episodic memories that
should be merged into 100 semantic memories, the consolidation logic
lives in the brain, not the substrate. The substrate exposes
`invalidate_cache` (drop pointer) and primes on the next read; the
schedule and merging are the brain's job.

### 9.3 No scoring beyond L2² distance

`SearchResult.score` (`lake.rs:31`) is the estimated L2² distance
from the RaBitQ scan + rerank. There is no inner-product scoring, no
cosine, no learned scoring head, no contradiction scoring, no recency
bias. The substrate returns "vector neighbours by L2² distance."
Anything beyond that is the brain's job.

For agents that need cosine similarity, the standard workaround is
to L2-normalize vectors before insertion; cosine becomes L2² up to a
constant. For inner-product, this is harder and would require a
substrate-level change.

### 9.4 No reward modeling or trajectory verdicts

ReasoningBank-style verdicts ("was this trajectory good?"), reward
signals from RL (PPO, DPO, etc.), or any kind of supervised signal —
none of these are substrate concerns. The substrate stores vectors;
the application decides which vectors deserve to live.

### 9.5 No procedural skill learning

ruLake stores compressed vector codes for retrieval; it does not
learn skills. A "procedural memory" in the agent-system sense is a
prompt template + tool-call pattern that worked; the substrate stores
the embedding of that pattern, but the *learning* of "this pattern
worked" is outside scope.

### 9.6 No write path

ruLake v1 is read-optimized and append-only on the read side. To add
data, you write to the backend (e.g. via `LocalBackend::append`) and
the next coherence check sees it. There is no `lake.insert(...)`
method — by design, per ADR-156 §Decision §3.

For an agent that needs high-write workloads (each step is a write),
the application owns the backend and the substrate caches the read
side. This is a deliberate boundary; it keeps the substrate small
and the consistency story clear.

### 9.7 No wire-level auth

`docs/review/security.md` §1 is explicit: "Wire-level auth (no
HTTP/gRPC layer ships)." ruLake is a Rust crate, not a server. If you
want a remote ruLake service, you wrap it in your own RPC layer with
auth. Per `docs/review/security.md` §2/I2, the publish/refresh sidecar
protocol does not authenticate the publisher either — filesystem ACLs
are the answer today.

### 9.8 No GDPR-compliant crypto-shred

Per ADR-155 §Non-goals: "Not GDPR-compliant out of the box. v1
supports phase-1 logical delete with 30-day phase-2 backend delete.
Crypto-shredding (same-day) is v2." For an agentic system that
processes PII subject to right-to-be-forgotten, the substrate's
`invalidate_cache` is *necessary* but not *sufficient*; the backend
must also crypto-shred.

### 9.9 No Parquet, BigQuery, Iceberg, Delta, or Snowflake backends

`docs/review/capabilities.md` §4: "Every M2+ adapter is roadmap, not
code." Today only `LocalBackend` and `FsBackend` ship. For an agent
deployment that needs to read from BigQuery, you write the
`BackendAdapter` yourself (per `backend.rs:110-146`, the trait has 4
required methods + 2 with defaults).

This is not a bug — the trait surface is small and stable
(`docs/review/capabilities.md` §1) — but it is a real implementation
gap if your agent's source data lives in a managed warehouse.

### 9.10 No contradiction or trust modeling

When two memories disagree — "the user said X" vs "the user said
not-X" — the substrate has no opinion. Both are returned as ranked
hits; the brain decides what to do.

ADR-156 §Decision Alternative-A explicitly rejects absorbing
contradiction logic into the substrate.

### 9.11 The README oversell — kernel claims

`docs/review/capabilities.md` §6 G1–G3 catalogs three places where the
README claims kernel features as ruLake capabilities when they
actually live in `ruvector-rabitq`:

- `VectorKernel` trait (ADR-157) is not present in this crate.
- AVX2 / AVX-512 / runtime CPUID dispatch live in the dependency.
- Hadamard rotation (ADR-158) requires constructing
  `RabitqPlusIndex` directly; ruLake's `new()` always uses Haar.

For an agent operator evaluating ruLake, this means: kernel
performance is excellent because of `ruvector-rabitq`, not because of
ruLake. Both are true; the README has not always disambiguated.

### 9.12 The standalone-build gap

`docs/review/capabilities.md` §5 + §6 G8: this repository can't be
built without the parent RuVector workspace because of `path =
"../ruvector-rabitq"` and `version.workspace = true` declarations in
`Cargo.toml`. For an operator evaluating ruLake standalone (cloning
just this repo), `cargo build` will fail. This is a packaging gap
that does not affect the runtime semantics, but it does mean a new
contributor cannot reproduce the benchmark numbers without the rest
of the workspace.

### 9.13 The forgotten-edge-cases shortlist

For completeness, the deep review surfaced a handful of operator
foot-guns that an agent-platform integrator should know about:

- **`with_max_cache_entries` is a soft cap.** Pinned entries are
  never evicted (`cache.rs:243-249`). For an agent platform with
  high pin pressure, the cap can be exceeded.
- **`Frozen` requires a first prime.** Per `cache.rs:880-883`,
  `Frozen` skips coherence only after the pointer is installed. The
  first query is full Fresh-mode behaviour.
- **Default `current_bundle` pulls every vector** if a backend
  author doesn't override (`docs/review/security.md` M2). Real
  backends override; the trait signature does not enforce.
- **mtime-as-generation** (FsBackend) has 1-second resolution; rapid
  sub-second writes won't be detected by the cache
  (`docs/review/security.md` L2).

None of these are blockers for agent use cases; all of them are
documented; an integrator should plan around them.

---

## 10. Open research questions

These are honest unknowns — places where the substrate's behaviour
under agent-specific workloads is not yet measured, where the design
might benefit from agent-specific extensions, or where the boundary
between substrate and brain might shift.

### 10.1 How does the witness scheme interact with embedder model drift?

The witness covers `(data_ref, dim, rotation_seed, rerank_factor,
generation)` (`bundle.rs:362-390`). It does **not** cover the
embedding model that produced the vectors at `data_ref`. If the
embedder is upgraded — say, OpenAI text-embedding-3-small →
text-embedding-3-large — the new vectors live at a new `data_ref`
(typical) or the same `data_ref` with bumped `generation` (also
typical). Either way, the witness changes.

But what if an operator silently swaps embedders without bumping the
generation? Two vectors with the same id but different embedding-
model would now share a witness, and queries would silently mix
old-model and new-model neighbours.

**Open question:** should the bundle carry an opaque `embedder_id`
field that participates in the witness? It would prevent the silent-
swap class of error. Cost: bundle schema change; would need a
`format_version` bump (currently 2). Likely a v3 conversation.

For agentic AI specifically, this matters because agents survive
embedder upgrades — a long-running agent that started in 2025 with
text-embedding-3-small and now runs with text-embedding-3-large
needs the substrate to detect and force-reprime on the change.

### 10.2 Should the bundle protocol carry a vector-clock for true cross-cluster consistency?

The witness chain proves cache coherence within a single cluster
where the publisher and the readers share a filesystem. For multi-
cluster deployments — agents running in two AWS regions, both
reading from a shared upstream — the publish/refresh protocol does
not have a vector-clock; it's a single linear `generation` number.

**Open question:** for cross-cluster agentic deployments (one swarm
in us-east, one in eu-west), is a vector-clock or causal-history
field needed on the bundle? Today, both clusters would race on
publish; the last writer wins (`docs/review/security.md` I3).

For most deployments this is fine (publishers are typically a single
controlled process); for distributed publish workflows it's an
unsolved question.

### 10.3 Can the LRU policy learn from agent reward signals?

The current LRU evicts unpinned entries by `last_used` Instant
(`cache.rs:548-565`). Cheap and correct. For an agent that has
*reward signals* — "the trajectory I retrieved from session X led to
a successful outcome" — the eviction policy could in principle weight
recently-rewarded sessions higher.

**Open question:** should the substrate expose a `pin_with_score(key,
score)` API that biases LRU? Or is this brain logic that should live
above the substrate (the brain calls `lake.invalidate_cache` on
sessions it doesn't want, never on rewarded ones)?

ADR-156 §Decision Alternative-A says "the substrate does not interpret
brain semantics," which suggests the brain owns this. But there is a
case to be made that a generic "score-weighted LRU" is substrate-
appropriate (the substrate doesn't know what scores mean; it just
weights eviction by the operator-supplied score).

### 10.4 Should `Consistency` be a per-collection setting?

Today, `with_consistency` is a `RuLake`-wide setting (`lake.rs:69-73`).
For agentic systems with mixed memory classes — episodic at
`Eventual { ttl_ms: 1_000 }`, identity at `Frozen` — the operator
must use multiple `RuLake` instances.

ADR-155 §"Open questions" notes this:

> Per-backend override deferred — no customer has asked and the
> surface is easy to add later.

For agents, this is asking now. The two-instance workaround works but
is awkward (multiple `cache_stats()`, multiple snapshot dirs, etc.).

**Open question:** should `Consistency` move from `RuLake` to
`(BackendAdapter, CollectionId)` as a registration-time setting? The
plumbing change is moderate (the cache already keys by
`(backend, collection)`); the API change is semver-minor.

### 10.5 What is the right substrate-level forget primitive for agents?

Substrate-level forget is `invalidate_cache`: drop the pointer; GC
the entry if last; the next query re-primes. This is correct for
"this memory is stale; re-fetch from the backend." It is *not*
correct for "this memory should never be retrievable again" (the
backend still has it; the next query would re-prime).

For agentic forget — "the user revoked consent for this memory" —
the substrate needs a stronger primitive that prevents re-priming
even if a query comes in. Today, the operator must (a) delete from
the backend AND (b) call `invalidate_cache`; the substrate does not
expose a "denylist" of backend ids that should never be primed.

**Open question:** should the substrate expose a
`lake.deny_pull(key)` method that prevents future primes for a key,
even if the backend still has the data? It would make the agent's
GDPR-style forget loop a single substrate call. Cost: introduces
mutation state in the cache; arguably brain logic. Defer until a
brain operator asks.

This ties into ADR-156 §Open questions §3: "Is Frozen the right
default for bundles produced by RVF ingest jobs?" Agents have a
similar question: "Is `deny_pull` the right primitive for
right-to-be-forgotten?"

---

## 11. Appendix A — API reference cheat-sheet (the agent-relevant subset)

The full surface is documented in `docs/review/capabilities.md` §1
(20 methods on `RuLake`). The subset most-used by agent code:

```rust
// Construction
let lake = RuLake::new(rerank_factor: usize, rotation_seed: u64);
//                         ^^^^^^ 20 → 100% recall@10 at D=128 clustered
//                                          ^^^^^^^^^^^^^^^ MUST be same
//                                                      across processes
//                                                      that should share

// Modes
let lake = lake
    .with_consistency(Consistency::Fresh)            // strict
    .with_consistency(Consistency::Eventual { ttl_ms: 60_000 })
    .with_consistency(Consistency::Frozen)           // never recheck
    .with_max_cache_entries(256);                    // soft LRU cap

// Mount backends
lake.register_backend(Arc::new(LocalBackend::new("kb"))).unwrap();
lake.register_backend(Arc::new(FsBackend::new("fs", "/var/data").unwrap())).unwrap();

// Query
let hits: Vec<SearchResult> = lake.search_one("kb", "main", &query, k)?;
let batched = lake.search_batch("kb", "main", &queries, k)?;
let federated = lake.search_federated(&[("kb", "main"), ("doc", "main")], &query, k)?;

// Each hit:
// SearchResult { backend: String, collection: String, id: u64, score: f32 }
// (lake.rs:26-32)

// Coherence — write side
let path = lake.publish_bundle(&("kb".to_string(), "main".to_string()), "/var/publish/")?;

// Coherence — read side
let result: RefreshResult = lake.refresh_from_bundle_dir(&key, "/var/publish/")?;
// result is one of: UpToDate / Invalidated / BundleMissing

// Snapshot + warm restart
let path = lake.save_cache_to_dir(&key, "/var/snapshot/kb/")?;
let n_loaded = lake.warm_from_dir(&key, "/var/snapshot/kb/")?;

// Forget
lake.invalidate_cache(&key);

// Diagnostics
let stats = lake.cache_stats();
let hit_rate = stats.hit_rate();          // Option<f64>
let avg_prime_ms = stats.avg_prime_ms();  // Option<f64>
let by_backend = lake.cache_stats_by_backend();   // HashMap<BackendId, _>
let by_coll = lake.cache_stats_by_collection();   // HashMap<CacheKey, _>
let witness = lake.cache_witness_of(&key);        // Option<String>
let refcount = lake.cache_refcount_of(&witness);  // u32
let n_entries = lake.cache_entry_count();         // usize
```

That's the entire agent-relevant surface, in one screen.

---

## 12. Appendix B — Operating-mode decision matrix per agent class

| Agent class | Consistency | TTL | LRU cap | Snapshot? | Federated? | Notes |
|---|---|---|---|---|---|---|
| Compliance / payments | `Fresh` | n/a | 64 | yes (forensic) | no | Witness-anchored audit per query |
| Conversational RAG | `Eventual` | 60 s | 256 | yes (warm-restart) | yes | Recall ≥ 0.85 across silos |
| Tool router (≤ 100 tools) | `Eventual` | 1 hr | 512 | yes | yes | Tool catalogs rotate slowly |
| Tool router (> 1000 tools) | `Eventual` | 1 hr | 1024 | yes | partial | Pre-filter targets; sharded cache TBD |
| Long-running daemon | `Eventual` | 30 s | 64 | yes | optional | Snapshot every 5 min |
| Replay / forensics | `Frozen` | n/a | 64 | mount snapshot | optional | No backend needed |
| ReasoningBank active session | `Eventual` | 1 s | 64 | yes | yes (cross-session) | Snapshot at session end |
| ReasoningBank archive | `Eventual` | 1 hr | 256 | yes | yes | Federate over many sessions |
| Multi-agent swarm shared | `Eventual` | 5 s | 64 | yes (per swarm) | yes (across silos) | Same seed across all agents |
| Identity / policy | `Frozen` | n/a | 4 | yes (read-only mount) | no | Operator-published bundles |

These are starting points, not prescriptions. The right values
depend on the application's freshness needs and memory budget.

---

## 13. Appendix C — Gaps inherited from the deep review

For an agent operator deploying ruLake in production, the deep
review (`docs/review/`) catalogs things that are honest gaps between
the README's claims and the shipped behaviour. Those most relevant
to agents:

| Gap | Source | Agent impact |
|---|---|---|
| README's "kernel" capabilities live in `ruvector-rabitq`, not ruLake | `docs/review/capabilities.md` §6 G1, G2 | None for users; clarity for evaluators |
| Hadamard rotation requires `RabitqPlusIndex` directly, not via ruLake | `docs/review/capabilities.md` §6 G3 | None unless you need Hadamard for high-D recall |
| RBAC, PII enforcement, lineage emission are M4 roadmap | `docs/review/capabilities.md` §6 G4 | Real for compliance use cases |
| Default `current_bundle` does a full pull on every search | `docs/review/security.md` M2 | Real footgun if backend author doesn't override |
| FsBackend symlink/TOCTOU on `root` | `docs/review/security.md` M3 | Multi-tenant hosts only |
| `Generation::Opaque(String)` is unbounded inside `PulledBatch` | `docs/review/security.md` M4 | Federated deployments with third-party backends |
| `LocalBackend::append` no growth cap | `docs/review/security.md` M5 | LocalBackend is test substrate; copy-paste risk |
| Standalone-build gap (`Cargo.toml` workspace inheritance) | `docs/review/capabilities.md` §5 | Evaluators cloning standalone repo |
| `with_max_cache_entries` is a soft cap (pinned entries not evicted) | `docs/review/capabilities.md` §7 / `cache.rs:243-249` | Real for high-pin-pressure agent platforms |
| Global cache mutex serializes per-query bookkeeping | `docs/review/performance.md` §6 B1 | At ≥ 50K QPS / very high concurrency only |
| Per-result `String` allocation in `SearchResult` | `docs/review/performance.md` §6 B2 | At very high QPS only |
| `search_batch` doesn't parallelize across queries | `docs/review/performance.md` §6 B3 | Underuse on CPU-only batch workloads |
| Thundering-herd on cold prime (N threads, 1 winner) | `docs/review/performance.md` §6 B5 | Swarm-launch only; tens of seconds wasted CPU |
| Mutex `unwrap()` on poisoning bricks the Lake | `docs/review/security.md` M1 | Latent DoS; not directly exploitable |

None of these are blockers for the verticals in this paper. All are
documented; an integrator who reads the deep review will plan around
them.

---

## 14. Appendix D — Worked memory-class taxonomy for the bundle tag

The `RuLakeBundle.memory_class: Option<String>` field
(`bundle.rs:144-155`) is opaque to the substrate. Agent systems can
adopt any taxonomy; below is a worked example that maps cleanly to
common cognitive-architecture vocabularies.

| Tag | Definition | Typical lifecycle | Suggested mode |
|---|---|---|---|
| `episodic` | A single observed event in a session | Created at observation; consolidated to `semantic` after reflection; deleted on session-end (or aged out) | `Eventual { ttl_ms: 1_000 }` |
| `semantic` | A consolidated fact the agent has learned | Created by consolidation; mutated by contradiction-handling; never deleted (ages out) | `Eventual { ttl_ms: 60_000 }` |
| `procedural` | A skill / pattern that worked | Created by reflection; effectively immutable | `Frozen` |
| `identity` | Agent's own self-description | Operator-managed; rotation requires out-of-band sign-off | `Frozen` |
| `policy` | What the agent cannot do | Operator-managed; immutable per session | `Frozen` |
| `observation` | Raw sensor / tool-output stream | Created continuously; consolidated before eviction | `Eventual { ttl_ms: 100 }` |
| `tool-catalog` | Tool descriptions for routing | Operator-managed; rotates on registry update | `Eventual { ttl_ms: 3_600_000 }` |
| `ledger` | Financial / accounting facts | Backend-managed; rotates per transaction | `Fresh` |
| `trajectory` | Past agent decision path | Created per session; archived after session-end | `Eventual { ttl_ms: 1_000 }` for active, `Frozen` for replay |
| `pattern` | Distilled trajectory shape | Created by ReasoningBank-style distillation | `Eventual { ttl_ms: 60_000 }` |

The substrate stores any string the brain hands it. The taxonomy
above is one possible convention; another brain layer might use
`hot` / `warm` / `cold` instead. Either is fine. The substrate's job
is to surface the tag through stats and bundle inspection, not to
opine on what it should mean.

---

## Closing note

ruLake is a substrate. The seven verticals above are not "ruLake
products" — they are application shapes that the substrate's six
guarantees (recall / verify / forget / rehydrate / location-
transparency / compact-deferred per ADR-156) make easy to build.

For agent-platform engineers reading this, the takeaway is:

1. The shipped surface is small, stable, well-tested
   (`docs/review/capabilities.md` §10).
2. The hit-path overhead is measured at 1.00–1.03× direct rabitq
   (`docs/review/performance.md` §8).
3. The witness chain gives provenance for free.
4. The federation primitive scales to ≤ ~100 silos without
   modification; beyond that, the perf review's B1 ceiling is the
   next ceiling.
5. The brain — whatever brain you build — owns semantics, scoring,
   contradiction, compaction, reward modeling. The substrate
   deliberately doesn't.

Where ruLake is the right substrate: agent memory hierarchies,
multi-agent shared world models, long-running autonomous agents,
tool routing at moderate scale, federated RAG with provenance,
ReasoningBank-style trajectory storage. Where it isn't yet: managed-
warehouse-backend deployments without a custom adapter, cross-cluster
distributed publish, GDPR crypto-shred-as-a-service.

For the verticals where it fits, it fits unusually cleanly. The
question for agent-platform vendors is whether the substrate's
opinions (read-mostly, witness-anchored, content-addressed) match
their architecture's opinions. If yes, this is the substrate to plug
in. If no, the deep review's honest catalog of gaps tells you what
would need to change first.
