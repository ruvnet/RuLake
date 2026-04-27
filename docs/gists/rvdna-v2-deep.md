# rvDNA v2 — A Deep Introduction

## TL;DR

rvDNA v2 is a single-file binary format that packages a sample's genome together with its precomputed AI representations — k-mer embeddings, attention windows, variant tensors, protein graphs, methylation tracks, biomarker series, provenance — and pins the whole bundle to a 32-byte cryptographic witness that any other process can re-derive from the bytes themselves. It does for genomic intelligence what a Parquet file does for tabular data: take the expensive part (encoding, embedding, calling) once, and let every future query land on a cache lookup. ruLake is a natural host because ruLake already speaks that exact witness — same SHAKE-256 recipe, same `BackendAdapter` trait, same federation primitive — so making `.rvdna` a first-class lake substrate is a wrapping job, not a rebuild.

## Introduction

For a long time, genomic data has lived in formats designed for the steps right before science happens: FASTA for raw bases, BAM for aligned reads, VCF for variants, BED/bedGraph for tracks. Each format is excellent at being a serialisation target for one tool, and each one is a re-parsing tax for everyone downstream. A typical "look at this gene across a cohort" question becomes ten read paths, ten parsers, ten in-memory representations, and a notebook full of ad-hoc joins. The intelligence you compute — embeddings, attention, calibrated variant likelihoods, polygenic scores — does not survive the round trip; it lives in a `.npz` next to the script, or in nothing at all.

rvDNA v1 (the prototype at `vendor/ruvector/examples/dna/`, ratified by ADR-013 in that subtree) attacked this by inverting the priority order. Treat the *intelligence* as the primary artefact and the raw bases as one of its sections. Pack everything into a single mmap-friendly binary so a query is a memory address, not a parser invocation. The v1 implementation reaches a measured 12 ms for a five-gene end-to-end demo and supports sub-microsecond random access into k-mer vectors. As a self-contained crate it works; as a federation citizen it does not. v1's checksum is a CRC32 over the header only, not over payloads, and not in the cryptographic shape that makes content-addressed caches share entries across deployments.

ADR-007 calls this gap out plainly (`docs/adrs/ADR-007-rvdna-as-rulake-substrate.md:96`): "v1 ships as a self-contained crate. It does not federate. Two `.rvdna` files on two hosts are independent; querying across them requires loading both into one process and orchestrating the combine by hand."

v2 closes that gap with a deliberately small change. Every v2 file carries a 96-byte **bundle pointer** at offset `0x00B0`, and the witness inside that pointer is byte-isomorphic to a `RuLakeBundle` (`crates/core/src/bundle.rs:113`) — the same struct ruLake's GCS, IPFS, and Local backends already emit. Nothing about ruLake has to change for `.rvdna` files to participate in its cache, its MCP surface, or its federation primitive.

The reason a precomputed, witness-anchored representation matters *now* — as opposed to "eventually" — is that the cost asymmetry has flipped. Models like ESM-2, Nucleotide Transformer, and HyenaDNA produce embeddings that are expensive enough to compute that doing it twice is wasteful, but cheap enough to *use* that interactive workflows are realistic. A protein-language embedding for a single human gene takes seconds on a GPU and microseconds to query. The economics demand a representation that says "I was computed once against this exact model checkpoint, here is the proof, here is the artefact, do not recompute me."

That is the contract v2 ratifies. The witness binds the model identity (`model_checkpoint_lo`, bundle pointer offset `0x4C`), the section payloads (BLAKE3 checksums folded through `sections_root`), and the profile flags (clinical vs research) into one 32-byte digest. A v2 file generated against ESM-2 v0.4.1 cannot be confused with one generated against v0.5.0 even if every other field matches. A clinical-profile file cannot be re-served as research without rotating the witness. The representation is honest about what it is.

## The file format

A v2 file is eight sections, a 64-byte header, a 112-byte section table, the 96-byte bundle pointer, an optional manifest, and a global BLAKE3 footer. The magic is `RVDNA\x02\x00\x00`; v1's magic was `RVDNA\x01\x00\x00`, so a reader can dispatch on the third byte (`docs/research/rvdna/v2-spec.md:68`). Sections are 64-byte aligned for mmap.

