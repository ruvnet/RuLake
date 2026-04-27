# Changelog

All notable changes to **ruLake** are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html);
each substrate / companion-server crate ships and versions independently
per [ADR-001](docs/adrs/ADR-001-standalone-repo-strategy.md).

This file covers the **substrate / companion-server expansion** that
landed across late April 2026 (iters 18–29 of the `/loop` working
session). Earlier history is in the git log (the `rulake` core was
published to crates.io as `2.2.0` and `rulake-wasm` to npm as `2.2.1`
prior to this batch).

---

## [Unreleased] — 2026-04-26 → 2026-04-27

The headline shift: ruLake gained two **substrate adapters** beyond the
storage-tier adapters (`gcs-backend`, `ipfs-backend`) — one genomic
(`rvdna-backend`), one quantum (`ruqu-backend`) — plus an MCP companion
server for each. Every shipped ADR also got a 2,500–3,700-word deep gist
in `docs/gists/`.

Late in the batch (iters 32–34) cross-component testing surfaced two
real bugs at the Console ↔ rvdna-mcp boundary that no in-isolation test
would have caught: missing CORS on the new MCP server, and an SSE-frame
selection bug in the Console's parser. Both fixed; both now permanently
locked under smoke scripts that drive the full wire end-to-end.

### Added — substrate adapters

- **`rvdna-backend/` v0.0.1** ([ADR-007](docs/adrs/ADR-007-rvdna-as-rulake-substrate.md))
  — hot-tier (T0) BackendAdapter for `.rvdna` v2 files. RAM-resident
  k-mer vectors with witness derivation byte-isomorphic to
  `RuLakeBundle::new` (memory_class = `genomic`). 6 tests pass; T1/T2
  land in v0.1/v0.2.
- **`ruqu-backend/` v0.0.1** ([ADR-008](docs/adrs/ADR-008-ruqu-as-rulake-substrate.md))
  — StateVector quantum simulator BackendAdapter (≤16 qubits, mini-IR:
  H/X/Y/Z/S/T/Rz/CX). Witness derivation byte-isomorphic
  (memory_class = `quantum`). 9 tests pass; Stabilizer / TensorNetwork
  in v0.1, Hardware + QEC scheduler in v0.2.

### Added — companion MCP servers

- **`mcp-rvdna/` v0.0.1** — five tools (`rvdna_find`,
  `rvdna_call_variants`, `rvdna_translate`, `rvdna_score`,
  `rvdna_lineage`); 17 tests pass. `rvdna_lineage` is the live
  trust-anchor demo with `RVDNA_WITNESS_DRIFT` refusal when the pinned
  witness disagrees with the live re-derivation. End-to-end HTTP smoke
  at `mcp-rvdna/scripts/http-smoke.sh` (build → launch → MCP handshake
  → tools/list).
- **`mcp-ruqu/` v0.0.1** — five tools (`ruqu_simulate`, `ruqu_verify`,
  `ruqu_replay`, `ruqu_optimize`, `ruqu_qec_schedule`); 13 tests pass.
  Library-only in v0.0.1 by design (HTTP transport lands in v0.1).
  R-1 / R-2 / R-3 mitigations enforced at handler entry (empty/overlong
  `circuit.id` → `RUQU_INVALID_CIRCUIT_ID`; qubit cap 16 →
  `RUQU_QUBIT_CAP_EXCEEDED`).

### Added — benchmarks

- Criterion harness for **5 crates**: `rvdna-backend`, `ruqu-backend`,
  `gcs-backend`, `ipfs-backend`, `mcp-server`. Results captured in
  [`docs/research/benchmarks/v0.0-substrates.md`](docs/research/benchmarks/v0.0-substrates.md)
  and [`docs/research/security/shipping-substrates-v2.md`](docs/research/security/shipping-substrates-v2.md).
  Headline numbers:
  - `rvdna-backend::pull_vectors` (n=1024, dim=384): **35.9 GiB/s**
  - cache cold→hot through `RuLake::search_one`: **555×** speedup
  - `ruqu-backend::simulate` (14 qubits, 4 layers): **2.15 Gelem/s**
  - `gcs-backend::pull_vectors` (1k × 64): 184 µs / 1.29 GiB/s
  - `ipfs-backend::verify_witness`: 692 ns (dim-invariant)
  - `mcp-server::audit::emit` at 256-cap steady state: 1.27 µs

