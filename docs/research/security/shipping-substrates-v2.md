# Shipping substrates v2 — bench harness + focused security review

Status: **draft, /loop pass 1**
Branch: `research/management-ui`
Date: 2026-04-26
Scope: `crates/gcs-backend/` (ADR-155), `crates/ipfs-backend/` (ADR-005), `crates/mcp-server/` (ADR-004 v0.8)

This document is the bench-harness + focused-security counterpart to the
new-substrate (rvdna, ruqu) reviews being added in parallel. It covers the
three already-shipping substrates so the /loop directive
("benchmarked, security reviewed, optimized") applies to them too.

Method:
- Each crate gets ONE focused criterion bench targeting the load-bearing
  in-process op. Network and live-service paths are excluded — the goal is
  to lock down a per-substrate baseline that subsequent /loop passes can
  detect regression against, not to measure GCS or kubo.
- Security review focuses on five buckets: input validation at the
  boundary, panic-on-poison / lock starvation, witness-equality
  assumptions, JWT scope leakage (mcp-server only), and resource bounds.
- No substrate `src/` files were modified by this pass. Findings that
  would require code changes are recorded as Recommendations, not
  applied.

Hardware: `ruvultra` (the host the operator commits from). Numbers below
are wall-clock from criterion v0.5 with `--warm-up-time 1
--measurement-time 3 --sample-size 20-30`. They are baselines for
regression tracking, not absolute claims.

---

## 1. `crates/gcs-backend/` — ADR-155 §M2 (Parquet on GCS)

### 1.1 Bench results

Bench file: `crates/gcs-backend/benches/pull_vectors.rs`
Run: `cargo bench --manifest-path crates/gcs-backend/Cargo.toml`

| Bench                                         | n × dim       | median        | throughput        |
| --------------------------------------------- | ------------- | ------------- | ----------------- |
| `gcs_pull_vectors_in_memory`                  | 100  × 16     | 18.74 µs      | 325.7 MiB/s       |
| `gcs_pull_vectors_in_memory`                  | 1000 × 64     | 184.78 µs     | 1.29 GiB/s        |
| `gcs_pull_vectors_in_memory`                  | 1000 × 384    | 742.01 µs     | 1.93 GiB/s        |
| `gcs_current_bundle_cheap_path` (dim known)   | —             | 963 ns        | n/a               |

Notes:
- Throughput is computed against the *vector-payload* footprint
  (`n * dim * 4 B`) not the on-wire Parquet size; the parquet-decode
  + arrow→`PulledBatch` conversion clearly scales sub-linearly with row
  count (the per-row cost flattens as the batch amortises footer/decoder
  setup).
- The cheap-path `current_bundle()` at <1 µs confirms the ADR-004
  §Resources contract holds: the override does NOT fall through to
  `pull_vectors`. A regression here (e.g. a future refactor that
  accidentally drops the operator-supplied `dim` short-circuit) would
  show up as a 4-6 order-of-magnitude jump.
- The GCS network round-trip is **not measured** — `object_store::memory::InMemory`
  is the substrate. A subsequent /loop pass with `RULAKE_GCS_LIVE_TEST=1`
  can layer on real-bucket numbers.

### 1.2 Security review

**Input validation at boundary (`pull_vectors`)**
- `dim > MAX_PULLED_DIM` is rejected (backend.rs:203). Good — bounds
  the per-row Vec allocation.
- `DimensionMismatch` is raised when row widths disagree
  (backend.rs:398). Good — defends against a corrupted Parquet file
  that mixes row widths within a list-typed column.
- The decoder downcasts to `Float32Array` / `ListArray` /
  `FixedSizeList` and returns `RuLakeError::Backend` (not panic) on
  type mismatch. Good — a Parquet file with a deliberately mistyped
  `vector` column (e.g. `LIST<DOUBLE>`) lands in the error path.
- **No `n_rows` cap**. A 100 GB Parquet object would be read fully into
  RAM. ADR-155 §M2 puts the cache layer above us — but the backend
  itself has no per-pull row ceiling. See Recommendation R-GCS-1.

