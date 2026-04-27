# A management UI for ruLake — should we build it?

**Scope.** This note evaluates whether ruLake should ship a browser-side
management UI built on Vite.js, the `rulake-wasm` substrate, the
`rulake/http` MCP client, the `rvf-*` workspace under
`vendor/ruvector/crates/rvf/`, and the wider RuVector capability set.
The user's framing was casual ("should we create a management UI using
Vite.js, RVF, and related RuVector tools/capabilities?") — the work
here is to pin that down to something an engineering team could
either build or refuse, and to defend the call against named
alternatives.

**Method.** Every claim about ruLake's behaviour or surface is grounded
in a specific file path or ADR. Every claim about RVF or ruvector-* is
grounded in a path under `vendor/ruvector/crates/`. Where a UI claim
depends on a capability that does not exist in mcp-server today, that
gap is named explicitly and the change required is described.

**Style.** White-paper voice. No marketing. Where the answer is "no"
or "not what was asked," that is stated in plain English with the
reasoning attached.

---

## Contents

1.  The question, scoped tightly
2.  Personas and the work they actually do
3.  Screen-by-screen decomposition
4.  Tech stack — Vite.js, framework pick, `rulake-wasm`, `rulake/http`,
    RVF, ruvector-*
5.  Capability gaps — what mcp-server, ruLake, rulake-wasm, and RVF
    would each have to land
6.  Architecture — process, transport, trust boundaries
7.  Rejected alternatives — MCP Inspector, Grafana, CLI-first,
    embed-in-host, no-UI-better-docs
8.  MVP — what we would actually build, in priority order
9.  Out of scope — what we would explicitly refuse to ship in v0.1
10. Verdict
11. Open questions

---

## 1. The question, scoped tightly

A "management UI" is a portmanteau. The user's phrasing buys the
designer flexibility, but unless the scope is pinned down before the
first commit, the result is a dashboard that does many things badly.
The candidate scopes — listed from narrowest to widest — are:

1.  **Witness explorer** (visual sidecar verifier, drag-drop,
    cross-bundle diff). The existing
    `examples/wasm/01-witness-verifier-browser/` is a one-page
    proof-of-concept of this; a real UI would expand it to history,
    diff, and a connected backend.
2.  **Observability dashboard** (live `rulake://stats` and
    `rulake://stats/by-backend` rollups, per-collection cache
    geometry, audit-log tail). Read-only. Operator-facing.
3.  **Agent-developer playground** (paste a query embedding, see hits +
    `decision_trace`, replay refusals, exercise the four intents).
    Builder-facing. The natural first audience for "how does ruLake
    behave in my prompt?"
4.  **Bundle and federation curator** (list registered backends, drag
    backends into shards, publish bundles, refresh from disk, save and
    warm cache snapshots). Operator-facing. Mutation-tier.
5.  **RBAC / config editor** (edit `[[allow]]` blocks in `mcp.toml`,
    preview the resulting `tools/list` filter, pre-mint JWTs with
    chosen scopes for testing). Operator-facing. Privileged.
6.  **Full RuVector control surface** (browse RVF segments, visualise
    GNN/graph state, manage DiskANN shards, `ruvector-postgres` browse,
    one-pane-of-glass for the entire vendored workspace).

Scope (1) is the cheapest ship — it is roughly two weekends of work on
top of what already builds. Scope (6) is the multi-quarter "let's
build everything in RuVector a homepage" project that nobody asked
for and would compete with `rvf-server`'s already-shipping dashboard
(`vendor/ruvector/crates/rvf/rvf-server/src/http.rs:9` exposes
`GET /`, `/assets/*`, and an embedded `DASHBOARD_SEG` at type
`0x11` defined in `vendor/ruvector/crates/rvf/rvf-types/src/dashboard.rs:1`).

**This note adopts scopes (2) + (3) + a subset of (4) as the working
definition of "management UI for ruLake."** That is: a
read-mostly observability + playground + bundle-viewer for an
operator running a `rulake-mcp` server. Scope (1) is folded in as a
sub-screen because it is cheap and it is the one screen that sells
the cryptographic posture in ten seconds. Scopes (5) and (6) are
deferred (see §9).

The reasoning for this scoping:

-   ruLake is one layer in a stack — it sits **above** RVF (the
    "system of record") and **below** the brain-substrate consumers
    (`mcp-brain`, `mcp-brain-server`, `mcp-gate`). A UI scoped to
    "everything in the stack" is always the wrong answer because
    each layer earns the right to its own surface independently.
    `rvf-server` ships an RVF dashboard already; `mcp-brain-server`
    will get its own when it earns one. The ruLake UI is the ruLake
    UI, not a RuVector control panel.
-   The MCP server is the only place ruLake state escapes
    in-process today. The HTTP transport
    (`mcp-server/src/http.rs`) is the wire any UI must speak; the
    resources (`rulake://stats`, `rulake://stats/by-backend`,
    `rulake://bundle/{backend}/{collection}` —
    `mcp-server/src/server.rs:597-624`) are the only structured-read
    surface. A UI that bypasses MCP duplicates work the agentic
    path already paid for.
-   Mutation surfaces (publish, refresh, save, warm, invalidate) all
    exist as MCP tools today
    (`mcp-server/src/server.rs:331-438`). They are the obvious
    "manage" actions. RBAC mutation does not exist yet
    (`mcp.toml` is file-config), so it is out of MVP scope.

With that scope, the rest of this document is concrete.

---

## 2. Personas and the work they actually do

There are five plausible audiences for a ruLake UI. Not all of them
are served best by a UI; each is evaluated honestly.

### 2.1 Operator running `rulake-mcp` in production

**What they do today.** Configure `mcp.toml`, start `rulake-mcp http
--bind 0.0.0.0:8788 --auth jwt …`, watch stderr or `--audit-file`,
react to alerts, occasionally call `rulake_publish_bundle` /
`rulake_warm_from_dir` over the wire. They live in `tail -f` and
`jq` against the audit JSONL.

**Pain points the UI would relieve.**

-   No live cache-stats visualisation. They poll
    `rulake://stats` via curl + jq and eyeball it.
