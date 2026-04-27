# rvDNA v2 — Specification

**Status.** Draft. Supersedes
`vendor/ruvector/examples/dna/adr/ADR-013-rvdna-ai-native-format.md`
(v1, accepted 2026-02-11).

**Date.** 2026-04-26 (draft).

**Authors.** ruv.io, ruLake architecture, drafted against the brief
the user issued for "rvDNA v2 — the next iteration of the genomic
intelligence file format and pipeline".

**Branch of record.** `research/management-ui` (the active loop branch
where this corpus is being authored).

**Canonical citations.** v1 source at `vendor/ruvector/examples/dna/`;
ruLake at `src/`, `mcp-server/src/`, `docs/adrs/`, `node-wasm/`.

---

## a. Status and supersession

### a.1 Relationship to v1

rvDNA v1 (`vendor/ruvector/examples/dna/adr/ADR-013-rvdna-ai-native-format.md`)
defined a 7-section binary format with these promises:

1. Magic-prefixed 64-byte header with a CRC32 checksum
   (`vendor/ruvector/examples/dna/src/rvdna.rs:127`).
2. 2-bit packed DNA with a separate N-bit mask
   (`vendor/ruvector/examples/dna/src/rvdna.rs:272`).
3. Pre-computed k-mer vectors stored in HNSW-ready form, with
   optional int8 quantisation for 4× memory reduction
   (`vendor/ruvector/examples/dna/src/rvdna.rs:680`).
4. Sparse attention matrices in COO format
   (`vendor/ruvector/examples/dna/src/rvdna.rs:391`).
5. Variant tensors: per-position genotype likelihoods stored as f16,
   with binary-search lookup on position
   (`vendor/ruvector/examples/dna/src/rvdna.rs:540`).
6. Protein embeddings + GNN contact graphs (header design, not yet
   implemented in v1's `rvdna.rs` writer — only the format ADR
   specifies it; the runtime types live in `protein.rs`).
7. Epigenomic CpG sites + Horvath clock coefficients
   (`vendor/ruvector/examples/dna/src/epigenomics.rs`).
8. JSON metadata with per-section CRC32C checksums and provenance.

Plus the v1 `pipeline.rs` orchestrator
(`vendor/ruvector/examples/dna/src/pipeline.rs:115`) ties eight stages
together: k-mer analysis → variant calling → protein analysis →
biomarker streaming → biomarker risk scoring → epigenomic → pharma →
.rvdna emission.

### a.2 What v2 keeps verbatim from v1

v2 inherits v1's *content*. The 7 sections, the 2-bit DNA encoding,
the COO attention layout, the f16 variant tensor, the CSR protein
graph, the methylation tracks: all unchanged in shape. v2 modifies
the *envelope*, not the payload. A v1 reader that ignores unknown
header bytes can read v2 sections; v2 readers will, on demand, accept
v1 files and synthesise the missing v2 fields (see §m).

### a.3 What v2 modifies and why

| What v1 does | What v2 does | Why |
|---|---|---|
| 64-byte header with `version_major=1` (`rvdna.rs:127`). | 64-byte header with `version_major=2`, plus a new 96-byte **bundle pointer** appended after the section table. | The bundle pointer is what makes a `.rvdna` file federation-ready: it carries the SHAKE-256 witness, dimension, rotation seed, and rerank factor that ruLake needs (`src/bundle.rs::compute_witness`) to share cache entries across deployments without trusting payload bytes. |
| Per-section CRC32C checksums (`rvdna.rs:53`). | Per-section BLAKE3 checksums; the bundle pointer's `rvf_witness` is SHAKE-256(32). | BLAKE3 for payload (faster than CRC32C at scale, parallelisable, modern), SHAKE-256 for the witness (parity with `src/bundle.rs::compute_witness` which is hard-coded to SHAKE-256 because the brain layer above it consumes the same digest). The split is deliberate. |
| Magic `RVDNA\x01\x00\x00` (`rvdna.rs:49`). | Magic `RVDNA\x02\x00\x00`. | Non-overlapping with v1 magic; readers can dispatch on the third byte. |
| Codec enum: None / LZ4 / Zstd (`rvdna.rs:69`). | Per-section codec selection (None / Zstd / Zstd-dict). LZ4 dropped from defaults but reserved at value `1` for v1 read compatibility. | LZ4 was the v1 default for fast genomic decode. v2 moves to Zstd (better ratio, slightly slower) per-section because the cold tier (raw DNA) cares about ratio while the hot tier (k-mer vectors) cares about decode speed and prefers `Codec::None`. |
| Single-sample envelope. | Single-sample default + multi-sample manifest mode (a `.rvdna` may carry an array of `SamplePayload` blocks, each with its own bundle pointer). | Cohort federation needs a way to carry many samples in one file without inventing a new container; v2's manifest mode is a thin layer that points at multiple per-sample byte ranges in the same file. |
| K-mer block sized by caller (`rvdna.rs:802` `with_kmer_vectors(.., block_size: u64)`). | K-mer block sized by tier policy (T0 = whole-gene; T1 = 2 KB windows; T2 = chromosome-scale). The encoder picks the size based on the `--tier-budget` flag. | v1 leaves block size to the caller, which means cross-file compatibility is a lottery. v2 standardises three sizes so `rvdna-backend` can pull the right granularity without re-decoding. |
| No streaming append model. | Optional streaming-mode flag with monotonic per-epoch generation bumps. | Answers the brief's "static artefacts vs streaming biological state" question. See §j. |

### a.4 What v2 explicitly drops

v2 drops nothing from v1's *format*. v2 drops two pieces of v1's
*scope*:

1. **Quantum genomics path (v1 ADR-002)**. v1 left a
   `ruqu-algorithms` integration as research-phase. v2 considers it
   out-of-scope for the file format itself; quantum algorithms
   remain a layer above, consuming `.rvdna` files unmodified.
