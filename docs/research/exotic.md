# Exotic and frontier verticals — ruLake research note

## Frame

This note explores verticals where a witness-coherent vector cache opens up
something genuinely different from what a vector database, a key-value store,
or a vanilla in-process index can offer. The previous three notes in this
directory ([`agentic.md`](agentic.md), [`ai-ml.md`](ai-ml.md),
[`edge.md`](edge.md)) work the central path: agents, RAG, on-device. This one
sits at the edges — domains where one or more of:

- the **provenance** primitive (witness-pinned bytes, not just IDs)
- the **freezable consistency** primitive (`Consistency::Frozen`)
- the **portable atomic publish** primitive (`table.rulake.json` sidecar)
- the **content-addressed cross-process dedup** (witness sharing)
- the **federated rerank** primitive (adaptive `k' = max(5, global/K)`)

does load-bearing work that no commodity off-the-shelf system already does
cleanly. Where one of those is the *feature* — not just an implementation
detail — there's a vertical worth exploring.

The discipline this note holds:

1. **Cite real API.** All pseudocode uses signatures from `crates/core/src/lake.rs`,
   `crates/core/src/cache.rs`, `crates/core/src/backend.rs`, `crates/core/src/bundle.rs`, `crates/core/src/fs_backend.rs`.
   No invented symbols.
2. **Honor the M1+M1.5 line.** Today, what ships is `LocalBackend`,
   `FsBackend`, and the full cache + coherence + bundle + persistence +
   federation surface. Parquet, BigQuery, Iceberg, Delta, RBAC, PII passthrough,
   OpenLineage emission, GPU kernels, mmap'd persistence, HNSW-on-RaBitQ —
   all M2+ roadmap, not code. Verticals that depend on these say so explicitly.
3. **No magical thinking.** Speculative is fine; "the witness solves
   everything" is not. Each vertical ends with a "Gaps and what would have
   to change" subsection.
4. **Honest scoping for sensitive domains.** Healthcare, defense, financial
   services framings are about lawful, audited, defensive contexts —
   compliance substrate, not bypass.

### What changed in this repo since the deep review

The standalone-build gap that the [capability review](../review/capabilities.md)
flagged — `Cargo.toml` workspace inheritance + `path = "../ruvector-rabitq"`
that walked out of the repo — has been closed (see
[ADR-001](../adrs/ADR-001-standalone-repo-strategy.md)). A fresh
`git clone --recurse-submodules` now produces a working `cargo build`
and `cargo test --release` (43/43). Verticals below that depend on a
clean evaluator path (regulatory submissions, scientific reproducibility,
defense-procurement audits) are first-order beneficiaries.

### Ground rules for "concrete implementation outline" sections

Every vertical below sketches an implementation against ruLake's *real*
surface. Specifically:

```rust
use std::sync::Arc;
use rulake::{
    cache::Consistency,
    backend::{BackendAdapter, CollectionId, PulledBatch},
    LocalBackend, RuLake,
};

let backend = Arc::new(MyBackend::new(...));
let lake = RuLake::new(20, 42)
    .with_consistency(Consistency::Eventual { ttl_ms: 60_000 })
    .with_max_cache_entries(256);
lake.register_backend(backend)?;

let hits = lake.search_one("backend-id", "collection-name", &query, 10)?;
let key = ("backend-id".to_string(), "collection-name".to_string());
lake.publish_bundle(&key, "/path/to/sidecar/dir/")?;
```

Custom backends implement four required methods:

```rust
impl BackendAdapter for MyBackend {
    fn id(&self) -> &str { "my-backend" }
    fn list_collections(&self) -> Result<Vec<CollectionId>> { ... }
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> { ... }
    fn generation(&self, collection: &str) -> Result<u64> { ... }
}
```

Plus an optional `current_bundle(collection)` for backends that have a
preferred witness representation.

Witness derivation lives in `crates/core/src/bundle.rs::compute_witness` and is
length-prefixed + domain-separated SHAKE-256(32) over
`(data_ref, dim, seed, rerank, generation)` with a tag byte that
distinguishes `Generation::Num(n)` from `Generation::Opaque(bytes)` —
the regression is exercised by tests after the 2026-04-23 audit found
the original concatenation was collision-prone.

---

## Vertical 1 — Healthcare: clinical decision support with HIPAA-grade audit

### Why

Clinical decision support (CDS) systems retrieve "similar prior cases" or
"matching clinical guidelines" given a patient encounter embedding. The
problem isn't retrieval quality — modern embedding models do that fine. The
problems are:

1. **Auditability.** When a CDS surfaces a recommendation, regulators and
   plaintiffs want to know which exact bytes informed the answer. ICD code
   sets, drug interaction tables, clinical guidelines, and prior-case corpora
   all change weekly to monthly. "We retrieved from version-3.2 of the
   guideline corpus" is not enough — the audit needs to prove the retrieval
   was over the *exact bytes* of that version, not a re-indexed copy with
   silent drift.
2. **Frozen replay for medico-legal review.** A complaint filed in 2027 about
   a 2026 recommendation needs to reproduce the retrieval state as of the
   recommendation moment. Standard vector DBs make this trivially false:
   they re-index, re-shard, and reorganize without keeping the bit-state.
3. **Tenant isolation under shared infrastructure.** Hospital systems share
   a CDS vendor's infrastructure but cannot share patient data; the
   index-level coherence has to be tenant-scoped without the vendor's
   operations team needing per-tenant flag-juggling.

ruLake's witness chain is precisely the cryptographic anchor missing from
LangChain/LlamaIndex VectorStore interfaces. `Consistency::Frozen` is
precisely the regulatory snapshot mode no managed vector DB exposes as a
first-class knob.

### Concrete implementation outline