-   Audit-log triage is ad-hoc. Filtering by `principal`, `intent`,
    `outcome`, and `code` ("show me every `WITNESS_MISMATCH_REFUSED`
    in the last hour") is grep-driven.
-   Bundle witnesses are opaque hex strings. Diffing the witness on
    disk against the witness in cache requires two CLI calls and a
    visual compare.

**Persona rank.** **High.** This persona does not strictly need a UI
(they already have CLI alternatives for every piece) but they would
use one daily if it existed. The leverage of consolidating cache
stats + audit tail + bundle resource read into one pane is real —
it cuts a 6-step grep dance to one click.

**Caveat.** Production operators frequently work from a jump host
without a browser. The UI must remain useful when proxied (`ssh -L`)
or run locally against a remote MCP endpoint. That biases toward a
static SPA + a fetch transport, not a server-rendered admin app.

### 2.2 Agent developer building a ruLake-backed tool

**What they do today.** Write Rust / Python / Node / TypeScript code
that calls `rulake_query` and reasons about the response. They may
use `examples/nodejs/04-mcp-tool/` as a starting point. The
`decision_trace` (`mcp-server/src/planner.rs:257-293`) is gold for
debugging routing and refusals, but parsing it from a JSON dump in
the terminal is awkward.

**Pain points the UI would relieve.**

-   Iterating on a query (different `vector`, `k`, `risk`,
    `freshness_ms`) requires recompiling the test harness. A
    "playground" tab that lets them paste a query, send it, and
    see the structured response side-by-side is the obvious win.
-   `decision.refusals[]` and `decision.degraded_advice` are
    structured but invisible. A drill-down per route ("why did
    `lake-prod/docs.public` refuse this query?") removes a class of
    "is this me or is this the server" debugging sessions.
-   Browser-side witness verification — `verifyBundleJson` from
    `node-wasm/src/lib.rs:158` already works — gives the developer
    a fast way to sanity-check that the server's `provenance.witness`
    matches the bundle the publisher claims to be serving.

**Persona rank.** **High.** This is the audience that drives
ruLake adoption. A friction-free playground is the ruLake answer
to the OpenAI Playground / Anthropic Console / Pinecone Explorer
tier of tooling that competitors have. Without one, the only entry
point is "read the README and write code" — survivable but
suboptimal.

### 2.3 Researcher inspecting bundles, witnesses, federation topology

**What they do today.** Pull `table.rulake.json` sidecars and the
matching `index.rbpx` snapshot, run `rulake_verify_witness`-style
checks, compare across releases. Heavy users of the
`examples/wasm/01-witness-verifier-browser/` page when it ships.

**Pain points the UI would relieve.**

-   No witness history view. A bundle's `generation` (`Num` or
    `Opaque`) advances over time; the cryptographic chain that proves
    "this dataset is the legitimate descendant of that one" is not
    rendered anywhere.
-   No federation topology visualisation. `search_federated`
    (`src/lake.rs`) takes `&[(&str, &str)]` and runs in parallel; the
    routing decision and per-route latencies appear in
    `decision.backends_used` but a topology diagram is not.

**Persona rank.** **Medium.** Real but small audience. The
witness-history and topology views are nice-to-haves, not v0.1 work.
The cheap part — drag-drop a sidecar, see fields + recomputed
witness — already exists at
`examples/wasm/01-witness-verifier-browser/index.html`. Folding it
into the UI is a sub-screen.

### 2.4 End user of an agent that is built on ruLake

**What they do today.** Use the agent. Have no idea ruLake exists.
Possibly want a "what does the agent remember about me?" view in
some applications.

**Persona rank.** **Out of scope.** A "personal memory inspector" is
the right surface for a Cognitum chip / `mcp-brain-server`
application, not for ruLake. ruLake is a substrate; the agent that
embeds it is responsible for its own end-user UX. A ruLake UI that
tries to serve end users blurs the contract.

### 2.5 Curious bystander evaluating ruLake against alternatives

**What they do today.** Read the README, scan the BENCHMARK, possibly
clone and run an example. They want to "see ruLake work" in five
minutes without setting up a backend.

**Pain points a UI would relieve.**

-   The five-minute demo today is "drag a sidecar onto the browser
    verifier" — which is good, but they only see the
    cryptographic-witness story. They never see the
    cache-coherent-search story, which is the actual point of
    ruLake.
-   A hosted demo against a curated `LocalBackend` would let them
    run real queries with `decision_trace` in the right pane and
    `cache_stats` updating in real time. Five-minute story becomes
    "click here, type a query, see how ruLake answered it."

**Persona rank.** **Medium-high for marketing leverage, low for
engineering merit.** The cost of a curated-demo deployment is real;
the upside is concentrated in the first two weeks of release. If
the UI exists for personas 2.1 and 2.2 anyway, this falls out for
free.

### Persona ranking summary

| Persona                      | UI value | Already-served by | Build for v0.1? |
|------------------------------|----------|-------------------|-----------------|
| Operator in production       | High     | CLI + grep        | **Yes** (read-only) |
| Agent developer              | High     | Code + repl       | **Yes** (playground) |
| Researcher                   | Medium   | Browser verifier  | Partial (sub-screen) |
| End user of an agent         | None     | The agent itself  | **No** |
| Curious bystander            | Med-high | README            | Falls out free  |

---

## 3. Screen-by-screen decomposition

Each screen below is rated by:

-   **Backed by:** which existing capability (file path) backs it
-   **Gap:** what would need to land in mcp-server / ruLake /
    rulake-wasm / RVF for the screen to ship
-   **MVP:** whether it is in or out of the v0.1 build

### 3.1 Connect / Auth

The UI's first screen. Picks an MCP endpoint URL and an auth mode.

-   **Bearer.** Paste a token; the UI stores it in `sessionStorage`
    and threads it into `RuLakeHttp` constructor's `token` opt
    (`node/http.mjs:108`). The server-side bearer check is
    `mcp-server/src/auth.rs::BearerAuth::verify`.
-   **JWT.** Same shape as bearer (paste an `Authorization: Bearer
    <jwt>`). The server validates signature, `iss`, `aud` (RFC 8707),
    `exp`, and maps `scope` claims to `mcp:rulake:read|publish|admin`
    (see `mcp-server/src/http.rs:314-333`). The UI does not need to
    understand the JWT — it just sends it.
-   **mTLS.** **Impossible from a browser.** Browsers do not let
    JavaScript control which client certificate is presented during
    the TLS handshake. The handshake-level cert selection happens
    before any JS runs. The UI must surface a clear error here:
    "mTLS requires a non-browser client (use the CLI or `rulake/http`
    from Node.js)."
-   **None.** Useful for `--auth none --bind 127.0.0.1` dev. The UI
    just omits the `Authorization` header.

**Backed by:** `mcp-server/src/http.rs:39-60` (the `AuthMode` enum
and the `serve` entry point), `node/http.mjs:107-122`
(`_headers()`).

**Gap:** None for bearer/JWT/none. mTLS is permanently
browser-incompatible — that is a property of the TLS handshake, not
something a code change can fix. The UI must label that mode "CLI
only" instead of trying to support it.

**MVP:** Yes. Bearer + JWT + none. mTLS shows the error.

### 3.2 Backend and collection browser

Lists registered backends from `rulake_list_backends`
(`mcp-server/src/server.rs:323`). For each backend, lists known
collections with their cached witness, dim, generation, and last
prime. Each row links into the bundle viewer.

**Backed by:**

-   `rulake_list_backends` returns `{backends: [string]}`.
-   `rulake://stats/by-backend` returns per-backend roll-up.
-   `rulake://bundle/{backend}/{collection}` — listed by
    `list_resources` for every cached collection
    (`mcp-server/src/server.rs:614-624`) — returns
    `{backend, collection, witness, witness_present, cache_entries}`.

**Gap:**

-   The UI cannot enumerate **all** collections a backend knows
    about. The only collections that appear in `list_resources` are
    the ones already cached. A truly cold collection — registered
    but never queried — is invisible to MCP today. Two ways to
    close this:
    1.  Add a `BackendAdapter::list_collections` MCP tool. The
        trait already has `list_collections` per ADR-155; today
        nothing wires it through MCP. A new tool
        `rulake_list_collections{backend}` mapped to
        `Capability::Read` is ~30 LOC in `mcp-server/src/server.rs`
        and ~30 LOC in `mcp-server/src/planner.rs`.
    2.  Document the constraint and let the operator query
        each-collection-once at startup to populate the resource
        list. Cheap, brittle.
-   Per-collection `dim` and `last_prime_ms` are in
    `cache_stats_by_collection` (`src/lake.rs:126`) but not exposed
    over MCP. A `rulake://stats/by-collection` resource (parallel to
    `by-backend`) would close it. ~40 LOC in
    `mcp-server/src/server.rs::read_resource_json`.

**MVP:** Yes for the cached-only view. Defer cold-collection
enumeration to v0.2.

### 3.3 Live query playground

A textarea for an embedding (or a "generate random" button), an `intent`
picker (search / verify / explain / refresh — though v0.1 supports
`search` only per `mcp-server/src/planner.rs:78-80`), a target
selector (collection or routes), risk/freshness sliders, a
"Send" button. Right pane shows the parsed `QueryResponse`:

-   `data[]` table sorted by score
-   `provenance.witness` rendered as hex with a "verify in browser"
    button (uses `verifyBundleJson` from
    `node-wasm/src/lib.rs:158`)
-   `decision.chosen_action`, `reason_code`, `backends_used`,
    `refusals`, `degraded_advice` rendered as a structured drill-down
-   Latency from `decision.budget_used_ms`

The `RuLakeHttp` client (`node/http.mjs:160`) is the transport
verbatim:

