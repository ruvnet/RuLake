# ADR-007: rvDNA v2 as a ruLake-native substrate

## Status

**Accepted — Scaffolded (2026-04-27)** — `crates/rvdna-backend/` v0.0.1 is
landed (commit `08261ae`); `crates/mcp-rvdna/` v0.0.1 scaffold lands in the
following commit. v0.0 ships the hot-tier (T0) BackendAdapter only —
RAM-resident k-mer vectors with witness derivation byte-isomorphic to
`RuLakeBundle::new` (memory_class = `genomic`). 6 tests pass; bench
shows pull_vectors at 35.9 GiB/s and a 555× cache cold→hot ratio
(commit `ad251e5`). Security review (commit `beed210`) surfaced 6
findings (0 High, 1 Med, ...): R-2 mitigated, R-3 doc fixed, R-1
(witness/content decoupling) accepted for v0.0 with mitigation
deferred to the `mcp-rvdna` companion-server layer where untrusted
callers can inject collections.

T1 (warm mmap protein embeddings + attention) lands in v0.1; T2
(cold lazy raw DNA + epigenomic series) lands in v0.2 per §6 of
this ADR.

**Originally proposed (2026-04-27)** — drafted on
`research/management-ui` as the third file of the rvDNA v2 corpus,
after `v2-spec.md` and `integration-with-rulake.md`.

## Date

2026-04-27

## Authors

ruv.io · ruLake architecture, drafted against the user's brief for
"rvDNA v2 — the next iteration of the genomic intelligence file
format and pipeline". The brief is reproduced verbatim in the
preface to `docs/research/rvdna/README.md`; this ADR is the contract
that binds the brief to ruLake's existing primitives.

## Relates to

- [v2-spec.md](./v2-spec.md) — canonical rvDNA v2 file-format and
  behaviour spec. The format defined there is what ADR-007 ratifies.
- [integration-with-rulake.md](./integration-with-rulake.md) — the
  code-level companion sketching `crates/rvdna-backend/`, `crates/mcp-rvdna/`,
  and the Console hooks. The shapes proposed there are what ADR-007
  commits to building.
- [ADR-013 (v1) — `vendor/ruvector/examples/dna/adr/ADR-013-rvdna-ai-native-format.md`](../../../vendor/ruvector/examples/dna/adr/ADR-013-rvdna-ai-native-format.md)
  — the v1 file-format ADR that v2 supersedes. This ADR formally
  marks v1 ADR-013 as superseded for new files; v1 ADR-013 remains
  the spec of record for files in the wild emitted before v2 ships.
- [ADR-001 (v1) — `vendor/ruvector/examples/dna/adr/ADR-001-vision-and-context.md`](../../../vendor/ruvector/examples/dna/adr/ADR-001-vision-and-context.md)
  — v1's vision; v2 inherits the 100-year framing, the
  proven-foundation discipline, and the regulated-vs-research split.
- [`docs/adrs/ADR-155-rulake-datalake-layer.md`](../../adrs/ADR-155-rulake-datalake-layer.md)
  — ruLake's cache-first, witness-as-anchor framing that v2 must
  honour.
- [`docs/adrs/ADR-156-rulake-as-memory-substrate.md`](../../adrs/ADR-156-rulake-as-memory-substrate.md)
  — the substrate framing that gives `memory_class: "genomic"` (v2
  bundle pointer offset 0x49) its meaning.
- [`docs/adrs/sdk/ADR-004-rulake-mcp-server.md`](../../adrs/sdk/ADR-004-rulake-mcp-server.md)
  — the MCP server design `crates/mcp-rvdna/` mirrors. The `#[tool]` macro
  pattern, capability tiers, audit row shape, and JWKS-backed JWT
  flow all transfer.
- [`docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`](../../adrs/sdk/ADR-005-ipfs-backend-and-deploy.md)
  — the IPFS bundle-distribution path that v2's cross-site federation
  reuses unchanged.
- [`docs/adrs/ADR-006-rulake-console-vite-github-pages.md`](../../adrs/ADR-006-rulake-console-vite-github-pages.md)
  — the Console architecture that the Genomic 7th sidebar entry
  extends.

---

## Context

### What's true today

ruLake is shipping. As of iteration 16 of the `/loop` on
`research/management-ui`:

- `rulake@2.2.0` is published on crates.io.
- `mcp-server` is at v0.10 with eight tools (`rulake_query`,
  `rulake_list_backends`, `rulake_list_collections`,
  `rulake_publish_bundle`, `rulake_refresh_from_bundle_dir`,
  `rulake_save_cache_to_dir`, `rulake_warm_from_dir`,
  `rulake_invalidate_cache`) and four resources
  (`rulake://stats`, `rulake://stats/by-backend`,
  `rulake://bundle/{b}/{c}`, `rulake://audit/tail`). 68 passing tests
  + 1 ignored.
- The Console at `ui/` ships in tri-mode (Demo / WASM-local / Live)
  via Vite + React, deployed through GitHub Pages CI.
- IPFS bundle distribution exists at `crates/ipfs-backend/`; GCS at
  `crates/gcs-backend/`. Both implement `BackendAdapter`
  (`crates/core/src/backend.rs:110`).
- The bundle witness is SHAKE-256(32), length-prefixed and domain-
  separated, audit-hardened against the `Num(7)` vs
  `Opaque("\x07\0…")` collision (`crates/core/src/bundle.rs::compute_witness`).

In parallel, rvDNA v1 lives at `vendor/ruvector/examples/dna/`. It is
a complete prototype: 18 source files (~9.5 kloc), 15 ADRs, 172
passing tests, and a measured 12 ms end-to-end pipeline on five real
human genes
(`vendor/ruvector/examples/dna/README.md` line 164). The v1 file
format is defined by ADR-013
(`vendor/ruvector/examples/dna/adr/ADR-013-rvdna-ai-native-format.md`),
which specifies a 7-section binary with magic `RVDNA\x01\x00\x00`,
2-bit DNA encoding, CRC32 per-section checksums, and an HNSW-ready
k-mer vector index.

### What's missing

v1 ships as a self-contained crate. It does not federate. Two `.rvdna`
files on two hosts are independent; querying across them requires
loading both into one process and orchestrating the combine by hand.
v1's witness (`vendor/ruvector/examples/dna/src/rvdna.rs:127`
`header_checksum: u32`) is a CRC32 over the header only — it doesn't
cover the section payloads, doesn't carry the model identity, and
doesn't match the SHAKE-256 shape ruLake's cache uses to share
entries across deployments.

The user's brief argues this is the "perfect compositional fit": v1
already produces precomputed intelligence; ruLake is built to be the
access layer for exactly that kind of artefact. The argument
survives close reading, but only if rvDNA v2 carries a witness that
ruLake recognises and a backend wrapper that matches the
`BackendAdapter` trait shape.

### What this ADR commits to

Treating v2 as a first-class ruLake substrate. Concretely: the v2
spec lands as `docs/research/rvdna/v2-spec.md`; `crates/rvdna-backend/` and
`crates/mcp-rvdna/` are scaffolded as sibling crates following the
`crates/gcs-backend/` / `crates/ipfs-backend/` pattern; the Console grows a 7th
sidebar entry. None of this requires changing `rulake@2.2.0`'s public
API — every integration point already exists.

---

## Decision

We will:

### D1. Adopt v2-spec.md as the canonical rvDNA file format

The format defined in
[v2-spec.md](./v2-spec.md) — 8 sections (DNA, k-mer, attention,
variant, protein, epigenomic, biomarker, metadata), magic
`RVDNA\x02\x00\x00`, header + section table + 96-byte bundle pointer,
BLAKE3 per-section checksums, SHAKE-256(32) bundle witness — becomes
the spec of record for new `.rvdna` files. v1 ADR-013 is marked
superseded for new emissions; v1 readers continue to work on v1 files
in the wild.

Rationale: every modification from v1 (bundle pointer, BLAKE3
payload checksums + SHAKE-256 witness, biomarker section, profile
flags) is justified by a specific integration need with ruLake or by a
specific gap in v1 (model identity not bound to the witness).
v2-spec.md §a.3 itemises the diff with citations.

### D2. Map `.rvdna` v2 sections onto a three-tier `BackendAdapter` model

The three tiers from v2-spec §e:

- **Tier 0 (T0) — hot**: §1 k-mer vectors, in-RAM via
  `RvdnaT0Backend`. `Consistency::Fresh` per cache mode
  (`crates/core/src/cache.rs::Consistency`). Per-file cap of 512 MiB on top of
  ruLake's existing `MAX_PULLED_BYTES = 16 GiB`
  (`crates/core/src/backend.rs:62`).
- **Tier 1 (T1) — warm**: §2/§3/§4/§6 mmap'd via `RvdnaT1Backend`.
  `Consistency::Eventual { ttl_ms: 5_000 }`.
