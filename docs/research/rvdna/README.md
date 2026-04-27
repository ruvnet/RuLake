# rvDNA v2 Research Corpus

A four-document set that takes the original "rvDNA + ruLake = perfect
compositional fit" thesis and turns it into a shippable v2 spec, a
concrete integration plan, and a single Architecture Decision Record
that both bind. The corpus does not invent a new format; it ratifies
the v1 prototype at `vendor/ruvector/examples/dna/` as a first-class
ruLake substrate, fixing the gaps that prevented v1 from federating.

## 30-second pitch

rvDNA v1 is a precomputed genomic intelligence file (raw DNA + k-mer
vectors + attention + variant tensors + protein embeddings + epigenomic
tracks + biomarker streams) packaged as a single mmap-friendly
`.rvdna` blob. v1 ships a working pipeline that takes 12 ms end-to-end
on five real human genes and produces a binary that any future query
can hit with sub-microsecond random access. ruLake is a witness-
anchored vector cache that has shipped four backends (Local, Fs, GCS,
IPFS), an MCP server with eight tools, and a console — all built
around a SHAKE-256 bundle witness that makes any vector cache entry
content-addressable across deployments.

v2 closes a single missing seam: every `.rvdna` file emits a ruLake-
compatible bundle JSON, every section maps onto a tier of ruLake's
`BackendAdapter` trait, and a sibling `mcp-rvdna` server exposes the
genomic verbs through the same capability-gated tool surface that
ruLake's MCP server uses. The result is the system the original brief
described: rvDNA as the *immutable compute artefact*, ruLake as the
*adaptive retrieval layer*, and ruvector as the reasoning loop. The
"load 100 files, query variant similarity, return in <10 ms without
recompute" validation test from the brief becomes one of five
acceptance gates in ADR-007.

## Files in this directory