```javascript
const c = new RuLakeHttp(endpoint, { token });
await c.connect();
const res = await c.query({
  intent: "search",
  target: { collection: "memories" },
  search: { vector: queryEmbedding, k: 10 },
  risk: "medium",
  budget: { max_latency_ms: 100, max_results: 20 },
});
// res.data, res.provenance, res.trust_level, res.decision
```

**Backed by:** `rulake_query` (`mcp-server/src/server.rs:199`),
`RuLakeHttp.query` (`node/http.mjs:160`).

**Gap:** None for `intent: "search"`. The other intents fire
`PlanError::Internal` until the planner gains them. The UI should
gate the picker by what the server actually supports — easiest is to
hardcode "search" in v0.1 and grey out the others.

**MVP:** Yes. This is the agent-developer surface that earns the UI
its keep.

### 3.4 Bundle viewer

Drag-drop a `table.rulake.json` sidecar OR pick a
`rulake://bundle/{backend}/{collection}` from the list. Shows fields
(`format_version`, `data_ref`, `dim`, `rotation_seed`, `rerank_factor`,
`generation`, `pii_policy`, `lineage_id`, `memory_class`), the stored
witness, the recomputed witness, and a match indicator.

The drag-drop path is the existing
`examples/wasm/01-witness-verifier-browser/index.js` lifted into a
component. The MCP path adds a fetch via
`ListMcpResourcesTool` / `ReadMcpResourceTool` equivalent (we'd call
`resources/read` directly through `rulake/http`'s transport — that
method does not exist on `RuLakeHttp` today, only `query()`. See
gap below).

**Backed by:**

-   `rulake://bundle/{backend}/{collection}` resource
    (`mcp-server/src/server.rs:614-624`).
-   `verifyBundleJson` for client-side witness recompute
    (`node-wasm/src/lib.rs:158`).
-   `compute_witness_js` for "what witness should this bundle carry?"
    (`node-wasm/src/lib.rs:212`).

**Gap:**

-   `RuLakeHttp` (`node/http.mjs:106`) only exposes `query()`. It
    does not expose `resources/list` or `resources/read`. The fix is
    a small addition to the client — three methods (`listResources`,
    `readResource`, and an internal `callMethod` helper). ~30 LOC.
-   The bundle resource returns `{witness, witness_present,
    cache_entries}` — it does not return the full bundle JSON
    (`format_version`, `dim`, `rotation_seed`, etc). The full bundle
    is what `verifyBundleJson` expects. To make the UI's "verify the
    cached bundle" path work end-to-end, either:
    1.  Extend the resource to return the full bundle (rename to
        `rulake://bundle/{b}/{c}` returning the
        `RuLakeBundle`-shaped JSON; the data is one
        `lake.current_bundle(&key)` call away —
        `src/lake.rs:169`).
    2.  Add a separate `rulake_get_bundle` tool returning the
        `RuLakeBundle`. Tool surface is more discoverable; resource
        surface is more cache-friendly. Either works. Pick 1 for
        less surface area.

**MVP:** Yes for the drag-drop side (no server changes). Cached-bundle
verify is gated on the resource extension (~50 LOC server-side).

### 3.5 Stats dashboard

Live tile of:

-   `hits`, `misses`, `primes`, `invalidations`, `shared_hits`,
    `warm_installs` from `rulake://stats`
-   `hit_rate` and `avg_prime_ms` derived (already computed
    server-side, see `mcp-server/src/server.rs:660-661`)
-   per-backend rollup from `rulake://stats/by-backend`
    (`mcp-server/src/server.rs:665-684`)
-   sparkline of `last_prime_ms` over time (client-side ring buffer,
    poll every ~1s)

**Backed by:** Both resources are already implemented and shipping.

**Gap:** None for the rollups. Per-collection rollup is in the
`RuLake` struct (`cache_stats_by_collection`, `src/lake.rs:126`) but
not exposed over the wire — see §3.2 gap.

**MVP:** Yes. This is the operator's daily view.

### 3.6 Audit log viewer

Tail of the JSONL audit stream. Filterable by `principal`, `tool`,
`intent`, `outcome` (`ok`, `refused`, `degraded`, `error`), `code`,
and `trust_level`. Drill-down on a single entry shows the
`policy_decision` and `decision` blocks from
`mcp-server/src/audit.rs::AuditEntry`.

**Backed by:** Audit emission to a file via `--audit-file`
(`mcp-server/src/audit.rs::open_file`). Each line is the
`AuditEntry` struct serialised to JSON.

**Gap:** **Major.** The audit log is on-disk only. There is no
HTTP endpoint or MCP resource that streams it. To make the audit
viewer work, mcp-server must add one of:

1.  An MCP resource `rulake://audit?since=<ts>&limit=<n>` that reads
    the tail of the audit file. Pros: stays in the MCP wire model,
    capability-gated naturally (audit goes through the planner like
    any other read). Cons: not a stream — the client must poll.
2.  An MCP server-sent-events endpoint `GET /audit/stream` that
    streams new audit lines. Pros: real-time. Cons: lives outside
    the MCP wire (a sibling endpoint), needs its own auth gate.
3.  A new MCP tool `rulake_audit_tail{since, limit}` returning a
    page of entries. Pros: same wire as everything else. Cons:
    polling client.

Option 3 is the smallest delta — a tool that opens
`AuditSink::path()` and tails N entries newer than a timestamp.
Capability tier: `Admin`. ~60 LOC.

The UI poll cadence (1–2 s) is fine for human eyeballing; for
real-time alerting, use OTLP (mcp-server already has the
`tracing-opentelemetry` dep budgeted in ADR-004).

**MVP:** **Conditional.** The viewer is high-leverage but the
server-side gap is non-trivial. Recommend deferring to v0.2 and
shipping the cache-stats dashboard in v0.1.

### 3.7 Snapshot manager

Trigger `rulake_save_cache_to_dir` for a (backend, collection) pair,
then download the resulting `index.rbpx` + sidecar. Or upload an
existing snapshot and trigger `rulake_warm_from_dir`.

**Backed by:** Both tools exist
(`mcp-server/src/server.rs:386-424`). Both are admin-tier.

**Gap:** **Major.** Both tools take a `dir: String` that is a
**server-side path**. The server reads/writes there. There is no
HTTP file upload or download. To make snapshot manager work
end-to-end from the browser, mcp-server must add:

1.  A multipart `POST /snapshot/upload` endpoint that writes into a
    server-managed staging dir, then exposes that dir via a server-
    generated path token to `rulake_warm_from_dir`.
2.  A `GET /snapshot/download/{backend}/{collection}` endpoint that
    runs `save_cache_to_dir` into a temp dir and streams the result.

Both are non-trivial. They live outside the MCP wire, need their own
auth + capability gates, and need careful path-traversal protection
(every CVE in §1 of ADR-004 cited a path-traversal somewhere).

**MVP:** **No.** Defer entirely. The CLI path
(`rulake_save_cache_to_dir` against a server-local path, then
`scp` or volume mount) is fine for v0.1. Add a "this is a
server-local path" notice and a copy-to-clipboard CLI snippet
instead of an upload UI.

### 3.8 RBAC editor

Edit `[[allow]]` blocks in `mcp.toml`, preview the resulting
`tools/list` filter for a given JWT scope set, mint dev tokens for
testing.

**Backed by:** `AllowList::from_blocks` consumes the
`AllowBlock` shape (`mcp-server/src/allow.rs:38`). The
capability filter for `tools/list` is
`mcp-server/src/server.rs:566-585`.

**Gap:** **Major.** `mcp.toml` is read once at startup
(`McpConfig::default` and the loader in `main.rs`); there is no
mutation API. To make this work, mcp-server must add:

-   A reload-config tool or signal that re-parses `mcp.toml` and
    re-builds the `AllowList`. `tracing` already supports SIGHUP-
    style reloads of subscribers; the same shape works for config.