```rust
// One RuLake per tenant (hospital), Frozen consistency for the
// medico-legal-review path, Eventual for the live CDS path.
fn cds_for_tenant(tenant_id: &str, mode: CdsMode) -> Result<RuLake> {
    let backend = Arc::new(EhrBackend::new(tenant_id)?);

    let consistency = match mode {
        CdsMode::Live           => Consistency::Eventual { ttl_ms: 30_000 },
        CdsMode::MedicoLegal    => Consistency::Frozen,
        CdsMode::QualityReview  => Consistency::Frozen,
    };

    let lake = RuLake::new(20, /* rotation_seed = */ tenant_seed(tenant_id))
        .with_consistency(consistency)
        .with_max_cache_entries(64);

    lake.register_backend(backend)?;
    Ok(lake)
}

// On every CDS recommendation, capture the witness.
fn record_cds_event(
    lake: &RuLake,
    encounter: &PatientEncounterEmbedding,
) -> Result<CdsAuditRecord> {
    let backend = "ehr";
    let collection = "guidelines-v3.2";
    let hits = lake.search_one(backend, collection, &encounter.vec, 10)?;

    let key = (backend.to_string(), collection.to_string());
    let witness = lake.cache_witness_of(&key).expect("primed");

    Ok(CdsAuditRecord {
        encounter_id: encounter.id.clone(),
        witness_hex: hex::encode(witness),
        hits: hits.into_iter().map(audit_hit).collect(),
        recommendation_at: now(),
        consistency_mode: "Eventual{30s}".to_string(),
    })
}

// Replay 18 months later: warm a Frozen RuLake from an archived snapshot
// directory, run the same query, prove identical hits.
fn replay_for_litigation(
    snapshot_dir: &Path,
    encounter: &PatientEncounterEmbedding,
) -> Result<Vec<SearchResult>> {
    let lake = RuLake::new(20, archived_seed(snapshot_dir))
        .with_consistency(Consistency::Frozen);
    let key = ("ehr".to_string(), "guidelines-v3.2".to_string());
    lake.warm_from_dir(&key, snapshot_dir)?;
    lake.search_one("ehr", "guidelines-v3.2", &encounter.vec, 10)
}
```

The `EhrBackend` adapter wraps the institution's existing EHR vector store
(today: `LocalBackend` for the prototype, M2+ for Iceberg/Delta on the
clinical-data lake). `BackendAdapter::generation` returns the EHR's
content-version (e.g. `git-sha1` of the guideline corpus, `created_at`
millis of the patient cohort). The bundle sidecar — published nightly to
the institution's WORM (write-once-read-many) archive — is the artifact
the medico-legal team retains.

### Gaps and what would have to change

- **No PII passthrough today.** ADR-155 §M4 lists `pii_policy` on the
  bundle and `rvf-federation::pii` re-use as roadmap, not code. Production
  HIPAA deployment requires building this on top of ruLake; ruLake provides
  the witness substrate, not the PII enforcement.
- **No RBAC.** Per-clinician role-based filtering (`only attending physicians
  see drug-interaction warnings`) lives above ruLake. The `BackendAdapter`
  surface doesn't carry an identity context; a wrapper that filters
  `pull_vectors` results per-role is application work.
- **EHR backend doesn't ship.** `EhrBackend` above is illustrative; building
  it against Epic / Cerner / OpenEMR is a real adapter project. The trait is
  4 methods, but the underlying clinical-data plumbing is not trivial.
- **`Consistency` is per-`RuLake`, not per-collection.** A CDS that wants
  Eventual on the guidelines collection and Frozen on the cohort-of-record
  collection must instantiate two `RuLake`s today. ADR-155 notes this is
  "deferred until a customer asks"; healthcare is plausibly that customer.
- **Right-to-be-forgotten requires backend-side deletion.** ruLake's
  `invalidate_cache` drops a pointer; it does not crypto-shred. GDPR/HIPAA
  forget operations must reach the underlying storage and rotate the bundle.
  Pairing pattern is documented but not automated.

---

## Vertical 2 — Financial services: fraud detection + regulatory lineage

### Why

Financial services has the same audit pressure as healthcare with two extra
constraints:

1. **Latency.** Fraud retrieval at the auth-decision tier is sub-50ms p99.
   A 1.02× tax over a directly-called RaBitQ index is well within budget;
   a per-decision call out to a managed vector DB is not.
2. **Snapshot lineage for SOX, MiFID II, BCBS 239.** Regulators want to
   reproduce the exact retrieval state as of a quarterly close, an
   investigation date, or a customer complaint. The system of record
   (transaction log) plus a witness-pinned vector retrieval gives the
   complete picture.

The witness is exactly the immutable audit chain anchor. ruLake's
`Frozen` mode + `warm_from_dir` lets a forensics analyst rehydrate a
quarter-end retrieval substrate from cold storage in milliseconds, query
it as if live, and produce byte-exact results that match the original
decision.

### Concrete implementation outline

```rust
// Hot path: fraud decision at auth time.
struct FraudDetector {
    lake: Arc<RuLake>,
    backend_id: String,
    collection: String,
}

impl FraudDetector {
    fn new(backend: Arc<dyn BackendAdapter>) -> Result<Self> {
        let lake = RuLake::new(20, FRAUD_SEED)
            .with_consistency(Consistency::Eventual { ttl_ms: 5_000 })
            .with_max_cache_entries(8);
        lake.register_backend(backend.clone())?;
        Ok(Self {
            lake: Arc::new(lake),
            backend_id: backend.id().to_string(),
            collection: "txn-embeddings-30d".to_string(),
        })
    }

    fn score(&self, txn: &TxnEmbedding) -> Result<FraudDecision> {
        let neighbors = self.lake.search_one(
            &self.backend_id, &self.collection, &txn.vec, 50,
        )?;
        let key = (self.backend_id.clone(), self.collection.clone());
        let witness = self.lake.cache_witness_of(&key).unwrap();

        Ok(FraudDecision {
            score: aggregate_neighbor_risk(&neighbors),
            witness_hex: hex::encode(witness),
            decided_at: now_micros(),
        })
    }
}

// Quarter-end snapshot for regulators.
fn close_quarter(lake: &RuLake, archive_root: &Path, quarter: &str) -> Result<()> {
    let key = ("txn-store".to_string(), "txn-embeddings-30d".to_string());
    let dir = archive_root.join(format!("{quarter}/txn-embeddings-30d"));
    std::fs::create_dir_all(&dir)?;
    lake.save_cache_to_dir(&key, &dir)?;
    lake.publish_bundle(&key, &dir)?;
    Ok(())
}

// Forensic replay 14 months later.
fn replay_quarter(archive_root: &Path, quarter: &str, query: &TxnEmbedding) -> Result<...> {
    let lake = RuLake::new(20, FRAUD_SEED).with_consistency(Consistency::Frozen);
    let key = ("txn-store".to_string(), "txn-embeddings-30d".to_string());
    let dir = archive_root.join(format!("{quarter}/txn-embeddings-30d"));
    lake.warm_from_dir(&key, &dir)?;
    lake.search_one("txn-store", "txn-embeddings-30d", &query.vec, 50)
}
```

