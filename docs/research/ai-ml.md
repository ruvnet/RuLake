# ruLake for AI/ML Systems

A white-paper-grade survey of where the ruLake cache-coherent vector
execution fabric fits in production AI/ML stacks: retrieval-augmented
generation, embedding pipelines, vector feature stores, recommendation
systems, training-data curation, model serving, and offline experiment
infrastructure. Grounded in the actual M1 + M1.5 ruLake API
(`/home/ruvultra/projects/RuLake/src/lib.rs`), the measured
performance envelope from
`/home/ruvultra/projects/RuLake/BENCHMARK.md`, and the architectural
positioning recorded in `docs/adrs/ADR-155` through `ADR-158`.

This document is deliberately honest about what is shipped today
(`LocalBackend`, `FsBackend`) versus what is roadmap (Parquet,
BigQuery, Iceberg, Delta, Snowflake adapters). Where a vertical
depends on M2+ backends, that dependency is called out and the
required adapter shape sketched against the existing
`BackendAdapter` trait.

---

## Table of contents

1. Where ruLake fits in the modern AI stack
2. Vertical 1 — Production RAG with witness-pinned provenance
3. Vertical 2 — Federated multi-tenant RAG
4. Vertical 3 — RAG over a lakehouse
5. Vertical 4 — Embedding-pipeline cache
6. Vertical 5 — Vector-aware feature store
7. Vertical 6 — Recommendation systems
8. Vertical 7 — Training-data deduplication and curation
9. Vertical 8 — Model-shard serving (aspirational)
10. Vertical 9 — A/B and counterfactual evaluation
11. Performance budgeting for AI workloads
12. Reality check and gaps
13. Open questions

---

## 1. Where ruLake fits in the modern AI stack

### 1.1 The "below the framework, above the storage" gap

The dominant AI-application frameworks of 2024-2026 — LangChain,
LlamaIndex, Haystack, DSPy, semantic-kernel — all converged on the
same shape for retrieval. Each defines a thin abstraction over a
vector store and delegates the actual work to "something":

- LangChain `VectorStore` interface defines `add_texts`,
  `similarity_search`, `similarity_search_with_score`, and a long
  tail of integrations (`PineconeVectorStore`, `WeaviateVectorStore`,
  `FAISS`, `Chroma`, `PGVector`, `Qdrant`, `LanceDB`, …). The
  framework owns the orchestration; the vector store owns the bytes
  and the index.
- LlamaIndex `VectorStoreIndex` wraps a `VectorStore` plus a
  `StorageContext` plus a `DocStore`. Same delegation pattern:
  retrieval is `vector_store.query(VectorStoreQuery, **kwargs)`.
- Haystack `DocumentStore` (since v2 the `Document Store` protocol
  with `count_documents`, `write_documents`, `filter_documents`)
  delegates to `InMemoryDocumentStore`, `ElasticsearchDocumentStore`,
  `PineconeDocumentStore`, etc.

Each of these frameworks treats the vector store as a *black box that
returns top-K with scores*. None of them owns the cache between the
application and the store. None of them owns coherence when the
underlying bytes change. None of them gives you a verifiable receipt
that "this answer was grounded in those vectors at that snapshot."
That entire problem is delegated downstream.

ruLake is the substrate that lives in exactly that delegated slot.
It does not replace LangChain's `VectorStore`; it slots underneath
it. A framework integration looks like:

```text
LangChain RetrievalQA
    │
    ▼
LangChain RuLakeVectorStore  ← thin adapter
    │
    ▼
RuLake::search_one / search_federated
    │ (cache hit at 1.02× direct RaBitQ)
    ▼
RaBitQ-compressed in-memory cache
    │ (miss → pull + prime)
    ▼
BackendAdapter (Parquet/BigQuery/Iceberg/Delta/RVF/local)
    │
    ▼
S3 / GCS / Iceberg catalog / lakehouse
```

The contract ruLake offers up to a framework is small and well-typed:

```rust
// from src/lake.rs:445
pub fn search_one(
    &self,
    backend: &str,
    collection: &str,
    query: &[f32],
    k: usize,
) -> Result<Vec<SearchResult>>

// from src/lake.rs:491
pub fn search_federated(
    &self,
    targets: &[(&str, &str)],
    query: &[f32],
    k: usize,
) -> Result<Vec<SearchResult>>

// from src/lake.rs:600
pub fn search_batch(
    &self,
    backend: &str,
    collection: &str,
    queries: &[Vec<f32>],
    k: usize,
) -> Result<Vec<Vec<SearchResult>>>
```

`SearchResult` (`src/lake.rs:27`) carries `(backend, collection, id,
score)`. The `backend` and `collection` fields are the load-bearing
audit anchor: a framework receiving results knows which substrate
slice produced each hit, and through `RuLake::cache_witness_of` it
can resolve that slice back to a SHAKE-256 witness over the bytes
that produced the cache entry.

### 1.2 What ruLake is *not*

The discipline of staying out of the framework's job is what makes
ruLake composable. Explicitly:

- **Not an embedding model.** ruLake never converts text or images
  into vectors. Callers are expected to embed externally (sentence-
  transformers, OpenAI ada, Cohere embed-v3, Voyage, etc.) and pass
  `&[f32]` queries.
- **Not a re-ranker.** ruLake's "rerank" knob is the RaBitQ rerank
  factor — the count of pre-filter candidates that get exact L2²
  scoring. It is not a cross-encoder, not a learning-to-rank model,
  not a relevance signal beyond raw vector distance.
- **Not a RAG framework.** ruLake does not chunk, does not prompt,
  does not answer. It returns top-K vector ids and scores. The
  framework above ruLake handles document hydration, prompt
  assembly, LLM invocation, and citation rendering.
- **Not an LLM serving platform.** ruLake has no concept of tokens,
  KV caches, batching policies for autoregressive generation, or
  speculative decoding. (Vertical 8 sketches what would have to
  change to make it relevant in that space — but that is explicitly
  aspirational.)
- **Not a vector database.** Per `README.md`'s comparison table,
  ruLake "does not own storage." It rides whatever durable byte
  store you already have. This is the entire point.

### 1.3 The three properties that matter for AI/ML

Across every vertical in this document, three measured properties of
ruLake do the load-bearing work:

**Property A — Witness-anchored cache coherence.** Every cache entry
is keyed by a SHAKE-256 digest over `(data_ref, dim, rotation_seed,
rerank_factor, generation)`. This is computed in
`src/bundle.rs:362-390` (`compute_witness`):

```rust
fn compute_witness(
    data_ref: &str,
    dim: usize,
    rotation_seed: u64,
    rerank_factor: usize,
    generation: &Generation,
) -> String {
    use sha3::{
        digest::{ExtendableOutput, Update},
        Shake256,
    };
    let mut h = Shake256::default();
    h.update(b"rulake-bundle-witness-v1|");
    // length-prefixed, domain-separated, generation-tag-byte protected
    // ...
}
```

Two processes pointing at the same bytes produce the same witness
and share one compressed copy in memory. For RAG this means an
agent and its evaluator can share the embedding store with verifiable
identity. For lakehouse RAG this means every reader sees the same
index after a bundle rotation.

**Property B — Three-mode consistency knob.** From `src/cache.rs:54`:

```rust
pub enum Consistency {
    Fresh,                       // per-query backend check
    Eventual { ttl_ms: u64 },    // skip-within-TTL
    Frozen,                      // pin until explicit refresh
}
```

This is the SLA dial. Different AI workloads want different
guarantees: a compliance-bound RAG over financial filings wants
`Fresh`; a recommendation system wants `Eventual { ttl_ms: 60_000 }`;
a counterfactual experiment wants `Frozen`. Same crate, same cache,
different SLA per collection.

**Property C — Federated execution with adaptive rerank.** From
`src/lake.rs:491-560`, `search_federated` fans out across registered
backends in parallel via Rayon, divides the rerank budget per shard
(`max(MIN_PER_SHARD_RERANK, global_rerank / K)`), and per-shard
over-requests `k' = k + ⌈√(k·ln S)⌉` to close the data-skew gap that
naive `k`-per-shard fan-out leaves open in Weaviate / OpenSearch.

These three properties show up over and over in the verticals below.
When a vertical lands cleanly on ruLake, it is because one or more
of A/B/C is doing work that is otherwise duplicated, fragile, or
absent.

### 1.4 Measured performance envelope

The numbers used throughout this document are from
`/home/ruvultra/projects/RuLake/BENCHMARK.md` (single reproducible
run on a Ryzen-class commodity laptop, deterministic seeds, release
build). They are not extrapolated or rounded for marketing.

| Metric                                        | Value                       |
|-----------------------------------------------|-----------------------------|
| Cache-hit tax vs direct RaBitQ                | 1.01–1.03×                  |
| Single-shard QPS, n=100k, D=128, rerank×20    | 3,542 (Fresh) / 3,626 (Eventual) |
| Concurrent QPS, 4 shards × 8 clients, n=100k  | 36,715                      |
| Cold-start prime, parallel Rayon, n=5k        | 4.5 ms                      |
| Cold-start prime, parallel Rayon, n=100k      | 37.6 ms                     |
| Cold-start prime, +Hadamard rotation, n=100k  | 142.9 ms (single-thread)    |
| Recall@10, single-shard, D=128, rerank×20     | ≥ 0.90                      |
| Recall@10, 4-shard adaptive rerank, D=128     | ≥ 0.85                      |
| Bundle-sidecar publish/refresh round-trip     | filesystem rename + 24 B header |
| Witness                                       | SHAKE-256(32) hex (64 bytes) |

These are the numbers a latency budget for a chatbot, RAG endpoint,
or recommendation API has to absorb. They are the floor (under
`LocalBackend`); a real M2+ Parquet/BigQuery/Iceberg backend will add
network RTT to the cold-path prime, but the warm-path numbers carry
over unchanged because the warm path never round-trips to the
backend.

---

## 2. Vertical 1 — Production RAG with witness-pinned provenance

### 2.1 The lineage problem in production RAG

The 2024-2025 generation of RAG deployments hit a wall that is now
well-recognized in the regulated-finance and healthcare-LLM
literature: when a model says "according to the 10-K…", you need to
prove *which version of which 10-K*. SOX-relevant analyst reports,
FINRA-supervised research notes, and HIPAA-regulated clinical
summaries all have an implicit provenance contract that "the answer
was grounded in document X at revision Y, and revision Y is
verifiable end-to-end."