2. **FPGA acceleration plumbing (v1 ADR-001 §2 row "FPGA base
   calling")**. v2 doesn't reserve format bits for FPGA bitstream
   coupling. If FPGA paths re-emerge, they consume `.rvdna` like
   any other reader.

Neither drop affects the v1 writer's behaviour — both were
aspirational stubs, not shipping code in `vendor/ruvector/examples/dna/src/`.

---

## b. Goals and non-goals

### b.1 Goals (5)

1. **Query time in milliseconds, not minutes.** Inherit v1's measured
   12 ms for a full 8-stage demo on five genes
   (`vendor/ruvector/examples/dna/README.md` line 164) and add a
   ruLake cache layer that takes the marginal cost of a repeated
   query to ~1× the cache hit-path measurement, not the v1 full
   pipeline. Floor: any query the user has run before returns in
   < 1 ms cached.
2. **Zero recompute across sessions.** A `.rvdna` file produced once
   is consumed by every future query without re-encoding. The bundle
   witness proves the file's identity to a ruLake cache, so a warm
   cache hit (`src/cache.rs::VectorCache::can_skip_check_interned`)
   returns answers without touching disk.
3. **Cross-sample queries become trivial.** Federate over N samples
   in one `lake.search_federated` call (`src/lake.rs:521`). 10k
   samples scales by registering each as a backend collection; the
   adaptive per-shard rerank already handles the fan-out cost
   (`src/lake.rs:533`).
4. **IO + cache scaling, not compute scaling.** v2's tiered indexing
   model (§e) splits the format into hot/warm/cold tiers, each with
   its own ruLake `BackendAdapter` impl. Tier 0 (k-mer HNSW) lives
   in RAM with explicit `MAX_PULLED_VECTORS` caps
   (`src/backend.rs:60`); Tier 1 mmap'd; Tier 2 lazily pulled. No
   tier loads everything by default.
5. **Fully local by default.** Same as v1's privacy stance
   (`vendor/ruvector/examples/dna/README.md` line 21), plus the
   IPFS bundle-distribution path from `docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`
   for opt-in cross-site sharing. The default install never sends
   PHI off-host; a single CLI flag opts into the IPFS pin path.

### b.2 Non-goals (3)

1. **v2 does not replace upstream variant-calling tools.** GATK,
   DeepVariant, freebayes, etc. continue to be the production
   variant callers for clinical pipelines. v2 ingests their output
   (or v1's Bayesian caller) and stores it. v2's `rvdna_call_variants`
   tool exposes the stored variants, not a new caller.
2. **v2 does not ship clinical-decision-grade interpretation.** The
   polygenic risk scores, Horvath clock outputs, and CYP2D6
   recommendations from v1 are *informational*. A regulated layer
   above v2 (e.g. an FDA-cleared CDS) wraps them with the disclaimer,
   class-II controls, and human-in-loop discipline a clinical use
   demands. v2's clinical mode (§k) is about provenance and PHI
   handling, not about claiming clinical validity.
3. **v2 does not fragment the v1 pipeline.** Every v1 stage stays
   callable. v2 adds an envelope and an index path; it doesn't replace
   `vendor/ruvector/examples/dna/src/pipeline.rs:115`. Users running
   v1's `cargo run --release -p rvdna` produce v1 files; users running
   `rvdna v2 encode` produce v2 files; the binary at
   `vendor/ruvector/examples/dna/src/main.rs` continues to work
   unchanged.

---

## c. File format §1: physical layout

### c.1 ASCII overview

```
                ┌─────────────────────────────────────────────────────┐
       0x0000   │  Magic   "RVDNA\x02\x00\x00"            8 B         │
       0x0008   │  Header  version, codec, flags, sec  56 B           │
                ├─────────────────────────────────────────────────────┤
       0x0040   │  Section table (7 entries × 16 B)    112 B          │
                ├─────────────────────────────────────────────────────┤
       0x00B0   │  Bundle pointer                       96 B          │
                │   ├ rvf_witness (32 B SHAKE-256)                    │
                │   ├ dim (8 B)                                        │
                │   ├ rotation_seed (8 B)                              │
                │   ├ rerank_factor (8 B)                              │
                │   ├ generation kind+value (24 B)                     │
                │   ├ pii_policy_class (1 B enum)                      │
                │   ├ memory_class (1 B enum: "genomic"=0x01)          │
                │   ├ profile flags (2 B)                              │
                │   └ sidecar offset (8 B → MessagePack metadata)     │
                ├─────────────────────────────────────────────────────┤
       0x0110   │  Manifest (optional, multi-sample mode)              │
                │   - n_samples (4 B)                                  │
                │   - per-sample (offset, length, sample_id_off) 24 B  │
                ├─────────────────────────────────────────────────────┤
   align(64)    │  Section 0: 2-bit DNA + Phred quality (cold)        │
                ├─────────────────────────────────────────────────────┤
   align(64)    │  Section 1: K-mer HNSW vectors (hot, T0)            │
                ├─────────────────────────────────────────────────────┤
   align(64)    │  Section 2: Attention COO matrices (warm, T1)       │
                ├─────────────────────────────────────────────────────┤
   align(64)    │  Section 3: Variant f16 tensor (warm, T1)           │
                ├─────────────────────────────────────────────────────┤
   align(64)    │  Section 4: Protein CSR graphs (warm, T1)           │
                ├─────────────────────────────────────────────────────┤
   align(64)    │  Section 5: Epigenomic CpG + clock coeffs (cold)    │
                ├─────────────────────────────────────────────────────┤
   align(64)    │  Section 6: Biomarker time-series (warm/streaming)  │
                ├─────────────────────────────────────────────────────┤
   align(64)    │  Section 7: Metadata (MessagePack provenance)       │
                ├─────────────────────────────────────────────────────┤
       EOF      │  Footer "RVDNA_END" + global BLAKE3        16 B    │
                └─────────────────────────────────────────────────────┘
```

Diff from v1: same section count semantically, but v2 adds **section
7** (biomarkers; v1 stored these only in runtime via
`vendor/ruvector/examples/dna/src/biomarker_stream.rs:1`, never
serialised) and inserts the **bundle pointer** at offset `0x00B0`.

### c.2 Header (64 bytes, byte-for-byte)

```
Off   Sz  Type    Field               Notes
0x00   8  u8[8]   magic               "RVDNA\x02\x00\x00"
0x08   2  u16     version_major       2
0x0A   2  u16     version_minor       0
0x0C   4  u32     flags               see flag map below
0x10   8  u64     total_file_size     including footer
0x18   8  u64     sequence_length     total bases (sum across samples)
0x20   4  u32     num_sections        always 8 in v2 (0..7); 7 in v1
0x24   4  u32     section_dir_offset  always 0x40
0x28   1  u8      default_codec       0=None 2=Zstd 3=Zstd+dict (1 reserved for v1 LZ4)
0x29   1  u8      endianness          0xEF = little-endian
0x2A   2  u16     ref_genome_id       0=none 1=GRCh38 2=T2T-CHM13 3=hg19
0x2C   4  u32     num_chromosomes
0x30   8  u64     creation_timestamp  Unix seconds
0x38   4  u32     creator_version     packed semver: major<<16 | minor<<8 | patch
0x3C   4  u32     header_blake3_lo32  low 32 bits of BLAKE3(header[0..0x3C])
```

**Flag map (32 bits, bit 0 = LSB)**:

| Bit | Name | Meaning |
|---:|---|---|
| 0 | HAS_QUALITY | Phred 6-bit quality scores in §0 |
| 1 | HAS_KMER_INDEX | §1 populated |
| 2 | HAS_ATTENTION | §2 populated |
| 3 | HAS_VARIANTS | §3 populated |
| 4 | HAS_PROTEIN | §4 populated |
| 5 | HAS_EPIGENOMIC | §5 populated |
| 6 | HAS_BIOMARKERS | §6 populated (new in v2) |
| 7 | IS_PHASED | haplotype phase data present |
| 8 | KMER_QUANTIZED | §1 stores int8 codes, not f32 |
| 9 | ATTENTION_SPARSE | §2 in COO (always true in v2) |
| 10 | MMAP_SAFE | every section is 64-aligned, no compression on hot tier |
| 11 | MULTISAMPLE | manifest follows the bundle pointer |
| 12 | STREAMING | append-mode file with per-epoch generation bumps (§j) |
| 13 | CLINICAL_PROFILE | the bundle's `pii_policy` is `phi-strict` (§k) |
| 14 | RESEARCH_PROFILE | the bundle's `pii_policy` is `research-open` |
| 15 | WITNESS_FROZEN | declares `Consistency::Frozen` semantics |
| 16-31 | reserved | must be 0 in v2.0 |

### c.3 Section table (16 bytes per entry)

Identical layout to v1 (`rvdna.rs:115` `SectionEntry`):

```
u64 section_offset       u32 compressed_size       u32 uncompressed_size
```

Eight entries (sections 0..7). Section 7 is new in v2 (biomarkers).

### c.4 Bundle pointer (96 bytes)

This is the load-bearing addition. The bundle pointer is byte-for-byte
isomorphic to a `RuLakeBundle` (`src/bundle.rs:113`) serialised in
binary form. v2 readers construct an in-memory `RuLakeBundle` from
this block; v2 writers encode their `RuLakeBundle` instance into it.
The witness covers everything below (raw DNA, all tensors, metadata)
*and* the model checkpoint hash, so changing any upstream stage rotates
the witness deterministically (see §d).

```
Off   Sz  Type    Field
0x00  32  u8[32]  rvf_witness         SHAKE-256(32) — see §d for input recipe
0x20   8  u64     dim                 vector dim of §1 k-mer vectors
0x28   8  u64     rotation_seed       RaBitQ rotation seed (carried into ruLake)
0x30   8  u64     rerank_factor       RaBitQ rerank factor
0x38   1  u8      generation_kind     0x00=Num, 0x01=Opaque (matches src/bundle.rs:56)
0x39   7  u8[7]   reserved            zero-fill
0x40   8  u64     generation_value    if Num: the u64; if Opaque: byte offset into sidecar
0x48   1  u8      pii_policy_class    0=research-open, 1=phi-strict, 2=opaque
0x49   1  u8      memory_class        0x01="genomic" (matches ADR-156 enum)
0x4A   2  u16     profile_flags       bit 0=clinical, 1=research, 2=streaming
0x4C   4  u32     model_checkpoint_lo low 32 bits of SHA-256(model_id || ":" || version)
0x50   8  u64     sidecar_offset      offset of MessagePack lineage_id+pii_policy text
0x58   8  u64     sidecar_size
```

The `model_checkpoint_lo` is a hash over the strings the encoder used:
`"esm2-650M:v0.4.1"`, `"hyena-dna:1.0.0"`, etc. v1's gotcha was that
"`.rvdna` is model-bound" without making the binding explicit; v2 makes
it part of the witness input so a `.rvdna` file generated against
ESM-2 v0.4.1 cannot be confused with one generated against v0.5.0 even
if every other field matches.

### c.5 Sections — encoding, compression, checksum

Every section follows this contract:

1. **On-disk encoding**: the raw bytes specified per-section below.
2. **Optional compression**: per-section choice (`Codec::None | Zstd
   | ZstdDict`); the section table's `compressed_size` and
   `uncompressed_size` differ when codec is non-None.
3. **Checksum**: BLAKE3-256 of the *uncompressed* bytes, stored in the
   metadata sidecar (§7) as `sections[i].checksum_blake3` (hex).
4. **Alignment**: every section starts at a 64-byte boundary
   (`SECTION_ALIGN` in v1 `rvdna.rs:58`).

#### Section 0 — DNA + Phred quality (cold, T2)

Identical to v1's Section 0
(`vendor/ruvector/examples/dna/src/rvdna.rs:272` `encode_2bit`).
2 bits per base (A=00, C=01, G=10, T=11), separate N-bit mask,
optional 6-bit Phred quality block. Per-chromosome blocks of 16 KB
uncompressed.

Per-Mb storage: ~251 KB seq-only; ~1,001 KB with quality. Verbatim
from v1's measurement
(`vendor/ruvector/examples/dna/adr/ADR-013-rvdna-ai-native-format.md` line 73).

Compression default: `Zstd` (this section dominates file size at the
chromosome scale; ratio matters more than decode latency because v2
puts this in tier T2 — fetched on demand, not on the hot path).

#### Section 1 — K-mer vectors (hot, T0)

Inherits v1's layout
(`vendor/ruvector/examples/dna/src/rvdna.rs:680` `KmerVectorBlock`)
with one addition: a **canonical block size policy**.

| Tier policy | Block size | When used |
|---|---|---|
| `--tier-budget hot` | whole-gene (no internal segmentation) | Single-gene corpora; tier T0 in RAM. |
| `--tier-budget warm` | 2 KB windows | Default. Allows partial tier-T0 loads. |
| `--tier-budget cold` | chromosome-scale | Large genomes encoded for archival. |

Per-block fields unchanged from v1 (k, dim, region, vector, optional
int8 quantised vector + scale). v2 adds an explicit `model_id: String`
field per block — this is what feeds the `model_checkpoint_lo` in the
bundle pointer.

Compression default: `Codec::None` (this section is the hot tier;
decode latency wins). With `KMER_QUANTIZED` flag the int8 codes are
already 4× smaller than f32; further compression would hurt the
sub-microsecond random-access target inherited from v1
(ADR-013 line 153).

Checksum: BLAKE3 over the concatenation of all blocks' raw bytes.

#### Section 2 — Attention COO (warm, T1)

Verbatim v1 layout
(`vendor/ruvector/examples/dna/src/rvdna.rs:391` `SparseAttention`).
Per-window header: `u64 genomic_start | u32 nnz | u32 data_offset`.
Triplets: `u16 row | u16 col | f16 value` for index_dtype=u16; widen
to `u32 | u32 | f32` for index_dtype=u32.

Compression default: `Zstd` (the COO triplets compress well; this is
warm tier so decode tax is acceptable).

#### Section 3 — Variant f16 tensor (warm, T1)

Verbatim v1
(`vendor/ruvector/examples/dna/src/rvdna.rs:540` `VariantTensor`).
Per-variant: `u64 position | u8 ref(2-bit) | u8 num_alt | u8[num_alt]
alts | f16[G] genotype_likelihoods | f16 allele_freq | u8 filter_flags`
where `G = (num_alt+1)*(num_alt+2)/2`.

v2 addition: a Phred quality byte per variant (already in v1's
`VariantTensor::qualities` array but not exposed via the format ADR;
v2 makes it normative).

Compression default: `Zstd`.

#### Section 4 — Protein CSR graphs (warm, T1)

Verbatim v1 ADR-013 Section 4 (header design — the runtime types
live in `vendor/ruvector/examples/dna/src/protein.rs:1`). Per protein:
`u32 protein_id | u32 gene_id | u32 num_residues | u32 embed_offset |
u32 csr_rowptr_off | u32 csr_colidx_off | u32 csr_values_off | u32
annotation_off`. CSR graph: `row_ptr: u32[n+1]`, `col_idx: u32[edges]`,
`values: f16[edges]`. SS: `u8[n]`. Binding: `u8[n]` flags.

Compression default: `Zstd`.

#### Section 5 — Epigenomic + Horvath clock (cold, T2)

Verbatim v1 ADR-013 Section 5 + `vendor/ruvector/examples/dna/src/epigenomics.rs`.
CpG (12 B): `u64 position | f16 beta | u16 coverage`. Clock (12 B):
`u32 cpg_idx | f32 coeff | f32 intercept_contrib`.

Compression default: `Zstd` (cold tier).

#### Section 6 — Biomarker time-series (warm, streaming-aware)

**New in v2.** Serialises what v1's
`vendor/ruvector/examples/dna/src/biomarker_stream.rs:1` keeps in
memory only.

```
Header (24 B):
  u32 num_biomarkers
  u32 num_samples
  u64 epoch_start_unix_ms
  u32 sample_period_ms     (0 = irregular)
  u32 reserved
Per biomarker (16 B):
  u32 biomarker_id          (CodeSystem-aware: LOINC, custom)
  u16 dtype                 (0=f32, 1=f16, 2=int16)
  u16 unit_id               (UCUM enum)
  u64 series_data_offset
Series data:
  Per sample: timestamp_offset_ms (u32) || value (per dtype)
```

The streaming-mode flag (header bit 12) makes this section append-
only: a writer adds new samples, increments
`epoch_start_unix_ms`-derived generation counter, rotates the witness,
and rewrites the bundle pointer. ruLake's `Consistency::Eventual` and
`Consistency::Fresh` modes pick up the rotation the next time they
ask `current_bundle()` (`src/backend.rs:125`).

Compression default: `Codec::None` (append-friendly; streaming mode
needs to write the tail without re-encoding the prefix).

#### Section 7 — Metadata (MessagePack)

Identical to v1 Section 6 contents
(`vendor/ruvector/examples/dna/src/rvdna.rs:1041` `read_metadata`)
but at slot 7 in v2's table. MessagePack-encoded:

```
{
  "format": "RVDNA",
  "format_version": 2,
  "creator": "rvdna-encode/2.0.0",
  "encoder_args": { ... },
  "models": {
    "kmer": { "id": "fnv1a-d512", "version": "1.0" },
    "protein": { "id": "esm2-650M", "version": "0.4.1" },
    "variant_calling": { "id": "rvdna-bayesian", "version": "1.0" }
  },
  "sections": [
    { "kind": "dna_2bit",      "checksum_blake3": "..." },
    { "kind": "kmer_vectors",  "checksum_blake3": "...", "block_count": 42 },
    ...
  ],
  "lineage_id": "ol://jobs/rvdna-encode-2026-04-26-001",
  "pii_policy": "phi-strict | research-open | opaque",
  "annotations": [ ... ]
}
```

Compression default: `Zstd` (small section; ratio doesn't matter).

### c.6 BLAKE3 vs SHAKE-256: why the split

Two hash functions in one format is a smell unless the reason is
explicit. v2 carries both intentionally:

- **BLAKE3 for payload checksums**. Chosen because v2 sections can be
  large (multi-GB sequence + tensor for whole-genome corpora). BLAKE3
  is ~3× faster than SHA-256 on modern CPUs and parallelisable per
  64-byte chunk, which matters for the cold tier.
- **SHAKE-256 for the bundle witness**. Forced by parity with
  `src/bundle.rs::compute_witness`, which uses SHAKE-256 because the
  brain-substrate layer above ruLake (ADR-156, "rulake as memory
  substrate") consumes the same digest. Using BLAKE3 here would mean
  a `.rvdna` file's witness could not be compared with a ruLake
  bundle's witness without a re-hash — that defeats the whole point
  of "the bundle pointer IS a `RuLakeBundle`".

The split is documented in §d and in the ADR-007 consequences section.

### c.7 Footer (16 bytes)

```
0x00  8  u8[8]   magic_footer  "RVDNA_END" little-endian
0x08  4  u32     global_blake3 low 32 bits of BLAKE3(file[0..footer_offset])
0x0C  4  u32     footer_offset self-offset from file start
```

Same shape as v1 ADR-013 line 132 except the global checksum is
BLAKE3 (not XOR of CRCs).

---

## d. File format §2: the witness chain

### d.1 What the witness covers

The 32-byte SHAKE-256 in the bundle pointer (offset `0x00`) is the
output of `compute_witness` (`src/bundle.rs::compute_witness`) over a
canonical input string built from:

```
data_ref         "rvdna://{file_id}"  (file_id = BLAKE3 of the
                                        creator_version + creation_timestamp +
                                        first 1 KB of section 0)
dim              §1 k-mer vector dim
rotation_seed    bundle pointer offset 0x28
rerank_factor    bundle pointer offset 0x30
generation       bundle pointer offset 0x38..0x40 (kind tag + value)
```

Plus the v2-specific extension fields concatenated into the
`generation`'s `Opaque` payload when `generation_kind = 0x01`:

```
   model_checkpoint_lo (4 B)
   sections_blake3_root (32 B)  -- BLAKE3 of concatenated section checksums
   profile_flags (2 B)
```

The "sections_blake3_root" is the chain that makes any change to any
upstream stage rotate the witness. Re-encoding §1 with a new k-mer
embedding model changes its BLAKE3, which changes the root, which
changes the witness — exactly the v1 gotcha (`vendor/ruvector/examples/dna/README.md`
line 151) made explicit and computable.

### d.2 Why use ruLake's `compute_witness` verbatim

Because two systems looking at the same `.rvdna` file must compute
the same witness, byte-for-byte, without a shared library
dependency. The v2 spec borrows the `compute_witness` formula from
`src/bundle.rs:362` exactly: SHAKE-256 with the domain-separation
prefix `"rulake-bundle-witness-v1|"`, length-prefixed concatenation,
the variant-tag byte for `Generation::Num` vs `Opaque` (added in the
2026-04-23 security audit, `src/bundle.rs::Generation::hash_bytes`).

This means: a ruLake `BackendAdapter::current_bundle()` call and a
`.rvdna` file's bundle pointer both produce the same 32 bytes when
they describe the same data. Cache-sharing across deployments is a
free consequence (`src/cache.rs` "the cross-backend share").

### d.3 Witness-rotation triggers

The witness changes if and only if any of:

1. K-mer block bytes change (§1).
2. Attention COO bytes change (§2).
3. Variant tensor bytes change (§3).
4. Protein CSR bytes change (§4).
5. Epigenomic bytes change (§5).
6. Biomarker bytes change (§6) — including a streaming-mode append.
7. Bundle pointer's `dim`, `rotation_seed`, `rerank_factor`, `generation`,
   or `model_checkpoint_lo` changes.

It does NOT change if:

- Metadata (§7) annotations are added — annotations are caller-visible
  but not part of the digest (mirrors `RuLakeBundle::memory_class`'s
  exclusion from witness, `src/bundle.rs:572`).
- The file is re-compressed with a different codec (the
  `uncompressed_size` and uncompressed bytes are what get hashed).
- The footer's offset shifts because of compression.

This is the same discipline ruLake's bundle has — the `pii_policy`,
`lineage_id`, and `memory_class` fields are NOT part of the witness
(`src/bundle.rs:135`); they're metadata for governance, not identity.

### d.4 Verifying a witness without trusting the file

Anyone with the file can verify:

```rust
// Pseudocode against existing types.
let file = mmap::open("sample.rvdna v2")?;
let header = RvdnaHeader::from_bytes(&file[..0x40])?;
let bundle_ptr = BundlePointer::from_bytes(&file[0xB0..0x110])?;

// 1. Recompute every section's BLAKE3.
let section_checksums: Vec<[u8; 32]> = (0..8)
    .map(|i| blake3_section(&file, &header.sections[i]))
    .collect();
let sections_root = blake3::Hasher::new()
    .update(&section_checksums.concat())
    .finalize();

// 2. Build the Generation::Opaque payload.
let mut g = Vec::with_capacity(38);
g.extend_from_slice(&bundle_ptr.model_checkpoint_lo.to_le_bytes());
g.extend_from_slice(sections_root.as_bytes());
g.extend_from_slice(&bundle_ptr.profile_flags.to_le_bytes());

// 3. Recompute the witness using ruLake's exact function.
let bundle = rulake::RuLakeBundle::new(
    format!("rvdna://{}", file_id),
    bundle_ptr.dim as usize,
    bundle_ptr.rotation_seed,
    bundle_ptr.rerank_factor as usize,
    rulake::Generation::Opaque(hex::encode(g)),
);
assert_eq!(bundle.rvf_witness, hex::encode(bundle_ptr.witness));
```

The `node-wasm/src/lib.rs` `verifyBundleJson` already does this for
ruLake bundles
(`docs/adrs/ADR-006-rulake-console-vite-github-pages.md` line 22 lists
the exported names). v2 adds a sibling `verifyRvdnaWitness(bytes)`
binding in the same WASM crate so the Console can verify a `.rvdna`
file in-browser.

---

## e. File format §3: tiered indexing

The brief calls out a "memory pressure" gotcha: indexing everything
naively eats RAM at population scale. v2's answer is three tiers,
each implemented as a distinct ruLake `BackendAdapter`
(`src/backend.rs:110`).

### e.1 Tier 0 (T0) — Hot: k-mer HNSW in RAM

- **What's loaded**: §1 k-mer vectors only.
- **Where**: in-process RAM via `RuLake::register_backend(...)` of an
  `RvdnaT0Backend` instance (one per loaded `.rvdna` file).
- **Caps** (per
  `src/backend.rs:60` `MAX_PULLED_*` constants):
  `MAX_PULLED_VECTORS=100M`, `MAX_PULLED_DIM=8192`,
  `MAX_PULLED_BYTES=16 GiB`. v2 enforces a tighter
  `MAX_T0_BYTES_PER_FILE=512 MiB` to prevent a single sample from
  exhausting tier 0.
- **Cache behaviour**: `Consistency::Fresh` by default
  (`src/cache.rs::Consistency`).
- **Latency target**: < 1 ms p50 on warm cache; 12 ms p99 on cold
  prime (the v1 measured floor for full-pipeline encode).

### e.2 Tier 1 (T1) — Warm: protein + attention + variants, mmap'd

- **What's loaded**: §2, §3, §4 (and §6 in non-streaming mode).
- **Where**: `mmap::open()` on the file, with the relevant section
  byte ranges materialised lazily.
- **Caps**: `MAX_T1_FILES_OPEN=256` (per process); per-file mmap is
  bounded by the OS (`vm.max_map_count`).
- **Cache behaviour**: `Consistency::Eventual { ttl_ms: 5_000 }` —
  warm-tier coherence isn't on the hot path, 5-second freshness is
  fine.
- **Latency target**: 100 µs p50 for a window query; 1 ms p99 first
  hit per file.

### e.3 Tier 2 (T2) — Cold: raw DNA + epigenomic, fetched on demand

- **What's loaded**: §0, §5.
- **Where**: NEVER eagerly. Decoded only when a query asks for raw
  bases or methylation values. Behind a ruLake backend that proxies
  to either local disk, GCS (`gcs-backend/`), or IPFS
  (`ipfs-backend/`).
- **Caps**: `MAX_T2_DECODE_BYTES_PER_QUERY=64 MiB`. Anything above
  refuses with `RVDNA_T2_BUDGET_REFUSED`.
- **Cache behaviour**: `Consistency::Frozen` — the cold tier is
  immutable per file, witness-frozen.
- **Latency target**: 50 ms p50 on local disk; 500 ms p50 on GCS;
  variable on IPFS (kubo + gateway).

### e.4 `RvdnaT0Backend` sketch

The trait at `src/backend.rs:110` is four methods plus an optional
`current_bundle` override (which v2 always implements — that's the
witness-sharing path).

```rust
// In rvdna-backend/src/t0.rs (sketch — not invented APIs).
use rulake::{BackendAdapter, BackendId, CollectionId, PulledBatch,
             RuLakeBundle, Generation, Result, RuLakeError};

pub struct RvdnaT0Backend {
    id: String,
    /// Loaded .rvdna v2 file, mmap'd. The §1 vectors are read on
    /// `pull_vectors` and never mutated.
    file: Arc<RvdnaV2File>,
    /// Cached on first call; the bundle pointer's witness is the
    /// deterministic identity for the (file, collection) pair.
    bundle_cache: parking_lot::RwLock<Option<RuLakeBundle>>,
}

impl BackendAdapter for RvdnaT0Backend {
    fn id(&self) -> &str {
        &self.id
    }

    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        // One collection per gene/region in the §1 k-mer block list.
        self.file.kmer_collection_ids()
    }

    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        let (ids, vectors, dim, generation) = self
            .file
            .pull_kmer_collection(collection)
            .map_err(|e| RuLakeError::InvalidParameter(e.to_string()))?;
        Ok(PulledBatch {
            collection: collection.to_string(),
            ids,
            vectors,
            dim,
            generation,
        })
    }

    fn generation(&self, collection: &str) -> Result<u64> {
        // T0 is immutable per file → return file_id-derived constant.
        self.file
            .stable_generation(collection)
            .map_err(|e| RuLakeError::InvalidParameter(e.to_string()))
    }

    /// The override that makes cross-file cache sharing free.
    fn current_bundle(
        &self,
        collection: &str,
        rotation_seed: u64,
        rerank_factor: usize,
    ) -> Result<RuLakeBundle> {
        // The .rvdna file already carries a bundle pointer with a
        // pre-computed witness. We trust it (the witness is verifiable
        // against payload bytes by §d.4), so we synthesise a
        // RuLakeBundle that has the same witness.
        let ptr = self.file.bundle_pointer();
        // Build the Generation::Opaque payload that produces the
        // witness verbatim.
        let opaque = self.file.generation_opaque_for(collection)?;
        Ok(RuLakeBundle::new(
            format!("rvdna://{}/{}", self.file.file_id(), collection),
            ptr.dim as usize,
            rotation_seed,
            rerank_factor,
            Generation::Opaque(opaque),
        )
        .with_memory_class("genomic"))
    }

    fn supports_pushdown(&self) -> bool {
        // T0 is in-RAM HNSW; no benefit from "pushdown" — the cache
        // is already as close to the data as it gets.
        false
    }
}
```

T1 and T2 follow the same shape with mmap-window and lazy-decode
backends, respectively. Sketched in `integration-with-rulake.md`.

### e.5 What the tier mapping buys you

- **Population scale**: 10k samples × T0 (one collection per gene-of-
  interest at, say, 256-d × 5 genes × 100 vectors per gene = 0.5 MB
  per sample × 10k = 5 GB) fits in a single workstation's RAM. T1/T2
  stay on disk.
- **Federation**: `lake.search_federated(&[("rvdna_sample_001", "HBB"),
   ("rvdna_sample_002", "HBB"), ...], &query, k=10)` already works
  (`src/lake.rs:521`) the moment the backends are registered.
- **Witness sharing across copies**: the same `.rvdna` file copied to
  two hosts produces the same bundle witness (witness is over content,
  not file path) — `src/cache.rs::install_prebuilt_interned` shares
  the cache entry.

---

## f. Pipeline §1: encode

### f.1 CLI surface

```
rvdna v2 encode <input> [--output <path>] [--profile clinical|research]
                       [--model esm2|nt|hyena] [--tier-budget hot|warm|cold]
                       [--streaming] [--lineage-id <ol-job-id>]
                       [--pii-policy <class>] [--ipfs-pin]
```

Inputs: FASTA, FASTQ, BAM (read with `noodles` crate; v2 doesn't ship
its own BAM parser).

Outputs:
- `<output>.rvdna` — the v2 file.
- `<output>.bundle.json` — the bundle JSON for ruLake direct ingest
  (this is just the bundle pointer fields rendered as JSON; identical
  to `src/bundle.rs::RuLakeBundle::to_json` output).

### f.2 Stage sequence

Mirrors v1 `vendor/ruvector/examples/dna/src/pipeline.rs:115`
verbatim, with one append:

```
[1] Parse input → DnaSequence (raw)
        ↓
[2] 2-bit encode + N-mask + Phred quality → §0
        ↓
[3] K-mer vectorise (k=11, d=512, FNV-1a unless --model overrides) → §1
        ↓
[4] Optional: protein-LM embed via ESM-2 / NT / Hyena → §4
        ↓
[5] Compute attention windows over §1 vectors → §2
        ↓
[6] Bayesian variant call (v1 unchanged) → §3
        ↓
[7] Methylation profile + Horvath clock → §5
        ↓
[8] Biomarker series serialise (if --streaming) → §6
        ↓
[9] BLAKE3 every section, build sections_root
        ↓
[10] SHAKE-256 the bundle witness over (data_ref, dim, seed, rerank,
     Generation::Opaque(model || sections_root || profile_flags))
        ↓
[11] Write file: header → table → bundle pointer → sections → footer
        ↓
[12] (Optional) IPFS pin: kubo `pin/add` of the file path; record CID
     in bundle JSON's `data_ref` field.
```

### f.3 Witness coverage by stage

The witness covers steps 2–8 by way of the sections_root. Step 9 is
where the chain closes. Anything later — choosing a different
compression codec for §0, adding annotations to §7's metadata,
re-pinning to a different IPFS CID — does NOT rotate the witness.

### f.4 Bundle JSON output

```json
{
  "format_version": 2,
  "data_ref": "rvdna://blake3:abc123.../HBB",
  "dim": 512,
  "rotation_seed": 42,
  "rerank_factor": 20,
  "generation": "abc123...sections_root_and_model_hex",
  "rvf_witness": "shake256-32-bytes-hex",
  "pii_policy": "phi-strict",
  "lineage_id": "ol://jobs/rvdna-encode-2026-04-26-001",
  "memory_class": "genomic"
}
```

This file is what an operator drops next to a registered ruLake backend
to refresh the cache (`mcp-server/src/server.rs:391`
`rulake_refresh_from_bundle_dir`).

---

## g. Pipeline §2: query

Five intent verbs, mirroring `rulake_query`'s
`search/verify/explain/refresh` shape (see
`docs/adrs/sdk/ADR-004-rulake-mcp-server.md` for the parent design).

### g.1 `find` — k-mer similarity, witness-pinned

- **Input**: `(file_id_or_witness, query_seq, k)`.
- **Output**: `Vec<{ file_id, gene_id, position, score, witness }>`.
- **Cache key**: derived as `(t0_backend_id, collection_id, witness)`.
- **Latency target**:
  - p50: 0.5 ms (warm cache, single file). Floor: v1's `search_top10`
    benchmark in `vendor/ruvector/examples/dna/benches/dna_bench.rs:76`
    measured single-shard k-mer search; v2 adds ~1 % cache tax per
    `BENCHMARK.md`'s 1.02× hit-path measurement.
  - p99: 12 ms (first prime; v1's full-pipeline floor).
- **Refusal**: `RVDNA_WITNESS_MISMATCH_REFUSED` if the caller's
  asserted witness differs from the file's actual witness.

### g.2 `call` — variant calling intent (returns SNP/indel set)

- **Input**: `(file_id_or_witness, region: GenomicRegion, min_quality,
  min_depth)`.
- **Output**: `Vec<VariantCall>` (the v1 type at
  `vendor/ruvector/examples/dna/src/pipeline.rs:60` extended with a
  `witness: String` field).
- **Cache key**: `(t1_backend_id, "variants", witness, region_hash)`.
- **Latency target**:
  - p50: 200 µs (per v1's `1000-position variant scan` of 336 µs
    `vendor/ruvector/examples/dna/README.md` line 162; the cache
    keeps repeat calls under 50 µs).
  - p99: 5 ms.
- **Refusal**: `RVDNA_VARIANT_REFUSED_LOW_DEPTH` when the requested
  region has no variant calls above `min_depth`.

### g.3 `translate` — DNA → protein

- **Input**: `(file_id_or_witness, frame: 1|2|3|-1|-2|-3, region)`.
- **Output**: `(amino_acids: String, contacts: Vec<(usize,usize,f32)>,
  secondary_structure: Vec<char>)`.
- **Cache key**: `(t1_backend_id, "protein", witness, frame, region_hash)`.
- **Latency target**:
  - p50: 25 ns per kb (verbatim v1 measurement,
    `vendor/ruvector/examples/dna/README.md` line 161).
  - p99: 1 ms (cold tier for the protein graph).
- **Refusal**: `RVDNA_TRANSLATE_NO_ORF` when no ORF is present in the
  requested frame.

### g.4 `score` — polygenic risk + drug dosing

Both PRS and pharmacogenomic dosing are dot-products against a
coefficient vector — same shape, different vector. v2 unifies them
under `score`.

- **Input**: `(file_id_or_witness, score_id: "prs:cad" | "cpic:cyp2d6",
  optional_coefficient_override: Vec<f32>)`.
- **Output**: `{ score: f32, components: HashMap<String, f32>,
  category: String, witness: String }`.
- **Cache key**: `(t1_backend_id, "score", witness, score_id_hash)`.
- **Latency target**:
  - p50: 2 µs (verbatim v1 measurement of "Composite risk score (20
    SNPs)" `vendor/ruvector/examples/dna/README.md` line 165).
  - p99: 50 µs.
- **Refusal**: `RVDNA_SCORE_REFUSED_INSUFFICIENT_COVERAGE` when the
  required SNPs aren't present in §3.

### g.5 `verify` — re-hash a `.rvdna` file's witness

- **Input**: `(file_bytes_or_path)`.
- **Output**: `{ asserted_witness: String, computed_witness: String,
  match: bool, mismatched_sections: Vec<u8> }`.
- **Cache key**: not cached (always recomputes).
- **Latency target**:
  - p50: 50 ms for a 100 MB file (BLAKE3 throughput-bound).
  - p99: 500 ms for a 1 GB file.
- **Refusal**: `RVDNA_VERIFY_TAMPERED` when computed witness differs
  from asserted; `RVDNA_VERIFY_VERSION_TOO_NEW` when format_version
  exceeds reader's max.

---

## h. MCP tools (genomic surface for agents)

Five tools exposed by a sibling `mcp-rvdna/` crate, mirroring the
shape of `mcp-server/`'s tools (see `mcp-server/src/server.rs:189`
for the `tool_router` macro pattern).

Capability tiers re-use the same enum as ruLake's `mcp-server`:
- `Read`: `mcp:rvdna:read`
- `Internal`: `mcp:rvdna:internal` (lineage)
- `Admin`: `mcp:rvdna:admin` (writes / re-encode)

A new tier:
- `Clinical`: `mcp:rvdna:clinical`. Required when the bundle's
  `pii_policy = "phi-strict"`. Implies `Read`.

### h.1 `rvdna_find`

- **Capability**: `Read`.
- **JSON request**:
  ```json
  {
    "file_id": "blake3:abc123...",
    "asserted_witness": "shake256-32-bytes-hex",
    "query_seq": "ATGGCCATTGTAATG",
    "k": 10
  }
  ```
- **JSON response**:
  ```json
  {
    "hits": [
      { "gene": "HBB", "position": 20, "score": 0.97,
        "witness": "shake256..." },
      ...
    ]
  }
  ```
- **Audit row**: `code=RVDNA_FOUND_OK`, `witness_in=asserted`,
  `witness_out=actual`, `result_size=k`. Refusal:
  `RVDNA_WITNESS_MISMATCH_REFUSED`.

### h.2 `rvdna_call_variants`

- **Capability**: `Read` (research) or `Read + Clinical` if PHI flag
  set.
- **JSON request**:
  ```json
  {
    "file_id": "blake3:...",
    "region": { "chromosome": "chr11", "start": 5226000, "end": 5226200 },
    "min_quality": 30,
    "min_depth": 10
  }
  ```
- **JSON response**:
  ```json
  {
    "variants": [
      {
        "position": 5226178,
        "reference": "A",
        "alternate": "T",
        "genotype_likelihoods": [0.001, 0.998, 0.001],
        "phred_quality": 47,
        "depth": 23,
        "witness": "shake256..."
      }
    ]
  }
  ```
- **Audit row**: `code=RVDNA_VARIANTS_OK`. Refusals:
  `RVDNA_VARIANT_REFUSED_LOW_DEPTH`, `RVDNA_TENANT_SCOPE_REFUSED`.

### h.3 `rvdna_translate`

- **Capability**: `Read`.
- **JSON request**:
  ```json
  {
    "file_id": "blake3:...",
    "frame": 1,
    "region": { "chromosome": "chr11", "start": 5226000, "end": 5226400 }
  }
  ```
- **JSON response**:
  ```json
  {
    "protein_sequence": "MVHLTPEEKSAVTALWGKVN...",
    "contacts": [[5, 18, 0.84], [12, 27, 0.76]],
    "secondary_structure": "HHHHCCEEEEHHHCC...",
    "witness": "shake256..."
  }
  ```
- **Audit row**: `code=RVDNA_TRANSLATE_OK`. Refusal:
  `RVDNA_TRANSLATE_NO_ORF`.

### h.4 `rvdna_score`

- **Capability**: `Read`.
- **JSON request**:
  ```json
  {
    "file_id": "blake3:...",
    "score_id": "cpic:cyp2d6",
    "context": { "drug": "codeine" }
  }
  ```
- **JSON response**:
  ```json
  {
    "score": 1.5,
    "components": { "rs1135840": 0.5, "rs3892097": 1.0 },
    "category": "intermediate-metabolizer",
    "recommendation_text": "Use morphine instead; CYP2D6 IM phenotype.",
    "witness": "shake256..."
  }
  ```
- **Audit row**: `code=RVDNA_SCORE_OK`. Refusal:
  `RVDNA_SCORE_REFUSED_INSUFFICIENT_COVERAGE`.

### h.5 `rvdna_lineage`

- **Capability**: `Internal`.
- **JSON request**:
  ```json
  { "file_id": "blake3:..." }
  ```
- **JSON response**:
  ```json
  {
    "witness_chain": [
      { "stage": "encode", "lineage_id": "ol://jobs/...", "witness": "shake256..." },
      { "stage": "ipfs-pin", "cid": "bafy...", "witness": "shake256..." }
    ],
    "model_checkpoints": {
      "kmer": "fnv1a-d512:1.0",
      "protein": "esm2-650M:0.4.1"
    },
    "sections": [
      { "kind": "dna_2bit",     "checksum_blake3": "abc...", "size_bytes": 12345 },
      { "kind": "kmer_vectors", "checksum_blake3": "def...", "size_bytes": 67890 }
    ]
  }
  ```
- **Audit row**: `code=RVDNA_LINEAGE_OK`.

### h.6 The six-code refusal vocabulary

Mirrors ruLake's `WITNESS_MISMATCH_REFUSED` discipline
(`mcp-server/src/server.rs` audit codes). All `RVDNA_*` codes are
disjoint from `RULAKE_*` so a single audit pipeline can serve both
servers.

| Code | When it fires |
|---|---|
| `RVDNA_WITNESS_MISMATCH_REFUSED` | Caller's `asserted_witness` ≠ file's `rvf_witness`. Tool refuses to run. |
| `RVDNA_VARIANT_REFUSED_LOW_DEPTH` | Variant region has no calls meeting `min_depth`. |
| `RVDNA_TRANSLATE_NO_ORF` | Frame contains no open reading frame. |
| `RVDNA_SCORE_REFUSED_INSUFFICIENT_COVERAGE` | PRS / CPIC requires SNPs absent from §3. |
| `RVDNA_TENANT_SCOPE_REFUSED` | Clinical mode + JWT scopes don't match the file's tenant. |
| `RVDNA_T2_BUDGET_REFUSED` | Cold-tier query exceeds `MAX_T2_DECODE_BYTES_PER_QUERY`. |

---

## i. Cross-sample federation

The user's brief calls this the "real play": treating genomes like
queryable vector memory and enabling population-scale pattern
discovery without recompute. v2 implements it by leveraging the
existing `search_federated` (`src/lake.rs:521`) with rayon parallel
fan-out and adaptive per-shard rerank
(`src/lake.rs:533` `MIN_PER_SHARD_RERANK = 5`,
`src/lake.rs:584` `over_request_k`).

### i.1 The federation shape

```
                   ┌────────────────────────────────────┐
                   │  mcp-rvdna server                   │
                   │  (caller-facing, capability-gated)  │
                   └─────────────────┬──────────────────┘
                                     │
                     ┌───────────────┴────────────────┐
                     │   RuLake (lake.rs:521)         │
                     │   .search_federated(targets)   │
                     └──────────┬───────────┬────────┘
                                │           │
              ┌─────────────────┴─┐      ┌──┴────────────────┐
              │ RvdnaT0Backend    │ ...  │ RvdnaT0Backend    │
              │ (sample 1, gene X)│      │ (sample N, gene X)│
              └─────────┬─────────┘      └─────────┬─────────┘
                        │                          │
                ┌───────┴──────┐            ┌──────┴───────┐
                │ .rvdna v2    │            │ .rvdna v2    │
                │ file (mmap)  │   ......   │ file (IPFS)  │
                └──────────────┘            └──────────────┘
```

### i.2 Per-cohort backend topology

For a 10k-sample cohort:
- **One ruLake instance** per process (no inter-process IPC).
- **N `RvdnaT0Backend` registrations**, one per sample. The
  registration cost is the §1 k-mer block size (default 2 KB / window
  → ~50 KB per gene-of-interest per sample), so 10k × 50 KB = 500 MB
  in RAM at registration time.
- **A single MCP tool call** (`rvdna_find`) translates to a fan-out
  over `targets = [(s_001, "HBB"), (s_002, "HBB"), ...]`.
- **Adaptive rerank**: 10k shards × global rerank 20 / 10k = 0.002 →
  floored at `MIN_PER_SHARD_RERANK=5` per shard. Total rerank cost
  is 50k vector reranks; the merge step picks the global top-K.

### i.3 Witness as cross-trial provenance anchor

Two sites collaborating on a multi-trial study:
- Each site encodes its samples to v2.
- Each site publishes the bundle JSONs (only) to a shared store.
  No raw `.rvdna` payload crosses the boundary unless explicitly
  authorised.
- A federated query at the meta-site: the meta-site doesn't have
  the §0 raw DNA, it has only bundle witnesses. It can verify the
  federated answer is reproducible by running the same query against
  the same witnesses at any point in the future.

This is the IPFS path made meaningful: the `.rvdna` files live on
IPFS (one CID per file), each pinned by the producing site; the
bundle JSONs are tiny (a few hundred bytes), portable, and
witness-verifiable end-to-end.

### i.4 IPFS bundle distribution

Per `docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`:

- The IPFS backend's `current_bundle()` already overrides
  `BackendAdapter::current_bundle` to return a witness-stable bundle
  pointing at the kubo CID.
- A `.rvdna` v2 file pinned at CID `bafy...` is referenced by
  `data_ref: "ipfs://bafy.../HBB"`.
- The `mcp-rvdna` server's `rvdna_find` over an IPFS-pinned file
  goes: caller → `RuLake::search_one` → cache miss → `BackendAdapter::pull_vectors`
  → kubo `cat` → §1 bytes → RaBitQ-compress → cache.
- Subsequent queries against the same witness hit the cache. The
  cold-prime cost is paid once per (CID, query-collection) per
  process lifetime.

### i.5 Witness-mismatch handling under federation

The brief warns about model-rotation: an ESM-2 v0.4.1 file and a
v0.5.0 file have different witnesses. Federation refuses to mix them
silently. The `mcp-rvdna` server returns:

```
RVDNA_TRANSLATE_OK with hits=[...] but partial=true
  + refused_shards=[
      { sample: "s_042", code: "RVDNA_WITNESS_MODEL_MISMATCH",
        expected_model: "esm2-650M:0.4.1",
        actual_model: "esm2-650M:0.5.0" }
    ]
```

The caller decides whether to re-encode the v0.5.0 sample, drop it
from the fan-out, or mark the result as model-heterogeneous. v2
never lies about provenance.

---

## j. Streaming biomarkers

Answers the brief's question 2: should `.rvdna` files be static
artefacts or evolve into streaming biological state? Both. The
default is static; streaming is opt-in via header bit 12.

### j.1 The streaming model

A streaming `.rvdna` v2 file is append-only on §6 (biomarker time-
series). Wearables and serial blood draws push new biomarker rows;
the encoder appends to §6, recomputes BLAKE3 over §6 only (the prefix
hash is unchanged), updates the `sections_root`, recomputes the
witness, and rewrites the bundle pointer (96 bytes at offset
`0x00B0`).

Per epoch:
- `epoch_start_unix_ms` advances.
- `generation_value` (bundle pointer offset `0x40`) is the
  monotonic epoch counter.
- The witness rotates.

### j.2 ruLake's coherence model handles it

`src/cache.rs::Consistency` already has the three states this needs:

- `Fresh`: every search asks the backend for the current bundle. A
  streaming file's witness rotation is picked up immediately; the
  cache invalidates and re-primes from §6.
- `Eventual { ttl_ms }`: the cache trusts itself for `ttl_ms`. New
  biomarker rows arrive but aren't seen until the next refresh
  window. Right knob for streaming RAG over wearable data — the user
  doesn't need millisecond freshness on heart-rate variability.
- `Frozen`: the cache treats this `.rvdna` file as immutable for its
  lifetime. Right knob for clinical replay (you want the answer the
  doctor saw, not the answer it would give now).

### j.3 The three-state bundle protocol read

Reusing `mcp-server/src/server.rs:391`
`rulake_refresh_from_bundle_dir`'s three-state response:
- `up_to_date`: cache witness matches file witness; no work.
- `invalidated`: cache witness differs; cache flushed and re-primed.
- `bundle_missing`: `.rvdna` file removed or unreachable; cache holds
  the last good answer (under `Frozen`) or returns
  `RULAKE_DEGRADED` (under `Fresh`).

### j.4 Append discipline

- The §6 trailer block carries a back-pointer to the previous epoch's
  byte offset, so a reader can walk backwards in time without re-
  decoding.
- The bundle pointer's `generation_value` is the only mutable field
  on disk during streaming. Everything else is immutable per epoch.
- A streaming `.rvdna` MUST set the `WITNESS_FROZEN` flag to 0; an
  encoder that sets `STREAMING=1` AND `WITNESS_FROZEN=1` is rejected
  with `RVDNA_INVALID_PROFILE`.

---

## k. Privacy + clinical mode

Answers the brief's question 1.

### k.1 Two profiles

- **`--profile research`** (default): `pii_policy = "research-open"`.
  All MCP tools are accessible with `mcp:rvdna:read`. Federation
  across cohorts is unrestricted. `Consistency::Eventual` is allowed.
- **`--profile clinical`**: `pii_policy = "phi-strict"`. Per-tool
  capability gate adds `mcp:rvdna:clinical`. Federation refuses
  cross-tenant unless JWT scopes present matching tenant claims.
  `Consistency::Eventual` is rejected; only `Fresh` or `Frozen`.

The profile is stored in two places: header bits 13/14 (for fast
dispatch) and the bundle pointer's `pii_policy_class` byte (for
witness-bound persistence). Both are checked; mismatch is
`RVDNA_PROFILE_INCONSISTENT`.

### k.2 OpenLineage emission

Clinical mode forces `lineage_id` to be set on every emitted bundle
(otherwise encode fails with `RVDNA_CLINICAL_REQUIRES_LINEAGE`). The
sidecar metadata (§7) includes the lineage row's full JSON for audit
replay.

### k.3 JWT-scope-driven tenant federation gate

Per `mcp-server/src/auth.rs:294` `scopes_to_caps`, the JWT scope
list maps to a capability set. v2 extends this with tenant claims:

```
JWT claims (sketch — not invented APIs):
{
  "scope": "mcp:rvdna:read mcp:rvdna:clinical",
  "rvdna_tenant": ["acme-hospital", "trial-cohort-7"]
}
```

The `mcp-rvdna` server's clinical-mode federation handler intersects
the requested file set with the JWT's tenant list. Files outside the
tenant list refuse with `RVDNA_TENANT_SCOPE_REFUSED`. The check is on
the bundle's metadata (§7's `tenant_ids`), not on the file path —
moving a file across hosts does not bypass the gate.

### k.4 Audit-tail mandate

In clinical mode, the `mcp-rvdna` server requires `--audit-tail` to be
on. Every tool invocation emits a row to the JSONL audit log
(`mcp-server/src/audit.rs::AuditRow`). The shape is:

```json
{
  "ts": "2026-04-26T12:34:56.789Z",
  "actor": "service-acme-hospital",
  "tool": "rvdna_call_variants",
  "args_hash": "blake3:abc...",
  "outcome": "ok",
  "result_size": 7,
  "trust_level": "clinical",
  "duration_ms": 47,
  "witness_in": "shake256...",
  "witness_out": "shake256...",
  "code": "RVDNA_VARIANTS_OK",
  "policy_decision": {
    "capability_required": "read+clinical",
    "capability_granted": "read+clinical+admin",
    "tenant_match": "acme-hospital"
  }
}
```

Same row shape as `mcp-server/src/server.rs:300` (where
`RULAKE_INTERNAL` is set), so existing audit ingestion works.

### k.5 What clinical mode does NOT do

- Does not encrypt the file at rest. The format is unchanged; PHI
  protection is the operator's job (LUKS, dm-crypt, etc.).
- Does not assert FDA / CE clearance. It asserts *posture*: PHI flag,
  audit, tenant scope. Clinical claims live in a layer above v2.
- Does not block research-profile files from being read by clinical
  callers. A clinical caller can read a research file (perhaps
  validating a research finding); the audit row records the cross-
  profile access for review.

---

## l. Validation test

The brief specifies: load 100 `.rvdna` files, query variant similarity
across them, return results in <10 ms without recompute. v2 expands
this into five measurable acceptance gates, mirroring the harness
shape of `vendor/ruvector/examples/dna/benches/dna_bench.rs:1`.

### l.1 Gate G1 — Load latency

- **Setup**: 100 v2 `.rvdna` files, each containing a single 2 kb
  gene region (HBB) with full §0..§5 sections. Total disk: ~50 MB.
- **Workload**: Register all 100 as `RvdnaT0Backend` instances on
  one ruLake.
- **Target**: total wall-clock < 500 ms (5 ms / file).
- **Failure mode**: any single file taking > 50 ms is a failure.

### l.2 Gate G2 — Index build latency (cold prime)

- **Setup**: After G1, fire one warm-up query on each backend so
  ruLake primes its RaBitQ-compressed cache.
- **Target**: total wall-clock for 100 primes < 1.5 s (15 ms / prime
  is the v1 full-pipeline floor; v2 cache prime should be ≤ that).
- **Failure mode**: `cache_stats.avg_prime_ms > 25`.

### l.3 Gate G3 — Federated query latency (the brief's <10 ms ask)

- **Setup**: After G2 (warm cache).
- **Workload**: One `rvdna_find` MCP call, k=10, query against all
  100 files in parallel.
- **Target**: p50 < 10 ms; p99 < 30 ms.
- **Floor**: v1's `search_top10` benchmark (line 76 of dna_bench.rs)
  measured single-shard k-mer search; v2 fans out across 100 shards
  with adaptive rerank `5` per shard. Per-shard cost ≤ 100 µs warm,
  rayon parallelism + merge ≤ 5 ms.
- **Failure mode**: any single query exceeding 50 ms.

### l.4 Gate G4 — Witness verify latency

- **Setup**: 100 v2 `.rvdna` files, each ~500 KB.
- **Workload**: Run `rvdna_lineage` on each (it forces a witness
  recompute via §d.4).
- **Target**: total wall-clock < 5 s (50 ms / file is the BLAKE3
  throughput for a 500 KB file).
- **Failure mode**: any single verify exceeding 200 ms.

### l.5 Gate G5 — Audit emit latency

- **Setup**: After G3 (warm), under `--audit-tail` enabled.
- **Workload**: 1000 sequential `rvdna_find` calls, all on the same
  warm backend.
- **Target**: p99 audit emit overhead < 50 µs per call (matches
  `mcp-server/src/audit.rs`'s measured ~30 µs append overhead).
- **Failure mode**: audit emit dominating query latency (any audit
  emit > 200 µs).

### l.6 Test harness shape (mirror of v1's dna_bench)

```rust
// rvdna-backend/benches/v2_acceptance.rs (sketch).
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn gate_g3_federated_query(c: &mut Criterion) {
    let lake = setup_100_rvdna_files(); // G1 + G2 already paid
    let query = sample_kmer_query();
    let targets: Vec<(String, String)> =
        (0..100).map(|i| (format!("s_{:03}", i), "HBB".to_string())).collect();
    let target_refs: Vec<(&str, &str)> =
        targets.iter().map(|(b, c)| (b.as_str(), c.as_str())).collect();

    c.bench_function("g3_federated_find_100_shards", |b| {
        b.iter(|| {
            black_box(lake.search_federated(&target_refs, &query, 10).unwrap())
        });
    });
}

criterion_group!(benches, gate_g3_federated_query);
criterion_main!(benches);
```

If G3's bench reports p50 < 10 ms with p99 < 30 ms over 100 samples
across 100 shards, the brief's validation test passes and v2 is
shippable.

---

## m. Migration from v1

### m.1 What v1 emits today

A v1 `.rvdna` file
(`vendor/ruvector/examples/dna/src/rvdna.rs:769` `RvdnaWriter::write`)
has:
- 64-byte header, magic `RVDNA\x01\x00\x00`.
- 7-section table.
- Sections 0..6 (no biomarkers section).
- Footer with CRC32-derived global checksum.

### m.2 How v2 reads v1

A v2 reader detects v1 by the magic byte at offset `0x07` (`\x01`
vs `\x02`). On v1 detection, the reader:
1. Reads the v1 header verbatim.
2. Synthesises a v2 bundle pointer at runtime (NOT written back to
   the file) by:
   - Computing BLAKE3 over each v1 section.
   - Building a `sections_root`.
   - Setting `model_checkpoint_lo = 0` (v1 didn't track this; the
     witness is therefore derivable but the model identity is
     unrecorded).
   - Computing the witness using ruLake's `compute_witness`.
3. Returns a `RvdnaV2File` handle that exposes the same API as a
   native v2 file.

The synthesised witness is stable for a given v1 file but does NOT
match what a v2-encoded equivalent of the same input would produce.
The reader marks this in the file handle as `synthesised_witness:
true`, which surfaces in `rvdna_lineage` output.

### m.3 The `migrate` subcommand

```
rvdna v2 migrate <v1-file> [--output <v2-path>] [--re-embed] [--profile ...]
```

Two modes:

- **`--re-embed off`** (default): re-package v1 sections into v2
  envelope. The bundle pointer is the synthesised one (witness derived
  from existing v1 bytes; no model upgrade). Fast: < 100 ms / file.
- **`--re-embed on`**: re-run the v2 encode pipeline (§f) on the v1
  file's §0 raw DNA. The witness rotates because §1 (k-mer vectors)
  is recomputed against a current model. Slow: 12 ms / 5-gene file
  per v1 measurement, scaled.

### m.4 Backwards compatibility for v1 readers

A v1 reader pointed at a v2 file fails at the magic check
(`vendor/ruvector/examples/dna/src/rvdna.rs:212` rejects unknown
magic). v2 makes no attempt to be readable by v1 — the bundle pointer
between header and sections would confuse v1's section-offset arithmetic.
Operators with v1 dependencies should pin to v1 readers and migrate at
their own cadence.

### m.5 Migration acceptance

The `rvdna v2 migrate` subcommand has its own acceptance test:
- Take `vendor/ruvector/examples/dna/`'s shipped sample output
  (running `cargo run --release -p rvdna` produces a v1 `.rvdna`).
- `rvdna v2 migrate sample.rvdna --output sample.v2.rvdna`.
- Assert:
  - `RvdnaV2File::open(sample.v2.rvdna)` succeeds.
  - Every v1 section's content round-trips byte-identically.
  - `rvdna_find` against the migrated file returns the same hits as
    a v1 native query against the source file.

---

## n. Open questions

Honest gaps. None of these block v0.0 of `rvdna-backend/`; all should
be resolved before v1.0.

1. **Model-rotation policy**. When ESM-2 ships a new checkpoint,
   how do we batch-regenerate (operator pain, but keeps witnesses
   uniform across a cohort) vs lazy-rotate (cheap, but federation
   refuses mixed-cohort queries)? Default proposal: lazy with an
   opt-in batch tool (`rvdna v2 reembed --cohort <dir> --model esm2:0.5.0`),
   but the right answer probably depends on cohort size and clinical
   timeliness constraints.
2. **T1 / T2 boundary at population scale**. With 1M samples, even
   tier T0 won't fit in workstation RAM. The natural answer is a
   GCS-backed T0 (GCS Parquet of the §1 vectors), but that pushes
   "k-mer vectors live on cloud storage" — does the cache hit-rate
   target survive?
3. **Federation across cohorts with conflicting consent**. Sample A
   is consented for trial X but not Y; sample B is consented for both.
   A federated query that touches A's `.rvdna` for trial Y must refuse,
   but the JWT tenant claim is a coarse instrument. Probably needs a
   per-sample consent vector in §7 metadata, intersected per query.
4. **Browser-side decode budget**. The `node-wasm/` crate compresses
   to ~149 KB (per
   `docs/adrs/ADR-006-rulake-console-vite-github-pages.md` line 22).
   Adding `verifyRvdnaWitness` for v2 adds maybe ~50 KB more. Where's
   the ceiling before the browser budget hurts? The Console deep-review
   (`docs/research/console-deep-review.md`) has historical numbers
   but not for v2 yet.
5. **Streaming witness rotation under crash**. If a wearable pushes
   3 biomarker samples and the encoder crashes between BLAKE3 and
   witness commit, the on-disk file has §6 bytes that aren't covered
   by the §6 checksum. Recovery: the next reader detects mismatch and
   either (a) rolls back §6 to the last covered position, or (b)
   refuses to open. Default: refuse. Needs an explicit flush protocol.
6. **Multi-sample manifest mode interaction with witness**. If a
   `.rvdna` carries 100 samples in one file (manifest mode, §c.4),
   does the witness cover the whole manifest or per-sample? Default
   proposal: per-sample, and the file carries 100 bundle pointers.
   But where do they go physically? Probably right after the manifest,
   one 96-byte block per sample. Spec-it explicitly before v0.0.
7. **Browser CDS feedback loop**. The five MCP tools are designed for
   agents and headless callers. The Console wants a one-screen
   "explain this variant" view that combines `rvdna_call_variants` +
   `rvdna_translate` + `rvdna_score`. Does that compose at the MCP
   layer or does it need a v2 CLI subcommand `rvdna v2 explain
   <region>`? Probably both, but the priority isn't obvious.

---

## References

- v1 ADR-013 (this document supersedes):
  `vendor/ruvector/examples/dna/adr/ADR-013-rvdna-ai-native-format.md`.
- v1 vision: `vendor/ruvector/examples/dna/adr/ADR-001-vision-and-context.md`.
- v1 perf targets: `vendor/ruvector/examples/dna/adr/ADR-011-performance-targets-and-benchmarks.md`.
- v1 source: `vendor/ruvector/examples/dna/src/rvdna.rs`,
  `pipeline.rs`, `kmer.rs`, `variant.rs`, `protein.rs`,
  `epigenomics.rs`, `pharma.rs`, `health.rs`, `biomarker.rs`,
  `biomarker_stream.rs`.
- v1 benches: `vendor/ruvector/examples/dna/benches/dna_bench.rs`.
- v1 DDD: `vendor/ruvector/examples/dna/ddd/{architecture,domain-model,bounded-context-map}.md`.
- ruLake bundle: `src/bundle.rs::compute_witness`,
  `src/bundle.rs:113` `RuLakeBundle`.
- ruLake backend trait: `src/backend.rs:110` `BackendAdapter`.
- ruLake federation: `src/lake.rs:521` `search_federated`.
- ruLake cache modes: `src/cache.rs::Consistency`.
- ruLake MCP server: `mcp-server/src/server.rs:189` (tool router),
  `mcp-server/src/auth.rs:294` `scopes_to_caps`.
- ruLake IPFS backend: `docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`.
- ruLake Console: `docs/adrs/ADR-006-rulake-console-vite-github-pages.md`.
- ruLake MCP server ADR: `docs/adrs/sdk/ADR-004-rulake-mcp-server.md`.
- ruLake substrate framing: `docs/adrs/ADR-156-rulake-as-memory-substrate.md`.