- **Tier 2 (T2) — cold**: §0/§5 lazy via `RvdnaT2Backend<Inner>`.
  `Consistency::Frozen`. The `Inner` is whichever existing
  `BackendAdapter` provides the cold-tier bytes (Local, GCS, IPFS).

Each tier is a separate `BackendAdapter` impl. The trait at
`crates/core/src/backend.rs:110` is the only contract surface — the four required
methods (`id`, `list_collections`, `pull_vectors`, `generation`) plus
the `current_bundle` override that makes the witness chain visible to
ruLake's cache (`crates/core/src/cache.rs` "the cross-backend share").

The full `RvdnaT0Backend` sketch is in
`integration-with-rulake.md` §1.3.

### D3. Make every v2 file's bundle pointer compatible with ruLake's `RuLakeBundle`

The 96-byte bundle pointer at offset `0x00B0` of every v2 file is
byte-isomorphic to a `RuLakeBundle` (`crates/core/src/bundle.rs:113`) serialised
in binary form. The witness at the start of the pointer is the
output of `compute_witness` (`crates/core/src/bundle.rs::compute_witness`) over
the same input shape ruLake uses elsewhere — same domain-separation
prefix, same length-prefixing, same `Generation` variant tag byte
that closed the 2026-04-23 security audit
(`crates/core/src/bundle.rs::Generation::hash_bytes`).

The Generation::Opaque payload v2 uses (`v2-spec.md` §d.1) packs:

```
model_checkpoint_lo (4 B)  ||  sections_blake3_root (32 B)  ||  profile_flags (2 B)
```

so changing any of the model checkpoint, the section payload, or the
profile rotates the witness deterministically.

The result: a `.rvdna` v2 file, the bundle JSON it emits at encode
time, and a `RuLakeBundle` synthesised at runtime by
`RvdnaT0Backend::current_bundle()` all carry the same
`rvf_witness`. Cache sharing across deployments — the
`crates/core/src/cache.rs` "cross-backend share" path — works for free.

### D4. Expose the genomic surface through a sibling MCP server, not as new tools on `mcp-server`

`crates/mcp-rvdna/` is a separate crate, mirroring `crates/mcp-server/`'s shape
(`crates/mcp-server/src/server.rs:189`). Five tools:

- `rvdna_find` — k-mer similarity, witness-pinned (Read tier)
- `rvdna_call_variants` — variant calls in a region (Read + Clinical
  when PHI flag set)
- `rvdna_translate` — DNA → protein + contacts + secondary structure
  (Read tier)
- `rvdna_score` — polygenic risk + pharmacogenomic dosing (Read tier)
- `rvdna_lineage` — witness chain + model checkpoints + section
  digests (Internal tier)

Capability gate by JWT scopes mirroring the `scopes_to_caps` pattern
from `crates/mcp-server/src/auth.rs:294`. New scopes:
- `mcp:rvdna:read`
- `mcp:rvdna:clinical`
- `mcp:rvdna:internal`
- `mcp:rvdna:admin`

Refusal vocabulary (six codes; mirrors ruLake's
`WITNESS_MISMATCH_REFUSED` discipline): `RVDNA_WITNESS_MISMATCH_REFUSED`,
`RVDNA_VARIANT_REFUSED_LOW_DEPTH`, `RVDNA_TRANSLATE_NO_ORF`,
`RVDNA_SCORE_REFUSED_INSUFFICIENT_COVERAGE`,
`RVDNA_TENANT_SCOPE_REFUSED`, `RVDNA_T2_BUDGET_REFUSED`. All
prefixed with `RVDNA_` so the audit pipeline can distinguish them
from `RULAKE_*` codes.

Why a separate server, not new tools on `mcp-server`: the genomic
surface has a different audience (clinical / research scientists +
agents), a different capability tier (Clinical), and a different
refusal vocabulary. Bundling them on `mcp-server` would dilute its
identity (substrate access) and make `tools/list` filtering harder
to reason about.

The full sketch is in `integration-with-rulake.md` §2.2.

### D5. Federate via `lake.search_federated`, no new federation primitive

Cross-sample queries use the existing
`RuLake::search_federated` (`crates/core/src/lake.rs:521`) with rayon parallel
fan-out and adaptive per-shard rerank
(`crates/core/src/lake.rs:533` `MIN_PER_SHARD_RERANK = 5`,
`crates/core/src/lake.rs:584` `over_request_k`). v2 doesn't invent a new
federation API — the genomic case is just N `RvdnaT0Backend`
registrations and one `search_federated(targets, query, k)` call.