| § | Section | Tier | Codec default | What it carries |
|---|---|---|---|---|
| 0 | DNA + Phred quality | Cold (T2) | Zstd | 2-bit packed bases, N-mask, optional 6-bit Phred |
| 1 | K-mer HNSW vectors | Hot (T0) | None | Per-block embeddings, optional int8 codes, per-block model id |
| 2 | Attention COO matrices | Warm (T1) | Zstd | Sparse triplets per genomic window |
| 3 | Variant f16 tensor | Warm (T1) | Zstd | Position, alt alleles, genotype likelihoods, Phred |
| 4 | Protein CSR graph | Warm (T1) | Zstd | Embeddings, contacts, secondary structure |
| 5 | Epigenomic + Horvath clock | Cold (T2) | Zstd | CpG positions, beta values, clock coefficients |
| 6 | Biomarker time series | Warm (streaming-aware) | Zstd | Per-biomarker rows, per-epoch back-pointers |
| 7 | Metadata (MessagePack) | Per-section provenance | Zstd | Section checksums, lineage row, tenant ids |

Section 6 is the only fully new section relative to v1; it serialises what v1's `biomarker_stream.rs` kept in memory only. Section 7 is the lineage / provenance home and is what an audit pipeline reads.

The **bundle pointer** at `0x00B0` is the load-bearing addition. Its layout is documented in `docs/research/rvdna/v2-spec.md:265`:

| Off | Size | Field | Notes |
|---|---|---|---|
| 0x00 | 32 B | `rvf_witness` | SHAKE-256(32), the cache anchor |
| 0x20 | 8 B | `dim` | Dimensionality of §1 vectors |
| 0x28 | 8 B | `rotation_seed` | RaBitQ seed, carried into ruLake |
| 0x30 | 8 B | `rerank_factor` | RaBitQ rerank factor |
| 0x38 | 1 B | `generation_kind` | Matches `Generation` variant tag at `crates/core/src/bundle.rs:56` |
| 0x40 | 8 B | `generation_value` | Numeric epoch or sidecar offset |
| 0x48 | 1 B | `pii_policy_class` | 0 research-open, 1 phi-strict, 2 opaque |
| 0x49 | 1 B | `memory_class` | 0x01 = `"genomic"` per ADR-156 |
| 0x4A | 2 B | `profile_flags` | clinical / research / streaming bits |
| 0x4C | 4 B | `model_checkpoint_lo` | Low 32 bits of SHA-256(model_id ":" version) |
| 0x50 | 8 B | `sidecar_offset` | Points at MessagePack lineage in §7 |
| 0x58 | 8 B | `sidecar_size` | |

Reading those 96 bytes and constructing a `RuLakeBundle` produces something whose `verify_witness()` (`crates/core/src/bundle.rs:191`) will return `true` against the very same digest — which means a `.rvdna` file is *also* a publishable bundle, not just a thing that has a bundle next to it.

The format is split into three tiers (`docs/research/rvdna/v2-spec.md:600`-area). T0 is the hot path: §1 k-mer vectors, in RAM, `Consistency::Fresh`, capped at 512 MiB per file on top of ruLake's 16 GiB total `MAX_PULLED_BYTES` (`crates/core/src/backend.rs:62`). T1 is mmap'd: §2 / §3 / §4 / §6, 5-second eventual consistency, bounded by `vm.max_map_count`. T2 is lazy: §0 / §5, fetched only when a query asks for raw bases or methylation, frozen consistency, with a 64 MiB per-query decode budget that refuses with `RVDNA_T2_BUDGET_REFUSED` if exceeded. The reference v0.0.1 implementation only ships T0 — the rest are roadmapped per ADR-007 §Implementation sequencing.

## Capabilities

Concretely, here is what an operator can do with v2 today (or will be able to do once T1 / T2 land per the roadmap below):