-   An admin-tier tool `rulake_reload_config` that re-runs
    `AllowList::from_blocks` and atomically swaps the
    `Arc<AllowList>` in the planner.
-   A "preview filter" tool that takes a candidate allow-list and
    a candidate JWT scope set, and returns the filtered tools list
    + visible (backend, collection) pairs. Pure function, ~40 LOC.

The dev-token mint is a separate concern — the UI generates a
self-signed JWT for testing only, never against a real IdP.

**MVP:** **No.** RBAC editing is the kind of thing operators do once
during setup. CLI + edit `mcp.toml` + restart is fine. Defer to
v0.2 or later.

### 3.9 Decision-trace explainer

Drill-down on `chosen_action`, `reason_code`, `backends_used`,
`refusals`, `degraded_advice` from the playground response. This is
a rendering layer on top of the playground's response — not a
separate screen.

**Backed by:** `decision` block on every `QueryResponse`
(`mcp-server/src/planner.rs:257-293`).

**Gap:** None.

**MVP:** Yes (folded into 3.3).

### 3.10 Federation builder

Drag backends into shards. Preview the adaptive rerank
`k' = max(5, global_k / num_routes)`. Send a federated query.
Render per-route latency bars.

**Backed by:** `Target.routes` on `QueryRequest`
(`mcp-server/src/planner.rs:84-88`) maps directly to
`RuLake::search_federated`'s `&[(&str, &str)]`. The visualisation
is client-side.

**Gap:** None for the call. The "preview adaptive rerank" math is
client-side; the formula is in `src/lake.rs` (rerank-adaptive logic
mentioned in ADR-155). The UI would re-implement that math in JS to
preview before send.

**MVP:** **No** for v0.1. The federation builder is genuinely
useful but it is the most complex screen and adds JS-side coupling
to the rerank math (which can drift from the Rust). Defer to v0.2.

### 3.11 Screen summary

| #   | Screen                | MVP? | Server-side gap |
|-----|------------------------|------|-----------------|
| 3.1 | Connect / auth         | Yes  | None (mTLS off-limits) |
| 3.2 | Backend / collection   | Yes  | List-collections tool (small) |
| 3.3 | Query playground       | Yes  | None |
| 3.4 | Bundle viewer          | Yes  | Resource extension to return full bundle JSON |
| 3.5 | Stats dashboard        | Yes  | None |
| 3.6 | Audit log viewer       | No   | Audit-tail tool (~60 LOC) |
| 3.7 | Snapshot manager       | No   | Upload/download HTTP endpoints |
| 3.8 | RBAC editor            | No   | Config reload tool + preview-filter tool |
| 3.9 | Decision-trace         | Yes  | None (rendering layer) |
| 3.10| Federation builder     | No   | None — pure client work, just deferred |

The MVP is screens 3.1, 3.2, 3.3, 3.4, 3.5, 3.9. Six screens, of
which only 3.4 needs a small server change.

---

## 4. Tech stack

### 4.1 Vite.js

Vite is the right pick. The reasoning:

-   ESM-native, Rollup-based prod build, sub-second dev rebuilds.
    Critical for the "iterate on a query and see the response in the
    same heartbeat" playground feel.