LangChain and LlamaIndex offer "document IDs" and "metadata" fields
on retrieved chunks, but the integrity of those IDs is only as
strong as the discipline of the pipeline that wrote them. There is
no built-in mechanism to prove that "the chunk LangChain returned
under id `doc-12345` is the same chunk that was indexed at ingest
time, with the same embedding, from the same source bytes." If an
embedding pipeline is rebuilt, if a chunker is re-tuned, if a vector
store is re-indexed, the IDs survive but their meaning silently
drifts. This is the failure mode that gets flagged in audit and
post-incident review.

### 2.2 Witness-pinned chunks via the bundle protocol

ruLake's `RuLakeBundle` (`src/bundle.rs:113`) is exactly the
provenance anchor RAG pipelines need. The fields are:

```rust
pub struct RuLakeBundle {
    pub format_version: u32,        // 2 currently
    pub data_ref: String,           // URI of source bytes
    pub dim: usize,
    pub rotation_seed: u64,
    pub rerank_factor: usize,
    pub generation: Generation,     // backend coherence token
    pub rvf_witness: String,        // SHAKE-256(32) hex
    pub pii_policy: Option<String>,
    pub lineage_id: Option<String>, // OpenLineage job id
    pub memory_class: Option<String>,
}
```

A RAG ingest pipeline that publishes a bundle for each indexed
collection — call it `bundles/research-2025q1/table.rulake.json` —
gives every downstream retrieval a verifiable receipt. The witness
covers the `data_ref` (the URI of the source 10-K), the `dim` (the
embedding model's output size), the `rotation_seed` and
`rerank_factor` (the quantization parameters that fixed the cache
state), and the `generation` (a backend snapshot id, an Iceberg
snapshot UUID, a Delta CDF version).

Because the witness is computed deterministically and length-prefixed
(`src/bundle.rs:362-390`), and because `verify_witness` recomputes it
on every read (`src/lake.rs:200-228` — the `refresh_from_bundle_dir`
path), a tampered sidecar cannot silently poison the cache. Test
`fs_read_rejects_tampered_sidecar` (in `src/bundle.rs`) gates this
behavior.

### 2.3 Lineage_id wiring to OpenLineage

The `lineage_id: Option<String>` field on the bundle is intended to
carry an OpenLineage job id — the canonical "this artifact was
produced by that pipeline run" identifier. A RAG ingest pipeline
emits OpenLineage events for the `chunk → embed → index` job and
threads the resulting job id through into the bundle:

```rust
// At end of ingest job
let bundle = lake
    .publish_bundle(&("rag-store", "research-2025q1"), bundle_dir)?;
// Then publish a v3-style RuLakeBundle that includes the lineage_id:
let b = RuLakeBundle::new(
    "iceberg://catalog/research/embeddings",
    768,
    rotation_seed,
    rerank_factor,
    Generation::Opaque(snapshot_uuid),
)
.with_lineage_id("ol://ingest-v17/run-2026-04-25T03:14:59Z");
b.write_to_dir(bundle_dir)?;
```

(`with_lineage_id` is at `src/bundle.rs:270-273`.)

When a retrieval returns `SearchResult { backend, collection, id,
score }`, the calling RAG layer can look up the cache pointer's
witness via `lake.cache_witness_of(&(backend.into(), collection.into()))`
(`src/lake.rs:134`) and join it back to the OpenLineage event log to
produce a citation chain like:

```
Answer cites:
  10-K(Acme Corp, 2024 FY)
    · ingested by ol://ingest-v17/run-2026-04-25T03:14:59Z
    · index witness 7a3b…f201
    · iceberg snapshot 9ef…cba (matched at query time)
```

This is the citation-with-receipt that audit-grade RAG needs and
that today is bolted on with brittle out-of-band metadata stores.
ruLake puts the receipt in the substrate.

### 2.4 PII policy passthrough

The `pii_policy: Option<String>` field (`src/bundle.rs:138`) is
opaque to ruLake. The crate documentation is explicit about this: it
"doesn't interpret it in v1; it passes it through so governance
layers can enforce." For RAG pipelines this is the integration point
for PII tagging systems (Microsoft Presidio, Google Cloud DLP,
custom regex catalogs).

A pipeline that classifies each source document at ingest time
publishes the resulting policy handle on the bundle. A retrieval
endpoint that pulls a `SearchResult` joins the witness back to the
bundle, reads `pii_policy`, and applies the appropriate redaction or
access-control filter before showing the user.

### 2.5 Concrete topology — financial-research RAG

For a financial-research firm building a production RAG over SEC
filings, the topology is:

```
Source bytes:    s3://filings/10-K/{cik}/{accession}.html
Ingest job:      langchain.text_splitter.RecursiveCharacterTextSplitter
                 → cohere/embed-v3-multilingual
                 → write to iceberg://research/embeddings (snapshot N)
Bundle publish:  table.rulake.json with
                 data_ref="iceberg://research/embeddings",
                 dim=1024,
                 rotation_seed=DEPLOYMENT_SEED,
                 rerank_factor=20,
                 generation=Opaque(snapshot_uuid_N),
                 lineage_id="ol://research-ingest/run-{ts}",
                 pii_policy="gov://policies/sec-public"
Cache layer:     RuLake::new(20, DEPLOYMENT_SEED)
                   .with_consistency(Consistency::Eventual { ttl_ms: 300_000 })
                   .with_max_cache_entries(64)
Reader:          RAG endpoint calls lake.search_one("iceberg",
                   "research-2025q1", &query_emb, 20)
Audit:           For each SearchResult, lake.cache_witness_of(&key)
                   joined to OpenLineage emits the citation chain.
```

Iceberg here is a roadmap backend (M2+ — see §12), but the bundle
protocol, witness chain, and search path all work today against the
shipped `LocalBackend` and `FsBackend`. A research firm could
prototype the full pipeline against `FsBackend` writing `ruvec1`
files to disk and migrate to Iceberg when the adapter ships.

### 2.6 Why this isn't solved by metadata fields on existing vector stores

Pinecone, Weaviate, Qdrant, and Milvus all support per-vector
metadata. None of them have a cryptographic anchor over the *index
state itself*. If the index is rebuilt, metadata survives but its
guarantee that "this id maps to the same chunk" is purely procedural
— enforced only by pipeline discipline. ruLake's witness covers the
quantization state along with the data reference, so a re-quantized
index produces a new witness even on the same source bytes. That is
the property a financial regulator wants when asking "show me the
exact retrieval state at the time the answer was generated."

---

## 3. Vertical 2 — Federated multi-tenant RAG

### 3.1 The SaaS RAG provider topology

A growing class of products — Vectara, Mendable, Dust, Glean —
offers RAG-as-a-service: each tenant uploads their own corpus, and
the provider runs retrieval over per-tenant collections without
either tenant seeing the other's data. The traditional architectures
are:

- **Per-tenant indexes in a shared vector DB** (one Pinecone
  namespace per tenant, one Qdrant collection per tenant, etc.).
  Resource fairness across tenants is the vector DB's problem;
  cache-level isolation is implicit because each tenant's vectors
  occupy distinct storage.
- **Per-tenant vector DB instances** (one Qdrant per tenant, one
  Weaviate per tenant). Stronger isolation, much higher operational
  cost.
- **Shared embedding store with row-level access control** (PGVector
  with RLS, Vespa with ACLs). Compact, but the cache is shared
  across tenants and the consistency story is muddled.

ruLake's witness-addressed cache offers a fourth option that
combines the operational simplicity of the shared store with strong
per-tenant isolation: **per-tenant collections in a shared backend,
witness-isolated caches, fair LRU sizing across tenants.**

### 3.2 Cache isolation by witness

Each tenant's collection has a unique `data_ref` (e.g.
`s3://tenant-{id}/embeddings/`), which produces a unique witness
under `compute_witness` (`src/bundle.rs:362`). Cache entries are
keyed by witness (`src/cache.rs:1-10`):

> Wraps `ruvector_rabitq::RabitqPlusIndex`. Cache entries are keyed
> by the [`RuLakeBundle`] SHAKE-256 witness, NOT by `(backend_id,
> collection)`. Two backends serving the same logical dataset — same
> `data_ref`, same rotation seed, same rerank factor, same
> generation — produce the same witness and share one compressed
> cache entry.

The corollary is that **two tenants with different `data_ref`s never
share a cache entry**, because their witnesses are guaranteed
distinct. There is no risk of cross-tenant leakage through the
cache. A tenant's queries only ever see hits against their own
witness.

### 3.3 Fair LRU sizing

`RuLake::with_max_cache_entries(n)` (`src/lake.rs:78`) caps the cache
at `n` distinct compressed entries with LRU eviction over unpinned
entries (`src/cache.rs:548-565`). For a SaaS RAG provider, this is
the knob that makes capacity planning tractable:

```rust
let lake = RuLake::new(20, 42)
    .with_consistency(Consistency::Eventual { ttl_ms: 60_000 })
    .with_max_cache_entries(1024);  // ~1024 hot tenants in cache
```

Tenants whose collections have not been queried recently fall out of
the cache; the next query primes them back. The bound holds across
all tenants — there is no per-tenant cache-size knob to misconfigure.

### 3.4 Cross-tenant federation (the controlled case)

Some SaaS scenarios need cross-tenant federation: a workspace admin
querying across all their team members' personal corpora, a
benchmark-eval product running queries across opted-in customer
slices. `search_federated` (`src/lake.rs:491`) handles this
directly, with each tenant collection as a shard:

```rust
let hits = lake.search_federated(
    &[
        ("s3", "tenant-a-research"),
        ("s3", "tenant-b-research"),
        ("s3", "tenant-c-research"),
    ],
    &query,
    10,
)?;
```

The adaptive per-shard rerank (`max(5, global_rerank / K)`) and
per-shard over-request (`k' = k + ⌈√(k·ln S)⌉`) make the
shard-count-vs-recall tradeoff predictable. At K=3 with a global
rerank of 20, each tenant collection runs a per-shard rerank of 6,
and each over-requests `k'=12` for `k=10` — closing the data-skew
gap that hits Weaviate and Elasticsearch federated queries.

### 3.5 Per-tenant audit and observability

`RuLake::cache_stats_by_collection` (`src/lake.rs:126`) returns a
`HashMap<CacheKey, PerBackendStats>`. For the SaaS provider this
gives per-tenant hit rate, prime count, invalidation count, and
shared-hit count without any extra instrumentation:

```rust
for (key, stats) in lake.cache_stats_by_collection() {
    let (backend, collection) = key;
    metrics.tenant_hit_rate
        .with_label_values(&[&collection])
        .set(stats.hit_rate().unwrap_or(0.0));
    metrics.tenant_primes
        .with_label_values(&[&collection])
        .inc_by(stats.primes);
}
```