| Path                             | Lines | What it does                                                                   |
|----------------------------------|------:|--------------------------------------------------------------------------------|
| `README.md` (this file)          |   ~150 | Index, persona reading guides, answers to the brief's two sharpening questions |
| `v2-spec.md`                     | ~1,800 | Canonical rvDNA v2 file-format and behaviour spec (supersedes v1's ADR-013)    |
| `integration-with-rulake.md`     | ~1,000 | Code-level integration: `rvdna-backend/`, `mcp-rvdna/`, Console hooks         |
| `ADR-007-rvdna-as-rulake-substrate.md` |  ~750 | The decision record that binds the spec and the integration plan together     |

Read order is the order they appear above. Each file cites real paths
into v1 (`vendor/ruvector/examples/dna/`) and ruLake (`src/`,
`mcp-server/src/`, `docs/adrs/`). No invented APIs.

## Reading guide by persona

The brief asks two questions to sharpen the design. Both decompose
into "which audience is this serving?" and the spec serves three.

### Persona 1: Clinical-pipeline operator (regulated context)

You sequence patients, you owe HIPAA, you owe traceability, and a
wrong answer has consequences a research scientist's wrong answer
does not. You care about:

1. **Answer to brief Q1**: v2 ships a `--profile clinical` mode
   (`v2-spec.md` §k) that pins `pii_policy: "phi-strict"` on every
   bundle, mandates an OpenLineage `lineage_id`, refuses cross-tenant
   federation unless JWT scopes match (`mcp-server/src/auth.rs:294`
   `scopes_to_caps`), and forbids `Consistency::Eventual` in favour
   of `Consistency::Fresh` or `Consistency::Frozen`. The clinical
   profile is a configuration, not a fork — research and clinical
   `.rvdna` files are byte-identical except for the bundle metadata.
2. **Where to start in v2-spec.md**:
   - §a Status & supersession (which v1 promises hold)
   - §d Witness chain (every emitted bundle is reproducible)
   - §h MCP tools, especially the `rvdna_lineage` audit tool
   - §k Privacy + clinical mode
   - §l Validation test (acceptance gates)
3. **Where to start in integration-with-rulake.md**: the
   `mcp-rvdna/` section — the JWT scope `mcp:rvdna:clinical` is what
   gates clinical-tier tools; the audit row shape mirrors
   `mcp-server/src/audit.rs` so existing log pipelines work unchanged.
4. **Where to start in ADR-007**: §Decision items 6 and 7 (clinical
   profile, federation refusal under tenant mismatch); §Verification
   gate G3 (clinical replay test).

### Persona 2: Research / discovery scientist (exploratory context)

You're hunting for population-scale signal across 10k samples; you
care about latency, recall, and the cost of asking a new question of
data you've already loaded once. You care about:

1. **Answer to brief Q1**: v2 ships a `--profile research` (default)
   that turns on `Consistency::Eventual { ttl_ms }` and unlocks
   `search_federated` (`src/lake.rs:521`). You can blend cohorts,
   you can cross-trial query, the witness chain still proves
   reproducibility but freshness checks are amortised.
2. **Answer to brief Q2 (static vs streaming)**: §j Streaming
   biomarkers spec'd a v2 mode where `.rvdna` files become append-
   only ring buffers. Wearable data and serial blood draws push new
   biomarker rows; the witness rotates per epoch. Same file format,
   same retrieval API, evolved temporal semantics. The static
   default and the streaming mode share the bundle witness format —
   you choose at encode time, the consumer doesn't have to know.
3. **Where to start in v2-spec.md**:
   - §c Physical layout, §d Witness chain (so you trust what you query)
   - §e Tiered indexing (so cohort queries don't OOM your laptop)
   - §g Query verbs, especially `find` and `score`
   - §i Cross-sample federation
   - §j Streaming biomarkers
4. **Where to start in integration-with-rulake.md**: the
   `rvdna-backend/` section — the T0/T1/T2 tier wiring is what makes
   "load 100 files, <10 ms query" actually run on a workstation.

### Persona 3: Platform engineer wiring rvDNA into agents

You're building agentic systems that need to ground responses in a
patient's real genome (or a research cohort) without round-tripping
to Python tooling. You care about:

1. The MCP tool surface (`v2-spec.md` §h): five verbs (`rvdna_find`,
   `rvdna_call_variants`, `rvdna_translate`, `rvdna_score`,
   `rvdna_lineage`) that mirror the shape of `rulake_query` in
   `mcp-server/src/server.rs:189`. Capability-gated. JSON-schema'd.
   Audit-logged with codes from a six-code refusal vocabulary.
2. **How v2 plugs into the existing MCP plane**: `mcp-rvdna/` is a
   sibling crate of `mcp-server/`, not an extension. The two servers
   can share an audit pipeline because both emit
   `mcp-server/src/audit.rs::AuditRow` with disjoint code prefixes
   (`RULAKE_*` vs `RVDNA_*`).
3. **Where to start in v2-spec.md**: §h MCP tools, §m Migration
   from v1 (so your agents can read existing v1 corpora).
4. **Where to start in integration-with-rulake.md**: the `mcp-rvdna/`
   section and the Console hooks section (so the `Genomic` 7th
   sidebar entry is something you can demo).
5. **Where to start in ADR-007**: §Decision items 4 and 5 (MCP tool
   surface, federation pattern); §Verification gates G2 and G4.

## Two questions to sharpen this — answers

The brief asks the user to pick clinical-vs-research and static-vs-
streaming. The spec refuses to pick: both are first-class profiles.
Concretely:

- **Clinical vs research**: v2 supports both via a `Profile` enum
  attached to the bundle's `pii_policy` field. `pii_policy:
  "phi-strict"` triggers tenant-scope federation refusal and forces
  audit-tail mandate; `pii_policy: "research-open"` removes both.
  Same file format. Same query API. Different governance posture.
- **Static vs streaming**: v2 supports both. The default is static
  (one-shot encode, infinite reads). The streaming mode adds a
  monotonic-clock generation bump per epoch, which causes the bundle
  witness to rotate, which causes ruLake's coherence model
  (`src/cache.rs::Consistency`) to handle freshness correctly without
  v2 needing to invent a new staleness model. Streaming is just
  static with a per-epoch witness rotation policy.

## What's NOT in this corpus

- A `cargo` build of `rvdna-backend/`. ADR-007 lands first; the
  scaffold is the next commit AFTER ADR-007 acceptance.
- A regulatory-class clinical claim. v2 ships clinical *posture*
  (PHI metadata, tenant scopes, audit). Clinical *interpretation*
  (CDS, FDA-cleared dosing) is a layer above v2 — explicitly out of
  scope per `v2-spec.md` §b non-goals.
- A new vector-search engine. ruLake's RaBitQ-compressed cache and
  HNSW substrate handle that; v2 just registers `.rvdna` sections
  with them via the existing `BackendAdapter` trait at
  `src/backend.rs:110`.

## What ships next (after this corpus lands)

1. ADR-007 review + acceptance.
2. `rvdna-backend/` v0.0 scaffold — `Cargo.toml`, the three
   `BackendAdapter` impls (T0/T1/T2), one passing round-trip test
   against a v1 `.rvdna` file from `vendor/ruvector/examples/dna/`.
3. `mcp-rvdna/` v0.0 — `rvdna_find` only, witness-pinned, capability-
   gated by a new `mcp:rvdna:read` JWT scope.
4. Console: a 7th sidebar entry (`Genomic`) that surfaces the
   `rvdna://bundle/{file_id}` resource and runs a witness-verify
   round-trip against the existing `node-wasm/` `verifyBundleJson`
   path. Implementation: see `integration-with-rulake.md` §Console
   hooks.