The bundle sidecar (≈300 bytes per collection per snapshot) is the
artifact the bank's audit team retains. The full `index.rbpx` snapshot
(2.5 MB at n=5000 D=128, scaling roughly linearly) goes into the
WORM-tier object store. `replay_quarter` reproduces the exact retrieval
state — same RaBitQ codes, same neighbor order, same witness — without
the original write path being available.

### Gaps and what would have to change

- **Per-decision witness logging is application work.** ruLake exposes
  `cache_witness_of`; persisting `(decision_id → witness_hex)` to the
  audit log is not in scope. The pattern is straightforward but needs
  an emitter.
- **OpenLineage emission is M2+.** The natural mapping `witness → lineage_id`
  is sketched in the README and ADR-155 §M4 but not implemented.
- **Multi-region replication is single-writer.** A bank with US, EU, APAC
  trading desks all writing to the embedding store needs a coordination
  layer above the bundle protocol. Today the protocol assumes one writer
  publishes, many readers refresh.
- **`Frozen` mode does not pin the backend.** If the backend deletes the
  underlying bytes, future warm-restart still works (the snapshot is
  self-contained), but `Fresh` queries break. Operational policy must
  retain the bytes in the durable backend for the regulatory horizon
  (typically 7 years).

---

## Vertical 3 — Climate and earth observation: planet-scale federated retrieval

### Why

Earth-observation programs (Landsat, Sentinel, Planet, NAIP, MAXAR
commercial) generate petabytes per year of multispectral imagery. The
retrieval question — *"show me anomalies that look like this one"* — is a
similarity search over learned embeddings of land-cover patches. The hard
constraints:

1. **Per-pass collections.** Each satellite pass is a natural shard. New
   passes arrive continuously. The system must federate over thousands of
   small-to-medium collections without a per-pass schema migration.
2. **Sensor-modality federation.** Optical / SAR / hyperspectral / thermal
   embeddings live in separate models and separate collections. A query
   for "tailings dam failure" wants to fan out across all four modalities
   and merge.
3. **Bundle rotation as the only writer signal.** Imagery is written once
   and rarely updated; the backend's `generation` is naturally the pass
   timestamp. The bundle protocol's atomic publish is exactly the right
   primitive.
4. **Researcher workstations + cloud workers share the same cache.**
   Witness-content-addressed cross-process dedup means a researcher's
   laptop and a cloud Spark job that both query "the May 14, 2026 Sentinel-2
   pass over the Bay Area" share one compressed cache entry across the
   institution's network.

### Concrete implementation outline

```rust
// A backend per sensor modality.
fn build_eo_lake() -> Result<RuLake> {
    let lake = RuLake::new(20, EO_SEED)
        .with_consistency(Consistency::Eventual { ttl_ms: 3600_000 })
        .with_max_cache_entries(512);
    lake.register_backend(Arc::new(SentinelBackend::new()?))?;
    lake.register_backend(Arc::new(LandsatBackend::new()?))?;
    lake.register_backend(Arc::new(SarBackend::new()?))?;
    lake.register_backend(Arc::new(HyperspectralBackend::new()?))?;
    Ok(lake)
}

// Anomaly query — fan out across modalities + the latest 30 days of passes.
fn find_similar_anomalies(
    lake: &RuLake,
    query_patch: &PatchEmbedding,
    after: Date,
) -> Result<Vec<RankedHit>> {
    let mut shards: Vec<(&str, String)> = vec![];
    for (backend, collections) in active_collections_after(after) {
        for c in collections {
            shards.push((backend, c));
        }
    }
    let shard_refs: Vec<(&str, &str)> = shards
        .iter()
        .map(|(b, c)| (*b, c.as_str()))
        .collect();

    lake.search_federated(&shard_refs, &query_patch.vec, 50)
}
```

`SentinelBackend::generation(pass_id)` returns the ESA-published
acquisition timestamp; the witness encodes that, so any change to the
underlying L1C product (reprocessing) cleanly invalidates downstream
caches without manual coordination. A nightly cron publishes new passes'
bundle sidecars to a public bucket; institutional caches refresh against
that bucket. `Eventual { ttl_ms: 3600_000 }` (1 hour) is the freshness
floor — interactive analysts get sub-second hits, batch jobs see at most
1-hour-old generations.

### Gaps and what would have to change

- **Backends don't ship.** `SentinelBackend`, `LandsatBackend` are
  hypothetical. Building them against ESA / USGS open data is real work,
  but they only need 4 methods.
- **Per-shard recall floor at high shard count.** Adaptive per-shard
  rerank `k' = max(5, global/K)` is exercised at K=4 in the test suite;
  K=300 (one shard per pass over 30 days at 10 passes/day) is well
  outside the tested envelope. Per-shard over-request
  `k' = k + ⌈√(k·ln S)⌉` is the documented mitigation but the gate
  test only validates K=4. Empirical recall at K=300 needs measurement.