This is operationally what a SaaS RAG provider needs to spot a noisy
tenant whose constant ingest churn is invalidating the cache and
spiking prime cost across the system, or to identify a power tenant
whose query volume justifies a dedicated cache budget.

### 3.6 What's still on the SaaS provider's plate

ruLake doesn't solve the rest of multi-tenancy:

- **Authn/authz at the API gateway** — the SaaS provider still maps
  request → tenant_id → cache key and validates that the requester
  has rights to that tenant's corpus.
- **PII handling per tenant policy** — the `pii_policy` field on the
  bundle carries a handle, but the actual redaction/masking lives in
  the application layer.
- **Cost attribution** — query QPS per tenant, prime ms per tenant,
  embedding API spend per tenant. The `PerBackendStats` numbers feed
  these reports but the billing logic is outside the substrate.

What ruLake does solve cleanly: cache isolation by witness, fair
memory budgeting via LRU, and verifiable per-tenant lineage through
the bundle.

---

## 4. Vertical 3 — RAG over a lakehouse

### 4.1 The vector-column-in-a-table pattern

Iceberg, Delta, and Apache Hudi increasingly support array<float>
columns suitable for storing embeddings alongside the source rows.
The pattern is:

```sql
CREATE TABLE research.papers (
    id BIGINT,
    title STRING,
    abstract STRING,
    embedding ARRAY<FLOAT>,   -- e.g. dim=768 for ada-v2
    snapshot_id BIGINT,
    PRIMARY KEY (id)
) USING iceberg
PARTITIONED BY (snapshot_id);
```

Storing the vectors in the table — rather than in a sidecar vector
DB — is operationally attractive: one ACID storage layer, one set of
backups, one access-control surface. The hard part is *querying* the
vector column. Iceberg has no built-in ANN index. Delta's vector
search is an early Databricks feature with limitations. Hudi
likewise.

The lakehouse model says "your data is here." The query path needs
something to make ANN search over that data fast. ruLake is exactly
that something: a cache layer that primes from the table on snapshot
rotation and serves at compressed-index speed.

### 4.2 Bundle generation = table snapshot id

The cleanest fit between ruLake's bundle protocol and a lakehouse is
to use the table snapshot id as the bundle's `Generation`. From
`src/bundle.rs:54-61`:

```rust
pub enum Generation {
    Num(u64),
    Opaque(String),
}
```

For Iceberg, `Generation::Num(snapshot_id)` (Iceberg snapshot ids
are i64 positive). For Delta, `Generation::Num(version)`. For Hudi,
`Generation::Opaque(commit_time)`.

Because the witness is computed over `(data_ref, dim, rotation_seed,
rerank_factor, generation)`, **the cache invalidates exactly when
the snapshot rolls.** No polling, no diffing, no manual refresh — a
new snapshot → new generation → new witness → next coherence check
sees the mismatch and triggers a re-prime.

### 4.3 BackendAdapter trait for an Iceberg catalog

The `BackendAdapter` trait (`src/backend.rs:110`) has four required
methods plus an optional fifth:

```rust
pub trait BackendAdapter: Send + Sync {
    fn id(&self) -> &str;
    fn list_collections(&self) -> Result<Vec<CollectionId>>;
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch>;
    fn generation(&self, collection: &str) -> Result<u64>;
    fn current_bundle(
        &self,
        collection: &str,
        rotation_seed: u64,
        rerank_factor: usize,
    ) -> Result<crate::RuLakeBundle> { /* default impl */ }
    fn supports_pushdown(&self) -> bool { false }
}
```

A pseudocode `IcebergBackend` for vector-column tables looks like:

```rust
pub struct IcebergBackend {
    id: String,
    catalog: Arc<dyn IcebergCatalog>,
    namespace: String,
}

impl BackendAdapter for IcebergBackend {
    fn id(&self) -> &str { &self.id }

    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        // Each Iceberg table is a collection.
        self.catalog
            .list_tables(&self.namespace)
            .map(|ts| ts.into_iter().map(|t| t.name).collect())
            .map_err(into_rulake_err)
    }

    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        let table = self.catalog
            .load_table(&self.namespace, collection)?;
        let snapshot = table.current_snapshot()
            .ok_or_else(|| empty_collection())?;
        // Use Iceberg's data-file scan — pushed down to Parquet readers.
        let scan = table.new_scan().snapshot_id(snapshot.snapshot_id());
        let mut ids = Vec::new();
        let mut vectors = Vec::new();
        let mut dim = 0;
        for batch in scan.to_arrow()? {
            let id_col = batch.column_by_name("id")?
                .as_any().downcast_ref::<Int64Array>()?;
            let emb_col = batch.column_by_name("embedding")?
                .as_any().downcast_ref::<ListArray>()?;
            for row in 0..batch.num_rows() {
                ids.push(id_col.value(row) as u64);
                let v = emb_col.value(row);
                let vf = v.as_any().downcast_ref::<Float32Array>()?;
                if dim == 0 { dim = vf.len(); }
                vectors.push(vf.values().to_vec());
            }
        }
        Ok(PulledBatch {
            collection: collection.to_string(),
            ids, vectors, dim,
            generation: snapshot.snapshot_id() as u64,
        })
    }

    fn generation(&self, collection: &str) -> Result<u64> {
        let table = self.catalog
            .load_table(&self.namespace, collection)?;
        Ok(table.current_snapshot()
            .ok_or_else(|| empty_collection())?
            .snapshot_id() as u64)
    }

    fn current_bundle(
        &self,
        collection: &str,
        rotation_seed: u64,
        rerank_factor: usize,
    ) -> Result<crate::RuLakeBundle> {
        let table = self.catalog
            .load_table(&self.namespace, collection)?;
        let snapshot = table.current_snapshot()
            .ok_or_else(|| empty_collection())?;
        // Read schema for dim — no full pull required.
        let schema = table.schema();
        let emb_field = schema.field_by_name("embedding")?;
        let dim = emb_field.list_element_count();
        Ok(crate::RuLakeBundle::new(
            format!(
                "iceberg://{}/{}/{}@snapshot={}",
                self.id, self.namespace, collection,
                snapshot.snapshot_id()
            ),
            dim,
            rotation_seed,
            rerank_factor,
            crate::Generation::Num(snapshot.snapshot_id() as u64),
        ))
    }
}
```

The crucial design move is overriding `current_bundle` to read only
the schema (for `dim`) without pulling vectors, mirroring what
`FsBackend::current_bundle` does (`src/fs_backend.rs:321-355`). A
naive default implementation would call `pull_vectors` to derive
`dim`, which on a million-row Iceberg table would be catastrophic on
the coherence-check hot path.

### 4.4 Snapshot-aligned cache hits

The flow on query becomes:

1. Query arrives. Calling code invokes
   `lake.search_one("iceberg", "research_papers", &q, 10)`.
2. `ensure_fresh` (`src/lake.rs:638`) asks the backend for its
   current bundle. `IcebergBackend::current_bundle` reads the
   table's current snapshot id without pulling vectors.
3. Witness comparison:
   - **Match** — cache pointer is fresh; serve hits.
   - **Mismatch but witness already in pool** — another collection
     pointer (or another `RuLake` process via the sidecar protocol)
     primed this snapshot's witness. Pointer moves, zero prime work.
     This is the `shared_hits` counter (`src/cache.rs:88`).
   - **Mismatch and witness new** — pull vectors, compress with
     RaBitQ, prime the cache, serve.
4. Search runs against the RaBitQ-compressed cache: ~0.27 ms/query
   at n=100k under `Consistency::Eventual` (BENCHMARK.md headline).

The snapshot-aligned cache hit is the property that makes lakehouse
RAG actually fast. A query against a 100k-row vector column on
Iceberg, served from the ruLake cache, runs at 3,626 QPS warm-cache
single-thread — same speed as direct RaBitQ. The Iceberg layer is
*only* consulted on cold start or snapshot rotation.

### 4.5 Atomic snapshot rotation via the sidecar protocol

The Iceberg ingest job that produces snapshot N+1 can publish a new
ruLake bundle sidecar to a known location:

```rust
// In the ingest job, after committing snapshot N+1:
let lake_publisher = RuLake::new(rerank_factor, rotation_seed);
lake_publisher.register_backend(Arc::new(IcebergBackend::new(...)))?;
// Force a prime so we have a witness to publish, then write the sidecar.
let _ = lake_publisher.search_one("iceberg", "research_papers", &warmup_q, 1)?;
lake_publisher.publish_bundle(
    &("iceberg".into(), "research_papers".into()),
    "/var/rulake/sidecars/iceberg/research_papers/",
)?;
```

Reader processes mount that sidecar directory (e.g. via S3 + a
local cache, or via NFS, or via a mounted GCS prefix) and run a
sidecar daemon:

```rust
loop {
    match lake.refresh_from_bundle_dir(&key, sidecar_dir)? {
        RefreshResult::Invalidated => {
            metrics.snapshot_rotations.inc();
            log::info!("ruLake cache rotated for {:?}", key);
        }
        RefreshResult::UpToDate => {}
        RefreshResult::BundleMissing => {
            metrics.bundle_missing.inc();
        }
    }
    std::thread::sleep(Duration::from_secs(5));
}
```

(This pattern is shipped as `examples/sidecar_daemon.rs`.)

The result: a snapshot rotation in Iceberg propagates to every
reader's ruLake cache within 5 seconds, atomically, with witness
verification on the way in. No central control plane, no consensus
protocol, no coordination outside the bundle file's atomic rename.

### 4.6 What makes this M2+ work

The shipped `BackendAdapter` trait, the bundle protocol, the cache,
the federation path, the sidecar protocol — all of it works against
the contract above. The missing piece is the actual `IcebergBackend`
crate (and its peers `DeltaBackend`, `ParquetBackend`,
`BigQueryBackend`). Per `BENCHMARK.md` §"What's NOT benchmarked":

> - **Real-backend network latency.** `LocalBackend::pull_vectors`
>   is an in-process HashMap read; the Fresh-mode tax reported above
>   is the floor, not the ceiling. Real backends (Parquet on S3,
>   BigQuery via Storage Read API) add 10-100 ms per prime. Measured
>   numbers land in M2.

A Parquet/Iceberg/Delta adapter is shaped exactly like `FsBackend`
(`src/fs_backend.rs`) — a 250-line reference impl plus its
backend-specific decoder. The trait is correctly factored to make
this a per-adapter crate concern.

---

## 5. Vertical 4 — Embedding-pipeline cache

### 5.1 The recompute-on-rebuild problem

