# 08 — Audit ledger

The Audit screen is a live tail of every MCP call the Console (or the
connected MCP) has emitted. It is the single source of truth for "what
happened, and was it allowed". Rows are JSONL, append-only, locally
persisted in IndexedDB, and survive page reloads.

![Audit screen — JSONL tape with timestamp, principal, tool, target, result code colour-coded by outcome](../../assets/console-audit.png)

## Where rows come from

The tail mixes two streams:

1. **Local rows** — every Console action that touches the wire (Connect,
   Browse refresh, Bundle recompute, Playground send) appends a row via
   `RuStore.appendAudit({...})`. These rows carry a small green `●` next
   to their timestamp to mark them as locally-generated.
2. **Fixture rows** — a static set of demo entries from
   `ui/src/lib/data.js` `AUDIT`. These exist so the tape reads usefully
   in DEMO mode.

Server-side `mcp-server` also keeps its own audit ledger (see
`crates/mcp-server/src/audit.rs`) and the v0.6 ADR-005 work added
symmetric audit shape across read and mutation tools (see CHANGELOG R-MCP-1).
That ledger is not surfaced into the browser tail today; it is exposed via
the `rulake_audit_tail` admin tool and via structured tracing.

## Filtering by env

The audit row carries a `target` field shaped `<backend>/<collection>`.
The screen filters this against whichever environment tab is active in
the topbar:

- `lake-prod` shows rows with target prefix `lake-prod/`
- `lake-eu` shows rows with target prefix `lake-eu/`
- `lake-edge` shows rows with target prefix `lake-edge/`

Local rows always show regardless of env (they typically target an
endpoint URL, not a backend id).

## Filtering by outcome

The right side of the header has a four-state segmented control:

- **all** — no outcome filter
- **ok** — only `outcome === "ok"`
- **degraded** — only `outcome === "degraded"`
- **refused** — only `outcome === "refused"`

The shapes line up with `crates/mcp-server/src/audit.rs::AuditEntry`:

```rust
pub struct AuditEntry {
    pub ts: u64,
    pub principal: String,
    pub tool: String,
    pub target: String,
    pub k: u64,
    pub ms: u64,
    pub code: String,
    pub outcome: String,  // "ok" | "refused" | "degraded" | "error"
}
```

## Reading a row

```
13:41:08.812  agent-7f       query                   lake-prod/memories  · k=10  · 11ms   OK
13:41:06.991  agent-7f       query                   lake-eu/memories    · k=10  · 28ms   STALE_BUNDLE_FALLBACK
13:41:06.044  agent-q2       query                   lake-prod/docs.public · k=10 · 4ms   WITNESS_MISMATCH_REFUSED
13:41:04.840  jules@ruv      warm_from_dir           lake-prod/tickets.support · k=- · 1820ms  OK
13:41:04.011  agent-q2       query                   lake-edge/changelog · k=5   · 2ms    POLICY_DENIED
```

Left to right: timestamp, principal, tool name (with the `rulake_` prefix
elided), target, k+ms summary, code. The whole row is colour-coded by
outcome:

- green text = `ok`
- amber text = `degraded`
- amber-red text = `refused`

## Event types you will see

This list is not exhaustive (substrates and policy hooks can emit their
own codes), but covers the common surface:

### Read path
- **`OK`** — successful query, witness matched, response served from
  cache or after a prime.
- **`OK_VERIFIED_MAIN`** / **`OK_VERIFIED_WORKER`** — Playground emits
  these to indicate which transport ran the search.
- **`STALE_BUNDLE_FALLBACK`** — `Eventual` consistency mode allowed a
  degraded response when the cache was past TTL.
- **`STALE_BUNDLE_GUARD`** — `Fresh` consistency refused the same
  condition.
- **`WITNESS_MISMATCH_REFUSED`** — server-reported witness disagreed with
  the recompute. Always-refuse.
- **`POLICY_DENIED`** — JWT scope, `pii_policy`, or risk class rejected.
- **`BUDGET_EXCEEDED`** — exceeded `budget.max_latency_ms`.

### Mutation path
- **`PUBLISH_QUEUED`** — `rulake_publish_bundle` accepted; new generation
  is being computed.
- **`OK`** for `publish_bundle` / `refresh_from_bundle_dir` /
  `invalidate_cache` / `save_cache_to_dir` / `warm_from_dir` —
  mutation succeeded. Audit shape is symmetric with reads as of v0.6.

### Browse / list
- **`LIST_COLLECTIONS_OK`** — refresh succeeded; row carries `k=<count>`
  of collections discovered.
- **`LIST_COLLECTIONS_FAILED`** — refresh failed; the message is a
  trimmed exception string.

### Connect path
- **`SAVED_LOCAL`** — endpoint persisted to IndexedDB.
- **`INIT_OK`** — MCP `initialize + notifications/initialized + tools/list`
  all succeeded; row carries the discovered tool count.
- **`CONNECT_FAILED`** — handshake failed. Causes range from CORS preflight
  rejection to TLS to 401. See [10 — Troubleshooting](./10-troubleshooting.md).

### Bundle / witness path
- **`WITNESS_MATCH`** — recompute agreed.
- **`WITNESS_MISMATCH`** — recompute disagreed.
- **`WITNESS_COMPUTE_ERROR`** — `rulake-wasm` raised before producing a
  hash. Almost always indicates a malformed bundle.

### IPFS path
- **`IPFS_BUNDLE_VERIFIED`** — fetched bytes hashed to the expected witness.
- **`IPFS_WITNESS_MISMATCH`** — hashed but disagreed (CID-substitution).
- **`IPFS_FETCH_FAILED`** — gateway returned non-2xx.
- **`IPFS_NETWORK_ERROR`** — fetch threw before a response.
- **`IPFS_OK`** / **`IPFS_HTTP_ERROR`** — used by the Bundle screen's
  raw-fetch debug strip.

## Clearing local rows

The **Clear local** button in the header removes every locally-generated
row from IndexedDB. Fixture rows are not touched (they live in JS, not
IndexedDB). The button is disabled when there are no local rows.

## Exporting

Open devtools `Application > Storage > IndexedDB > rulake-store > audit`
to export rows. Each row is the same `AuditEntry` shape as the server
emits, so it round-trips cleanly into any JSONL pipeline.

## When the audit tail saves you

A few realistic scenarios:

- A query returns nothing. Check the audit row for the same `ts` — was it
  `WITNESS_MISMATCH_REFUSED` (the cache lied) or `BUDGET_EXCEEDED` (you
  asked for too much in too little time)?
- The Console shows `○ DEMO` after you clicked Connect. Look for
  `CONNECT_FAILED` — the message field has the actual error. CORS gives
  one shape, 401 another, TLS another.
- A Browse refresh comes back empty. `LIST_COLLECTIONS_OK k=0` means the
  server has no backends registered (probably a configuration gap).
  `LIST_COLLECTIONS_FAILED` means the call itself failed — the message has
  the cause.

The audit ledger is the first place to look when something on a more
visual screen does not match expectations.