- **Federation is in-process rayon, not cross-host.** Federation across
  a researcher's laptop and a cloud Spark cluster requires either
  exposing ruLake over an HTTP/gRPC layer (M2+ "Wire" roadmap) or
  syncing the bundle dir + warming a local copy.
- **Per-collection consistency mode would help.** Recent passes (last 24h)
  want `Eventual`; archived passes (>90d) want `Frozen` for reproducibility.
  Today this requires two `RuLake` instances; trivial but worth noting.

---

## Vertical 4 — Genomics and proteomics: homology search with publication-grade reproducibility

### Why

Modern protein structure prediction (ESM-2, AlphaFold 3) and DNA-LM
embeddings make distance in the embedding space a useful homology signal
across reference proteomes / genomes. Two genuine constraints:

1. **Reproducibility for publication.** A 2026 paper cites
   "we found 17 homologs of P12345 in UniRef90 v2026_01 with cosine
   distance < 0.12." A reviewer in 2028 must reproduce that exact result.
   ESM-2 embeddings are deterministic; UniRef90 changes; the question
   "what was the index state when the search ran" needs a portable answer.
2. **Cross-institution federation without data movement.** Reference
   proteome embeddings live at multiple national resources (UniProt,
   PDB-derived, MGnify metagenomic). A unified search across all three
   today requires either pulling everything to one place (bandwidth +
   policy nightmare) or running three separate queries and merging by
   hand.

The witness is exactly the publication anchor: the artifact a paper
attaches as supplementary, that a reviewer `warm_from_dir`s into a
reproduction environment.

### Concrete implementation outline

```rust
// Per-resource backend. Each `pull_vectors` reads the resource's
// vector blob (pre-embedded, distributed by the resource itself).
struct UniRefBackend { release: String }

impl BackendAdapter for UniRefBackend {
    fn id(&self) -> &str { "uniref" }
    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        Ok(vec![CollectionId {
            name: format!("uniref90-{}", self.release),
            dim: 1280,  // ESM-2-650M embedding dim
            row_count: count_uniref90(&self.release)?,
        }])
    }
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        let path = uniref90_blob_path(&self.release, collection)?;
        load_pulled_batch_from_disk(&path)
    }
    fn generation(&self, _: &str) -> Result<u64> {
        Ok(parse_release_to_u64(&self.release))  // e.g. 202601
    }
}

// Homology search across three resources.
fn find_homologs(query: &ProteinEmbedding, k: usize) -> Result<Vec<Homolog>> {
    let lake = RuLake::new(20, HOMOLOGY_SEED)
        .with_consistency(Consistency::Frozen);
    lake.register_backend(Arc::new(UniRefBackend { release: "2026_01".into() }))?;
    lake.register_backend(Arc::new(PdbBackend { release: "2026.01.15".into() }))?;
    lake.register_backend(Arc::new(MgnifyBackend { release: "v6.0".into() }))?;

    lake.search_federated(&[
        ("uniref", "uniref90-2026_01"),
        ("pdb",    "pdb-2026.01.15"),
        ("mgnify", "mgnify-v6.0"),
    ], &query.vec, k)
}

// Publication artifact: the bundle sidecars.
fn publish_supplementary(lake: &RuLake, out_dir: &Path) -> Result<()> {
    for backend in &["uniref", "pdb", "mgnify"] {
        for collection in lake.collections_for(backend)? {
            let key = (backend.to_string(), collection.clone());
            let sub_dir = out_dir.join(backend).join(&collection);
            std::fs::create_dir_all(&sub_dir)?;
            lake.publish_bundle(&key, &sub_dir)?;
            lake.save_cache_to_dir(&key, &sub_dir)?;
        }
    }
    Ok(())
}
```

The `out_dir` published by `publish_supplementary` is the paper's
supplementary data archive (typically a few hundred MB at proteome scale,
plus 300-byte sidecars per collection). A reviewer in 2028 downloads it,
runs `warm_from_dir` against the same dirs, runs `search_federated` with
the published query embedding, and gets byte-identical results. The
witness chain is the cryptographic anchor that proves they really did.

### Gaps and what would have to change

- **No UniRef/PDB/MGnify backend ships.** Real adapters are domain work;
  a postdoc-week each.
- **n=100M vectors approaches the M1 tested envelope ceiling.** UniRef90
  has ~150M sequences. The performance review's lock-state contention
  finding (B1 — sharded `CacheState` is the next ceiling) starts to
  matter at this scale and concurrent users.
- **Embedding model versioning is not in the witness.** Two researchers
  using different ESM-2 checkpoints (650M vs 3B) on the same UniRef90
  release would get different RaBitQ codes; the witness would differ
  (because the embeddings differ), but only after a rebuild. A
  "model_revision" field on the bundle (currently absent) would make
  this explicit; today it's encoded implicitly in `data_ref`.
- **Semi-static bundles favor the `FsBackend` pattern.** UniRef releases
  are quarterly; that's a pure write-once-read-many backend. `FsBackend`
  works as-is; a managed S3/GCS BackendAdapter would be a one-week
  adapter (M2+ adjacent).

---

## Vertical 5 — Robotics fleet learning: per-robot experience, fleet-coherent memory

### Why

A fleet of N robots (Amazon-warehouse, Waymo-fleet, Boston Dynamics-style
quadrupeds, agricultural drones) wants to share learned experience without
each robot uploading raw sensor data to the cloud. Each robot embeds its
local experience (success/failure trajectories, environment perception,
action-outcome pairs) and the fleet should benefit:

- A robot encountering an unfamiliar situation queries against the fleet's
  collective embedded experience.
- New robots inheriting the fleet's accumulated memory boot directly into
  competence (warm-restart from the published fleet bundle).
- The fleet's "memory of last week" can be frozen for a regression test
  against this week's controller.