Embedding inference is expensive. A modern ingest pipeline running
sentence-transformers/all-mpnet-base-v2 over a 10-million-document
corpus takes hours of GPU time; the same pipeline running OpenAI
ada-v2 costs hundreds to thousands of dollars in API spend. When
the pipeline is re-run — for any reason: source corpus updated,
embedding model upgraded, chunk size re-tuned — the cost is paid
again.

Most production embedding pipelines today implement bespoke caching:
"hash the source text, look up in Redis/PostgreSQL, fall through to
inference if miss." This works but it's:

- Per-pipeline reimplementation (every team rolls their own).
- Cache coherence is on the implementer (when does a hash collide
  with a stale entry?).
- No cross-process or cross-deployment sharing.
- No model-revision discipline (does an upgrade from `ada-001` to
  `ada-002` invalidate? from `ada-002.20240101` to
  `ada-002.20240115`?).

### 5.2 ruLake as the embedding cache (with the witness as the hash)

The pattern is:

```
Witness input:  (model_name, model_revision, source_doc_hash)
Cache value:    The vector itself (as a 1-row "collection")
Backend:        Either RVF (durable witnessed segments) or FsBackend
                (`ruvec1` files), or an inference-on-miss adapter.
```

The witness here is doing exactly what an embedding cache key needs
to do: it is a cryptographic anchor over *the parameters that
determine what the embedding should be*. If any of those parameters
change — model upgrade, revision bump, source text edit — the
witness changes, the cache misses, and the pipeline recomputes only
the changed entries.

### 5.3 Frozen consistency for stable model+rev

For a deployed embedding model at a fixed revision, `Frozen`
consistency (`src/cache.rs:77`) is the right mode. The semantics
from the source:

> Caller asserts the bundle at this `(backend, collection)` key is
> immutable for the cache's lifetime — never re-check the backend,
> never invalidate on generation bump. Designed for witness-sealed
> historical snapshots: the audit tier.

The embedding cache for `(model=ada-v2, rev=20250101)` is exactly a
witness-sealed historical snapshot. Once primed, it never needs to
round-trip to the backend again.

### 5.4 Cross-pipeline sharing

The witness-addressed cache (per `src/cache.rs:1-10`) means two
pipelines running with the same `(model_name, model_revision,
source_doc_hash)` share one in-memory copy. For an organization
running multiple RAG products against the same source corpus —
research, search, customer support, internal Q&A — the embedding
cache is shared across all of them at the substrate level, no
coordination required.

The `shared_hits` counter (`src/cache.rs:88`) reports when this
happened: the cache resolved a `(backend, collection)` pointer to a
witness already cached under another pointer, and zero prime work
was done. For an embedding pipeline, every `shared_hit` is a
recomputation avoided.

### 5.5 Save-and-warm-restart for inference replay

`save_cache_to_dir` and `warm_from_dir` (`src/lake.rs:263, 378`) let
the embedding cache survive process restart without re-running
inference:

```rust
// At end of ingest job
lake.save_cache_to_dir(
    &("rvf".into(), "embeddings-2026-q1".into()),
    "/var/embeddings/snapshots/2026-q1/",
)?;

// At next pipeline run
let fresh = RuLake::new(rerank_factor, rotation_seed)
    .with_consistency(Consistency::Frozen);
let n = fresh.warm_from_dir(
    &("rvf".into(), "embeddings-2026-q1".into()),
    "/var/embeddings/snapshots/2026-q1/",
)?;
log::info!("warmed {n} embeddings without inference");
```

The on-disk format is two files: `index.rbpx` (the RaBitQ-compressed
codes) and `table.rulake.json` (the bundle sidecar with witness).
The bundle's witness covers all the parameters that must match the
original, so a tampered or mismatched snapshot fails loudly:

> `warm_from_dir: index dim {} != bundle dim {}` (`src/lake.rs:407`)
> `warm_from_dir: index rerank_factor {} != bundle rerank_factor {}` (`src/lake.rs:414`)

### 5.6 Inference-on-miss backend adapter

A custom `BackendAdapter` whose `pull_vectors` actually calls the
embedding API closes the loop:

```rust
pub struct InferenceBackend {
    id: String,
    model: Arc<dyn EmbeddingModel>,  // sentence-transformers, ada, etc.
    source_store: Arc<dyn SourceStore>,
}

impl BackendAdapter for InferenceBackend {
    fn id(&self) -> &str { &self.id }

    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        self.source_store.list_corpora()
    }

    fn pull_vectors(&self, corpus: &str) -> Result<PulledBatch> {
        let docs = self.source_store.load_corpus(corpus)?;
        let texts: Vec<&str> = docs.iter().map(|d| d.text.as_str()).collect();
        let vectors = self.model.embed_batch(&texts)?;
        Ok(PulledBatch {
            collection: corpus.to_string(),
            ids: docs.iter().map(|d| d.id).collect(),
            vectors,
            dim: self.model.output_dim(),
            generation: self.source_store.corpus_version(corpus)?,
        })
    }

    fn generation(&self, corpus: &str) -> Result<u64> {
        self.source_store.corpus_version(corpus)
    }

    fn current_bundle(
        &self,
        corpus: &str,
        rotation_seed: u64,
        rerank_factor: usize,
    ) -> Result<crate::RuLakeBundle> {
        Ok(crate::RuLakeBundle::new(
            format!(
                "embed://{model}@{rev}/{corpus}",
                model = self.model.name(),
                rev = self.model.revision(),
                corpus = corpus,
            ),
            self.model.output_dim(),
            rotation_seed,
            rerank_factor,
            crate::Generation::Num(self.source_store.corpus_version(corpus)?),
        ))
    }
}
```

The first query against a `(model, rev, corpus)` triple primes the
cache by running inference. Every subsequent query — across the same
process, across other processes warming the same bundle, across
restarts — serves from the compressed cache at ~0.27 ms/query
warm-cache speed. The witness-anchored sharing means the inference
cost is paid once per witness, not once per query and not once per
process.

---

## 6. Vertical 5 — Vector-aware feature store

### 6.1 The dual offline/online problem

Feature stores (Tecton, Feast, Hopsworks, Vertex AI Feature Store)
solve a coherence problem: training reads features from a batch
store; serving reads features from an online store; the two stores
must be derived from the same logical feature definition or the
model's training/serving skew goes off. The pattern is:

```
Feature definition (declarative)
    │
    ├── Offline materialization → Snowflake / BigQuery / Iceberg
    │       (training)
    │
    └── Online sync → Redis / DynamoDB / Bigtable
            (serving, low-latency)
```

For non-vector features this works well. Scalar floats and integers
fit cleanly into KV stores. Vector features (user embeddings, item
embeddings, multi-modal joint embeddings) break the pattern: they
are too large to put in Redis efficiently, the indexes are not
shared between offline and online stores, and the consistency story
between training and serving becomes "trust the pipeline."

### 6.2 Witness-coherent offline/online via shared bundles

ruLake offers a different decomposition. The vector feature *itself*
lives in one durable backend (the lakehouse, RVF, S3 Parquet); both
offline and online paths sit on top of ruLake reading that backend,
and the witness chain guarantees they see the same compressed bytes.

Concretely:

```
                     ┌─────────────────────────────┐
                     │    user_embeddings table    │
                     │     (Iceberg snapshot N)    │
                     │     RuLakeBundle witness    │
                     │     7a3b…f201               │
                     └──────────────┬──────────────┘
                                    │
                  ┌─────────────────┴─────────────────┐
                  │                                   │
        ┌─────────▼──────────┐               ┌────────▼──────────┐
        │  Offline trainer   │               │  Online server    │
        │  RuLake instance A │               │  RuLake instance B │
        │  Frozen mode       │               │  Eventual 60s mode │
        │  cache witness     │               │  cache witness     │
        │     7a3b…f201      │               │     7a3b…f201      │
        └────────────────────┘               └────────────────────┘
                  │                                   │
                  │     CROSS-PROCESS SHARING:        │
                  │  Same witness → same compressed   │
                  │  bytes in both processes' cache   │
                  └───────────────┬───────────────────┘
                                  │
                  Two processes, one in-memory
                  compressed copy (when on same host),
                  identical ranking semantics anywhere.
```

The training job runs `Consistency::Frozen` against a specific
snapshot to guarantee no drift during the training run. The online
serving job runs `Consistency::Eventual { ttl_ms: 60_000 }` to
absorb a live snapshot rotation within 60 seconds. Both produce
byte-identical retrieval results when queried against the same
snapshot, because the witness is byte-identical and the cache codes
are deterministic in the rotation seed.

### 6.3 The training-serving skew guarantee

Training-serving skew for vector features is the production failure
mode that kills recommendation models. The model trained against a
particular embedding store and the model served against an updated
embedding store rank items in subtly different orders, and offline
A/B win rates fail to reproduce online.

Under the ruLake topology:

- The `rotation_seed` is a deployment-wide constant.
- The `rerank_factor` is a deployment-wide constant.
- The `data_ref` and `generation` are the same when both sides
  reference the same snapshot.
- Therefore the witness is the same.
- Therefore the RaBitQ codes are the same.
- Therefore the ANN search is the same.

The substrate guarantee is mechanical, not aspirational. Test
`rulake_matches_direct_rabitq_on_local_backend`
(`tests/federation_smoke.rs`) gates the byte-exact match between
ruLake's federation path and direct RaBitQ at the same seed.

### 6.4 Comparison with Tecton / Feast / Hopsworks

| Concern              | Tecton        | Feast         | Hopsworks     | ruLake-on-lakehouse |
|----------------------|---------------|---------------|---------------|---------------------|
| Vector feature store | Recent add-on | Recent add-on | Yes (FG)      | Yes (substrate)     |
| Online ANN search    | Pinecone/etc. | Faiss/etc.    | OpenSearch    | RaBitQ in-memory    |
| Witness over codes   | No            | No            | No            | SHAKE-256           |
| Cross-process share  | No            | No            | No            | Yes (witness)       |
| Snapshot pinning     | Partial       | No            | No            | `Consistency::Frozen` |
| Backend agnosticism  | Bound         | Plugin        | Bound         | Trait-defined       |

The point is not that ruLake replaces a feature store. The feature
store still owns feature definitions, materialization scheduling,
point-in-time joins, and online sync orchestration. What ruLake
adds, specifically for vector features, is the substrate guarantee
that offline and online see the same compressed bytes via the same
witness.

### 6.5 Memory-class tag for feature taxonomy

The `memory_class: Option<String>` field on the bundle
(`src/bundle.rs:144-154`) is intentionally opaque to ruLake but
useful for higher-level taxonomy. A feature store layered on top
might set:

- `memory_class: "user-embedding"` for per-user vectors.
- `memory_class: "item-embedding"` for catalog items.
- `memory_class: "joint-multi-modal"` for image-text joint
  embeddings.
- `memory_class: "session"` for short-lived behavioral vectors.

The `cache_stats_by_collection` (`src/lake.rs:126`) view already
gives per-collection attribution, and a future extension might
aggregate stats by `memory_class`. For now the tag is observed
through the bundle when feature inventory tools introspect the
substrate.

---

## 7. Vertical 6 — Recommendation systems

### 7.1 Two-tower retrieval as the canonical case

The two-tower model (Yang et al., 2020, "Mixed Negative Sampling for
Learning Two-Tower Neural Networks in Recommendations"; further
developed in Google's Pinpoint/Deep Retrieval line) is the
production architecture for large-scale recommendation:

```
Query tower (run per request):
    user features + context + recent history
        → user-embedding ∈ R^d

Candidate tower (run offline + indexed):
    item features + content + popularity + freshness
        → item-embedding ∈ R^d

Retrieval:
    top-K(item-embedding) by cosine(user-embedding, item-embedding)
```

The query tower runs every request — typically a small model on CPU
or a fast GPU inference. The candidate tower runs offline over the
catalog (millions to billions of items) and produces a vector index
that the query tower searches against.

The retrieval substrate is the bottleneck. Industry deployments use
Faiss IVF/HNSW, ScaNN, Vespa's nearest-neighbor index, or
proprietary stacks. ruLake fits this slot with three specific
advantages: cache-coherent updates as the catalog rotates, witness-
verified candidate-tower output, and federated retrieval across
sharded catalogs.

### 7.2 Federated retrieval across user/item/context shards

Real recommendation systems shard their candidate index by multiple
axes:

- **Item shards** — catalog partitioned by category, freshness, or
  popularity tier. A "fresh items" shard is rebuilt every few
  minutes; an "evergreen" shard rotates daily.
- **User shards** — user-similarity partitioned, useful for
  collaborative filtering.
- **Context shards** — geo-partitioned, language-partitioned, or
  audience-partitioned slices.

`search_federated` (`src/lake.rs:491`) is the query-side merge:

```rust
let hits = lake.search_federated(
    &[
        ("rvf", "items-fresh"),       // updates every 5 min
        ("rvf", "items-evergreen"),   // updates daily
        ("rvf", "items-promoted"),    // updates hourly
    ],
    &user_embedding,
    100,  // top-100 candidates for downstream re-ranking
)?;
```

The adaptive per-shard rerank (`max(5, global_rerank / K)` —
`src/lake.rs:474, 511-519`) keeps the rerank budget roughly
constant in K. With `rerank_factor=20` and K=3 shards, each shard
runs `rerank_factor=6` and the global merge produces the top-100
correctly, with measured recall@10 ≥ 85% on clustered data
(BENCHMARK.md "concurrent clients × shard count").

The per-shard over-request `k' = k + ⌈√(k·ln S)⌉` (`src/lake.rs:546-560`)
is the recall safety margin: at k=100, S=3, each shard returns
top-103 (`100 + ⌈√(100·1.099)⌉ = 111`) instead of top-100, so
data-skew across shards (one shard happening to hold a
disproportionate fraction of the true top-100) doesn't drop hits.

### 7.3 Latency budget breakdown for a recommendation request

A typical recommendation API SLO is p99 ≤ 100 ms end-to-end.
Decomposing the budget for a two-tower system on ruLake:

| Step                                       | Budget        | Floor        | Notes |
|--------------------------------------------|---------------|--------------|-------|
| Auth + request parse                       | 5 ms          | 1 ms         | gateway |
| User-tower embedding (small model)         | 15-30 ms      | 8 ms (CPU)   | sentence-transformers ~50M params |
| ruLake federated search (3 shards, k=100)  | 5-10 ms       | 0.5 ms       | warm cache, 36k QPS at 4-shard means ~0.11 ms/query in steady state |
| Re-ranker (cross-encoder over top-100)     | 30-50 ms      | 20 ms        | typical ms-BERT 100M params |
| Business logic / impression dedup / ranker | 5-10 ms       | 2 ms         | application |
| Response serialization + return            | 5 ms          | 1 ms         | gateway |
| **Total**                                  | **65-110 ms** | **32.5 ms**  | floor with all caches warm |

ruLake's contribution to the budget is essentially zero on the warm
path. The cold path is bounded by prime time:

| n             | Serial prime | Parallel prime | + Hadamard | Notes |
|---------------|--------------|----------------|------------|-------|
| 5,000         | 22 ms        | 4.5 ms         | 7.2 ms     | per-shard for small fresh shard |
| 50,000        | 213 ms       | 19.6 ms        | 72.7 ms    | per-shard for medium shard |
| 100,000       | 421 ms       | 37.6 ms        | 142.9 ms   | per-shard for large shard |

A million-item catalog sharded into 10 shards of 100k each, primed
in parallel via Rayon, takes ~38 ms/shard concurrent. With Hadamard
rotation and the per-shard parallel prime, even a cold start of a
serving fleet fits within seconds, not minutes.

### 7.4 Witness-pinned A/B testing for recommendation models

A common pattern in recommendation work is shadow-traffic A/B
testing: a candidate model serves alongside the baseline, both
receive the same query, and the offline evaluator compares ranking
quality. For this to be a fair comparison, both models must see the
*same retrieval candidates* — otherwise differences in observed
ranking quality conflate model differences with retrieval
differences.

`Consistency::Frozen` against a witness-pinned snapshot guarantees
both models see byte-identical retrieval:

```rust
let baseline = RuLake::new(20, 42).with_consistency(Consistency::Frozen);
let candidate = RuLake::new(20, 42).with_consistency(Consistency::Frozen);

baseline.warm_from_dir(&key, "/snapshots/items-2026-04-25/")?;
candidate.warm_from_dir(&key, "/snapshots/items-2026-04-25/")?;

// Both processes have witness-identical caches.
assert_eq!(
    baseline.cache_witness_of(&key),
    candidate.cache_witness_of(&key),
);

// Their retrieval outputs are byte-identical.
let q = embed(&request);
let h1 = baseline.search_one("rvf", "items", &q, 100)?;
let h2 = candidate.search_one("rvf", "items", &q, 100)?;
assert_eq!(h1, h2);  // gated by determinism
```

Test `rulake_matches_direct_rabitq_on_local_backend`
(`tests/federation_smoke.rs`) is the substrate-level guarantee that
makes the assertion above mechanical, not aspirational.

### 7.5 Streaming catalog updates with bundle rotation

Modern recommendation catalogs are not static. Items go in and out
of stock, prices change, content gets re-categorized, new items
arrive every minute. A typical pattern:

1. Streaming pipeline (Flink, Beam, Spark Streaming) consumes
   inventory events.
2. Every 5 minutes, the pipeline writes a new snapshot of the items
   table (Iceberg / Delta).
3. The pipeline publishes a fresh `RuLakeBundle` sidecar.
4. Reader processes' cache-sidecar daemons pick up the new bundle
   within seconds, invalidate the stale cache, and re-prime on the
   next miss.

The bundle protocol's atomic write semantics (`src/bundle.rs:291-332`,
test `fs_write_is_atomic_under_crash_simulation`) mean a reader
never sees a torn sidecar even if the publisher crashes mid-write.
The witness check on read (`src/lake.rs:200-228`) means a
maliciously-tampered sidecar is rejected before any cache state is
mutated. The result is a streaming-update path that is operationally
simple — no central coordination, no consensus protocol — and
cryptographically robust.

---

## 8. Vertical 7 — Training-data deduplication and curation

### 8.1 The Common Crawl problem

Training-data curation for large language models is dominated by
deduplication. Common Crawl chunks land in multiple S3 prefixes,
Wikipedia revisions duplicate across snapshots, code repositories
get mirrored, and the same document fragment can appear in dozens
of derivative corpora. Effective deduplication is one of the levers
that produces the largest training-quality wins (the
RefinedWeb/FineWeb/SlimPajama line of work has documented this in
detail).

The standard approach is locality-sensitive hashing (LSH) over
n-grams or MinHash signatures, with cluster-based filtering. The
expensive step is the all-pairs similarity computation across
billions of candidate chunks.

### 8.2 Witness-content-addressed dedup

ruLake's witness-addressed cache offers a different angle on the
problem. The witness covers `(data_ref, dim, rotation_seed,
rerank_factor, generation)`. If two pipelines both index the same
chunk into ruLake under the same `(data_ref, generation)`, they
produce the same witness and share one compressed cache entry. The
deduplication is implicit in the substrate.

Concretely, an ingest pipeline doing data curation can use ruLake as
the dedup engine:

```rust
// For each candidate chunk:
let chunk_hash = blake3(chunk_text);  // content-addressed source
let bundle = RuLakeBundle::new(
    format!("chunk://{chunk_hash}"),  // data_ref includes the hash
    embedding_dim,
    rotation_seed,
    rerank_factor,
    Generation::Num(0),
);
// If a cache entry already exists under this witness, the chunk
// is a content-level duplicate of one we've already seen.
let already_seen = lake.cache_witness_of(&key) == Some(bundle.rvf_witness.clone());
```

For chunks that arrive from three different S3 prefixes but contain
the same content, the witness collides intentionally and the cache
holds one copy. The `cache_refcount_of(witness)` (`src/lake.rs:147`)
reports how many pipelines pointed at the same witness — a direct
"how many duplicates of this chunk" counter.

### 8.3 Near-duplicate detection via vector similarity

Strict content-hash dedup misses near-duplicates: the same paragraph
with one word changed, the same article re-formatted, the same code
snippet with comments stripped. For these, vector-similarity-based
dedup is the standard tool. ruLake's federation handles this at
scale:

```rust
// Sharded by source (S3 prefix, Wikipedia dump, GitHub mirror, …).
let near_dupes = lake.search_federated(
    &[
        ("rvf", "common-crawl-2024"),
        ("rvf", "wikipedia-2024"),
        ("rvf", "github-archive"),
        ("rvf", "stack-exchange"),
    ],
    &candidate_chunk_embedding,
    50,  // top-50 near-neighbors across all sources
)?;

// Apply a similarity threshold (e.g. cosine > 0.95).
let dupes: Vec<_> = near_dupes
    .into_iter()
    .filter(|h| h.score < SIMILARITY_THRESHOLD)
    .collect();
```

The federation path (`src/lake.rs:491-560`) parallelizes the search
across shards via Rayon and merges by score globally, with the
adaptive rerank keeping the per-shard cost bounded.

### 8.4 Witness-locked dataset versions