-   Already the de-facto pick across RuVector. `rvf-server` already
    serves a Vite-built dashboard — see
    `vendor/ruvector/crates/rvf/rvf-server/src/http.rs:48` ("Three.js
    dashboards alongside the embedded `DASHBOARD_SEG`"). Picking the
    same toolchain keeps the cognitive overhead across the
    workspace consistent.
-   `wasm-pack --target web` integrates cleanly with Vite's
    `?init` import — the existing
    `examples/wasm/01-witness-verifier-browser/index.js:7` proves
    the integration works.

### 4.2 Framework — Svelte vs React vs Solid vs Vue

Recommendation: **Svelte 5**. Reasoning, with the trade-offs called
out:

-   **Svelte 5.** Fine-grained reactivity (runes) maps naturally to
    the streaming MCP responses. Bundle size ~10-20 KB gz for a
    small SPA — keeps the total payload (with `rulake-wasm` ~149 KB
    and Vite glue) under 200 KB compressed. Built-in stores match
    the "one source of truth per resource" pattern. Downside: smaller
    ecosystem than React; some MCP-Inspector-style components may
    need to be hand-rolled.
-   **React.** Largest ecosystem. Easy to find people. Downside:
    bundle-size baseline (~40 KB for React+ReactDOM gz, plus
    typically 30-60 KB of UI-library glue) doubles the payload for
    no concrete benefit in a single-purpose dashboard. State
    management for streaming MCP responses needs Tanstack-Query or
    similar; another dep.
-   **Solid.** Closest to Svelte in size and performance, finer
    reactivity than Svelte 5 in some respects. Downside: smaller
    user base, fewer ready-made components.
-   **Vue.** Sized between Svelte and React. The composition API
    works well for this kind of dashboard. No concrete advantage
    over Svelte for this workload.
-   **Vanilla.** The existing browser verifier is vanilla and
    works. For a six-screen dashboard, the per-screen state
    management starts to get unwieldy without a framework. Vanilla
    is the right call only if the UI is genuinely a single-screen
    "verify a sidecar" page.

**Pick Svelte 5.** If the team already has React muscle memory,
React is a defensible second choice — the bundle-size cost is
real but not load-bearing for an internal tool. Do NOT pick
vanilla for a six-screen UI; do NOT pick Solid unless someone on
the team already ships Solid in production.

### 4.3 UI primitives

Pick a headless library: **Headless UI** (Tailwind-CSS-friendly),
**Radix Primitives** (React) ported equivalents, or **Bits UI**
(Svelte). All three give accessible primitives (combobox, dialog,
tabs, command-palette) without imposing a visual style. Tailwind
for styling — gets the MVP shipped without bikeshedding CSS.

### 4.4 `rulake-wasm`

The browser-side substrate. Already publishing to npm (the 24h
cooldown after the botched first publish republishes at 2.2.1 per
the brief). The UI uses it for:

-   `verifyBundleJson(json: string)` — the bundle viewer's
    cryptographic check. Server returns hits with a witness;
    client verifies the witness locally. The trust boundary
    becomes cryptographic, not network — the UI proves the server
    served what it claimed regardless of TLS, CORS, or middleware
    integrity.
-   `computeWitness(data_ref, dim, rotation_seed, rerank_factor,
    generation)` — for the "what witness should this bundle carry?"
    sanity check during publishing.
-   `searchBruteForceL2(vectors, ids, dim, query, k)` — out-of-scope
    for v0.1 because the UI does not own the vector corpus, but
    available for future "demo against an in-browser corpus"
    use cases.

The wasm import is one line in Vite:

```javascript
import init, { verifyBundleJson, computeWitness, buildInfo }
  from "rulake-wasm";
await init();
```

Bundle: ~149 KB compressed (per the brief). At gzip on the wire
this fits comfortably in a single round trip.

### 4.5 `rulake/http`

The fetch-based MCP client (`node/http.mjs`). Edge-friendly, works
in any runtime that has `fetch` and `ReadableStream`. The UI
imports it directly:

```javascript
import { RuLakeHttp } from "rulake/http";
```

In the browser this resolves to `node/http.mjs` (an ESM module —
no Node-specific APIs are used; `fetch`, `TextDecoder`, and
`ReadableStream` are all browser globals). The current shape
(`query()` only) needs the small extension noted in §3.4 — three
methods to expose the MCP `resources/list` and `resources/read`
RPCs. That work belongs in `node/http.mjs` itself, not in the UI.

### 4.6 RVF — `vendor/ruvector/crates/rvf/`

RVF is the "system of record" beneath ruLake. It is a vendored
sibling, not a runtime dependency of the MCP server today. The
question is whether the UI should surface RVF.

The honest answer: **not in v0.1**, possibly in v0.3. Reasoning:

-   RVF has its own dashboard. `rvf-server`
    (`vendor/ruvector/crates/rvf/rvf-server/src/http.rs:1-13`)
    exposes `GET /` for a dashboard, `GET /assets/*` for static
    assets, AND embeds a pre-built Vite dashboard in a
    `DASHBOARD_SEG` (segment type `0x11`, defined in
    `vendor/ruvector/crates/rvf/rvf-types/src/dashboard.rs`).
    The RVF team has explicitly chosen Vite + Three.js (per the
    `rvf-server` source comment) for that surface.
-   The ruLake UI duplicating RVF segment-browser features would
    confuse the boundary. ruLake's MCP server does not expose
    RVF segments — it exposes ruLake bundles
    (`table.rulake.json`), which are the application-layer view
    of what backends serve. Mixing the two layers in one UI mixes
    the two contracts.
-   If the UI needs an "open the underlying RVF" button per
    bundle, it should deep-link into a separately-deployed
    `rvf-server` dashboard for that bundle's `data_ref`. That is
    one HTML anchor, not a feature.

The ruLake UI uses **`rulake-wasm`** (witness verify), not RVF
crates. RVF is the right place for segment-level introspection;
ruLake is the right place for cache-coherent search and bundle-
level provenance.

### 4.7 Other RuVector capabilities

-   **`ruvector-postgres`.** A Postgres extension for vector
    columns. Surfacing this in the ruLake UI would be a category
    error — it is its own product with its own surface.
-   **`ruvector-graph` / `ruvector-gnn` / `ruvector-attention`.**
    Neural / graph capabilities. WASM-buildable variants exist
    (`ruvector-graph-wasm`, `ruvector-gnn-wasm`,
    `ruvector-attention-wasm` per
    `vendor/ruvector/crates/`). Out of scope for ruLake's UI;
    they belong in a separate "RuVector explorer" if anyone
    builds one.
-   **`ruvector-server`.** An alternate HTTP transport for
    ruvector-core. Not consumed by ruLake's MCP server. Out of
    scope.
-   **`ruvector-cli`.** A CLI surface. Complementary to the UI,
    not subordinate to it. The UI does not "wrap" the CLI;
    they share a transport.
-   **`rvf-wasm` / `rvf-solver-wasm`.** WASM builds of the RVF
    runtime. Useful if the UI ever wants to inspect RVF segments
    locally — defer until that need is real.

The summary: the UI imports `rulake-wasm` and `rulake/http`. It
deep-links into `rvf-server` for RVF-level introspection. It does
not reach into the rest of the RuVector workspace.

---

## 5. Capability gaps — what would have to land

Listed by where the work lives. Each gap is honest about what does
not exist today; the README and prior ADRs have occasionally
overstated and this note should not.

### 5.1 In `mcp-server/`

-   **(small)** `rulake_list_collections{backend}` MCP tool that
    proxies to `BackendAdapter::list_collections`. Capability:
    `Read`. ~30 LOC each in `server.rs` and `planner.rs`. Lets the
    UI show registered-but-cold collections in the browser.
-   **(small)** Extend `rulake://bundle/{b}/{c}` to return the full
    `RuLakeBundle` JSON (currently returns
    `{witness, witness_present, cache_entries}` only —
    `mcp-server/src/server.rs:685-705`). The full bundle is one
    `lake.current_bundle(&key)` call away (`src/lake.rs:169`).
    Makes the cached-bundle verifier path work end-to-end.
-   **(small)** `rulake://stats/by-collection` resource parallel to
    `by-backend`. ~40 LOC in
    `mcp-server/src/server.rs::read_resource_json`. Surfaces
    per-(backend, collection) hit/miss/prime/dim/last_prime_ms. The
    in-memory data is already there
    (`cache_stats_by_collection`, `src/lake.rs:126`).
-   **(medium)** `rulake_audit_tail{since, limit}` admin-tier tool.
    Reads from `AuditSink::path()`'s file. ~60 LOC in
    `audit.rs` + tool wiring. Required to ship the audit viewer
    (deferred from MVP).
-   **(medium)** `rulake_reload_config` admin-tier tool. Re-parses
    `mcp.toml`, atomically swaps `Arc<AllowList>` in the planner.
    Required for the RBAC editor (deferred from MVP).
-   **(large)** Snapshot upload/download HTTP endpoints. Sit
    outside the MCP wire. Needs path-traversal protection,
    capability gating, content-length limits, and probably a
    chunked transfer for large `.rbpx` files. Required for the
    snapshot manager (deferred from MVP).
-   **(small, separate)** **CORS.** `mcp-server/src/http.rs` does
    not currently set CORS headers. Browser SPAs hosted on a
    different origin than the MCP server cannot talk to it without
    CORS. The fix is `tower-http`'s `CorsLayer`; the design
    question is the policy. Recommend: explicit allow-list of
    origins via `--cors-allow-origin <url>` repeated, default
    same-origin only. ~30 LOC. Alternative: serve the UI from the
    same origin as the MCP server (then no CORS needed) — see §6.

### 5.2 In `rulake/` (the host crate)

No changes needed for v0.1. The MVP screens read from existing
public surface (`cache_stats`, `cache_stats_by_backend`,
`current_bundle`, `cache_witness_of`, `cache_entry_count`).

### 5.3 In `rulake-wasm/` (`node-wasm/`)

No changes needed for v0.1. `verifyBundleJson` and
`computeWitness` cover the browser-side cryptographic surface. A
v0.2 want: a streaming-friendly `verifyBundleStream` for very
large sidecars (today everything is bounded at 64 KiB by
`MAX_BUNDLE_BYTES`; that is fine for the foreseeable shape but
deserves a future-proof note in case format v3 grows the
payload).

### 5.4 In `node/http.mjs`

-   **(small)** Add `listResources()`, `readResource(uri)`, and
    `callTool(name, args)` (a generic tool dispatcher) on
    `RuLakeHttp`. Today only `query()` exists; the playground
    needs the generic `callTool` for the publish/admin tier
    tools, and the bundle viewer needs `readResource`. ~40 LOC.
-   **(small)** Surface the `decision.degraded_advice` body when
    the server returns 429. Today the client throws a generic
    `RuLakeHttpError` (`node/http.mjs:42-52`); the
    `RuLakeHttpError.code` and `.status` are populated but the
    JSON body's `hints` array is dropped. The UI wants to render
    those hints. ~10 LOC.

### 5.5 In RVF

No changes needed. The ruLake UI does not consume RVF directly.
If a future "open the underlying segment" deep-link feature is
added, that becomes a `rvf-server` URL contract concern.

### 5.6 Gap summary

| Layer       | Small (MVP)                | Medium (v0.2)            | Large (v0.3+) |
|-------------|----------------------------|--------------------------|---------------|
| mcp-server  | list_collections, full-bundle resource, by-collection stats, CORS | audit_tail, reload_config | snapshot upload/download |
| ruLake      | none                       | none                     | per-collection consistency |
| rulake-wasm | none                       | streaming verify         | none          |
| node/http   | listResources, readResource, callTool, hints-on-429 | none      | none          |
| RVF         | none                       | none                     | none          |

The MVP requires roughly **150 LOC of mcp-server changes** and **50
LOC of `node/http.mjs` changes**. That is not free, but it is
small enough to land in one pull request.

---

## 6. Architecture

### 6.1 Process and file layout

A new `ui/` directory at the repo root. Self-contained Vite
project. Builds to `ui/dist/` as static assets. Two deployment
modes:

**Mode A — separate static host.** `ui/dist/` is served by any
static host (Cloudflare Pages, GitHub Pages, S3 + CloudFront,
`python -m http.server`). Talks to a remote `rulake-mcp` over CORS.
Suits the "I have a dev laptop, the server is in prod" case.

**Mode B — embedded in mcp-server.** mcp-server gains an
`--ui-dir <path>` flag that, when set, serves the directory at
`GET /` with a fall-through to `index.html` for SPA routes. Same
origin as the MCP wire, no CORS needed. Suits the
"single-binary, deploy as one unit" case. The pattern is exactly
what `rvf-server` already does
(`vendor/ruvector/crates/rvf/rvf-server/src/http.rs:42-46`,
`router_with_static`).

Recommendation: **Build for Mode A first.** It's simpler and works
for both the dev-against-remote and curated-demo personas. Mode B
can be added by wiring a `tower-http::services::ServeDir` into
`mcp-server/src/http.rs` later — ~20 LOC, no UI changes needed.

### 6.2 ASCII architecture

```
┌────────────────────────────────────────────────────────────────┐
│  Browser SPA                                                    │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │ Vite + Svelte 5                                          │  │
│  │ ┌────────────────┐  ┌────────────────────────────────┐   │  │
│  │ │ rulake/http    │  │ rulake-wasm                    │   │  │
│  │ │  - connect()   │  │  - verifyBundleJson            │   │  │
│  │ │  - query()     │  │  - computeWitness              │   │  │
│  │ │  - callTool()  │  │  - searchBruteForceL2 (future) │   │  │
│  │ │  - readResource│  │                                │   │  │
│  │ └────────────────┘  └────────────────────────────────┘   │  │
│  └──────────────────────────────────────────────────────────┘  │
│             │                                  │                │
│             │ HTTP + Streamable-HTTP MCP       │                │
│             │ (fetch + SSE)                    │                │
└─────────────┼──────────────────────────────────┼────────────────┘
              │                                  │ (offline,
              │                                  │  client-side
              │                                  │  crypto)
              ▼
┌────────────────────────────────────────────────────────────────┐
│  rulake-mcp (mcp-server)                                       │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │ http.rs ── auth / replay / rate-limit / sessions         │  │
│  │ server.rs ── tool router (rulake_query, _list_backends,  │  │
│  │              _publish_bundle, _refresh_…, _save_…,       │  │
│  │              _warm_…, _invalidate_…, _list_collections*) │  │
│  │ resources ── rulake://stats, …/by-backend,               │  │
│  │              …/bundle/{b}/{c}, …/by-collection*          │  │
│  │ planner.rs ── search/verify/explain/refresh              │  │
│  │ audit.rs ── JSONL append-only                            │  │
│  └──────────────────────────────────────────────────────────┘  │
│                              │                                  │
│                              ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │ rulake (host crate)                                      │  │
│  │  - RuLake::cache_stats / _stats_by_backend / _by_collection │
│  │  - RuLake::current_bundle / cache_witness_of / cache_entry_count │
│  │  - RuLake::search_one / search_federated / search_batch  │  │
│  └──────────────────────────────────────────────────────────┘  │
│                              │                                  │
│                              ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │ Backends: LocalBackend, FsBackend, gcs-backend,          │  │
│  │           ipfs-backend, …                                │  │
│  └──────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────┘

(*) marks server-side surface added for the UI per §5.1.
```

### 6.3 Auth layering

| Mode    | Browser? | Recommended use                       |
|---------|----------|---------------------------------------|
| none    | Yes      | Local dev only (`--bind 127.0.0.1`)   |
| bearer  | Yes      | Dev / demo. Token in `sessionStorage`.|
| JWT     | Yes      | Production. PKCE + IdP redirect for token acquisition; UI never sees the user's password. |
| mTLS    | **No**   | CLI clients only.                     |

For JWT in the browser: the UI must not handle the OAuth
authorization-code exchange itself unless an OAuth client_secret
is involved (it should not be — public clients use PKCE). The
flow:

1.  UI redirects to `https://idp.example.com/authorize?…&code_challenge=…`.
2.  IdP authenticates the user, redirects back with `?code=…`.
3.  UI exchanges the code at the IdP's `/token` endpoint for an
    access token containing `mcp:rulake:read` (or higher) scopes
    bound to `aud=https://rulake.example.com`.
4.  UI threads the token into `RuLakeHttp` constructor's `token`.
5.  Server validates the JWT per `mcp-server/src/auth.rs::JwtAuth`
    (RFC 8707 audience check, signature verification via JWKS).
6.  The token's `scope` claim drives the per-request
    `CapabilitySet` (`mcp-server/src/policy.rs::REQUEST_CAPS`),
    which intersects with the server-wide caps to gate
    `tools/call`.

The session-binding in `mcp-server/src/sessions.rs` records
(`session_id` → `principal`, `client_id`) on first sighting and
rejects any subsequent request with a different tuple — this means
a browser's `mcp-session-id` cookie cannot be replayed against a
different JWT. The UI should be aware that closing and re-opening
the tab loses the session id (kept in memory by `RuLakeHttp`); a
fresh `connect()` creates a new session, which is correct.

### 6.4 Witness verification flow (the differentiator)

The trust boundary is cryptographic, not network. A query response
flows:

1.  UI sends `tools/call rulake_query` over HTTPS.
2.  Server returns `{data, provenance: {witness: "abc…"}, decision}`.
3.  UI fetches the matching `rulake://bundle/{b}/{c}` resource.
4.  UI calls `verifyBundleJson(bundleJson)` — recomputes
    SHAKE-256(32) over the load-bearing fields locally.
5.  If the recomputed witness equals `provenance.witness`, the UI
    renders a green "verified" badge. The user has now proven the
    server served what it claimed without trusting TLS, the
    network, or any middleware.

This is the single most important UX in the entire UI. The
existing `examples/wasm/01-witness-verifier-browser/` is the
proof-of-concept; folding it into the connected app is the
"oh, this is different" moment for an evaluator.

### 6.5 Versioning and compatibility

The UI's `package.json` pins `rulake-wasm` and `rulake/http` (an
export from the `rulake` umbrella package per the brief) to the
ruLake repo's current version (`2.2.x`). The MCP wire protocol
version is `2025-03-26` per `node/http.mjs:14`; mcp-server speaks
the same (per ADR-004 §spec table). When `rmcp` bumps to a newer
spec, the UI's compatibility matrix needs an entry.