### Added — security reviews

- Two focused review documents:
  [`docs/research/security/v0.0-substrates.md`](docs/research/security/v0.0-substrates.md)
  (rvdna + ruqu) and
  [`docs/research/security/shipping-substrates-v2.md`](docs/research/security/shipping-substrates-v2.md)
  (gcs + ipfs + mcp-server).
- 6 findings on rvdna/ruqu (0 High, 1 Med, 5 Low/Info); 4 findings on
  shipping substrates (0 High, 2 Med). Detailed dispositions below
  under **Fixed** and **Documented**.

### Added — deep gists

- 10 narrative companions in [`docs/gists/`](docs/gists/), one per
  shipped ADR — 2,500–3,700 words each, ~31,000 words total. Files:
  `standalone-repo-deep.md`, `python-sdk-deep.md`, `node-sdk-deep.md`,
  `mcp-server-deep.md`, `ipfs-backend-deep.md`, `console-deep.md`,
  `rvdna-v2-deep.md`, `ruqu-v2-deep.md`, `datalake-layer-deep.md`,
  `memory-substrate-deep.md`. Each cites the ADR + reference impl
  with `file:line` pointers.

### Added — Console (App Store + 7-route smoke)

- App Store route added to the Console
  ([`ui/src/components/screens.jsx:2018+`](ui/src/components/screens.jsx))
  — substrate marketplace listing the 4 shipping/scaffolded crates
  with status tags (`SHIPPING` / `SCAFFOLDED`), install commands
  (Rust + MCP companion + npm + wasm), ADR / gist / research links.
- New `tag-amber` CSS rule (`ui/src/styles/styles.css`) for the
  `SCAFFOLDED` status, distinct from `SHIPPING` (verifier green) and
  `PROPOSED` (cyan).
- E2E smoke ([`ui/scripts/smoke.sh`](ui/scripts/smoke.sh)) extended
  from 6 → 7 routes with App Store coverage; full run is green
  (5 audit codes, 0 console errors).

### Fixed — security

- **R-IPFS-1 (Med)** — `IpfsBackend::fetch_bundle` was emitting a
  `tracing::warn!` on `data_ref ≠ ipfs://{cid}` mismatch but returning
  the bundle anyway. CID-substitution surface — a legitimately-witnessed
  bundle re-pinned under a different CID would still be trusted. Fixed
  to **hard refuse** with code `IPFS_BUNDLE_CID_MISMATCH`
  (`ipfs-backend/src/backend.rs:283-289`, commit `56b497b`).
- **R-MCP-1 (Med)** — Audit shape was asymmetric across read vs
  mutation tools. `rulake_query` emitted a fully-shaped `AuditEntry`
  with `PolicyDecision` on every outcome, but the 5 mutation handlers
  (`rulake_publish_bundle`, `rulake_refresh_from_bundle_dir`,
  `rulake_save_cache_to_dir`, `rulake_warm_from_dir`,
  `rulake_invalidate_cache`) emitted nothing. Added a private
  `audit_mutation()` helper on the server struct that derives outcome +
  code from the `Result<T, McpError>` and emits a fully-shaped entry —
  every mutation now leaves an audit trail symmetric with reads
  (`mcp-server/src/server.rs`, commit `56b497b`).
- **R-2 (Med)** — `ruqu-backend::state_vector::simulate` had no qubit
  bound; `Circuit::dim()` saturates at 31, allowing a 32 GiB
  allocation. Mitigated by new `ruqu-backend/src/limits.rs` with
  `MAX_QUBITS_V0_0 = 16`, `MAX_QUBITS_HARD = 30`, and
  `enforce_qubit_cap()` (commit `beed210`). `mcp-ruqu` calls this
  before `execute()` and refuses with `RUQU_QUBIT_CAP_EXCEEDED`.
- **R-3 (Low)** — `Gate::Rz { theta }` doc-comment claimed witness
  binding it doesn't have. Replaced with a v0.0 caveat that points at
  the v0.0.2 plan to hash gate parameters into a content-derived
  `data_ref` (commit `126f993`).

### Documented — security findings accepted for v0.0