**Panic-on-poison / lock starvation**
- `inner.collections` and `inner.schema_cache` use `RwLock` with
  `.unwrap()` (backend.rs:111, 122, 133, 139). A poisoned lock will
  panic. Reasoning is sound (any panic *while holding* these locks
  reflects a corrupted invariant; recovering would silently expose
  partial state) — but document it.
- The blocking `runtime.block_on(...)` calls hold neither lock, so
  there's no risk of an async-blocking-sync inversion across them.

**Witness-equality assumptions**
- The bundle here lives in core-lib (`RuLakeBundle::new` →
  SHAKE-256(32)). The GCS backend just feeds inputs (`data_ref`,
  `dim`, `seed`, `rerank_factor`, `Generation::Num(generation)`).
- `generation_of(meta)` falls back to `last_modified.timestamp() as
  u64` when `meta.version` is absent or unparsable (backend.rs:332-341).
  In test (`InMemory`) this gives second-resolution generations; in
  prod GCS always supplies a numeric `version`. **The fallback is
  audit-silent** — a misconfigured bucket that strips the version
  header would silently degrade cache coherence to second-granularity.
  See Recommendation R-GCS-2.

**Findings**

| Sev | Title                                                                | Status                |
| --- | -------------------------------------------------------------------- | --------------------- |
| Lo  | `pull_vectors` has no row-count cap                                  | Open (R-GCS-1)        |
| Lo  | Generation fallback to `last_modified` is silent on misconfig        | Open (R-GCS-2)        |
| Inf | RwLock unwrap is intentional and documented — accept                  | Resolved by review    |

**Recommendations**

- **R-GCS-1**: Add a `max_rows` ceiling (operator-configurable, default
  e.g. 50M) to `pull_vectors`, surfaced as
  `RuLakeError::InvalidParameter` once the in-progress decode crosses
  it. Cuts the worst-case-OOM blast radius from "size of the largest
  bucket object" to a known constant.
- **R-GCS-2**: Emit `tracing::warn!` once per (bucket, object) when
  `meta.version` is absent and we fall through to `last_modified`.
  Operators auditing for cache-coherence regressions today have no
  signal that the fallback fired.

---

## 2. `crates/ipfs-backend/` — ADR-005 (witness-anchored bundle distribution)

### 2.1 Bench results

Bench file: `crates/ipfs-backend/benches/verify_bundle.rs`
Run: `cargo bench --manifest-path crates/ipfs-backend/Cargo.toml`

| Bench                                              | dim    | median        | throughput  |
| -------------------------------------------------- | ------ | ------------- | ----------- |
| `ipfs_verify_witness`                              | 8      | 693.94 ns     | n/a         |
| `ipfs_verify_witness`                              | 128    | 726.87 ns     | n/a         |
| `ipfs_verify_witness`                              | 1024   | 691.81 ns     | n/a         |
| `ipfs_bundle_json_roundtrip`                       | 8      | 1.094 µs      | 297.2 MiB/s |
| `ipfs_bundle_json_roundtrip`                       | 128    | 1.218 µs      | 268.5 MiB/s |
| `ipfs_bundle_json_roundtrip`                       | 1024   | 1.082 µs      | 303.1 MiB/s |
| `ipfs_current_bundle_cheap_path` (operator CID+dim)| —      | 841.7 ns      | n/a         |

Notes:
- Witness recomputation is `dim`-invariant (~700 ns) as expected — the
  SHAKE-256 input is a fixed-shape canonical encoding of bundle fields,
  not the vector body.
- The full JSON round-trip (serialize → bytes → from_json with size
  caps → verify) sustains ~270-300 MiB/s. With the `from_json` 64 KiB
  cap that's a hard ceiling of ~250k bundle parses/sec per core.
- Cheap-path `current_bundle()` at ~840 ns confirms ADR-005 §4: when
  the operator pre-sets CID + dim, no kubo/gateway HTTP call is made.
  Live IPFS round trips are excluded by design (a mock-kubo bench would
  measure hyper, not the substrate).

### 2.2 Security review

**Input validation at boundary**
- `validate_cid_shape` (backend.rs:511) rejects empty / whitespace /
  control-byte CIDs at register time. Good first line.