ruLake's witness sharing means the fleet's coordinator publishes one
bundle; every robot that warms from it gets the *same* compressed bytes,
keyed by the same witness, deduped at the kernel page-cache level if
they share storage.

### Concrete implementation outline

```rust
// Each robot runs ruLake against a local FsBackend.
fn build_robot_lake(robot_id: &str) -> Result<RuLake> {
    let local_dir = format!("/var/robotmem/{robot_id}");
    let backend = Arc::new(FsBackend::new(&local_dir)?);

    let lake = RuLake::new(20, fleet_seed())
        .with_consistency(Consistency::Eventual { ttl_ms: 60_000 })
        .with_max_cache_entries(64);

    lake.register_backend(backend)?;

    // On boot, warm from the latest fleet bundle if present.
    let key = ("fs".to_string(), "fleet-experience".to_string());
    let fleet_bundle = "/var/fleet/experience/latest";
    if std::path::Path::new(fleet_bundle).exists() {
        let _ = lake.warm_from_dir(&key, fleet_bundle);
    }

    Ok(lake)
}

// Coordinator runs nightly: aggregates each robot's local experience
// into a fleet bundle, publishes it.
fn aggregate_and_publish(robot_dirs: &[PathBuf], out_dir: &Path) -> Result<()> {
    let merged_backend = MergedExperienceBackend::from_robot_dirs(robot_dirs)?;
    let coord = RuLake::new(20, fleet_seed());
    coord.register_backend(Arc::new(merged_backend))?;

    let key = ("merged".to_string(), "fleet-experience".to_string());
    coord.search_one("merged", "fleet-experience", &warmup_query(), 1)?;  // prime
    coord.publish_bundle(&key, out_dir)?;
    coord.save_cache_to_dir(&key, out_dir)?;
    Ok(())
}

// Inference time: a robot queries.
fn what_have_we_learned(lake: &RuLake, situation: &SituationEmbedding) -> Result<Vec<Memory>> {
    lake.search_one("fs", "fleet-experience", &situation.vec, 16)
}
```

The fleet bundle (typically a few hundred MB to a few GB depending on
fleet size and embedding dim) is published once per night to the fleet's
distribution channel — a CDN, an OTA bucket, or an mDNS-discoverable
local server. Each robot warms from it in seconds. Because the witness
is content-addressed, two robots with identical local cached bytes share
zero overhead at the storage layer.

### Gaps and what would have to change

- **Single-writer assumption.** The fleet has one coordinator that
  aggregates and publishes. Multi-coordinator (e.g. regional dispatch
  centers) requires a coordination layer above the bundle protocol —
  CRDT-ish merge, or witness-aware Last-Writer-Wins with a vector clock
  in the bundle metadata. Both are application-level today.
- **Fleet aggregation is application work.** `MergedExperienceBackend`
  in the pseudocode is a one-week adapter; real implementation depends
  on what the robots store (trajectories, action-value embeddings,
  scene graphs).
- **OTA distribution is not in scope.** ruLake produces bundles; getting
  bundles to robots (signed, version-pinned, A/B-rollback-able) is OTA
  framework work.
- **Per-robot LRU pressure.** A robot with 32 MB of cache budget and a
  fleet bundle with 100 collections can't keep them all primed. Today
  `with_max_cache_entries(n)` evicts unpinned LRU; a per-priority
  eviction policy ("keep navigation embeddings hot, evict cosmetic
  ones") is roadmap.

---

## Vertical 6 — Scientific reproducibility: portable provenance unit

### Why

The reproducibility crisis in computational science is partly a
data-versioning problem. A paper says "we used GLOVE-840B + the 2026
Wikipedia dump" and three years later the author can't even reconstruct
which exact dump. Datasets churn, bucket policies change, intermediate
caches get GC'd. The artifacts that *do* persist (paper PDF, GitHub
repo with code) don't carry the bit-state of the data they ran on.

The witness is exactly the missing portable provenance unit. A 300-byte
`table.rulake.json` sidecar — attached to a paper as supplementary
material, hashed into a Zenodo / OSF deposit, embedded in a README —
is enough to prove reproduction. A reviewer with the same backend bytes
warms a `RuLake`, refreshes against the sidecar, gets `UpToDate`. If the
backend bytes have drifted, they get `Invalidated`; if the bytes are
present but the witness mismatches, the verification surfaces the
exact discrepancy.

### Concrete implementation outline

```rust
// What an author puts in their supplementary material.
fn publish_paper_artifacts(lake: &RuLake, paper_dir: &Path) -> Result<()> {
    let supp = paper_dir.join("supplementary").join("rulake-bundles");
    std::fs::create_dir_all(&supp)?;

    for (backend, collection) in lake.referenced_collections() {
        let key = (backend.clone(), collection.clone());
        let sub = supp.join(format!("{backend}__{collection}"));
        std::fs::create_dir_all(&sub)?;
        lake.publish_bundle(&key, &sub)?;
        lake.save_cache_to_dir(&key, &sub)?;
    }
    Ok(())
}

// What a reviewer runs to verify.
fn verify_paper_artifacts(supp_dir: &Path) -> Result<VerifyReport> {
    let mut report = VerifyReport::default();
    let lake = RuLake::new(20, /* paper-published seed */ 42)
        .with_consistency(Consistency::Frozen);

    for entry in std::fs::read_dir(supp_dir)? {
        let dir = entry?.path();
        let (backend, collection) = parse_dirname(&dir)?;
        let key = (backend.clone(), collection.clone());
        match lake.warm_from_dir(&key, &dir) {
            Ok(n)  => report.warmed.push((collection, n)),
            Err(e) => report.failed.push((collection, e.to_string())),
        }
    }
    report.witness_chain = lake.cache_witness_summary();
    Ok(report)
}
```

The supplementary directory is small (the bundle is 300 bytes; the
`index.rbpx` snapshot is ~D/8 bytes per vector plus rerank overhead).
For a paper with a 1M-vector retrieval corpus at D=768, the snapshot is
~100 MB. That's well within Zenodo's 50 GB-per-record limit and routine
for OSF.