For 10k-sample cohorts: `targets = [(s_001, "HBB"), (s_002, "HBB"),
...]`, fan-out across 10k shards, per-shard rerank floored at 5,
merge-sort by score, return top-K. The existing infrastructure
handles every step.

Cross-site federation reuses the IPFS path from
`docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`: each site
publishes bundle JSONs (only) to a shared store; the meta-site
verifies witnesses end-to-end without ever touching raw `.rvdna`
payloads. This is the architecture the user's brief described as the
"genomic inference fabric".

### D6. Two profiles — clinical and research — selected at encode time

Per v2-spec §k:

- `--profile research` (default): `pii_policy = "research-open"`. All
  MCP tools accessible with `mcp:rvdna:read`. Federation
  unrestricted. `Consistency::Eventual` allowed.
- `--profile clinical`: `pii_policy = "phi-strict"`. Per-tool
  capability gate adds `mcp:rvdna:clinical`. Federation refuses
  cross-tenant unless JWT scopes match. `Consistency::Eventual`
  rejected; only `Fresh` or `Frozen`. OpenLineage `lineage_id`
  mandatory. Audit-tail mandatory.

The profile is bound to the bundle witness via `profile_flags` (v2
bundle pointer offset 0x4A). A clinical-profile file cannot be
re-served as research-profile without rotating the witness.

### D7. Migrate v1 files via `rvdna v2 migrate` — re-package by default, re-embed on flag

Two migration modes (v2-spec §m.3):

- `rvdna v2 migrate <v1-file>` (default): re-package v1 sections into
  the v2 envelope. The bundle pointer is synthesised; the witness is
  derived from existing v1 bytes. Fast (< 100 ms / file), cheap, no
  model upgrade.
- `rvdna v2 migrate <v1-file> --re-embed`: re-run the v2 encode
  pipeline on §0 raw DNA. The witness rotates because §1 is
  recomputed against a current model. Slow (12 ms / 5-gene file
  scaled), gives uniform cohort identity.

Operators choose. v1 readers continue to work on v1 files; v2
readers can read v1 files via the synthesised-witness path
(v2-spec §m.2) but mark the witness as `synthesised: true` in
`rvdna_lineage` output.

---

## Consequences

### Positive

1. **Cross-sample query becomes a `lake.search_federated` call.**
   No new federation code, no new MCP tool, no new cache layer.
   The "load 100 files, query similarity, <10 ms" validation test
   from the brief reduces to one method invocation against existing
   infrastructure.
2. **Existing IPFS / GCS backends carry `.rvdna` bundles for free.**
   `crates/ipfs-backend/` and `crates/gcs-backend/` already implement
   `BackendAdapter`; `RvdnaT2Backend<Inner>` wraps them as the cold
   tier. No new networking, no new storage layer.
3. **The Console's Bundle screen verifies any `.rvdna` witness with
   the same WASM code today's bundles use.** `node-wasm/`'s
   `verifyBundleJson` becomes the basis for `verifyRvdnaWitness`;
   the cryptographic verify path is shared. The Genomic screen is a
   UI shell, not a re-implementation of crypto.
4. **The audit pipeline serves both servers.** `mcp-rvdna` emits
   `AuditRow` with the exact shape of `crates/mcp-server/src/audit.rs`; the
   `RVDNA_*` code prefix makes the rows distinguishable, but a
   single ingestion pipeline handles both. Operators don't run two
   audit stacks.