Training-data curators publish "dataset versions" — RefinedWeb v1.0,
SlimPajama-627B, FineWeb-Edu — and downstream model trainers depend
on those exact byte-level versions for reproducibility. The bundle's
witness is exactly the right primitive for this:

```rust
// At dataset publication time:
let bundle = RuLakeBundle::new(
    "rvf://datasets/refinedweb-v1-0",
    768,
    DATASET_ROTATION_SEED,
    20,
    Generation::Opaque("v1.0-2026-04-25-final".to_string()),
)
.with_lineage_id("ol://refinedweb-curate/run-final");
let witness = bundle.rvf_witness.clone();

// Published in the dataset card:
//
//   RefinedWeb v1.0
//   ruLake witness: 7a3b91f4e5d2…f201
//   …
```

A model trainer can verify they are training against the published
version by warming the cache from a downloaded snapshot and checking
the witness:

```rust
let n = lake.warm_from_dir(&key, "/data/refinedweb-v1-0/")?;
assert_eq!(
    lake.cache_witness_of(&key).as_deref(),
    Some("7a3b91f4e5d2…f201"),
);
log::info!("training against published RefinedWeb v1.0 ({} chunks)", n);
```

The `warm_from_dir` path (`src/lake.rs:378-441`) verifies the
witness internally and rejects mismatched snapshots, so the assert
above is belt-and-braces — the loud-failure-on-mismatch is already
in the substrate.

### 8.5 What this isn't

Vector-similarity-based dedup over a billion-chunk corpus needs a
backend that can hold a billion-row vector index. `LocalBackend` and
`FsBackend` aren't that backend (the `MAX_PULLED_VECTORS` cap is
100 million per `src/backend.rs:60`, deliberately conservative).
The full topology depends on the M2+ Parquet/Iceberg backends plus
sharding to keep per-shard size in the 10-100M range. The
cap can be raised with explicit operator review (per the comment in
`src/backend.rs`), but the architecture for billion-row dedup
still needs to break the corpus into shards of manageable size.

What ships today is the dedup primitive, the federation path, the
witness mechanism, and the test harness — `LocalBackend` proves the
loop end-to-end. The billion-row case waits on Iceberg/Delta
adapters.

---

## 9. Vertical 8 — Model-shard serving (aspirational)

### 9.1 Why this section is honest about limitations

The previous verticals all map cleanly onto ruLake's shipped
M1 + M1.5 surface. This one does not. Model-shard serving — the
problem of serving sharded large-model weights or activation caches
across processes — has architectural overlap with ruLake's
witness-coherent cache, but the mismatch in workload shape is real
and worth stating up front.

ADR-158 (`docs/adrs/ADR-158-optional-rotation-and-qvcache-positioning.md`)
positions ruLake explicitly in relation to QVCache, and notes:

> They differ in what they optimize:
> - **QVCache optimizes recall-adaptive eviction.** Its headline
>   contribution is an online-learned, region-local threshold that
>   decides when a cached answer is "close enough" for a new query
>   …
> - **ruLake optimizes witness-authenticated cross-process sharing.**

This vertical asks whether ruLake's cross-process sharing extends to
*non-vector* artifacts that might benefit from witness-anchored
content-addressing. The honest answer is: not today, but it is
worth thinking through what would have to change.

### 9.2 The KV-cache sharing problem

LLM inference reuses computed key/value tensors across requests when
the prompt prefix is shared. Production serving systems (vLLM,
TensorRT-LLM, SGLang) implement prefix-cache sharing across
requests, but cross-process sharing is harder: the cache lives in
GPU memory and is process-local. Cross-tenant or cross-replica
prefix sharing is an open area.

A naive port of ruLake's pattern would key the KV-cache by:

```
witness = SHAKE-256(model_id || revision || prefix_token_ids || quantization_params)
```

Two processes serving the same model with the same prompt prefix
would witness-collide and could share the cache. The mechanism is
analogous to ruLake's `(data_ref, dim, rotation_seed, rerank_factor,
generation)` chain.

### 9.3 What would have to change in ruLake

To support this aspirationally:

1. **The cache value shape** — today, `CacheEntry::index` is
   `Arc<RabitqPlusIndex>` (`src/cache.rs:213`). For KV-cache
   serving, the value would be a tensor or a tensor reference. The
   trait would need generalization, and the existing RaBitQ-specific
   APIs would need to live behind a feature flag.

2. **GPU memory residency** — the current cache holds compressed
   codes in CPU RAM with the Arc-drop-lock pattern allowing
   concurrent CPU scans. KV-cache serving needs GPU residency, which
   means UCX/NCCL-style zero-copy handoff between processes (or a
   GPU-resident shared-memory equivalent).

3. **Witness over tensor parameters** — the witness today covers
   bundle-level metadata. For KV caches, it would need to cover
   tensor dtype, layout, sharding pattern, quantization scheme. The
   schema is much richer than a vector bundle.

4. **Mutation semantics** — vector caches in ruLake are immutable
   per-witness (per `src/cache.rs:22`: "the index is immutable once
   built"). KV caches are append-only as more tokens are generated.
   A KV-cache adaptation would need either witness-per-prefix-length
   or a different consistency model.

None of these are *philosophically* incompatible with ruLake's
design; they are substantial extensions. The point of including this
vertical is to acknowledge the design space, not to claim it is
solved.

### 9.4 Where ruLake's shipped surface does help model serving

Even setting aside the KV-cache speculation, ruLake's shipped
surface is useful for serving-adjacent retrieval tasks:

- **Speculative-decoding draft model** lookup tables. A small draft
  model produces candidate continuations; a vector index over
  pre-computed continuation embeddings can rank candidates fast.
  Standard ruLake usage.
- **Retrieval-augmented decoding** — at every generation step, the
  model retrieves passages from a corpus to condition the next-token
  distribution. This is RAG inside the decode loop, and ruLake's
  warm-cache 0.27 ms/query at n=100k fits inside the per-token
  budget.
- **Adapter routing** — for a mixture-of-experts or LoRA-router
  setup, the router might use a vector embedding of the request to
  pick the right expert/adapter. ruLake serves the router's index.

### 9.5 The honest verdict

ruLake is not a model-serving substrate today. It is a vector-
retrieval substrate with properties (witness-anchored coherence,
cross-process sharing) that share ancestry with what model-serving
caches need. A future "ruLake-for-tensors" crate could reuse the
bundle protocol and witness chain, but it would be a different
codebase with different lock topology, different memory residency,
and different consistency semantics. Treating that future as
shipped today would mislead.

---

## 10. Vertical 9 — A/B and counterfactual evaluation

### 10.1 The "freeze the world" problem

Counterfactual evaluation in recommendation, search, and ranking
asks: "if we had served a different policy in production, what
would the user metrics have been?" Off-policy evaluation requires
counterfactual reasoning over logged data with the assumption that
the *retrieval substrate* was held fixed across the comparison. If
the retrieval substrate drifts during the experiment — index
rebuilds, snapshot rotations, embedding model updates — the
counterfactual estimate is biased by retrieval noise rather than
policy differences.

The standard mitigation is "freeze the index for the duration of
the experiment." This is operationally awkward: it requires either
running a parallel index, or holding production updates for the
experiment window, or accepting the bias.

### 10.2 Frozen consistency as the substrate primitive

`Consistency::Frozen` (`src/cache.rs:77`) is exactly this primitive:

```rust
let eval_lake = RuLake::new(20, 42).with_consistency(Consistency::Frozen);
eval_lake.warm_from_dir(&key, "/snapshots/eval-baseline-2026-04-25/")?;

// For the entire experiment window, this lake serves out of the
// frozen snapshot. Production lake (separate process) keeps
// rotating. The two never share the cache because the experiment
// process has Frozen mode.
```

The `can_skip_check_interned` path (`src/cache.rs:862-885`) handles
Frozen mode explicitly:

```rust
Consistency::Frozen => {
    let inner = self.inner.lock().unwrap();
    inner.pointers.contains_key(key)
}
```

After the first prime, every subsequent search skips the coherence
check entirely. The substrate is mechanically pinned to the warmed
snapshot.

### 10.3 Witness verification across experiment cohorts

Multi-arm experiments (control + variant + variant) need every arm
to see the same retrieval state. Each arm runs its own RuLake
instance in Frozen mode against a snapshot directory; the witness
verification on `warm_from_dir` (`src/lake.rs:378-441`) ensures
every arm warmed against the same bytes:

```rust
let baseline = RuLake::new(20, 42).with_consistency(Consistency::Frozen);
let variant_a = RuLake::new(20, 42).with_consistency(Consistency::Frozen);
let variant_b = RuLake::new(20, 42).with_consistency(Consistency::Frozen);

for lake in [&baseline, &variant_a, &variant_b] {
    lake.warm_from_dir(&key, "/snapshots/eval-baseline/")?;
}

// Validate cross-arm witness identity.
let w0 = baseline.cache_witness_of(&key);
assert_eq!(w0, variant_a.cache_witness_of(&key));
assert_eq!(w0, variant_b.cache_witness_of(&key));
```

If any arm warmed against a stale or corrupted snapshot, the witness
mismatch fails the assertion — the experiment is rejected as
invalid before any user impressions are logged.

### 10.4 Replay over historical snapshots

Counterfactual evaluation often replays historical user traffic
against a candidate policy. The `save_cache_to_dir` /
`warm_from_dir` cycle plus historical snapshot retention gives a
clean replay topology:

```
Historical snapshots (retained):
    /snapshots/2026-04-01/  ← items table at 2026-04-01 EOD
    /snapshots/2026-04-08/
    /snapshots/2026-04-15/
    /snapshots/2026-04-22/

Replay job:
    For each historical date D:
        lake = RuLake::new(20, 42).with_consistency(Frozen)
        lake.warm_from_dir(&key, "/snapshots/D/")
        For each logged query Q at date D:
            new_hits = lake.search_one(..., Q.user_emb, K)
            old_hits = Q.observed_hits
            metrics.policy_delta(D, new_hits, old_hits)
