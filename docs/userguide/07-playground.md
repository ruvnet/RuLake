# 07 — Playground

The Playground is the screen where you actually issue queries. It builds
a `tools/call rulake_query` envelope, dispatches it (live wire or
WASM-local), and renders the response with a side-panel decision trace
and a printable witness receipt.

![Playground screen — query box, k/risk/target controls on the left, hit table and decision trace on the right](../../assets/console-playground.png)

## The form

Left pane, top to bottom:

- **Query · semantic** — free-text input. Embedded with
  `text-embedding-3-small` at request time into a 1024-d vector if you
  configured a key in the [Connect](./04-connect.md) screen's Storage
  card. Otherwise embedded with a deterministic fixture vector (so the
  recall results are stable but not semantically meaningful).
- **Target** — pick one collection or `federated · prod+eu`. The
  federated option tells the planner to fan out across both backends and
  merge results. The list is hard-coded in demo mode; in live mode it
  reflects whatever `rulake_list_collections` returned.
- **k (results)** — top-K count. Default 10. Honoured at the cache layer.
- **Risk** — `low` / `medium` / `high`. Surfaced to policy hooks; the
  default Cloud Run deploy treats all three as `read`-permitted, but a
  policy-hook-fronted server can refuse `high` from anonymous principals.

## Sending a query

Click **▶ Send** or press `⌘↵` (mac) / `Ctrl+Enter` (win/linux) with the
focus inside the textarea. The Send button stays disabled while a query
is in flight.

What runs end-to-end on Send:

1. **Embed the query** — provider call if configured, deterministic
   fixture otherwise. Time recorded as `embed:Nms`.
2. **Fan out the search** — `rulake-wasm`'s `searchL2(...)` runs the
   actual top-K against an in-browser fixture corpus, *or* the offload
   path runs in a Web Worker if Workers are enabled in Storage settings.
   Time recorded as `search:Nms (transport)`.
3. **Recompute the witness** — `rulake-wasm`'s `computeWitness(...)` runs
   over the response provenance. Time recorded as `witness:Nms`.
4. **Audit row** — the audit tail gains `OK_VERIFIED_<TRANSPORT>` or
   `WITNESS_MISMATCH_REFUSED`.

Top-right of the response section shows `● WITNESS MATCH` (green) or
`MISMATCH` (amber); the printed receipt on the right pane stamps either
`✓ MATCH` or `VERIFYING`.

## Reading the response

### Hits table

Five columns: rank, score, id, snippet, date. Score is the cosine-or-L2
similarity in `[0, 1]`; the green bar in the score cell is normalised
against the top hit. ID rendering is monospace cyan to make
copy-paste-into-grep painless.

### Decision trace (right pane)

Below the witness receipt is a `decision_trace` block. The named pieces:

- **`reason_code`** — concise reason the planner picked the path it did.
  The most common is `CACHE_HIT_FRESH` (cache was warm and the consistency
  check passed). Refusal codes are the same set surfaced on the
  [Stats](./02-stats-screen.md) screen.
- **`chosen_action`** — `serve_from_cache`, `prime_from_backend`,
  `serve_from_pinned`, `refuse`.
- **`budget`** — `used_ms / max_ms`. Crossing `max_ms` always refuses
  with `BUDGET_EXCEEDED`.
- **`backends_used`** — one row per backend touched, with per-backend
  latency and hit count. For a single-target query this is one row; for a
  federated query it is two or more.
- **`refusals`** — any backend the planner declined to consult, and why.
  In the example shipped with the fixture this is
  `lake-eu/memories STALE_BUNDLE_GUARD — Cached witness is 1 generation
  behind freshness budget (1500ms)`.

### Witness receipt

Top-right of the right pane: `RULAKE · WITNESS · SHAKE-256 · 32 bytes`,
the bundle name, the issued-at, the trust level, and the full 64-hex.
Click anywhere on the hex to copy.

## Consistency modes

The cache layer exposes three consistency modes (see
`crates/core/src/cache.rs:55`). The Playground does not surface them
explicitly — they are configured server-side per `RuLake` instance — but
every refusal you see on this screen ties back to which one is in force.

| Mode | Behaviour | Use case |
|---|---|---|
| **`Fresh`** (default) | Consult the backend's current bundle on every search. | Compliance, finance, policy-enforced workloads where any stale answer is worse than a slower answer. |
| **`Eventual { ttl_ms }`** | Trust the cache for up to `ttl_ms` between checks. Higher QPS; backend updates may be ignored for up to `ttl_ms`. | Search, RAG, recommendation — where a small staleness window is a trade every customer accepts. |
| **`Frozen`** | Caller asserts the bundle is immutable for the cache's lifetime. Never re-check the backend, never invalidate on generation bump. | Witness-sealed historical snapshots. The audit tier. An explicit `refresh_from_bundle_dir` call still invalidates; the guarantee is about automatic checks. |

A `STALE_BUNDLE_GUARD` refusal under `Fresh` becomes `STALE_BUNDLE_FALLBACK`
(degraded, not refused) under `Eventual`. Under `Frozen` neither code can
fire — the cache will not even check.

## Saving and loading queries

- **Save** opens a modal that writes `{label, query, k, risk, target}` to
  IndexedDB. Saved queries are local to the browser.
- **Load** opens a modal listing every saved query; pick one and the form
  populates. The audit tail does not record loads.

## Wire payload

The bottom-right of the right pane shows the exact JSON envelope the
Console will send on the next Send, with the current `k` and `risk`
substituted. Useful when you need to reproduce a Playground query from a
shell:

```bash
curl -fsS -X POST https://rulake-mcp.ruv.io/ \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'mcp-session-id: <session>' \
  -d '{
    "jsonrpc": "2.0",
    "id": 9,
    "method": "tools/call",
    "params": {
      "name": "rulake_query",
      "arguments": {
        "intent": "search",
        "target": { "collection": "memories" },
        "search": { "k": 10 },
        "risk": "medium"
      }
    }
  }'
```