- **R-1 (Med, both substrates)** — Witness is decoupled from collection
  *content*; bound only to inputs (data_ref, dim, seed, etc.). Cache
  poisoning surface once `mcp-rvdna` / `mcp-ruqu` expose mutation
  tools. Mitigated *partially* in mcp-rvdna v0.0.1 by exposing **zero
  mutation tools** (registry pre-populated by the operator at process
  start; the trust-anchor `rvdna_lineage` refuses on drift). Full
  mitigation deferred to v0.0.2 when content-hash-derived `data_ref`
  lands.
- **R-4 (Low, both)** — `RwLock` poison panics flip the whole backend
  into a denied state. Accepted as fail-fast; no fix.
- **R-5 (Info, both)** — `f64` `sin`/`cos` cross-platform ULP drift is
  latent; harmless today because the witness doesn't hash `theta`.
  Flagged so the R-3 fix preserves the witness contract.

### Changed — ADR statuses reconciled with reality

| ADR | Was | Now |
|---|---|---|
| 002 (Python SDK) | Proposed (no source yet) | **Accepted v2.2.0** |
| 003 (Node SDK)   | Proposed (no source yet) | **Accepted v2.2.0** |
| 005 (ipfs-backend) | Proposed (no crate yet) | **Accepted v0.1 (R-IPFS-1 closed)** |
| 007 (rvdna substrate) | Proposed | **Accepted — Scaffolded v0.0.1** |
| 008 (ruqu substrate)  | Proposed | **Accepted — Scaffolded v0.0.1** |
| 155 (datalake layer)  | Accepted M1 | **Accepted M3** (gcs/ipfs/rvdna/ruqu + 3 MCP servers) |
| 156 (memory substrate)| Proposed (positioning) | **Accepted (positioning ratified)** |

ADRs **157** (Optional Accelerator Plane / VectorKernel) and **158**
(Rotation kind / QVCache positioning) genuinely remain **Proposed** —
both explicitly note "no code changes accompany this ADR" as part of
their decision; the design contract IS the deliverable.

### Changed — README

- New **Substrate adapters** section (`README.md`) inventorying
  ipfs-backend / rvdna-backend / ruqu-backend with status, ADR links,
  and headline bench numbers. Plus a second table covering mcp-rvdna
  + mcp-ruqu.
- New **Console** section pointing at `ui/`, the 7-route App Store, and
  the `agent-browser` smoke contract.

### Fixed — cross-component bugs (iter 32)

Surfaced by the first end-to-end test driving the Console at :4173
against rvdna-mcp at :17441:

- **mcp-rvdna missing CORS layer** — Console got `Failed to fetch`
  on the preflight `OPTIONS /mcp`. Fixed by porting `mcp-server`'s
  CORS pattern (`mcp-server/src/http.rs:287+`) into
  `mcp-rvdna/src/http.rs`: echo requesting Origin, expose
  `mcp-session-id` + `mcp-protocol-version`, short-circuit OPTIONS
  with 204. Tightening to an allow-list deferred to v0.0.2.
- **Console SSE parser grabbed the keepalive `data:\n` line first** —
  rmcp emits an empty `data:\n` keepalive frame BEFORE the JSON-bearing
  one, and the old `find(startsWith)` was happy to pick the empty
  string and JSON.parse it (silently caught, falling back to
  `toolsCount = 0`). Every successful response read as "0 tools"
  regardless of actual count. Fixed in `ui/src/components/screens.jsx`
  by filtering empty `data:` payloads and JSON.parsing each candidate
  until one yields a `result` field.

### Added — smoke scripts (iters 29, 31, 33)

Three scripts that lock the cross-component flows in CI:

- **`mcp-rvdna/scripts/http-smoke.sh`** — builds rvdna-mcp, launches
  on `127.0.0.1:17441`, walks the MCP handshake (initialize →
  notifications/initialized), asserts all 5 expected tool names land
  in `tools/list`, then calls `rvdna_lineage` with an unknown
  (backend, collection) pair and asserts the `RVDNA_UNKNOWN_COLLECTION`
  refusal carries through the SSE response.
- **`ui/scripts/smoke-cross-mcp.sh`** — orchestrates vite preview
  (port 4173) + rvdna-mcp (port 17441) + agent-browser, drives the
  Console's Connect screen at the rvdna-mcp URL, asserts the banner
  reads `initialize OK · Nms · 5 tools` and the `INIT_OK` audit row
  lands in IndexedDB. The script that proves iter 32's two fixes
  hold under automation.