```

The `warm_from_dir` is byte-exact restore (test
`warm_from_dir_byte_exact_after_save`, alluded to in the
substrate-acceptance suite). The replayed retrieval is exactly what
the production system would have served at that snapshot.

### 10.5 Compute-only replay vs full pipeline replay

There is a useful distinction between two replay modes:

- **Substrate-only replay** — the retrieval is held fixed; only
  downstream re-ranking, prompting, or LLM behavior varies. ruLake
  in Frozen mode handles this directly.
- **Full pipeline replay** — both the retrieval and the model vary.
  Useful when validating that a retrieval upgrade and a model
  upgrade compose correctly.

For full pipeline replay, multiple ruLake instances with different
witness-pinned snapshots run side by side, and the experiment frame
chooses which combination to evaluate. The substrate's per-process
witness identity guarantees that "snapshot A retrieval + model X
ranking" and "snapshot A retrieval + model Y ranking" are
genuinely controlled comparisons — the only variable is the model.

---

## 11. Performance budgeting for AI workloads

### 11.1 When does the 1.02× tax dominate?

The cache-hit tax of 1.01-1.03× over direct RaBitQ
(BENCHMARK.md headline) is small enough that it does not dominate
any AI workload's latency budget. Concretely:

- **Chatbot / RAG** — typical p99 budget 1-3 seconds (LLM
  generation dominates). Retrieval at 0.27 ms/query (cache hit at
  n=100k, single-thread) is invisible in the budget. The 1.02× tax
  adds 0.005 ms — five microseconds.
- **Recommendation** — p99 budget 50-100 ms (re-ranking dominates).
  Retrieval at ~5 ms federated (per §7.3) is small. Tax adds 0.1 ms.
- **Semantic search UI autocomplete** — p99 budget 30-50 ms
  (network RTT dominates). Retrieval at 0.27 ms is small. Tax adds
  ~5 µs.
- **Batch / offline** — latency budget effectively unbounded;
  throughput matters. The tax is invisible in throughput terms
  because the QPS difference is in the noise (3,542 vs 3,681 at
  n=100k single-shard).

The tax becomes visible only in benchmarks that try to measure it
specifically, like the BENCHMARK.md headline. In production
workloads it is well below measurement noise.

### 11.2 When does prime time matter?

Prime time is the cost of priming a cache entry on cold start or
after invalidation. It scales linearly with `n` per the
BENCHMARK.md serial column (211 ms / 50k → 421 ms / 100k). With
parallel Rayon prime, the scaling stays linear but the constant
drops: 19.6 ms / 50k → 37.6 ms / 100k.

Prime time matters when:

- **Process startup** (cold start of a serving fleet). With
  `warm_from_dir`, prime is replaced by a file read + bundle
  verification. Per the substrate-acceptance test, this is ~O(file
  read) — single-digit ms for typical collections.
- **Snapshot rotation** (an Iceberg snapshot rolls). The first
  query against the new snapshot pays the prime cost. Subsequent
  queries serve from cache. With 5-minute rotation cadence, the
  per-rotation cost is negligible amortized.
- **Cache miss under bounded LRU.** If `with_max_cache_entries` is
  set tight and many tenants compete for cache slots, miss rate
  rises and prime time becomes the latency tail. Mitigation: size
  the cache for working-set, monitor `last_prime_ms` and
  `hit_rate`.

The honest budget for a serving fleet:

```
Cold-start path:
    process boot                     1-3 s
    warm_from_dir per collection     5-50 ms each
    first-query header parse + auth  10 ms
                                    ────
    total to first query             1.05 s (typical)

Steady-state path:
    cache hit                        0.27 ms (warm, n=100k)
    cache miss (bounded LRU)         37 ms (parallel prime, n=100k)
                                    ────
    p99 if miss-rate ≤ 1%            ~0.6 ms