### Gaps and what would have to change

- **The witness covers the index state, not the embedding model.** Two
  papers with the same source documents but different embedding models
  produce different witnesses (because `pull_vectors` returns different
  bytes), which is correct — but a reviewer who cannot get the same
  embedding model artifacts cannot reproduce. Witness-as-provenance
  punts the model-versioning problem to a separate (well-understood)
  artifact: the embedding-model checkpoint hash.
- **No standard format for citing a witness in a paper.** A short
  conventional notation — `rulake:abcd…1234` — would help. Not ruLake's
  job to standardize but worth noting.
- **Cross-language reviewer access.** ruLake is Rust-only today. A Python
  reviewer needs a wheel that exposes `warm_from_dir` and `search`. The
  witness verification logic is small and a `pyo3` binding is a one-week
  project — feasible but not shipped.
- **Long-tail backend availability.** If the paper's backend was a
  proprietary database that no longer exists, warm-restart from the
  snapshot still works (the index is self-contained), but `Fresh` mode
  cannot validate against the original backend. For provenance the
  snapshot is sufficient; for repeat experimentation against new data,
  the original backend or a faithful replay is needed.

---

## Vertical 7 — Smart cities and digital twins: per-asset federated retrieval at city scale

### Why

A "digital twin" of a city (or a factory, or a port, or a power grid)
maintains an embedding for every asset — every traffic signal, every
pump, every transformer, every camera, every sensor stack. Retrieval
patterns:

- "Find traffic signals that look like the failing one we just identified
  on Main St" (similarity search over recent telemetry embeddings).
- "Which transformers are showing patterns similar to the one that failed
  in 2026?" (similarity over historical fault embeddings).
- "What asset class does this anomaly resemble?" (federated search across
  signal/pump/transformer/camera embedding shards).

Per-asset collection is the natural shard. Federation across asset classes
is the natural query pattern. Bundle rotation when sensors update is the
natural write event.

### Concrete implementation outline

```rust
// One backend per asset class.
fn build_city_lake() -> Result<RuLake> {
    let lake = RuLake::new(20, CITY_SEED)
        .with_consistency(Consistency::Eventual { ttl_ms: 30_000 })
        .with_max_cache_entries(2048);

    for class in &["signals", "pumps", "transformers", "cameras", "sensors"] {
        let backend = Arc::new(AssetClassBackend::new(class)?);
        lake.register_backend(backend)?;
    }
    Ok(lake)
}

// Anomaly triage — fan out across asset classes.
fn triage_anomaly(lake: &RuLake, anomaly: &AnomalyEmbedding) -> Result<Vec<MatchedAsset>> {
    let shards: Vec<(&str, &str)> = vec![
        ("signals", "telemetry-recent"),
        ("pumps",   "telemetry-recent"),
        ("transformers", "telemetry-recent"),
        ("cameras", "events-recent"),
        ("sensors", "events-recent"),
    ];
    lake.search_federated(&shards, &anomaly.vec, 50)
}

// Sensor update event triggers a bundle rotation.
fn on_telemetry_window_close(lake: &RuLake, class: &str, window: &str) -> Result<()> {
    let key = (class.to_string(), format!("telemetry-{window}"));
    let dir = format!("/var/twin/{class}/{window}");
    std::fs::create_dir_all(&dir)?;
    lake.publish_bundle(&key, &dir)?;
    Ok(())
}
```

`AssetClassBackend::generation` returns the latest telemetry-window
boundary (e.g. floor(time / 30s)). Bundle rotation cleanly invalidates
caches across the city's analyst workstations; cross-process witness
sharing means an analyst's laptop and the city's dashboard server share
one compressed cache copy.

### Gaps and what would have to change

- **Asset-class backends do not ship.** Each is a real adapter against
  the city's SCADA / GIS / video-management systems. Real work.
- **Per-asset granularity stresses the "shard count" envelope.** A city
  with 10,000 transformers isn't going to have one collection per
  transformer — that breaks the federation budget. The natural shard is
  asset-class × geographic-quadrant, which keeps shard counts in the
  10s–100s.
- **Privacy boundary.** Camera embeddings of public spaces touch privacy
  regimes (BIPA, GDPR Art. 9 if biometric). PII passthrough on the
  bundle is roadmap; today this requires application-layer filtering.
- **Event-driven publication is application-layer.** ruLake exposes
  `publish_bundle`; wiring it to MQTT / Kafka / cloud pub-sub is glue.

---

## Vertical 8 — Spaceflight and interplanetary: atomicity over light-second links

### Why

Deep-space missions communicate over multi-second to multi-minute
round-trips (Mars: 4–24 minutes one-way; Jupiter: 35–52 minutes).
Bandwidth is bounded by the deep-space network schedule and link budgets.
The on-spacecraft AI (perception, planning, fault management) needs:

1. **Atomic, witness-verified updates.** A bundle update from Earth
   that arrives mid-stream must be either applied wholly or not at all
   — no torn-write pathology that bricks the spacecraft's perception
   model. The bundle protocol's temp+rename+fsync pattern + SHAKE-256
   witness verification is exactly the primitive.
2. **Frozen reference data with versioned rollback.** Star catalogs,
   surface-feature embeddings, planned-trajectory waypoints — all
   reference data that the spacecraft pins and only updates on
   ground-controller request. `Consistency::Frozen` plus
   `warm_from_dir` from a cold radiation-hardened SSD is the natural
   primitive.
3. **Tiny bundle sidecars over rate-limited links.** A 300-byte sidecar
   plus a "fetch full snapshot from Earth at next pass" trigger is
   bandwidth-efficient. The full snapshot transfer happens during the
   scheduled communication window; the sidecar is the cheap "is the
   spacecraft current?" probe.

### Concrete implementation outline