- The actual CID multibase + multihash validation is delegated to
  kubo / the gateway, by design ("no `cid` crate; canonical validator
  is on the wire"). This is **defensible** because every kubo/gateway
  response surfaces an error for malformed CIDs — an attacker who
  smuggles a junk CID through `register` would still get a
  `RuLakeError::Backend` on the first `cat`.
- `from_json` enforces 64 KiB hard cap and 4 KiB per-field cap on the
  way back from `cat` (`bundle.rs:218`, `bundle.rs:243+`). Good.
  This caps the tail-risk of a maliciously-pinned bundle that would
  otherwise force unbounded allocation in serde.
- The fetched bundle's `data_ref` is compared to `format!("ipfs://{cid}")`
  but a mismatch only emits a `tracing::warn!` (backend.rs:283-289) —
  it does NOT refuse the bundle. The witness still has to verify per
  `RuLakeBundle::from_json`'s closing path, so the bundle isn't
  *silently* trusted, but operators reading the audit log might miss
  the warning. See Recommendation R-IPFS-1.

**Panic-on-poison / lock starvation**
- `inner.collections` `RwLock` is `.unwrap()`d throughout — same
  Rationale as gcs-backend. Acceptable.
- `inner.runtime.block_on(...)` runs `cat` / `add` / `pin/add`
  futures. The reqwest client has no operator-configurable timeout
  (it gets the default). A misbehaving kubo (slow-response or
  half-open TCP) could pin a calling thread. See R-IPFS-2.

**Witness-equality assumptions**
- The witness check is `expected == self.rvf_witness` (`bundle.rs:199`),
  i.e. byte-equality on the SHAKE-256(32) hex string. That's correct
  given the witness is canonical hex — there's no leading-zero
  ambiguity, no case-insensitivity gap (the encoder is fixed-case).
- `Generation::Opaque(cid)` is the IPFS-side generation — string
  compare is fine because CIDs are content-addressed.
- The `data_ref ≠ ipfs://{cid}` warn-only path is the ONE place where
  the witness still verifies but the *meaning* of the bundle (which
  CID it claims to be) drifts from where it was actually fetched.
  Witness equality alone doesn't close that gap.

**Findings**

| Sev | Title                                                                  | Status              |
| --- | ---------------------------------------------------------------------- | ------------------- |
| Med | `data_ref` mismatch is warn-only, not refuse                           | Open (R-IPFS-1)     |
| Lo  | reqwest client has no operator-tunable timeout for cat / add / pin     | Open (R-IPFS-2)     |
| Inf | CID validation deferred to kubo — defensible, document explicitly      | Resolved by review  |

**Recommendations**

- **R-IPFS-1**: Promote the `data_ref ≠ ipfs://{cid}` mismatch from
  `tracing::warn!` to a hard `RuLakeError::Backend`. The witness check
  alone doesn't catch a CID-substitution attack where the attacker
  re-pinned a *legitimately-witnessed* bundle under a different CID.
  This is a small change (1-2 lines, `backend.rs:283-289`) and closes
  the only remaining trust gap in the read path.
- **R-IPFS-2**: Surface a `connect_timeout` + `request_timeout` on the
  reqwest client builder (`backend.rs:114`). Default 5 s connect /
  30 s request is reasonable; operators can override via env or config.

---

## 3. `crates/mcp-server/` — ADR-004 v0.8 (Streamable HTTP + JWT scopes → CapabilitySet)

### 3.1 Bench results

Bench files: `crates/mcp-server/benches/audit_sink.rs`,
`crates/mcp-server/benches/tools_list_filter.rs`
Run: `cargo bench --manifest-path crates/mcp-server/Cargo.toml`

| Bench                                       | size            | median        |
| ------------------------------------------- | --------------- | ------------- |
| `mcp_audit_emit_below_cap/push_only`        | 100 emits/iter  | 103.4 µs      |
| `mcp_audit_emit_at_cap/push_pop`            | 1 emit/iter     | 1.27 µs       |
| `mcp_audit_tail_snapshot`                   | n=16            | 5.80 µs       |
| `mcp_audit_tail_snapshot`                   | n=64            | 36.86 µs      |
| `mcp_audit_tail_snapshot`                   | n=256 (cap)     | 137.0 µs      |
| `mcp_tools_list_filter`                     | 1 cap (read)    | 399.8 ns      |
| `mcp_tools_list_filter`                     | 2 caps          | 435.2 ns      |
| `mcp_tools_list_filter`                     | 3 caps (admin)  | 570.2 ns      |
| `mcp_capset_from_csv`                       | 1 token         | 27.06 ns      |
| `mcp_capset_from_csv`                       | 8 tokens        | 99.10 ns      |
| `mcp_capset_from_csv`                       | 64 tokens       | 669.3 ns      |

Notes:
- **At-capacity audit emit**: ~1.27 µs steady-state (push_back +
  pop_front + serde_json::to_value of an entry). At 786k emits/sec/core
  the audit sink is comfortably faster than the rest of the request
  path; the 256-entry ring is the right scale.
- **Below-capacity emit at 100 entries/iter** ≈ ~1.0 µs per emit.
  Push-only is slightly faster than push-pop, as expected.
- **Tail snapshot at n=256** (the `rulake://audit/tail` worst case)
  is 137 µs — under the 200 ms TTL the resource targets, with 1000×
  headroom. Safe.
- **Tools-list filter** is 400-570 ns across realistic operator caps.
  This is the per-`tools/list` call cost; well under any reasonable
  request-budget threshold.
- **CapabilitySet::from_csv** scales linearly with token count
  (~10 ns/token). The 64-token case is contrived (ADR-004 maxes at 4
  real labels) but worth tracking — the JWT path runs from_csv per
  token verify, so a misbehaving IdP that emits dozens of `mcp:rulake:*`
  scopes won't cliff the auth path.

### 3.2 Security review

**Input validation at boundary**
- `BearerAuth::verify` uses constant-time compare (`subtle::ConstantTimeEq`,
  auth.rs:80). Good — defends against the timing-attack class on the
  bearer plaintext.
- `JwtAuth::verify` validates signature, `iss`, `aud` (RFC 8707
  Resource Indicators), and `exp` via `jsonwebtoken`. Uses the
  configured algorithm whitelist; rejects unknown / `none`. Good.
- JWT scope parsing accepts both `scope` (space-separated string) and
  `scp` (array). `scopes_to_caps` (auth.rs:294) maps three known
  labels — `mcp:rulake:{read,publish,admin}` — and silently drops
  anything else. **Unknown scopes are not audit-logged.** A bug in
  the IdP that emits `mcp:rulake:Admin` (capital A) would silently
  reduce the token's grant. See R-MCP-2.
- The audit `principal` for bearer mode uses `DefaultHasher` (auth.rs:100)
  — the comment correctly notes this is an attribution channel, not a
  security boundary, and constant-time compare gates access. Accept.

**Panic-on-poison / lock starvation**
- `AuditSink` deliberately recovers from poisoned mutexes via
  `into_inner()` (audit.rs:81-84, 130-134). Reasoning: never crash the
  request path on an audit-write failure. This is the right call —
  the alternative would be to swallow panics with `catch_unwind`
  which is uglier.
- The `policy::REQUEST_CAPS` task-local is read with `try_with`
  (policy.rs:33) and falls through to server-wide caps when unset.
  No lock involved.
- `JwtAuth::new` `panic!`s on invalid PEM (auth.rs:200, 204).
  Acceptable at startup — but if the JWKS rotation path ever feeds
  bytes back through the same constructor, it would be a remote-DoS
  vector. Need to verify the JWKS path bypasses the panic path. See
  R-MCP-3.

**Witness-equality assumptions**
- `mcp-server` does not compute witnesses directly; it forwards
  `RuLakeBundle::verify_witness` results. The `rulake://bundle/...`
  resource reads `cache_witness_of` (server.rs:771), which is a
  fail-open `Option<String>` — when the cache hasn't seen the
  collection yet, the resource returns `witness_present: false`.
  This is intentional and matches ADR-004 §Resources. Accept.

**JWT scope leakage**
- `effective_caps()` does the **intersection** of server-wide ∩
  per-request JWT (policy.rs:32-37). This is the correct shape — a
  server started with `--capabilities admin` cannot be coerced into
  granting admin to a token whose JWT only has `read`. **Verified by
  smoke test** `tools_list_filtered_by_capability_set`.
- `tools/list` filter (`server.rs:618-637`) uses the same
  `effective_caps()` source as per-call `require_cap` — dual
  enforcement, no scope leak via tool-name guessing.
- The `Capability::Internal` tier is the default for unknown tools
  (`server.rs:489-503`), i.e. unknown tools default-deny. Good — a
  future tool added without an explicit cap mapping fails closed.
- **One subtle gap**: `audit::AuditEntry::policy_decision` only logs
  `capability_required` + `capability_granted` for
  `rulake_query` (server.rs:232-235, 256-259, 280-283, 307-310). The
  publish/admin tools (`rulake_publish_bundle` etc.) also call
  `require_cap` but do NOT emit a `policy_decision` block on success.
  An auditor reading the JSONL stream gets weaker provenance for
  mutation calls than for reads. See R-MCP-1.

**Findings**

| Sev | Title                                                                       | Status            |
| --- | --------------------------------------------------------------------------- | ----------------- |
| Med | `policy_decision` block missing on publish/admin tool audit lines           | Open (R-MCP-1)    |
| Lo  | Unknown / case-mismatched JWT scopes are silently dropped, not audit-logged | Open (R-MCP-2)    |
| Lo  | `JwtAuth::new` panics on invalid PEM — verify JWKS-refresh path bypasses     | Open (R-MCP-3)    |
| Inf | Constant-time bearer compare + dual-enforcement tools/list — accept         | Resolved          |

**Recommendations**

- **R-MCP-1**: In every mutation tool handler
  (`rulake_publish_bundle`, `rulake_refresh_from_bundle_dir`,
  `rulake_save_cache_to_dir`, `rulake_warm_from_dir`,
  `rulake_invalidate_cache`), emit the `PolicyDecision` block with
  the same `capability_required` / `capability_granted` shape that
  `rulake_query` does. ~5 lines per handler. Closes the audit-shape
  asymmetry without touching the policy gate itself.
- **R-MCP-2**: When `scopes_to_caps` sees a scope that starts with
  `mcp:rulake:` but doesn't match a known label, emit a single
  `tracing::warn!` per (audience, scope) pair (debounce so a busy
  IdP doesn't flood the log). Surfaces typos / IdP misconfig that
  today degrade tokens silently.
- **R-MCP-3**: Replace the two `unwrap_or_else(|e| panic!(...))`
  in `JwtAuth::new` (auth.rs:200, 204) with `Result` propagation.
  The startup-time panic is fine for static keys (operator sees the
  failure on launch), but the same constructor sits behind
  `JwksKeys` rotation — a malformed JWKS push from the IdP would
  panic the server thread. Verify `jwks.rs` doesn't reach this
  constructor; if it does, fix the constructor.

---

## 4. Summary

| Crate         | Bench added                                      | Median load-bearing op   | Findings ≥ Med |
| ------------- | ------------------------------------------------ | ------------------------ | -------------- |
| gcs-backend   | `benches/pull_vectors.rs`                        | 184 µs (1k×64)           | 0 (2 Lo)       |
| ipfs-backend  | `benches/verify_bundle.rs`                       | 727 ns (verify_witness)  | 1 (R-IPFS-1)   |
| mcp-server    | `benches/{audit_sink,tools_list_filter}.rs`      | 1.27 µs (audit emit)     | 1 (R-MCP-1)    |

Two Med findings (R-IPFS-1 hard-refuse on CID/data_ref mismatch;
R-MCP-1 audit-shape asymmetry on mutation tools) are the highest-value
fixes for the next pass. Both are <10-line code changes that close
real gaps without architectural disruption.

All bench numbers above are **first-run baselines** intended for
regression detection in subsequent /loop passes. None of them are
publishable claims — the GCS network round-trip and the kubo HTTP
path are both excluded by design (each would measure the wrong layer).