**Variant calling without VCF re-parsing.** Section 3 is a fixed-shape f16 tensor: `u64 position | u8 ref | u8 num_alt | u8[num_alt] alts | f16[G] genotype_likelihoods | f16 allele_freq | u8 filter_flags`. A region query is a binary search and a slice — no text parsing, no INFO field expansion. The `rvdna_call_variants` MCP tool (`docs/research/rvdna/v2-spec.md:958`-area) hands back a JSON array of variants pinned to the file's witness. Subsequent calls for the same region land on a cache entry keyed by `(t1_backend_id, "variants", witness, region_hash)`.

**Cross-deployment cache via CID.** When a `.rvdna` file is pinned to IPFS at, say, `bafy...HBB`, the bundle's `data_ref` becomes `ipfs://bafy.../HBB`. Any other ruLake instance that pulls the same CID and computes its own bundle gets the same `rvf_witness` because the witness recipe is purely a function of bytes (`crates/core/src/bundle.rs:362`). The cache entry is shareable across hosts without ever shipping the cache itself — the IPFS path from `docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md` carries the bytes; the witness proves their identity.

**Lineage tracing.** The `rvdna_lineage` MCP tool returns the witness chain, the per-section BLAKE3 checksums, and the model checkpoint identifiers for every embedding-producing section. Operators auditing a clinical decision can trace from a result back through the file that produced it, the model that embedded it, and the bundle JSON that proves the file's identity. ADR-007 §D6 and v2-spec §k make this mandatory under `--profile clinical` (an OpenLineage `lineage_id` is required, and federation refuses cross-tenant unless JWT scopes match per `crates/mcp-server/src/auth.rs:294`).

**Worked example.** Imagine you have 10,000 sequenced samples, each encoded as a v2 `.rvdna` file with one 2 kb HBB region in §1. You want to ask: across the cohort, which samples have a k-mer profile within similarity 0.97 of this query sequence. The wiring is one ruLake instance, 10,000 `RvdnaT0Backend` registrations (default 2 KB k-mer block size puts each sample at ~50 KB in RAM, so 500 MB total at registration time), and one MCP call to `rvdna_find` with `targets = [(s_001, "HBB"), (s_002, "HBB"), ...]`. ruLake's `search_federated` (`crates/core/src/lake.rs:521`) fans out across all 10,000 shards in parallel via rayon, applies the adaptive per-shard rerank floored at 5 (`crates/core/src/lake.rs:533`), merges by score, and returns the global top-K. ADR-007 §G3 sets the acceptance bar at p50 < 10 ms and p99 < 30 ms — the brief's "load 100 files, query in <10 ms without recompute" gate, expanded to a population-scale shape.

The point of the worked example is not the latency number; it is that no new federation code, no new MCP tool, and no new cache layer is involved. Every primitive existed before v2; v2 just registers `.rvdna` files as backends.

## Trust chain

The witness is the trust anchor. Its definition is short enough to read in one sitting (`crates/core/src/bundle.rs:362`):

```rust
fn compute_witness(data_ref, dim, rotation_seed, rerank_factor, generation) -> String {
    let mut h = Shake256::default();
    h.update(b"rulake-bundle-witness-v1|");
    h.update(&(data_ref.len() as u64).to_le_bytes());
    h.update(data_ref.as_bytes());
    // ... length-prefixed dim, seed, rerank, generation ...
    let mut out = [0u8; 32]; reader.read(&mut out);
    hex::encode(out)
}
```

Five things matter about this function. First, the domain prefix `rulake-bundle-witness-v1|` makes the digest non-overlapping with any other SHAKE consumer. Second, every variable-length input is preceded by its length in little-endian u64 — so `"a|b"` and `"ab|"` cannot collide. Third, the `Generation` enum is variant-tagged at `crates/core/src/bundle.rs:82`; without that byte, `Generation::Num(7)` and `Generation::Opaque("\x07\0\0\0\0\0\0\0")` would have produced the same hash — a real bug closed by the 2026-04-23 security audit. Fourth, the output is a 32-byte SHAKE-256 digest hex-encoded; the format is fixed and small enough to fit in any audit row. Fifth, `RuLakeBundle::new` (`crates/core/src/bundle.rs:166`) computes the witness *during construction*, so a bundle is never observed without its anchor.

For rvDNA v2 specifically, the witness inputs are these (per ADR-007 §D3 and v2-spec §d.1):