The UI's own version is independent. Recommend SemVer-zero
(`0.x`) for the first year; promote to `1.0` only when the screen
inventory and the UX have stabilised under real operator use.

---

## 7. Rejected alternatives

Each alternative is named, evaluated honestly, and weighed against
the proposed UI.

### 7.1 MCP Inspector

[`modelcontextprotocol.io/inspector`](https://modelcontextprotocol.io/inspector)
ships an Electron app (and a hosted version) that connects to any
MCP server, lists tools and resources, lets you call them with
JSON arguments, and renders the response. It is real, free, and
exists today.

**What it covers for ruLake:**

-   The "list tools" view. `rulake_query`, `rulake_list_backends`,
    and the publish/admin-tier tools all show up filtered by the
    server's effective caps.
-   The "call tool" view. Paste JSON arguments into `rulake_query`,
    see the response. This is most of §3.3 (the playground).
-   The "list resources" view. Shows
    `rulake://stats`, `…/by-backend`, and the
    `…/bundle/{b}/{c}` URIs (after they're populated).
-   Auth: bearer + a custom-headers field. JWT works as a bearer
    token. No mTLS.

**What it does not cover:**

-   No structured rendering of `decision.refusals` or
    `degraded_advice`. You see the raw JSON; you make sense of it
    yourself.
-   No client-side witness verification. The witness is just a hex
    string in the response.
-   No live cache-stats dashboard. You can read the resource, but
    every poll is a manual click.
-   No audit tail. (Neither does the proposed UI in v0.1, so this
    is a wash.)
-   No bundle viewer. You can `read_resource` on the bundle URI,
    but again — raw JSON, no field highlighting, no recompute.

**Verdict:** MCP Inspector covers the agent-developer playground at
maybe 60% of the proposed UI's quality. For a developer trying
ruLake out for the first time, it is genuinely sufficient. For an
operator running the server in production, it falls short on the
observability story (no live stats, no witness verification, no
audit). The UI's incremental value over MCP Inspector is mostly
in the dashboard + witness-verify pieces, not in the playground.

This argues for **shipping a UI that focuses on the dashboard +
verify story and explicitly defers the playground to "use MCP
Inspector if you prefer."** A pragmatic stance.

### 7.2 Grafana / Prometheus on `rulake://stats`

mcp-server already has `metrics-exporter-prometheus` budgeted in
ADR-004 §200. A small change emits the cache stats as Prometheus
counters/gauges. Then any Grafana dashboard renders them.

**Pros.** Zero UI work. Operators already run Grafana. Standard
alerting flows (Alertmanager) plug in for free. Long-term storage
(Cortex / Mimir / VictoriaMetrics) is solved.

**Cons.** No witness verification. No audit. No playground. Only
covers §3.5 (the stats dashboard).

**Verdict:** This is the **right call for the stats screen
specifically**, and the UI should not try to compete with it. The
proposed UI's stats screen is for "I'm logged into the UI and
want a glance"; Grafana is for "I want sustained alerting and
historical trend." Both make sense in different contexts. Recommend
landing the Prometheus exporter independently of the UI work
(it's a 50-LOC change in mcp-server) and letting the two coexist.

### 7.3 CLI-only — `rulake-cli` if it existed

A `rulake-cli` binary (it does not exist today as a separate
crate; the CLI surface lives in `mcp-server`'s
`cargo run --bin rulake-mcp`) that exposes all the management
verbs. Plus `jq` for filtering.

**Pros.** Terminal-first agent operators prefer this. Scriptable.
SSH-friendly. No CORS, no JWT-in-browser, no Vite build pipeline.

**Cons.** Cannot render a real-time dashboard. Cannot drag-drop a
file. Cannot show a sparkline. Cannot present
`decision.refusals[]` in a way that a non-Rust human visually
groks.

**Verdict:** **Build the CLI surface in parallel** to the UI, not
instead of it. The MVP for both is small: a `rulake-cli` binary
that wraps the same MCP HTTP transport (calling
`tools/call rulake_query` etc.) covers operator power-use; the UI
covers the dashboard + verify story. The audiences overlap but
the artefacts are different — neither replaces the other.

### 7.4 Embed inside Claude Desktop / Cursor as a custom tool

Since the MCP server already speaks MCP, Claude Desktop and Cursor
already know how to call its tools. The "UI" becomes the chat
window: "Claude, what's the cache hit rate?" → Claude calls
`rulake_query`, narrates the result.

**Pros.** Zero UI work. Works for any user already in those tools.
Natural language interface.

**Cons.** No structured visualisation. No drill-down. No
witness verification badge (Claude can read the witness aloud, but
cannot prove it). Latency: every glance is a model round-trip.
Cost: every glance is tokens.

**Verdict:** **Complementary, not a replacement.** This is real and
useful for "ask the agent about the system" — but a real operator
cannot operate a service through a chat window any more than a
real surgeon can operate through a chat window. The UI is for
direct manipulation; the chat is for one-off questions.

### 7.5 No UI — better READMEs and examples

The agentic ecosystem is API-first. UIs are second-class. Every
hour spent on the UI is an hour not spent on the v0.3 backend
features (cloud backends, OpenLineage, …) and on the M4
governance plane.

**Pros.** Maximum focus. The README + examples already cover the
agent-developer entry path. Every senior engineer the project
needs to recruit reads code, not dashboards.

**Cons.** Operators are not exclusively senior engineers. The
witness-verification story does not sell itself in prose. A
five-minute hosted demo is the difference between an evaluator
who tries it and one who closes the tab.

**Verdict:** Defensible if the team's bandwidth is the scarce
resource and there is no operator-persona demand. If anyone in
production has asked "is there a console?" the answer is no —
build the UI. If nobody has asked, this is a real option to
defer for two more releases.

### 7.6 Comparison table

| Alternative          | Covers stats | Covers playground | Covers verify | Covers audit | Build cost |
|----------------------|--------------|-------------------|---------------|--------------|------------|
| Proposed UI (MVP)    | Yes          | Yes               | Yes           | No (v0.2)    | 2–4 weeks  |
| MCP Inspector        | Partial      | Yes               | No            | No           | 0 (exists) |
| Grafana + Prometheus | **Yes (better)** | No            | No            | No           | ~1 day exporter |
| CLI-only             | Partial      | Partial           | Partial       | Yes (jq)     | ~1 week    |
| Embed in Claude      | Partial      | Yes (chat-shaped) | No            | No           | 0 (exists) |
| No UI                | No           | No                | No (cli only) | No           | 0          |

The honest read of this table: **the proposed UI's incremental
value over the union of MCP Inspector + Grafana + CLI is the
witness-verify dashboard and the unified panel.** That is real
value, but it is a smaller wedge than "we have no console at all"
makes it sound. The verdict in §10 weighs accordingly.

---

## 8. MVP — what we would actually build

### 8.1 Scope

Six screens (per §3.11):

1.  Connect / auth
2.  Backend / collection browser
3.  Query playground
4.  Bundle viewer
5.  Stats dashboard
6.  Decision-trace explainer (rendering layer in the playground)

Top-3 leverage rank (the screens that earn the UI its keep):

1.  **Stats dashboard.** The operator's daily view. Replaces a
    `curl + jq + watch` loop. Highest cost / hour saved.
2.  **Bundle viewer with cryptographic verify.** The
    differentiator. The single screen that demonstrates ruLake
    is unlike any other vector store an evaluator has seen.
3.  **Query playground.** The agent-developer entry path. Without
    it, every developer's first interaction with ruLake is a code
    repl instead of a UI.

### 8.2 Tech choices (locked)

-   **Vite 5+** as the build tool.
-   **Svelte 5** as the framework (with the React fallback noted
    in §4.2).
-   **Tailwind CSS** + **Bits UI** (Svelte port of Radix) for
    primitives.
-   **`rulake/http`** as the MCP transport.
-   **`rulake-wasm`** for browser-side cryptographic verify.
-   **TypeScript** throughout — `RuLakeHttp` already has a `.d.ts`
    (`node/http.d.ts`).

### 8.3 File layout

A new top-level `ui/` directory in the repo:

```
ui/
├── package.json          # vite, svelte, tailwind, rulake/http, rulake-wasm
├── vite.config.ts
├── tsconfig.json
├── tailwind.config.ts
├── postcss.config.cjs
├── index.html
├── src/
│   ├── main.ts           # bootstrap — init wasm, mount App.svelte
│   ├── App.svelte        # top-level layout: sidebar + outlet
│   ├── lib/
│   │   ├── client.ts     # RuLakeHttp singleton + auth state
│   │   ├── wasm.ts       # rulake-wasm init + verify wrapper
│   │   └── stores.ts     # Svelte stores for stats / sessions
│   ├── routes/
│   │   ├── connect/      # 3.1
│   │   ├── browse/       # 3.2
│   │   ├── playground/   # 3.3 + 3.9
│   │   ├── bundle/       # 3.4
│   │   └── stats/        # 3.5
│   └── components/
│       ├── DecisionTrace.svelte
│       ├── WitnessBadge.svelte
│       ├── HitTable.svelte
│       └── Sparkline.svelte
├── public/
│   └── (favicon, etc.)
└── README.md             # how to run dev / build for prod
```

Total target: **~3,000 LOC TypeScript + Svelte**. Comparable to
the existing `examples/wasm/01-witness-verifier-browser/` (~400
LOC) scaled up by the screen count.

### 8.4 Build pipeline

-   **Dev.** `pnpm dev` (Vite serves on `:5173`). The dev server
    proxies `/mcp` to the configured MCP server origin (defaults to
    `http://127.0.0.1:8788`) so CORS is irrelevant during dev.
-   **Production build.** `pnpm build` → `ui/dist/` (~200 KB
    compressed total: ~50 KB Svelte+app, ~150 KB
    rulake-wasm).
-   **Distribution.** Two paths:
    1.  Static host: upload `ui/dist/` to GitHub Pages /
        Cloudflare Pages / S3. Configure the UI to talk to a remote
        MCP server URL entered at runtime.
    2.  Embedded: mcp-server gains `--ui-dir <path>`. Deploy as
        one binary. Pattern from
        `vendor/ruvector/crates/rvf/rvf-server/src/http.rs:42-46`.
-   **Docker.** Optional `Dockerfile.ui` that builds and serves
    the SPA via nginx — stretch goal, not v0.1 work.

### 8.5 Public URL surfaces

The UI itself: `https://rulake-ui.example.com/` (or
`https://rulake.example.com/ui/` if same-origin).

The UI calls these MCP endpoints:

-   `POST {mcp_url}` (e.g. `https://rulake.example.com/mcp`) for
    `initialize`, `tools/call`, `tools/list`, `resources/list`,
    `resources/read`.
-   On 401: redirect to login (bearer paste / JWT IdP redirect).
-   On 429 with `Retry-After` and a JSON body: render the
    `degraded_advice.hints` to the user as actionable prompts
    ("reduce k", "narrow target", …).

### 8.6 Realistic timeline (engineering days, single engineer)

| Phase                            | Days    |
|----------------------------------|---------|
| Vite + Svelte + Tailwind + wasm scaffold | 1   |
| Connect screen + RuLakeHttp wiring | 1     |
| Stats dashboard (one polling loop, two resources) | 2 |
| Bundle viewer (drag-drop reuse + cached-bundle fetch) | 2 |
| Backend / collection browser     | 1       |
| Playground (search intent only)  | 3       |
| Decision-trace component         | 2       |
| Polish, docs, accessibility pass | 2       |
| **Total**                        | **14 days** |

Plus **~1 day** of mcp-server changes from §5.1 (small bucket only).

A 3-week single-engineer project, or a 1-week pair-programming
sprint. **Not free, but not scary.**

### 8.7 Rollout

-   Ship in the **`v0.3`** release of `rulake-mcp` (next minor
    after the current `v0.8` of mcp-server, which lives at
    repository version `2.2.x`).
-   Documented in two places: a new `ui/README.md` and a section
    in the top-level `README.md` linking the hosted demo.
-   The hosted demo runs against a curated `LocalBackend` with a
    seed of synthetic vectors (no real user data, no real
    secrets, scope-`read` JWT pre-baked in the URL).
-   The README's screenshot collection gets a refresh.

---

## 9. What we would NOT build (even if the answer is yes)

Out-of-scope for the v0.1 UI, with reasoning:

-   **RBAC mutation.** `mcp.toml` is config-file-only. Editing it
    in a UI requires a config-reload tool, which is medium work,
    and a careful UX around "you are about to revoke your own
    `read` cap" guard rails. Defer to v0.2 or v0.3. Operators
    can edit the file and restart the server today.
-   **Snapshot upload over HTTP.** The mcp-server tools take
    server-local paths. Adding HTTP file transfer is non-trivial
    (path traversal, content-length, chunking, capability gating).
    The CLI + scp + volume mount is fine for v0.1.
-   **Federation builder.** Drag-and-drop topology builder is the
    most visually impressive screen and the deepest rabbit hole.
    The math is in Rust today (`src/lake.rs`); re-implementing
    "preview k' = max(5, global_k / num_routes)" in JS introduces
    drift risk. Ship with a "just edit the routes JSON" textarea
    for v0.1.
-   **RVF segment browser.** Out of scope per §4.6. Defer to
    `rvf-server`'s own dashboard; deep-link from the bundle
    viewer if useful.
-   **End-user "personal memory inspector."** Cognitum chip
    territory. Not ruLake's job.
-   **Multi-tenant management.** Operators today run one
    `rulake-mcp` per tenant. Multi-tenant management is an
    ADR-shaped problem, not a UI problem.
-   **Audit log viewer.** Conditional: if the
    `rulake_audit_tail` tool lands cheap, fold this into v0.1; if
    not, defer.
-   **Real-time streaming audit (SSE).** The MCP wire is request-
    response with SSE for streaming responses; a server-to-client
    audit stream lives outside that contract. Defer.
-   **Plugin marketplace, per-user themes, custom dashboards.**
    All standard "no-engineer-asked-for-this" features. Refuse
    by default.

---

## 10. Verdict

**Conditional yes.** Build the UI **if and only if** at least one
of these is true:

1.  The operator persona (§2.1) has asked for a console, in writing,
    in the last 90 days. (If it is hypothetical, the priority is
    less than the v0.3 cloud-backend work.)
2.  The agent-developer persona (§2.2) is bottlenecked on
    onboarding friction in measurable ways (e.g. evaluators close
    the tab before running their first query).
3.  The team has 3 engineer-weeks of capacity in the next quarter
    that is not better spent on cloud backends, OpenLineage, or
    the `mcp-brain` integration.

**If the answer is "yes" the next concrete commit is:**

```
mcp-server v0.9 — list_collections tool + full-bundle resource
```

That is the smallest server-side change needed to unblock the UI's
backend browser and bundle viewer. ~150 LOC, one PR, lands
independently. The UI is then a separate workstream.

**If the answer is "no" the next concrete non-UI improvement is:**

```
mcp-server v0.9 — Prometheus exporter + audit-tail tool
```

Both are smaller than the UI, both are higher-leverage for the
operator persona, and both are precursors that the UI would build
on later anyway.

The reasoning, in one paragraph: **the proposed UI is genuinely
useful but its incremental value over (MCP Inspector + Prometheus
+ CLI) is concentrated in two screens — the bundle-witness viewer
and the unified-panel UX. Those are real but small wedges. The
team's hour is better spent on cloud backends and the M4 governance
plane unless an actual operator has asked for a console.** When in
doubt, defer; the substrate matters more than the chrome.

The shape of the UI proposed in §3 and §6 is the right shape if
this work happens. The "what to build" is settled; the "whether to
build now" is the only live question.

---

## 11. Open questions

1.  **Has any actual ruLake operator asked for a console?** This is
    the only question that turns "conditional yes" into "yes."
    Before committing engineering time, do a quick survey of the
    five known production-ish deployments.
2.  **Is the MCP Inspector good enough for the agent-developer
    persona today?** Run the inspector against a `rulake-mcp`
    deployment and have a real agent developer try it. If they
    can be productive in 30 minutes, the playground screen is
    less urgent than this note assumes.
3.  **What is the MCP Inspector's roadmap on structured rendering
    for typed responses?** If it is gaining a JSON-Schema-driven
    custom renderer surface, the
    `decision_trace` problem solves itself for free. (As of
    `2025-11-25` this is not on the published roadmap; recheck
    quarterly.)
4.  **Is JWT + PKCE actually viable in a browser when scopes are
    `mcp:rulake:*`?** RFC 8707 audience constraints work; what we
    do not know empirically is whether the dominant IdPs (Auth0,
    Okta, Keycloak) handle the resource indicator in their
    discovery + token-exchange flows cleanly. Worth a one-day
    spike before committing the JWT auth path in the UI.
5.  **Does anyone want to surface `ruvector-graph` / `ruvector-gnn`
    visualisations in the same UI?** Per §4.7 the answer for ruLake
    is no, but the question may surface from RuVector consumers.
    The honest answer is "build a separate RuVector explorer if
    that need is real."
6.  **Is RVF's public surface stable enough to surface in the UI?**
    Per §4.6 the answer is "not yet for ruLake." But the question
    will recur as RVF stabilises. The first time the answer is
    yes is when the bundle viewer wants a "show underlying RVF
    segment" deep-link.
7.  **What is the size budget for the wasm payload?** `rulake-wasm`
    is ~149 KB compressed. If the UI ever wants to add
    `ruvector-graph-wasm` (graph rendering) or `rvf-wasm`
    (segment inspection), the budget gets tight quickly. A 1 MB
    initial-payload budget is the soft cap; over that and we
    code-split per route.

---

*End of note. This document is one engineering judgement, not a
spec. The recommended next action is to answer Q1 above before any
code is written.*