5. **The witness chain answers "model-bound" honestly.** v1's gotcha
   ("`.rvdna` is model-bound — changing embeddings requires
   regeneration") becomes a witness-rotation event in v2. A file
   built against ESM-2 v0.4.1 cannot be confused with one built
   against v0.5.0 even if every other field matches; the witness
   diverges and federation refuses to mix them silently.
6. **Clinical and research are configurations, not forks.** v2's
   profile selection is one bundle field plus one byte in the
   header. The same encode binary handles both. The same MCP server
   handles both. The capability gate handles the discrimination.

### Negative

1. **v1 users must re-encode (or accept the synthesised-witness
   migration path) to participate in federation.** The synthesised
   witness is stable but doesn't bind a model identity, so federated
   queries that mix synthesised and native v2 witnesses will surface
   `RVDNA_WITNESS_MODEL_MISMATCH` warnings (v2-spec §i.5). Operators
   who care about uniform cohort identity will run
   `rvdna v2 migrate --re-embed` across their corpora.
2. **Tier 0 RAM pressure at population scale is real.** Even with
   per-file caps and the 2 KB k-mer block default, 10k samples × 50
   KB per gene-of-interest sums to 500 MB at registration time. 1M
   samples is 50 GB, which doesn't fit on a workstation. v2-spec §n
   open question 2 anticipates this; the v0.3 answer is likely a
   GCS-backed T0 (k-mer vectors as Parquet), but that pushes "k-mers
   live on cloud storage" — a question for benchmarking, not
   speculation.
3. **Clinical-mode JWT discipline is non-trivial to wire.** The
   tenant-scope intersection (v2-spec §k.3) requires every JWT
   issuer to emit `rvdna_tenant` claims that match the bundle's
   §7 metadata `tenant_ids`. This is operator burden; misconfigured
   tokens will refuse with `RVDNA_TENANT_SCOPE_REFUSED` until the
   issuer is fixed. The remediation is a clear error message and a
   `rvdna v2 inspect <file>` subcommand that reports the file's
   tenant_ids; both are scoped into v0.2 of `mcp-rvdna`.
4. **Two MCP servers means two audit log streams.** Operators who
   want one log stream concatenate them; this is straightforward but
   needs to be documented in the `crates/mcp-rvdna/` README.
5. **The biomarker section (§6) is new code.** v1 keeps biomarker
   data in memory only (`vendor/ruvector/examples/dna/src/biomarker_stream.rs:1`);
   v2 serialises it. The streaming-mode flag (v2-spec §j) adds
   append-discipline edge cases (witness rotation under crash —
   v2-spec §n open question 5) that need a flush protocol.

### Neutral

1. **rvDNA stays a separate crate.** v2 does not absorb v1's
   `vendor/ruvector/examples/dna/` source into `rulake/`. That source
   continues to live in `vendor/`, continues to ship as the `rvdna`
   crate, continues to be the reference for biological correctness
   (Bayesian variant calling, Horvath clock, CYP2D6 calling). v2
   builds the *envelope* and the *integration*; v1 owns the
   *biology*.
2. **The Console grows by one sidebar entry, not by a fork.** Genomic
   sits next to Browse, Bundle, Cache, Audit, Witness, Help. It uses
   the same routing, the same sidebar, the same WASM crypto, the
   same MCP transport. Operators who don't care about genomics see a
   single extra nav item; operators who do see a first-class screen.
3. **The BLAKE3 vs SHAKE-256 split is documented and lives.** v2
   carries both because each serves a different role (BLAKE3 for
   payload throughput, SHAKE-256 for witness parity with ruLake). The
   alternative — picking one and losing either throughput or parity
   — was rejected. The split is documented in v2-spec §c.6.

---

## Verification

Five measurable acceptance gates. The brief's "load 100 files, query
variant similarity across them, return results in <10 ms without
recompute" is gate G3 below; the other four cover the prerequisites
and the operational tax.

### G1. Load latency

100 v2 `.rvdna` files, each ~500 KB containing one 2 kb gene region
with §0..§5 sections. Register all 100 as `RvdnaT0Backend` instances
on one ruLake.

- **Target**: total wall-clock < 500 ms (5 ms / file, including
  witness verification).
- **Failure mode**: any single file > 50 ms.
- **Test location**: `crates/rvdna-backend/tests/t0_register.rs`.

### G2. Cold-prime latency

After G1, fire one warm-up query on each backend. ruLake primes its
RaBitQ-compressed cache.

- **Target**: total wall-clock for 100 primes < 1.5 s; mean prime
  < 15 ms (the v1 full-pipeline floor).
- **Failure mode**: `cache_stats.avg_prime_ms > 25`.
- **Test location**: `crates/rvdna-backend/tests/t0_query.rs::cold_prime`.

### G3. Federated query latency (the brief's <10 ms ask)

After G2, one `rvdna_find` MCP call, k=10, query against all 100
files in parallel via `RuLake::search_federated`.

- **Target**: p50 < 10 ms, p99 < 30 ms.
- **Failure mode**: any single query > 50 ms.
- **Test location**: `crates/rvdna-backend/benches/v2_acceptance.rs::g3_federated_find_100_shards`.

### G4. Witness-verify latency

100 v2 files, ~500 KB each. Run `rvdna_lineage` on each (forces a
witness recompute via v2-spec §d.4).

- **Target**: total wall-clock < 5 s; mean verify < 50 ms.
- **Failure mode**: any single verify > 200 ms.
- **Test location**: `crates/rvdna-backend/tests/witness_parity.rs::verify_100`.

### G5. Audit emit overhead

Under `--audit-tail`, 1000 sequential `rvdna_find` calls on the same
warm backend. Measure per-call audit emit overhead.

- **Target**: p99 audit emit < 50 µs / call.
- **Failure mode**: any emit > 200 µs.
- **Test location**: `crates/mcp-rvdna/tests/audit_overhead.rs`.

If all five gates pass, the brief's "perfect compositional fit" claim
is verified empirically and v2 is shippable.

---

## Options considered

Real alternatives, considered and rejected, with the reason for
rejection so future readers don't relitigate.

### Option A — Extend `rulake/` to absorb v1 and ship a unified `rulake-genomic` feature

Take the v1 source at `vendor/ruvector/examples/dna/`, fold it into
the `rulake` crate behind a `genomic` feature flag, expose the v1
pipeline as a new ruLake API surface.

**Rejected** because:

1. v1 is biology + algorithms (Bayesian variant calling, Horvath
   clock, CYP2D6 calling). ruLake is substrate (cache + bundle +
   federation). Mixing them dilutes both: ruLake users get a
   1000-line `genomic` feature they can't turn off cleanly; v1 users
   get a substrate dependency they didn't ask for.
2. v1 ships with its own published versioning (`rvdna` on crates.io
   per `vendor/ruvector/examples/dna/README.md` line 12). Absorbing
   it into ruLake would either fork the version trajectory or break
   downstream consumers.
3. The `BackendAdapter` trait is the right composition primitive.
   Sibling crates use it (`crates/gcs-backend/`, `crates/ipfs-backend/`); making
   v2 a sibling continues the precedent.

### Option B — Treat `.rvdna` as opaque bytes, ship a single `rulake_genomic` MCP tool

Don't define a tier model, don't implement multiple `BackendAdapter`s.
Just expose one `rulake_genomic` tool that takes a path to a `.rvdna`
file and runs the v1 pipeline against it.

**Rejected** because:

1. Defeats the witness chain. The brief's whole point — the
   compositional fit — depends on `.rvdna` files participating in
   ruLake's cache as first-class citizens. Treating them as opaque
   bytes means re-encoding on every query, the exact failure mode v2
   was designed to fix.
2. Doesn't federate. A single tool can't take advantage of
   `lake.search_federated` (`crates/core/src/lake.rs:521`); you'd hand-roll the
   fan-out for every multi-sample query.
3. Loses the cross-deployment cache share. Two ruLake instances
   reading the same `.rvdna` file would each run the v1 pipeline
   independently; the same SHAKE-256 witness would never get
   computed because there'd be no `BackendAdapter` to compute it.

### Option C — Ship v2 only as a new file format; defer all integration work

Land v2-spec.md as a format-only ADR. Don't scaffold `crates/rvdna-backend/`
or `crates/mcp-rvdna/`. Let downstream consumers wire it up themselves.

**Rejected** because:

1. Without a reference `BackendAdapter` impl, there's no way to test
   that v2's bundle pointer actually works with ruLake's witness
   chain. The five verification gates in §Verification require the
   integration crates to exist.
2. The brief explicitly asks for the integration architecture, not
   just the file format. "Turn his long-form thinking into shippable
   spec + integration architecture with ruLake" is the exact phrase
   in the brief.
3. v1 already proved the format can stand alone. v2's contribution
   is the binding to ruLake; shipping the format without the binding
   is shipping nothing v1 didn't already ship.

### Option D — Keep CRC32 from v1, add a separate witness sidecar

Don't modify v1's per-section CRC32 checksums; just add a sidecar
file `<name>.witness.json` next to every `.rvdna` containing the
SHAKE-256 witness.

**Rejected** because:

1. Two-file artefacts violate v1's "single file, no sidecar" promise
   (`vendor/ruvector/examples/dna/adr/ADR-013-rvdna-ai-native-format.md`
   "Single file (no sidecar)" comparison row). Operators would have
   to ship two files for every sample, doubling distribution
   complexity.
2. The witness covers section payloads in v2; CRC32 only covered
   the header. Keeping CRC32 means the witness can't anchor
   end-to-end content, defeating its cross-deployment utility.
3. BLAKE3 is faster than CRC32C at scale (the genome-scale case
   v1 anticipated for whole-WGS files). The migration cost is one
   format-version bump that v2 needs anyway.

### Option E — Single profile (clinical-only), defer research mode

Ship only the `--profile clinical` mode in v2.0; research mode comes
later or never.

**Rejected** because:

1. v1's documented audience is overwhelmingly research / discovery
   (`vendor/ruvector/examples/dna/adr/ADR-001-vision-and-context.md`
   "100-Year Vision"). Forcing every user through clinical mode
   imposes JWT discipline and audit-tail mandates on workloads
   that don't need them.
2. The two profiles share 99% of the implementation. The cost of
   shipping both is one bundle-pointer byte (`pii_policy_class`)
   and one capability-tier check. The benefit is the entire research
   user base continues to function.

---

## Risks and mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| v1 corpus too large to migrate cheaply (operators with TB of v1 files) | Medium | Medium | `rvdna v2 migrate` defaults to fast re-package mode (no model upgrade, < 100 ms / file). Witness is synthesised; works for query, marked as `synthesised: true` in `rvdna_lineage` so callers know. |
| Tier 0 RAM pressure at population scale (1M+ samples) | High | High | Per-file 512 MiB cap + `MAX_T0_FILES_OPEN` per process. v0.3 introduces GCS-backed T0 (k-mer vectors as Parquet) for cohort-of-cohorts cases. |
| ESM-2 / NT / Hyena ship breaking checkpoint changes | High | Medium | Witness rotates deterministically on model change (v2-spec §c.4 `model_checkpoint_lo`). Federation refuses to mix witnesses cleanly with `RVDNA_WITNESS_MODEL_MISMATCH`. Operators batch-re-embed at their cadence. |
| Clinical-mode tenant scope misconfiguration | Medium | High (in clinical contexts) | Explicit `RVDNA_TENANT_SCOPE_REFUSED` with clear error message naming the missing claim. `rvdna v2 inspect <file>` subcommand reports the file's `tenant_ids` so operators can verify their JWT issuer config. |
| Two MCP servers (`mcp-server` + `mcp-rvdna`) increase deployment complexity | Medium | Low | Both servers can be run as a single process (separate ports, shared `Arc<RuLake>`); `crates/mcp-rvdna/main.rs` documents the co-process pattern. Audit logs concatenate into one stream. |
| Streaming-mode crash between BLAKE3 and witness commit corrupts §6 | Medium | Medium | Default behaviour: refuse to open files where §6's checksum doesn't match the witness. v0.3 introduces an explicit flush-protocol (write WAL → fsync → commit pointer) per v2-spec §n open question 5. |
| BLAKE3 vs SHAKE-256 split confuses readers | Low | Low | Documented in v2-spec §c.6 with the rationale (throughput vs parity); ADR-007 §Decision D3 reiterates the choice. The Console's Verify view labels each hash by purpose. |
| Browser-side decode of large `.rvdna` files (multi-GB) hits WASM memory limit | Medium | Medium | Genomic Verify view streams sections — never loads the full file into JS heap. Bundle pointer (96 B) + per-section hashes are all the verification needs; raw §0 bytes are streamed only on explicit user request. |
| v1 ADR-013 readers in the wild attempt to read v2 files | Low | Low | v2 magic differs by one byte (`\x02` vs `\x01`); v1 reader rejects on magic check (`vendor/ruvector/examples/dna/src/rvdna.rs:212`). Clean refusal, no silent corruption. |

---

## Compatibility and supersession matrix

How v2 interacts with existing artefacts in this repo and in the
v1 vendored submodule.

| Artefact | What v2 does | Why |
|---|---|---|
| v1 `.rvdna` files in the wild | Readable via v2's synthesised-witness path; migratable via `rvdna v2 migrate`. | Backward compatibility for existing v1 corpora. |
| v1 ADR-013 (`vendor/ruvector/examples/dna/adr/ADR-013-rvdna-ai-native-format.md`) | Marked superseded for new file emissions. v1 ADR-013 remains the spec of record for the v1 reader path. | Cleanly delineates which ADR governs which file format. |
| v1 source crate (`vendor/ruvector/examples/dna/`) | Untouched. Continues to build, test, and ship as `rvdna` on crates.io. | v1 owns the biology; v2 owns the envelope. No source merge. |
| ruLake `BackendAdapter` trait (`crates/core/src/backend.rs:110`) | Unchanged. v2 implements it three times (T0/T1/T2). | The trait is already the right shape; no API break. |
| `RuLakeBundle` (`crates/core/src/bundle.rs:113`) | Unchanged. v2's bundle pointer is byte-isomorphic to a serialised `RuLakeBundle`. | Forces witness parity; no new bundle type. |
| `compute_witness` (`crates/core/src/bundle.rs::compute_witness`) | Unchanged. v2 calls it (or computes the equivalent) with the v2-defined `Generation::Opaque` payload. | Cross-deployment witness sharing is the entire point. |
| `crates/mcp-server/` 8 tools | Unchanged. They continue to expose the substrate surface. | Genomic surface lives in `crates/mcp-rvdna/`, not here. |
| `crates/mcp-server/src/audit.rs` `AuditRow` shape | Unchanged. `mcp-rvdna` emits the same shape with `RVDNA_*` codes. | One audit pipeline serves both servers. |
| `crates/mcp-server/src/auth.rs` `scopes_to_caps` (line 294) | Unchanged. `mcp-rvdna` ships its own equivalent for the `mcp:rvdna:*` scope namespace. | Each server owns its scope-to-capability mapping. |
| `crates/gcs-backend/` and `crates/ipfs-backend/` | Unchanged. `RvdnaT2Backend<Inner>` wraps them as the cold tier. | Composition, not modification. |
| Console (`ui/`) sidebar (`ui/src/components/screens.jsx:17`) | One new entry (`Genomic`) added; existing entries untouched. | First-class genomic screen (§Decision D4 rationale extended in `integration-with-rulake.md` §3.1). |
| `node-wasm/` exports (`verifyBundleJson`, etc.) | Unchanged. v2 adds `verifyRvdnaWitness` as a sibling export. | Reuses the SHAKE-256 path; no new browser crypto. |
| `docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md` | Unchanged. v2 cross-site federation reuses the IPFS path verbatim. | The IPFS path was designed for content-addressed bundles; `.rvdna` is content-addressed. Free fit. |
| `docs/adrs/ADR-156-rulake-as-memory-substrate.md` (memory-class framing) | Unchanged. v2 always sets `memory_class: "genomic"` on emitted bundles. | One known consumer; the framing already accommodated it. |
| `docs/adrs/ADR-157-optional-accelerator-plane.md` | Unchanged. v2's tiered indexing model is a natural client of any future accelerator (the T0 HNSW could dispatch to a GPU/SIMD kernel via the `VectorKernel` trait when ADR-157 lands). | Forward-compatible by accident; the brief didn't ask but the shape is right. |

---

## Implementation sequencing

Two PRs after this ADR is accepted:

### PR 1 — `crates/rvdna-backend/` v0.0

- Crate scaffold per `integration-with-rulake.md` §1.1.
- `RvdnaT0Backend` only (T1/T2 deferred).
- `RvdnaV2File::open_and_verify` with full witness check.
- One round-trip test: open a v1 `.rvdna` from
  `vendor/ruvector/examples/dna/` (after one-time encoding via
  `cargo run --release -p rvdna`), `migrate` it to v2 in-memory,
  query it, compare result to a v1 native query against the source
  file.
- Acceptance gates G1 + G3 implemented as benches.
- Published as `rvdna-backend = "0.0.1"` to crates.io once green.

### PR 2 — `crates/mcp-rvdna/` v0.0

- Crate scaffold per `integration-with-rulake.md` §2.1.
- `rvdna_find` and `rvdna_lineage` only (other 3 tools deferred).
- stdio transport + bearer auth.
- Audit-tail JSONL emit.
- Tests: `tools_smoke.rs`, `clinical_refusal.rs` (the latter
  exercises the JWT tenant-scope refusal path).
- Acceptance gate G5 implemented.

After PR 2 passes CI, the Console's `Genomic` sidebar entry is the
third PR (`ui/src/components/screens.jsx` diff per
`integration-with-rulake.md` §3.3 + the `verifyRvdnaWitness` WASM
export).

---

## Decision summary

This ADR ratifies that:

- v2-spec.md is the canonical rvDNA file format.
- v2 sections map onto ruLake's `BackendAdapter` via three tiers
  (T0 / T1 / T2).
- Every v2 file's bundle pointer is byte-isomorphic to a
  `RuLakeBundle`; the witness is computed by the same SHAKE-256
  recipe ruLake uses.
- Five MCP tools (`rvdna_find`, `rvdna_call_variants`,
  `rvdna_translate`, `rvdna_score`, `rvdna_lineage`) are exposed
  by a sibling `crates/mcp-rvdna/` crate.
- Federation uses `lake.search_federated`; no new primitive.
- Two profiles (clinical / research) are bundle-bound configurations,
  not forks.
- v1 files migrate via `rvdna v2 migrate` — fast re-package by
  default, slow re-embed on flag.

The brief's claim that "rvDNA = precomputed intelligence,
ruLake = access optimization, ruvector = reasoning loop" survives
the close-reading discipline of this ADR. The compositional fit is
real, but only when v2 carries a witness ruLake recognises and a
backend wrapper that matches the trait it expects. v2 supplies both.