- `data_ref` is `rvdna://<file-id>/<collection>` or `ipfs://<cid>/<collection>` for IPFS-hosted files. The reference impl uses `rvdna://<file-id>/<model>@<rev>` so the model identity is visible inside the URI itself (`crates/rvdna-backend/src/witness.rs:1-50`).
- `dim` is the §1 vector dimensionality (typically 256 or 512).
- `rotation_seed` and `rerank_factor` are constants for a given lake instance — RaBitQ's compression depends on them.
- `generation` is `Generation::Opaque(...)` packing `model_checkpoint_lo (4 B) || sections_blake3_root (32 B) || profile_flags (2 B)`.

The result: rotate the model, rotate the witness. Mutate any payload byte, rotate the witness. Flip clinical to research, rotate the witness. Two replicas converge precisely because every input that *should* affect the cache entry is folded into the digest, and every input that *shouldn't* (file path, on-disk timestamp, codec choice) is excluded. The convergence property is checked in `crates/core/src/bundle.rs:397`-area unit tests for the generic bundle, and inherited by every backend that synthesises one through `RuLakeBundle::new`.

## Reference implementation status

The crate `ruvector-rulake-rvdna` v0.0.1 lives at `crates/rvdna-backend/`. What it ships today:

- `RvdnaT0Backend`, the hot-tier `BackendAdapter` (`crates/rvdna-backend/src/lib.rs:55`-area), with `id`, `list_collections`, `pull_vectors`, `generation`, `current_bundle`. v0.0.1 is deliberately schema-light: collections are held in a `BTreeMap` of `RvdnaCollection { ids, vectors, dim, generation }`, populated either by tests or by an in-process producer.
- The `witness` module (`crates/rvdna-backend/src/witness.rs:40`) builds a `RuLakeBundle` via `RuLakeBundle::new` exactly the way any other lake backend would, tagging `memory_class = "genomic"`. Determinism is asserted in the module's own tests.
- Two Criterion benches scaffolded for the eventual G1 / G2 acceptance gates: `pull_vectors` and `cache_priming`.

What v0.0.1 does *not* ship, and which is roadmapped per ADR-007 §Implementation sequencing and the integration-with-rulake plan:

- The actual `.rvdna` v2 file reader. v0.0.1 is a synthetic-fixture backend; v0.0.2 will read the bundle pointer at byte `0x00B0` directly out of the file's bytes (commented at `crates/rvdna-backend/src/lib.rs:14`).
- T1 (`RvdnaT1Backend`, mmap-windowed protein / attention / variants) and T2 (`RvdnaT2Backend<Inner>`, lazy raw DNA + epigenomic over Local / GCS / IPFS). Both are sketched in `docs/research/rvdna/integration-with-rulake.md`.
- `crates/mcp-rvdna/`, the sibling MCP server with the five genomic verbs (`rvdna_find`, `rvdna_call_variants`, `rvdna_translate`, `rvdna_score`, `rvdna_lineage`). PR 2 in the ADR's sequencing.
- The Console's seventh sidebar entry (`Genomic`) and the `verifyRvdnaWitness` WASM export. PR 3.
- The `rvdna v2 migrate` subcommand for v1 corpora (`docs/research/rvdna/v2-spec.md:1417`-area).