```

### 11.3 Realistic latency tables

For the workloads above, with all caches warm and `Consistency::Eventual`
60s TTL, single-shard:

| Workload                      | Retrieval QPS | Per-query cost | Notes |
|-------------------------------|---------------|----------------|-------|
| Chatbot (1 retr/turn)         | low           | 0.27 ms        | dominated by LLM |
| RAG (3 retr/turn, fed)        | medium        | 1.5 ms         | parallel federation across 3 collections |
| Recommendation (1 retr/req)   | high          | 0.27 ms        | hot-path search_one |
| Semantic search type-ahead    | very high     | 0.27 ms        | hits cache continuously |
| Batch dedup (n=10M, k=50)     | n/a           | ~3 ms/q        | bigger n than benchmark; extrapolated |

Multi-thread concurrent throughput at 4 shards × 8 clients hits
36,715 QPS (BENCHMARK.md). For a serving fleet with one ruLake
instance per pod and 50 pods, that is ~1.8M QPS aggregate retrieval
capacity — vastly more than typical AI workloads need.

### 11.4 Where it falls down

Two known sharp edges:

- **Network-backed Fresh mode.** `LocalBackend` makes
  `Consistency::Fresh` cheap (one HashMap read per query). A real
  Parquet/BigQuery backend makes Fresh expensive (one network RTT
  per query). For network-backed deployments, `Eventual` mode is
  load-bearing. Per BENCHMARK.md: "expect materially higher tax on
  BigQuery / Snowflake / S3-Parquet."
- **Many small collections.** The cache has per-collection
  bookkeeping (per-collection stats, per-collection pointer entries)
  that becomes overhead at very large collection counts. The
  performance review notes the global mutex on `VectorCache.inner`
  becomes the next bottleneck above ~32 concurrent threads on a
  single key. For deployments with thousands of small collections,
  per-shard cache instances are a workable mitigation.

---

## 12. Reality check and gaps

### 12.1 What's shipped today (M1 + M1.5)

Per the README "Status" section and verified against the source:

- `RuLake` entry point with full method surface (`src/lake.rs`).
- `VectorCache` with witness-addressed entries, three consistency
  modes, LRU bound (`src/cache.rs`).
- `RuLakeBundle` with SHAKE-256 witness, atomic FS write, tamper
  detection (`src/bundle.rs`).
- `BackendAdapter` trait with `LocalBackend` (in-memory) and
  `FsBackend` (`ruvec1` binary format on disk) implementations
  (`src/backend.rs`, `src/fs_backend.rs`).
- Federated search with adaptive per-shard rerank and per-shard
  over-request (`src/lake.rs:491-560`).
- Persistence: `save_cache_to_dir` / `warm_from_dir` with byte-exact
  restore (`src/lake.rs:263-441`).
- Sidecar protocol: `publish_bundle` / `refresh_from_bundle_dir`
  (`src/lake.rs:167-228`).
- 43 tests passing (21 unit + 22 integration), zero `unsafe` in
  ruLake.

This is enough to build every "Vertical 1, 2, 5, 7, 9" topology in
this document end-to-end against `LocalBackend` or `FsBackend`. The
substrate-level guarantees are mechanical, not aspirational.

### 12.2 What is roadmap (M2+)

Per the README's M2+ roadmap section:

- **Backends** — `ParquetBackend` (arrow), `BigQueryBackend`
  (Storage Read API), `IcebergBackend` (Nessie / Polaris catalog),
  `DeltaBackend` (CDF coherence), implied `SnowflakeBackend`.
- **Wire** — HTTP / gRPC protocol layer with OpenAPI schema.
- **Governance** — RBAC via OIDC/JWT, PII passthrough, OpenLineage
  emission with witness as lineage-id.
- **Kernels** — GPU in separate crates (`ruvector-rabitq-cuda`,
  `-rocm`, `-metal`), turbovec-style FastScan 4-bit LUT, WASM SIMD.
- **Acceleration** — mmap'd index persistence via `memmap2`, HNSW
  layer on top of RaBitQ via `hnsw_rs::datamap`.
- **SOTA integrations** — QVCache-style adaptive per-region rerank,
  SPIRE-style 8B-vector federation.

For verticals 3, 6, and 8 (RAG over a lakehouse, recommendation
with sharded catalogs, parts of model-shard serving), the M2
backends are load-bearing. They are not shipped today. Treating
them as shipped would mislead.

The contract for adding a new backend is clean and small (the
`BackendAdapter` trait — five methods, four required), and
`FsBackend` is a 250-line reference impl. The M2 backends are well-
specified; what they need is the implementation work plus integration
tests against real Parquet files / Iceberg catalogs / BigQuery
projects.

### 12.3 The "not benchmarked in v1" list

BENCHMARK.md is explicit about what numbers are missing:

- **Real-backend network latency.** `LocalBackend::pull_vectors` is
  in-process. Parquet on S3 adds 10-100 ms per prime. M2 measurement.
- **Recall regressions vs direct RaBitQ.** The test suite confirms
  byte-exact ordering at the same seed. Formal recall sweeps reuse
  `ruvector-rabitq::BENCHMARK.md`.
- **Push-down paths.** ADR-155 §Decision 4 defers backend-native
  vector ops to per-adapter Tier 2.
- **Cache memory footprint vs backend size.** LRU cap implemented
  but not yet tuned under memory pressure. M3 measurement.

The numbers used throughout this document are from the
single-thread / 8-client concurrent benches in BENCHMARK.md and are
honest about their scope.

### 12.4 Things the verticals assume that aren't yet trivial

- **OpenLineage integration.** The `lineage_id` field on the
  bundle is a string slot; emitting and consuming OpenLineage events
  is application work. There is no OpenLineage adapter shipped.
- **PII enforcement.** The `pii_policy` field is opaque. Hooking it
  to Microsoft Presidio or Google Cloud DLP is application work.
- **Embedding APIs in `BackendAdapter` impls.** Vertical 4 sketches
  an `InferenceBackend` that calls embedding models. There is no
  shipped inference backend; users implement it for their model.
- **Iceberg / Delta / Parquet wiring.** Vertical 3 depends on these.
  All M2+.

### 12.5 What ruLake does not do that is sometimes asked of it

- **Sparse retrieval (BM25, SPLADE).** Vertical 1's RAG section
  assumes dense retrieval. Hybrid sparse+dense systems need
  something else for the sparse leg (Tantivy, Lucene). ruLake handles
  the dense side; the application combines.
- **Filter-then-rank** (e.g. "top-K under metadata filter
  `lang=en`"). The current substrate returns top-K by vector
  distance, no metadata filter. Filtering is the application's job
  on the returned `SearchResult`s.
- **Online learning of the index** (incremental HNSW updates).
  RaBitQ codes are immutable per witness; mutation is bundle
  rotation. For workloads requiring sub-second incremental updates,
  the bundle-rotation cadence is the lower bound.

These are honest scope limitations, not bugs. ruLake is a
cache-coherent vector execution fabric. Things outside that scope
live in adjacent crates or at the application layer.

---

## 13. Open questions

### 13.1 Hybrid sparse + dense

The dominant pattern for production retrieval in 2024-2026 is hybrid
sparse + dense: a BM25 or SPLADE pass produces a candidate set,
which a dense vector pass re-ranks. The reverse pattern — dense
retrieval re-ranked by sparse — is also common.

Where does ruLake fit? The cache-coherent dense-vector substrate is
clear. The sparse leg is not in scope. A composition layer that
takes a sparse top-K from Tantivy and runs `lake.search_batch` on
the resulting embeddings is straightforward but outside the
substrate.

A more interesting composition: use the sparse output as a *filter*
on the dense pass. ruLake doesn't expose a "top-K from this id set"
API today. Adding one would require either a `RabitqPlusIndex`-level
filter primitive (which RaBitQ doesn't have) or a post-filter on
the global top-K (which is what most systems do today). Neither is
shipped; both are tractable.

### 13.2 Learned indexes

The 2023-2025 generation of "learned indexes" (DeepHash, learned
metric trees, attention-routed retrieval) suggest that index
structure can be data-adaptive. ruLake's witness chain assumes the
index is *deterministic in (data, seed, rerank)*; a learned index
violates this without careful witness extension to cover the
learned parameters.

The straightforward extension: the witness covers the trained
parameter checkpoint, so two processes loading the same checkpoint
produce the same witness and share the cache. This is structurally
identical to the embedding-pipeline witness in §5 (`(model_name,
model_revision, source_doc_hash)`) — the substrate generalizes
naturally to learned-index parameters as long as the parameters are
content-addressed.

### 13.3 Quantized re-ranking

The current ruLake rerank step (the `rerank_factor × k` candidates
that get exact L2² scoring) uses uncompressed f32 originals. For
high-recall workloads at extreme scale (DEEP1B-class corpora), even
the rerank step can be the bottleneck.

Quantized re-ranking (ScaNN-style 4-bit product quantization, FastScan
LUTs, the turbovec line) could replace the f32 rerank with a
compressed-float rerank, potentially trading a small recall hit for
a large speed gain on the rerank step. The README's M2+ roadmap
mentions "turbovec-style FastScan 4-bit LUT" as a kernel addition;
its impact on ruLake's recall guarantees is an open question worth
measuring.

### 13.4 Bundle protocol over object-storage event triggers

The current sidecar protocol assumes a polling reader (`while {
refresh_from_bundle_dir; sleep }`). For very large reader fleets,
polling becomes expensive and event-driven invalidation is
preferable.

Object stores expose event triggers — S3 Notifications via SQS or
EventBridge, GCS Pub/Sub notifications, Azure Blob Storage
EventGrid. A future "event-driven sidecar daemon" could subscribe to
"new bundle published" events and call `refresh_from_bundle_dir`
only on those events. The substrate already supports this — the
`refresh_from_bundle_dir` API doesn't care whether it was called
from a polling loop or an event handler — but the integration
shims aren't shipped.

### 13.5 Kernel acceleration for non-CPU targets

ADR-157 (the optional `VectorKernel` accelerator plane) reserves
space for GPU / Metal / WASM kernels as separate crates. None are
shipped. For AI/ML workloads where the embedding model already runs
on GPU, having the retrieval kernel also run on GPU would amortize
data-transfer cost and tighten the latency budget.

The crucial design decision is *where* the kernel boundary sits —
whether it is per-query (GPU only wins above some `min_batch`) or
per-collection (GPU pre-built indexes that serve all queries). The
`search_batch` API (`src/lake.rs:600`) is the plug-point that makes
per-batch GPU dispatch tractable; per-query GPU dispatch is
unlikely to win against the warm CPU cache.

### 13.6 Memory-class semantics for AI workloads

ADR-156 reserved the `memory_class: Option<String>` bundle field
for caller-defined cognitive labels (`"episodic"`, `"semantic"`,
`"procedural"`, `"identity"`). For AI/ML workloads, the obvious
labels are different:

- `"corpus"` — large source corpus chunks (RAG).
- `"feature"` — vector features for ML serving.
- `"item"` — recommendation catalog items.
- `"user"` — recommendation user embeddings.
- `"checkpoint"` — model-checkpoint vectors (as in §9.3
  speculation).

Whether ruLake should aggregate stats by `memory_class` is an open
question (ADR-156 §"Open questions" #2). For a SaaS RAG provider,
per-class hit-rate aggregation would be operationally useful. For
small deployments, per-collection stats are sufficient.

### 13.7 Witness-format extension for embedding-model identity

For verticals 4, 5, and 8 (embedding pipeline, feature store,
training-data dedup), the witness chain implicitly assumes the
embedding-model identity is folded into the `data_ref`. This is
adequate but indirect. A future witness-format extension could add
an explicit `model_ref: Option<String>` field that the witness
covers, making the model identity a first-class part of the
provenance.

ADR-158 §3 already discusses extending `WitnessV1` to include the
`RandomRotationKind`. A `WitnessV2` that covered both rotation kind
and model reference would be a one-time breaking change with
significant payoff for AI/ML provenance — it would make every
witness independently verifiable as "produced by this model on this
data with these compression parameters."

### 13.8 Federation across geographically distributed backends

The current `search_federated` parallelizes via Rayon across
in-process backend instances. For a multi-region deployment where
each region has its own backend, the federation needs network
hops and the latency budget changes substantially.

For a recommendation system with a US-East item shard, an EU item
shard, and an APAC item shard, the federation would issue parallel
requests across regions and merge results. Per-shard latency at
50-100 ms RTT dominates the per-shard rerank cost; the adaptive
rerank optimization is moot in this regime. What matters is per-shard
*cache locality* — each region serves out of its local cache, and
only the cold prime crosses the wide-area network.

This is structurally feasible against the shipped trait — a
`RemoteBackend` adapter that wraps an HTTP/gRPC client to a remote
ruLake instance — but no such adapter is shipped today.

### 13.9 Push-down to backend-native vector ops

ADR-155 §Decision 4 defers backend-native vector pushdown to per-
adapter work. For backends that have first-class vector search
(BigQuery Vector Search, Iceberg with vector indexes, Snowflake
Cortex), the optimal strategy might be:

- **Cold/large queries** — push down to the backend's vector op.
- **Hot/repeat queries** — serve from ruLake's cache.

The `BackendAdapter::supports_pushdown` flag (`src/backend.rs:143`)
exists as a forward-compatibility hook, but no current router logic
uses it. Determining the crossover point — at what `n`, query
volume, and cache hit rate the pushdown wins versus the cache-prime
strategy — is open work.

### 13.10 Compaction and forget semantics

ADR-156 explicitly defers compaction to "RVM / Cognitum (the brain
system)." For AI/ML use cases — particularly GDPR/CCPA compliance
in production RAG — "forget this user's data from the index" is a
real requirement, and ruLake's substrate-level "forget" (cache
invalidation) is not the same thing as cryptographic erasure of
the underlying bytes.

The pattern that respects the separation: ruLake invalidates the
cache pointer (`invalidate_cache`); the underlying RVF segment is
crypto-shredded by the brain/storage layer; the next prime against
the invalidated key fails because the bytes are gone, surfaced as
`UnknownCollection` or `InvalidParameter`. This works mechanically
but the user experience needs documentation to clarify what "forget"
means at each layer.

---

## Appendix A — File and API map for AI/ML implementers

For implementers landing on ruLake to build one of the verticals
above, the relevant pieces by file:

| File                              | What's there                                              |
|-----------------------------------|-----------------------------------------------------------|
| `src/lib.rs`                      | Module structure and public re-exports.                   |
| `src/lake.rs`                     | `RuLake` entry point, search APIs, sidecar primitives.    |
| `src/cache.rs`                    | `VectorCache`, `Consistency`, `CacheStats`, witness keys. |
| `src/bundle.rs`                   | `RuLakeBundle`, `Generation`, witness computation.        |
| `src/backend.rs`                  | `BackendAdapter` trait, `LocalBackend`, `PulledBatch`.    |
| `src/fs_backend.rs`               | `FsBackend` reference impl, ~250 lines, useful template.  |
| `src/error.rs`                    | `RuLakeError` enum.                                       |
| `tests/federation_smoke.rs`       | Substrate acceptance tests, federation gates.             |
| `examples/sidecar_daemon.rs`      | Bundle publish/refresh daemon pattern.                    |
| `examples/warm_restart.rs`        | save → ship → warm-restart cycle.                         |
| `BENCHMARK.md`                    | Reproducible numbers for budgeting.                       |
| `docs/adrs/ADR-155-*.md`          | Cache-first datalake-layer positioning.                   |
| `docs/adrs/ADR-156-*.md`          | Memory-substrate framing for agent brains.                |
| `docs/adrs/ADR-157-*.md`          | Optional accelerator plane.                               |
| `docs/adrs/ADR-158-*.md`          | Hadamard rotation, QVCache positioning.                   |
| `docs/review/capabilities.md`     | API-to-claim verification.                                |
| `docs/review/performance.md`      | Hot-path analysis, allocation patterns.                   |
| `docs/review/security.md`         | Defense-in-depth review.                                  |

For most AI/ML applications, the working surface is small:

1. `RuLake::new`, `with_consistency`, `with_max_cache_entries`,
   `register_backend`.
2. `lake.search_one`, `search_federated`, `search_batch`.
3. `lake.cache_stats`, `cache_stats_by_collection`.
4. `lake.publish_bundle`, `refresh_from_bundle_dir` (for
   cross-process coherence).
5. `lake.save_cache_to_dir`, `warm_from_dir` (for warm restart).
6. `Consistency::{Fresh, Eventual { ttl_ms }, Frozen}`.

That is the full surface needed to implement Vertical 1 (production
RAG) end-to-end. The other verticals add incremental surface
(custom backends, memory-class tags) but the core API is stable.

---

## Appendix B — Vertical-to-property mapping

For quick reference, which of properties A (witness coherence), B
(consistency knob), C (federated execution) does each vertical lean
on most:

| Vertical                          | A (witness) | B (consistency) | C (federation) | Shipped today? |
|-----------------------------------|:-----------:|:---------------:|:--------------:|:--------------:|
| 1. RAG with provenance            | strong      | medium          | weak           | yes            |
| 2. Multi-tenant RAG               | strong      | medium          | medium         | yes            |
| 3. Lakehouse RAG                  | strong      | strong          | medium         | M2 backends    |
| 4. Embedding-pipeline cache       | strong      | strong (Frozen) | weak           | yes            |
| 5. Feature store (vector)         | strong      | strong          | weak           | yes            |
| 6. Recommendation                 | medium      | medium          | strong         | M2 backends    |
| 7. Training-data dedup            | strong      | medium          | strong         | yes (LocalBackend; M2 for billion-row) |
| 8. Model-shard serving            | weak        | weak            | weak           | aspirational   |
| 9. A/B / counterfactual eval      | strong      | strong (Frozen) | weak           | yes            |

The "shipped today" column reflects M1 + M1.5 against the verticals
as described, with `LocalBackend` or `FsBackend` standing in for the
M2 backends where the topology depends on them.