```rust
// On-spacecraft setup.
fn boot_spacecraft_lake() -> Result<RuLake> {
    let lake = RuLake::new(20, MISSION_SEED)
        .with_consistency(Consistency::Frozen)
        .with_max_cache_entries(8);

    let backend = Arc::new(RadhardSsdBackend::new("/mnt/radhard/refdata")?);
    lake.register_backend(backend)?;

    // Warm every reference collection from the radhard SSD on boot.
    for collection in lake.list_collections("refdata")? {
        let key = ("refdata".to_string(), collection.name.clone());
        let dir = format!("/mnt/radhard/refdata/{}", collection.name);
        lake.warm_from_dir(&key, &dir).ok();  // tolerant on first boot
    }

    Ok(lake)
}

// Incoming uplink from Earth, scheduled comm window.
fn apply_ground_update(lake: &RuLake, uplink_dir: &Path) -> Result<()> {
    // The temp+rename atomicity in publish_bundle / save_cache_to_dir
    // ensures no torn writes — if we lose power mid-write, on next boot
    // the warm_from_dir reads the previous good snapshot.
    let key = parse_uplink_target(uplink_dir)?;
    lake.publish_bundle(&key, uplink_dir)?;
    lake.save_cache_to_dir(&key, uplink_dir)?;
    Ok(())
}

// Ground-controller probe: cheap sidecar fetch.
fn ground_probe_spacecraft_state(spacecraft_uplink: &SpacecraftLink) -> Result<WitnessReport> {
    // 300-byte fetch — fits in a single comm-window slot.
    let sidecar_bytes = spacecraft_uplink.fetch("table.rulake.json")?;
    let sidecar: RuLakeBundle = serde_json::from_slice(&sidecar_bytes)?;
    Ok(WitnessReport {
        witness_hex: hex::encode(&sidecar.witness),
        generation: sidecar.generation,
    })
}
```

The pattern is: ground probes the sidecar (cheap), decides whether to
schedule a full snapshot transfer (expensive, scheduled), uplinks during
next pass, spacecraft applies atomically. The witness is the cryptographic
ground-truth that ground and spacecraft agree on what bytes the spacecraft
holds.

### Gaps and what would have to change

- **Mutex-based hot path is not radiation-tolerant in the strict sense.**
  Single-event upsets are handled at the hardware/microkernel layer;
  ruLake assumes a working `std::sync::Mutex`. For Mars-rover-class
  systems this is fine (the OS abstracts SEU correction); for harder
  spacecraft the substrate would need to be wrapped in a
  fault-tolerant scheduler.
- **No power-aware eviction policy.** A spacecraft entering a low-power
  mode (eclipse, fault state) may want to flush all but the most critical
  collections. Today `with_max_cache_entries(n)` is a count cap, not a
  power-budget cap.
- **The `radhard SSD` backend is hypothetical.** A real spacecraft
  filesystem (RTEMS, LinuxRT, custom) needs an adapter. Trait is 4
  methods; the real challenge is the storage layer's quirks
  (write-cycle limits, ECC overhead, deterministic latency).
- **Cross-link distribution between multiple craft (formation flying)**
  is multi-writer territory; same single-writer caveat as the robotics
  fleet vertical applies.

---

## What ruLake would need to grow into

The verticals above stress different parts of the surface. Mapping
gaps to roadmap items:

### Backends (M2+, ADR-155 §M2)

Healthcare (Vertical 1), financial (V2), earth observation (V3),
genomics (V4), and smart cities (V7) all want a real cloud-backed
backend rather than `FsBackend`. The roadmap items:

| Backend          | Status   | Verticals that need it       | Approx. effort |
|------------------|----------|------------------------------|----------------|
| `ParquetBackend` | M2       | V3 (earth obs), V7 (cities)  | ~2–3 weeks     |
| `BigQueryBackend`| M2       | V4 (genomics, public refs)   | ~2 weeks       |
| `IcebergBackend` | M2       | V1 (healthcare lakehouse), V2 (banking) | ~3 weeks |
| `DeltaBackend`   | M2       | V1, V2 (Databricks shops)    | ~3 weeks       |
| `S3Backend`      | adjacent | V3, V4, V6 (publication artifacts) | ~1 week  |

The `BackendAdapter` trait is 4 methods; the real complexity is the
domain-side plumbing (catalog APIs, auth, schema discovery), not the
ruLake-side glue.

### Governance (M4, ADR-155 §M4)

V1 (healthcare PII), V2 (financial RBAC), V7 (city camera privacy),
and V4 (consortium genomics) all need:

- **PII passthrough via `rvf-federation::pii`.** Today the bundle has
  a `pii_policy` slot but no enforcement code path.
- **RBAC via OIDC/JWT.** Identity context plumbed through
  `BackendAdapter`. Today the trait is identity-blind.
- **OpenLineage emission.** `witness → lineage_id` mapping is documented
  in ADR-155 but not implemented.

### Wire layer (M2+, ADR-155 §M2)

V1 (replay across institutions), V2 (multi-region forensics), V3
(researcher-laptop ↔ cloud federation), V6 (cross-language reviewer
access) all want ruLake exposed as a service. HTTP / gRPC + OpenAPI
schema is the documented goal; today it's library-only.

### Multi-writer coordination

V5 (multi-coordinator robotics fleet), V8 (multi-spacecraft formation),
V2 (multi-region trading) all stress the single-writer assumption. The
bundle protocol assumes one publisher, many readers. Real multi-writer
is an unbounded research problem; a safer first step is a "Last
Witness Wins with vector clock" extension to the bundle metadata,
which keeps the existing format compatible.

### Sharded `CacheState`

Performance review item B1: the next concurrent-QPS ceiling above the
current ~37k is sharding the `CacheState` mutex by witness-bucket.
Verticals at high shard count and high concurrent reader count
(V3 earth-obs at K=300 shards, V4 genomics at consortium scale,
V7 cities with hundreds of analyst workstations) hit this first.