The acceptance gates the implementation is being built against are itemised in ADR-007 §Verification: G1 load latency (100 files in <500 ms), G2 cold prime (<1.5 s total, <15 ms mean), G3 federated query (the brief's <10 ms p50 ask), G4 witness verify (<5 s for 100 files), G5 audit emit (<50 µs p99 per call).

## Composition with ruLake

The trio is the simplest part of v2 to explain because it is structural rather than novel.

**`BackendAdapter` is the contract.** The trait at `crates/core/src/backend.rs:110` requires four methods (`id`, `list_collections`, `pull_vectors`, `generation`) and supports an optional `current_bundle` override. ruLake's GCS, IPFS, Local, and Fs backends all implement it; `RvdnaT0Backend` implements it; the trait does not know what genomics is. From ruLake's perspective, a registered rvDNA backend is just another source of vectors.

**The cache-witness anchor is shared.** The same `compute_witness` function that anchors a Parquet table, an IPFS pin, or an in-memory test substrate also anchors a rvDNA collection — because `RvdnaT0Backend::current_bundle` constructs a `RuLakeBundle` through `RuLakeBundle::new`, and `RuLakeBundle::new` calls `compute_witness`. Two ruLake instances reading the same `.rvdna` file derive the same witness independently and share the cache entry through ruLake's content-addressed dedup path (`crates/core/src/cache.rs::install_prebuilt_interned`).

**Federation is fan-out.** `RuLake::search_federated` (`crates/core/src/lake.rs:521`) is a parallel rayon walk over `(backend, collection)` pairs with an adaptive per-shard rerank. From v2's perspective, "federation across a cohort" is "register N rvDNA backends and call `search_federated`" — no new code, no new IPC, no new merge logic.

A substrate, in this framing, is a thing that satisfies the trait and produces witnesses through the canonical recipe. v2's job is not to invent a federation API or a cache layer; it is to be a substrate that ruLake can host without modification. ADR-007 §Compatibility table makes this explicit: every existing ruLake artefact (the trait, the bundle struct, the witness function, the GCS / IPFS backends, the `mcp-server` audit row, the Console crypto) is unchanged.

## Open questions

Several genuine unknowns remain in the ADR (`docs/research/rvdna/v2-spec.md:1488`-area). Model-rotation policy is the largest: when ESM-2 ships a new checkpoint, the choice between batch-regenerating an entire cohort (uniform witnesses, expensive) and lazy rotation (cheap, federation refuses heterogeneous cohorts) is operator-dependent and not yet decided. T1 / T2 boundaries at population scale (1M+ samples) push k-mer vectors past workstation RAM; a GCS-backed T0 is the likely v0.3 answer but trades cloud-storage hit rates against the "fully local by default" stance. The streaming-mode crash protocol — what happens when the encoder dies between the BLAKE3 update and the witness commit — needs an explicit WAL discipline before v0.0 ships. Multi-sample manifest mode interaction with the witness is sketched but not wired (one bundle pointer per sample, one big pointer for the whole manifest, or both?). Browser-side decode budget for `verifyRvdnaWitness` is uncertain. Each is honest about being unresolved, and none block the v0.0.1 hot-tier scaffold from building.

## References

- ADR-007: `/home/ruvultra/projects/RuLake/docs/adrs/ADR-007-rvdna-as-rulake-substrate.md`
- v2 spec: `/home/ruvultra/projects/RuLake/docs/research/rvdna/v2-spec.md`
- Integration plan: `/home/ruvultra/projects/RuLake/docs/research/rvdna/integration-with-rulake.md`
- Corpus README: `/home/ruvultra/projects/RuLake/docs/research/rvdna/README.md`
- Reference implementation (v0.0.1): `/home/ruvultra/projects/RuLake/crates/rvdna-backend/`
- ruLake bundle: `/home/ruvultra/projects/RuLake/crates/core/src/bundle.rs:113` (`RuLakeBundle`), `/home/ruvultra/projects/RuLake/crates/core/src/bundle.rs:362` (`compute_witness`)
- Backend trait: `/home/ruvultra/projects/RuLake/crates/core/src/backend.rs:110`
- Federation primitive: `crates/core/src/lake.rs:521` (`search_federated`)
- v1 source (the biology layer): `vendor/ruvector/examples/dna/`
- v1 ADR-013 (superseded for new files): `vendor/ruvector/examples/dna/adr/ADR-013-rvdna-ai-native-format.md`
- IPFS bundle distribution pattern reused by v2: `/home/ruvultra/projects/RuLake/docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`
- ADR-156 memory-class framing: `/home/ruvultra/projects/RuLake/docs/adrs/ADR-156-rulake-as-memory-substrate.md`
- ESM-2 / Nucleotide Transformer / HyenaDNA — referenced as model checkpoints in v2-spec §c.4 for the `model_checkpoint_lo` field; the spec does not pin URLs, treating them as opaque versioned identifiers (e.g. `"esm2-650M:0.4.1"`, `"hyena-dna:1.0.0"`).