- **`ui/scripts/smoke.sh --live`** extended (iter 34) to assert the
  Console reads exactly **8 tools** from the live mcp-server (the
  full read+publish+admin capability set). The first run of this
  assertion correctly caught a configuration gap — mcp-server was
  being launched without `--capabilities` flags so it defaulted to
  read-only (3 tools); fix updates the smoke launch line so
  tools/list exposes all 8 #[tool] handlers.

### Documentation

- **`CHANGELOG.md`** — this file (iter 30, updated through iter 45).
- **`README.md`** Substrate adapters / Console sections (iter 27).
- **`README.md`** End-to-end smoke contracts table (iter 37) — three
  scripts with what each covers and asserts.
- **`examples/README.md`** Substrate adapters section (iter 45) —
  points readers at the in-tree e2e tests, benches, and smokes that
  double as worked examples for the iter 18-22 substrate work.

### Added — orchestration (iter 38, iter 40)

- **`scripts/smoke-all.sh`** — single-command runner that walks the
  three smoke scripts in sequence with isolated subprocesses + a
  unified pass/fail summary. Auto-detects the `mcp-server` binary
  and runs `smoke.sh --live` when it's built; falls back to
  WASM-local-only otherwise. ~50 s wall time end-to-end on a warm
  build.
- **`ui/scripts/smoke-cross-mcp.sh`** Browse-refusal section
  (iter 36) — asserts the Console handles unknown-tool MCP errors
  cleanly (`LIST_COLLECTIONS_FAILED · refused` audit row + 0 console
  errors) when `Browse.refresh` fires `rulake_list_collections`
  against an MCP server that doesn't expose that tool.
- **`mcp-rvdna/scripts/http-smoke.sh`** CORS preflight section
  (iter 41) — direct curl `OPTIONS` request asserts 5 CORS response
  headers, fast (50 ms) and chrome-free regression check for the
  iter 32 fix.
- **`ui/scripts/smoke.sh`** `--live` tool-count assertion (iter 34) —
  Console must report exactly 8 tools from `mcp-server` with the
  full `read,publish,admin` capability set; surfaced and fixed a
  default-capability gap in the smoke launch line.

### Added — CI (iter 39)

- `.github/workflows/ci.yml` Rust matrix expanded **5 → 10 crates**:
  added `mcp-rvdna`, `mcp-ruqu`, `ipfs-backend`, `rvdna-backend`,
  `ruqu-backend`. Every standalone package per ADR-001 now builds +
  tests on every PR, fail-fast off so one regression doesn't mask
  another.
- New `smoke` job (depends-on `rust`) sets up Node 22 + Chrome via
  `browser-actions/setup-chrome`, builds `mcp-server` + `mcp-rvdna`
  binaries, then runs `scripts/smoke-all.sh`. The iter 32 bug-class
  (CORS, SSE-parser) and the iter 34 default-capability gap are
  both regression-locked at the workflow level.

### Fixed — iter 43-44 housekeeping

- **`mcp-server`** — cleared 5 lingering compiler warnings (unused
  imports `Capability` + `principal_for_client_cert`, dead-code
  field `Inner::last_reset`, dead-init `let mut transport = None`,
  unused-variable `let server = make_server();`). `cargo test
  --release` on mcp-server now emits zero warnings across lib + bin
  + 4 integration test binaries (40 + 8 + 20 tests pass).
- **`ruqu-backend`** — Gate enum's struct-variant fields (`q`,
  `theta`, `control`, `target`) all carry doc comments now;
  `#![warn(missing_docs)]` no longer fires. mcp-ruqu (path-dep on
  ruqu-backend) inherits the clean state.

All 10 Rust crates now build with **zero compiler warnings** on
`cargo build --release`.

---

## Reference

- Upstream: `rulake` v2.2.0 on crates.io
  ([crates.io/crates/rulake](https://crates.io/crates/rulake))
- Upstream: `rulake-wasm` v2.2.1 on npm
  ([npmjs.com/package/rulake-wasm](https://www.npmjs.com/package/rulake-wasm))
- Repo: [github.com/ruvnet/RuLake](https://github.com/ruvnet/RuLake)
- ADR family: [`docs/adrs/`](docs/adrs/)
- Deep gists: [`docs/gists/`](docs/gists/)
- Bench results: [`docs/research/benchmarks/`](docs/research/benchmarks/)
- Security reviews: [`docs/research/security/`](docs/research/security/)