### Embedding-model versioning in the witness

V4 genomics and V6 reproducibility would benefit from
`(model_name, model_revision)` being first-class witness fields rather
than encoded implicitly via `data_ref`. Strictly additive: doesn't
break existing witnesses, just makes ambiguity-free reproduction easier.

### Per-collection consistency mode

V1, V3, V5 all want different freshness budgets per collection.
Today consistency is `RuLake`-wide; a per-collection override would
remove the "instantiate two `RuLake`s" workaround.

### Power-aware / priority-aware eviction

V5 (robotics under battery), V8 (spacecraft entering eclipse),
V3 (researcher laptop vs cloud worker) want eviction to know about
priority and cost-to-rebuild, not just LRU recency. A pluggable
eviction policy on the existing LRU substrate is achievable.

### NEON / WASM kernels in `ruvector-rabitq` (ADR-157)

V3 (analyst on ARM laptop), V5 (Jetson AGX), V8 (RISC-V spacecraft
processors), V6 (browser-based reviewer) all want non-x86 kernels.
ADR-157 is the scaffolding; the kernels themselves ship in separate
crates per the ADR. None are committed today.

---

## Ranking

Verticals by **leverage × feasibility on today's M1+M1.5 surface**:

| Rank | Vertical | Leverage | Feasibility today | Notes |
|------|----------|----------|-------------------|-------|
| 1 | **V6 — Scientific reproducibility** | high | high | Works on the M1+M1.5 surface as-is. Bundle + warm-restart + witness IS the deliverable. Only missing piece is a `pyo3` binding for cross-language reviewer access. Highest fit-to-substrate. |
| 2 | **V2 — Financial fraud + lineage** | very high | medium | Hot path works today (Fresh/Eventual + LocalBackend); regulatory-grade snapshot replay works (Frozen + warm_from_dir). What's missing — OpenLineage emission, RBAC — is application work above ruLake, not blockers. |
| 3 | **V1 — Healthcare CDS audit** | very high | medium | Same shape as V2; the PII enforcement gap is the binding constraint, not the substrate. With a reasonable PII wrapper, deployable on M1+M1.5. |
| 4 | **V8 — Spaceflight reference data** | high | medium | Bundle atomicity + Frozen + warm-restart + small sidecar is *exactly* the right primitive. Missing: power-aware eviction, RT-grade mutex story. Mission-design timescales tolerate this. |
| 5 | **V5 — Robotics fleet learning** | high | medium | FsBackend + bundle rotation + cross-process witness sharing maps cleanly. Multi-coordinator and OTA distribution are real-but-bounded gaps. |
| 6 | **V3 — Earth observation** | very high | low–medium | Substrate fits beautifully but needs Parquet/S3 backends to be real (M2 roadmap). Recall envelope at K=300 shards is unmeasured. Real but delayed. |
| 7 | **V7 — Smart cities digital twins** | high | low | Needs many backends, privacy guardrails, and event-driven publication wiring. Substrate is a fit; surrounding infrastructure is not. |
| 8 | **V4 — Genomics homology search** | very high | low | Substrate fits at proteome scale only after the lock-state contention work (B1). UniRef/PDB/MGnify backends are domain-specific. High eventual leverage; longer path. |

### Adjacent verticals worth a shorter look

The following weren't given their own sections to keep this note focused
on highest leverage, but each is a genuine fit for one specific ruLake
primitive:

- **Drug discovery** — molecular fingerprint similarity over consortium-shared
  embeddings. Same shape as V4 genomics but with smaller per-tenant
  collections; adaptive per-shard rerank is a perfect fit. Gated by an
  M2+ secure-multi-tenant story.
- **Brain-computer interfaces** — sub-10ms hit path is feasible per
  BENCHMARK; the substrate's deterministic mutex-based hot path is
  acceptable for non-life-critical BCI. `Eventual { ttl_ms: 250 }` for
  a calibration-drift window. Power and form-factor constraints
  dominate; substrate is not the binding constraint.
- **Education — personalized tutoring at scale** — per-student episodic
  memory keyed by the ADR-156 substrate guarantees. FERPA audit story
  via witness is real. Same V1/V2-shape gap on PII passthrough.
- **Cognitive prosthetics** — overlap with V8 (atomicity over rate-limited
  links between device and cloud) and V5 (per-device + shared fleet
  bundle). Form-factor and power dominate. Substrate fits.
- **Cryptocurrency / DeFi indexers** — witness as Merkle-style commitment
  is intriguing; bundle sidecars over IPFS as the publish layer would
  work. The gap is the single-writer assumption — DeFi indexers are
  fundamentally multi-writer. Plausible after multi-writer coordination
  lands.
- **Material discovery** — same shape as V4 genomics; crystal-structure
  embeddings federated across DOE-lab supercomputer datasets. Domain
  inertia (everyone uses bespoke pipelines) is bigger than substrate
  inertia.
- **Quantum-classical hybrid** — cached embeddings as input to QML;
  witness anchors the classical-side snapshot a quantum job acted on.
  Speculative; quantum hardware cycles aren't in the latency budget
  ruLake competes for. Worth revisiting in 3–5 years.
- **Adversarial / red-team research** — the audit-driven witness
  collision regression (`Num(7)` vs `Opaque("\x07\0…")`) is a model
  for ongoing threat-model work. Specifically: adversarial bundle
  sidecars in a federated mesh — what does an attacker who can
  poison one shard's bundle do to a federated query? `Frozen` mode
  is the answer for sensitive contexts; the open question is for
  `Eventual` workloads.

### One-line "do not over-claim" reminder

Every vertical above is a fit for the *substrate*. None of them are
"ruLake out of the box." Each requires a real adapter, real glue, and
domain expertise. The leverage is that the *witness-coherent vector
cache* primitive is the right substrate for these problems, and the
substrate ships today on M1+M1.5 — which is real progress against
the alternative of "build it from scratch every time."
