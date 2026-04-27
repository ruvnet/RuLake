# ruLake MCP server — A Deep Introduction

## TL;DR

The ruLake MCP server is a Rust-native binary (`rulake-mcp`, crate `ruvector-rulake-mcp` at `crates/mcp-server/`) that exposes the in-process `RuLake` cache as a Model Context Protocol surface — eight tools at v0.9, scoped through a four-tier capability model (`read` / `publish` / `admin` / `internal`) that intersects server-wide CLI flags with per-token JWT scopes on every call. It speaks stdio (Claude Desktop / Cursor / Cline / Inspector — parent-process trust) and Streamable HTTP (the spec-current 2025-11-25 remote transport) from the same binary, runs every `RuLake::*` call on a bounded rayon worker pool with structured backpressure, and emits a JSONL audit row plus a 256-entry in-memory tail buffer for every tool invocation regardless of outcome. The server is the prototype consumer for ADR-155 §M4 governance and the load-bearing claim is that no tool returns data when the witness check fails.

## Introduction

The Model Context Protocol ([modelcontextprotocol.io](https://modelcontextprotocol.io)) is the dominant agent-to-tool wire as of mid-2026. Spec revisions to date are `2024-11-05` (retired), `2025-03-26` (introduced Streamable HTTP, deprecated HTTP+SSE), `2025-06-18`, and `2025-11-25` (current). Claude Desktop, Cursor, Continue, Cline, agentic-flow, OpenAI Apps, Replit Agent, and a long tail of internal agents all consume it. If ruLake wants to be agent-callable retrieval, MCP is the wire. The repo already carried prior art — a TypeScript MCP server at `examples/nodejs/04-mcp-tool/src/server.ts` exposing three tools (`rulake_search`, `rulake_verify_witness`, `rulake_bundle_info`) over stdio against a snapshot directory — and ADR-004 is honest in §Context that the demo is deliberately small (brute-force exact L2 because the Node side has no RaBitQ decoder; one-shot demo shape; no auth; no audit). It works as a developer demo; it is not the production surface.

The production-shaped MCP server has to satisfy four things the demo does not. The first is to **speak full ruLake**, not a snapshot subset — the agent should reach `search_one` / `search_federated` / `search_batch` against a *running* `RuLake` with live backends, in-process, so the 1.02× cache-hit tax from `BENCHMARK.md` survives the wire. A separate-process HTTP hop between MCP and ruLake reintroduces the 1–5 ms RVF-wire round-trip cost that the cache-first design is built to avoid. The second is to **run remote**, not just stdio — a ruLake instance on a backend host should be reachable from agents on other hosts (developer laptops, serverless functions, the Cloudflare-deployed agent), and MCP's 2025 transport revisions are explicit that remote is Streamable HTTP, not SSE. The third is to **refuse to be the prompt-injection vector**. The 2025 MCP CVE timeline is harsh — ADR-004 §Context cites CVE-2025-6514 (mcp-remote OS command injection), CVE-2025-53107 (`git-mcp-server` shell injection), CVE-2025-53818 (Kanban MCP command injection), CVE-2025-68143/68144/68145 (`mcp-server-git` path traversal + arg injection), CVE-2025-6515 (mcp prompt-hijack via session ID), and Anthropic's own `git-mcp-server` prompt-injection-via-tool-description from January 2026. Every one of those started with "the MCP author trusted something the agent fed them." The fourth is to **survive the M4 governance landing** — ADR-155 §M4 ships RBAC, PII, lineage, and audit through the cache layer, and the MCP server is the first place those primitives are surfaced to a real wire.

Spec status is concrete. The server targets `2025-11-25` (current) with backwards-compat to `2025-03-26` clients via the spec-defined fallback handshake; the `2024-11-05` HTTP+SSE transport is dropped outright (deprecated, materially worse posture). The library is `rmcp` 1.5 — the official Rust SDK from `modelcontextprotocol/rust-sdk`, tokio-based, with stdio + Streamable HTTP transports and OAuth helpers. ADR-004 §1 walks the alternatives: hand-rolled JSON-RPC against `serde_json` is rejected because the spec is at four revisions in 18 months and tracking it manually means owning every spec change as an engineering ticket; Neon-style or third-party MCP libraries (`mcp-sdk`, `mcpr`, `agenterra-rmcp`) are rejected as smaller user bases or obsolete forks. The official SDK is ~2k LOC of glue we would otherwise have to write, audit, and maintain.

The deeper reason the MCP server matters is structural: it is the first place ruLake stops being a library and becomes a control plane. A library-shaped piece of software has one user (the calling process) and one trust boundary (the function call). A control plane has multiple agent callers, multiple human operators, multi-tenant collections, capability flags, OAuth scopes, audit logs, rate buckets, and a planner that decides what to do with each request. ADR-004 ratifies the control-plane shape — the read-only-by-default posture, the capability gate, the witness-fail-closed contract, the bounded worker pool with structured backpressure — and the rest of the file is the working-out of those commitments against the MCP-spec and 2025-CVE realities.

## The decision in detail

ADR-004 makes nine numbered milestone decisions through versions v0.1 → v0.9. The load-bearing four are the library choice, the crate placement, the capability-gated tool surface, and the security posture (OAuth 2.1 + PRM + RFC 8707 Resource Indicators on the HTTP path).

The first is **`rmcp` 1.x as the wire library**. ADR-004 §1 walks the field — `rmcp` is the official SDK, tracks the spec, ships `rmcp-macros` for `#[tool]` ergonomics — and rejects the alternatives. The crate dependencies live in `crates/mcp-server/Cargo.toml:39`-area: `rmcp = { version = "1.5", default-features = false, features = ["server", "transport-io", "transport-streamable-http-server", "macros", "schemars"] }` plus the supporting stack (`hyper 1`, `tower 0.5`, `tokio 1.39`, `governor 0.7` for layered rate buckets, `flume 0.11` + `rayon 1.10` for the bounded worker pool, `jsonwebtoken 9` for RS256/ES256 JWT validation in v0.5, `tokio-rustls 0.26` for mTLS in v0.6).

The second is **crate placement as a sibling at `crates/mcp-server/`**, not `bin/rulake-mcp` inside the main crate and not `crates/rulake-mcp/` as a workspace member. ADR-001 explicitly rejects a root workspace, and ADR-004 §2 rejects the bin-inside-main-crate option because it would force every `cargo add rulake` consumer to pull tokio + hyper + rustls + rmcp into their dep graph (~50 transitive deps; build doubles for serving-process tooling no library consumer needs). The sibling-crate pattern mirrors `python/` and `node/` and consumes the parent crate via `rulake = { path = "..", version = "2.2.0" }` (`crates/mcp-server/Cargo.toml:33`) under the same submodule discipline ADR-001 commits to.

The third is the **capability-gated tool surface**. The four-tier model is `read` / `publish` / `admin` / `internal` (ADR-004 §4b), with the load-bearing intersection rule: server-wide CLI flags from `--capabilities` × per-token JWT scopes are intersected at request time and that intersection gates both `tools/list` visibility and per-call `require_cap` enforcement. The eight tools that ship at v0.9 (per ADR-004 §Status table and `crates/mcp-server/src/server.rs:613`-area `required_cap_for_tool`):

| tool | tier | wraps |
|---|---|---|
| `rulake_query` | read | search/verify/explain/refresh intents (the public decision-layer tool) |
| `rulake_list_backends` | read | enumerate registered backend ids |
| `rulake_list_collections` | read | per-backend collection list (NEW v0.9) |
| `rulake_publish_bundle` | publish | atomic write of `table.rulake.json` |
| `rulake_refresh_from_bundle_dir` | publish | three-state refresh (`up_to_date` / `invalidated` / `bundle_missing`) |
| `rulake_save_cache_to_dir` | admin | snapshot to disk |
| `rulake_warm_from_dir` | admin | restore from disk (returns vector count) |
| `rulake_invalidate_cache` | admin | drop pointer (substrate forget) |

`tools/list` filters by the request's effective `CapabilitySet`. A read-only caller sees three tools (`rulake_query`, `rulake_list_backends`, `rulake_list_collections`); admin sees all eight. The `tool_handler` macro at `crates/mcp-server/src/server.rs:712`-area emits the `ServerHandler` impl whose `list_tools` method (line 745) walks `tool_router.list_all()`, filters by `effective.has(required_cap_for_tool(&t.name))`, and returns the visible subset.

The fourth is the **security posture**. stdio has no auth (parent-process trust); Streamable HTTP has mandatory auth (bearer / OAuth 2.1 / mTLS, in priority order). The OAuth scope→capability map is fixed in v1: `mcp:rulake:read` → `Capability::Read`; `mcp:rulake:publish` → `Capability::Publish` (also requires Read); `mcp:rulake:admin` → `Capability::Admin` (also requires Publish + Read). Replay protection is layered: the per-request `MCP-Request-Id` nonce window (last 10 k seen, evicts oldest, replays inside the window rejected with HTTP 409); session binding to `(principal, client_id, mTLS-cert-fingerprint)` so a stolen token + new client cannot inherit an open session. v0.5 added RS256/ES256 JWT support with JWKS hot rotation (commit `0c3801c`). v0.6 added mTLS plus the `rulake://bundle/{backend}/{collection}` resource (commit `d100073`). v0.8 added per-call `CapabilitySet` from JWT scopes and HTTP backpressure (commit `67fc821`). The full per-version table lives in ADR-004 §Status.

| trade-off | what got picked | what got rejected | why |
|---|---|---|---|
| library | `rmcp` 1.x official SDK | hand-rolled JSON-RPC, third-party libs | spec at 4 revisions in 18 months; first-party tracks. |
| crate placement | sibling at `crates/mcp-server/` | `bin/` in main crate, workspace member | bin-in-main forces tokio/hyper/rustls into every library consumer; workspace forbidden by ADR-001. |
| transports | stdio + Streamable HTTP, no SSE | HTTP+SSE legacy | SSE deprecated since 2025-03-26; CVE-2025-6515 attacked the two-endpoint shape. |
| public tool | `rulake_query` (intent-shaped) | exposing 8 low-level kernel tools | "MCP is the decision layer, not a thin transport"; agent gets one tool, ops sees the kernel under `internal`. |
| auth | OAuth 2.1 + PRM + RFC 8707, plus bearer (dev-only) and mTLS | OAuth 2.0, DCR (Dynamic Client Registration), no auth | spec mandates 2.1 patterns; DCR enlarges attack surface. |
| concurrency | bounded `rayon::ThreadPool` + `flume` channel of size `--max-inflight` | unbounded `tokio::task::spawn_blocking` | unbounded saturates global blocking pool under burst; control plane needs explicit bounds. |
| audit | JSONL file + 256-entry in-memory tail ring | metrics-only | every tool call must explain itself for ADR-155 §M4. |

## Capabilities

The capability surface is the eight tools above plus three resources and the structured decision-layer response shape that `rulake_query` returns.

`rulake_query` is the public face. It takes an intent (`search` / `verify` / `explain` / `refresh`), a target (`{collection}` shorthand, `{routes: [[backend, collection], ...]}` for federated, or `{backends: [...]}`), per-intent argument blocks (`search.vector` + `k`, `verify.bundle_dir`, etc.), and policy knobs (`risk: low|medium|high`, `freshness_ms`, `budget.max_latency_ms`, `policy.witness_required`, `policy.allow_partial`, `policy.min_collections_hit`). The response carries `data` plus a `provenance` block (`witness`, `witness_verified`, `consistency`, `served_from`, `lineage_id`), a `trust_level` field (`verified` / `unverified` / `partial`), and a `decision` block (`chosen_action`, `reason_code` from a closed enum, `backends_planned` / `backends_used`, `consistency_used`, `budget_used_ms` / `budget_cap_ms`, `degraded`, `refusals`). The closed `reason_code` enum (ADR-004 §4a) covers `CACHE_HIT_FRESH`, `CACHE_HIT_EVENTUAL`, `STALE_CACHE_REMOTE_VALID`, `COLD_PRIME_THEN_SERVE`, `WITNESS_MISMATCH_REFUSED`, `BUDGET_EXCEEDED_FALLBACK_CACHE`, `BUDGET_EXCEEDED_REFUSED`, `POLICY_REFUSED_RISK`, `POLICY_REFUSED_ALLOWLIST`, `POLICY_REFUSED_PATH`, `PARTIAL_FEDERATION` — new values bump the ADR but never the wire schema, so machine consumers can pin alert filters.

The three MCP resources (URI-addressable read-only data, registered under the `rulake://` scheme):

| URI | source | notes |
|---|---|---|
| `rulake://stats` | `RuLake::cache_stats` | rollup: hits, misses, primes, hit_rate, avg_prime_ms |
| `rulake://stats/by-backend` | `cache_stats_by_backend` | per-backend cache stats |
| `rulake://bundle/{backend}/{collection}` | `cache_witness_of` (cheap path) | live witness for the (backend, collection); never invokes the default `current_bundle` impl that would do a full pull |
| `rulake://audit/tail` | `AuditSink::tail(256)` | last ≤256 audit lines as JSON array (v0.10, closes ADR-006 server-gap §V #2) |

The bundle resource is load-bearing for browser callers — the Console's Audit and Bundles screens read it without needing audit-file paths or shell access. The cheap-path discipline (read the cache pointer, never trigger the default `current_bundle` impl that would pull from the backend) is enforced at `crates/mcp-server/src/server.rs:888`-area.

A worked example. An agent uses Claude Desktop's MCP client to query a ruLake exposed over stdio:

```json
{
  "tool": "rulake_query",
  "intent": "search",
  "target": { "routes": [["local", "memories"], ["fs-prod", "memories"]] },
  "search": { "vector": [/* f32, len=128 */], "k": 10 },
  "risk": "medium",
  "freshness_ms": 5000,
  "budget": { "max_latency_ms": 50, "max_backends": 3, "max_results": 20 },
  "policy": { "witness_required": true, "allow_partial": false, "min_collections_hit": 1 }
}
```

The planner walks the request, consults `cache_stats_by_collection`, picks `search_federated` over `local + fs-prod`, runs the call on the bounded worker pool. The response carries the 10 hits in `data`, plus a decision block whose `reason_code: STALE_CACHE_REMOTE_VALID` and `reason: "cache stale on local backend; witness valid on fs-prod"` lets the calling agent log and adapt. The audit row at `crates/mcp-server/src/audit.rs::AuditEntry` records the full shape — `ts`, `transport: "stdio"`, `principal: "stdio:local"`, `tool: "rulake_query"`, `intent: "search"`, `outcome: "ok"`, `result_size: 10`, `trust_level: "verified"`, `duration_ms: 1.7`, `policy_decision: { capability_required: "read", capability_granted: ["read"] }`, `decision: { ... }` — and tees a copy into the 256-entry tail ring (`crates/mcp-server/src/audit.rs:111`-area) before writing to the file or stderr sink.

## Trust & correctness contract — JWT-scope → CapabilitySet gate

The trust contract for ADR-004 is the JWT-scope-to-CapabilitySet gate. Every tool call passes through the same dual enforcement:

`crates/mcp-server/src/server.rs:602` (`require_cap`) is the per-call gate. It calls `crate::policy::effective_caps(caps)` (the server-wide × per-request task-local intersection) and `effective.require(required)`, returning `McpError::invalid_request` on refusal. The same `effective_caps` is consulted by `list_tools` at `crates/mcp-server/src/server.rs:745`-area, so an agent whose JWT only grants `read` sees only read-tier tools even on a server started with `--capabilities admin`. An adversary who guesses tool names cannot bypass the visibility filter to call them — the per-call gate fails the request even if `tools/list` were spoofed.

The single source of truth for tool→capability mapping is `required_cap_for_tool` at `crates/mcp-server/src/server.rs:613`. The default for any unknown tool is `Capability::Internal`, which is invisible until `--capabilities internal` is granted; ADR-004 §4b explicitly notes "Safer than defaulting to Read." The mapping is one place; both the visibility filter and the per-call gate consult it.

The audit-symmetry property (closed by commit `56b497b`, R-MCP-1 from `docs/research/security/shipping-substrates-v2.md`) is the second arm of the contract. Before the fix, `rulake_query` emitted a fully-shaped `AuditEntry` with `PolicyDecision` on every outcome (`ok` / `refused` / `degraded` / `error`) but the five mutation handlers (`rulake_publish_bundle`, `rulake_refresh_from_bundle_dir`, `rulake_save_cache_to_dir`, `rulake_warm_from_dir`, `rulake_invalidate_cache`) emitted nothing — operators had no audit evidence for cache-mutation activity. The fix at `crates/mcp-server/src/server.rs:551`-area is a private `audit_mutation` helper that inspects a `Result<T, McpError>`, derives `outcome` (`ok` / `degraded` / `error` / `refused`) and `code` (`RULAKE_DEGRADED` / `RULAKE_INTERNAL` / `CAP_DENIED`) from the error message prefix, and emits a fully-shaped `AuditEntry`. Every mutation handler wraps its body in an async block and calls the helper exactly once before returning. The audit log is now symmetric across read and write paths.

The witness-fail-closed posture is non-negotiable. Every tool that touches an on-disk bundle (`rulake_publish_bundle`, `rulake_refresh_from_bundle_dir`, `rulake_warm_from_dir`, the `rulake://bundle/...` resource) propagates `RuLakeBundle::read_from_dir`'s witness verification (`crates/core/src/bundle.rs:340`-area). A `witness_verified: false` response is never silently served — the tool returns `isError: true` with the expected and actual witnesses in the body. The 256-entry tail ring at `crates/mcp-server/src/audit.rs:31` (`TAIL_CAPACITY`) tees every emit through `crates/mcp-server/src/audit.rs:111`-area before writing to the file/stderr sink, so the resource snapshot reflects what was *attempted* even if the file write failed (lock poisoned, disk full).

The bench gate (commit `8ce3689`, `crates/mcp-server/benches/audit_sink.rs` and `crates/mcp-server/benches/tools_list_filter.rs`) measures the contract's hot paths. Headline numbers from ruvultra (criterion v0.5): audit emit at the 256-entry ring boundary = 1.27 µs (push-only vs push-pop); tools/list capability filter at 1/2/3 server-wide caps = 400-570 ns; `CapabilitySet::from_csv` at 1/8/64 tokens stays bounded. The numbers exist so a regression ("the audit sink got 10× slower") is a CI-detectable degradation, not a tail-latency surprise.

## Reference implementation status

The crate `ruvector-rulake-mcp` v0.9 lives at `crates/mcp-server/`. The server has shipped through nine numbered milestones (`docs/adrs/sdk/ADR-004-rulake-mcp-server.md:14`-area):

| version | commit | headline |
|---|---|---|
| v0.1 | `3bcd237` | ADR-004 skeleton, decision-layer first |
| v0.2 | `488e36c` | Streamable HTTP + bearer auth |
| v0.3 | `2fc675c` | verify/explain intents + stats resources + CI |
| v0.4 | `efba70d` | RBAC + JWT + rate limit + replay; `tools/list` capability filter |
| v0.5 | `0c3801c` | RS256/ES256 + JWKS hot rotation + session binding |
| v0.6 | `d100073` | mTLS + `rulake://bundle` resource + IPFS-aware verify |
| v0.7 | `428575b` | `RuLake::current_bundle` accessor + structured backpressure |
| v0.8 | `67fc821` | per-call `CapabilitySet` from JWT scopes + HTTP backpressure |
| v0.9 | `26dbe2b` + `5b956a9` | `rulake_list_collections` tool + CORS layer for browser callers (closes ADR-006 server-gap §V #1) |

What ships at v0.9:

- Eight MCP tools per the table above; `#[tool]` macros decorate the methods on `RuLakeMcpServer` at `crates/mcp-server/src/server.rs:191`-area.
- Three rollup resources (`rulake://stats`, `rulake://stats/by-backend`, `rulake://audit/tail` from v0.10 commit `e2c2402`) plus per-(backend, collection) `rulake://bundle/...` resources synthesised from `cache_stats_by_collection` (`crates/mcp-server/src/server.rs:805`-area).
- stdio transport (`crates/mcp-server/src/server.rs:179`-area) plus Streamable HTTP (`crates/mcp-server/src/http.rs`).
- Auth modes: bearer (constant-time compare via `subtle`, dev-only on public bind), OAuth 2.1 / JWT with RS256/ES256 (`auth.rs`, `jwks.rs`), mTLS (`mtls.rs`). Replay protection in `replay.rs` and `sessions.rs`. Layered rate buckets via `governor` in `ratelimit.rs`. Bounded worker pool in `workers.rs` (`flume` channel of size `--max-inflight` over `rayon::ThreadPool` of size `cores * 2`).
- JSONL audit sink with 256-entry tail ring (`crates/mcp-server/src/audit.rs:31`). Audit-symmetry fix from commit `56b497b` (R-MCP-1) is in place — every mutation handler calls `audit_mutation` (`crates/mcp-server/src/server.rs:551`).
- Tests at v0.9: 37 unit + 8 e2e (HTTP) + 1 ignored (rmcp SSE-keepalive, commit `fdc2aee`) + 20 smoke + 1 doc-test = **65 passing, 1 ignored** per ADR-004 §Status.
- Criterion benches added in commit `8ce3689`: `audit_sink.rs`, `tools_list_filter.rs`.

What v0.9 does *not* ship, per ADR-004 §Open questions:

- The internal-kernel tool tier on the wire (`rulake_search_one`, `rulake_search_federated`, etc.) — these are the kernel `rulake_query` composes over, exposed only when `--capabilities internal` is granted (operator-only, no OAuth scope).
- Dynamic Client Registration (DCR, RFC 7591) — explicitly rejected for v1 (Alternatives §I); enlarges attack surface, anonymous client registration was the chain in early MCP gateway exploits.
- Resource subscriptions (notify on change) — rejected for v1; per-mutation notifications would flood every connected agent.
- MCP prompts — rejected for v1; every prompt the server ships is content the calling agent receives as authoritative.
- JS / Python tool plugins — rejected for v1; plugin loading is its own threat model.
- `npx`-style fetch-and-run distribution — explicitly rejected (§8); exactly the supply-chain shape `mcp-remote` had when CVE-2025-6514 landed.

The acceptance gates (ADR-004 §Verification "Acceptance benchmarks") are three load-shaped tests on a 100 k × 128 ruLake against a 60 s mixed workload (90% search, 8% verify, 2% explain) at 32 concurrent agents. Latency under load: p95 increase < 20%. Multi-tenant isolation: two agents on disjoint collections, one at 10× the rate of the other, B's p95 must not degrade > 25% relative to its solo baseline. Decision trace: every audit line explains itself, zero "we don't know why we did this" lines.

## Composition with the rest of ruLake

The MCP server is the integration point for the rest of the ADR set.

**The cache and federation primitives are the kernel.** `rulake_query` composes over `RuLake::search_one`, `search_federated`, `search_federated_with_rerank`, `search_batch` (`crates/core/src/lake.rs:521`). The planner picks the cheapest plan inside the budget; the worker pool runs the call; the audit row records the decision. Every hit is annotated with `_provenance: { backend, collection, witness }` so the calling agent has the metadata to apply its own sandboxing on tool outputs.

**The bundle and witness contract is honoured.** Every tool touching an on-disk bundle propagates the witness-fail-closed posture from `crates/core/src/bundle.rs:340`-area. The `rulake://bundle/{backend}/{collection}` resource reads through `cache_witness_of` (cheap path) and never triggers the default `current_bundle` impl. ADR-005's IPFS backend produces bundles whose witnesses the MCP server can verify without fetching the CIDs — the witness recipe is purely a function of bytes.

**The SDKs share the underlying `RuLake`.** A long-lived Node server (ADR-003) or Python process (ADR-002) can construct a `RuLake`, register backends, and let the in-tree MCP server expose those same backends to a Claude Desktop / Cursor / Cline client. Same `Arc<RuLake>`, two language entry-points, one cache.

**The Console (ADR-006) consumes the audit-tail resource.** The 256-entry ring at `crates/mcp-server/src/audit.rs:31` powers the Console's Audit screen in live mode (commit `e2c2402`). Browser callers reach it through the CORS layer added in v0.9 (commit `5b956a9`).

**The substrate scaffolds ride the shared schema.** ADR-007's rvDNA and ADR-008's ruQu backends ship companion MCP servers (`crates/mcp-rvdna/` from commit `a66d65f`, `crates/mcp-ruqu/` from commit `6d60cf7`) sharing the audit-row schema through a shared `audit-only` Cargo feature. Disjoint code prefixes (`RULAKE_*`, `RVDNA_*`, `RUQU_*`) let one log stream serve all three.

The MCP server is the prototype consumer of the M4 governance primitives. The audit JSONL maps onto OpenLineage's `RUN` events without field rename so the M4 work is "swap the sink", not "redesign the event shape". Capability flags compose multiplicatively with OAuth scopes and token TTL to give the progressive trust model — read is permanent and broad, publish is short-TTL and per-collection-scoped, admin is short-TTL and audit-flagged.

## Open questions

DCR (Dynamic Client Registration, RFC 7591) is explicitly v1.5 with a per-IdP allow-list — pre-registered clients are the v1 default but a self-serve agent customer may force the question. The `internal` capability tier is operator-only by design (no OAuth scope); the rationale may relax if a real ops automation needs a non-human-bearer-token path. Resource subscriptions are rejected today on flood-risk grounds; a real consumer (e.g. agentic-flow watching for cache invalidation) would reopen. The `examples/nodejs/04-mcp-tool/` TS demo is now legacy and stays in-tree as a JS-only reference; eventual retirement is v1.5 if no user pushes back. Finally, the asymmetry between read-tier audit shape (`rulake_query` always emits) and mutation-tier shape (`audit_mutation` helper) is correct but invites a consolidation pass — a single emit pipeline keyed off a `ToolDescriptor` — that v1.0 hardening will revisit.

## References

- ADR-004: `/home/ruvultra/projects/RuLake/docs/adrs/sdk/ADR-004-rulake-mcp-server.md`
- Crate manifest: `/home/ruvultra/projects/RuLake/crates/mcp-server/Cargo.toml`
- Server impl: `/home/ruvultra/projects/RuLake/crates/mcp-server/src/server.rs`
  - `RuLakeMcpServer` struct and constructors: `crates/mcp-server/src/server.rs:38`, `:62`
  - `rulake_query` tool: `crates/mcp-server/src/server.rs:191`
  - `rulake_list_backends` and `rulake_list_collections` (v0.9): `crates/mcp-server/src/server.rs:318`, `:329`
  - mutation tools (publish/admin): `crates/mcp-server/src/server.rs:365`-area
  - `audit_mutation` helper (R-MCP-1 fix, commit `56b497b`): `crates/mcp-server/src/server.rs:551`
  - `require_cap` per-call gate: `crates/mcp-server/src/server.rs:602`
  - `required_cap_for_tool` single source of truth: `crates/mcp-server/src/server.rs:613`
  - `list_tools` capability filter: `crates/mcp-server/src/server.rs:745`
  - resource handlers (`rulake://stats`, `rulake://bundle/...`, `rulake://audit/tail`): `crates/mcp-server/src/server.rs:838`-area
- Audit sink: `/home/ruvultra/projects/RuLake/crates/mcp-server/src/audit.rs`
  - `AuditSink` and 256-entry tail ring (`TAIL_CAPACITY`): `crates/mcp-server/src/audit.rs:31`, `:34`
  - `emit` with tee-into-tail-before-write (write-failure tolerant): `crates/mcp-server/src/audit.rs:100`
  - `AuditEntry` schema (mirrors ADR-004 §7): `crates/mcp-server/src/audit.rs:152`
  - ISO-8601 timestamp without chrono dep: `crates/mcp-server/src/audit.rs:200`
- Other server modules: `crates/mcp-server/src/auth.rs`, `jwks.rs` (v0.5), `mtls.rs` (v0.6), `policy.rs`, `planner.rs`, `workers.rs`, `ratelimit.rs`, `replay.rs`, `sessions.rs`, `allow.rs`, `http.rs`.
- Tests: `crates/mcp-server/tests/smoke.rs`, `http_e2e.rs`, `fixtures/`.
- Criterion benches (commit `8ce3689`): `crates/mcp-server/benches/audit_sink.rs`, `tools_list_filter.rs`.
- Sibling-crate discipline (no workspace, sibling at `crates/mcp-server/`): ADR-001.
- Prior-art TS demo (kept as reference): `examples/nodejs/04-mcp-tool/src/server.ts`.
- Public Rust surface the server wraps: `crates/core/src/lib.rs:53`-area; `crates/core/src/lake.rs:521` (federation); `crates/core/src/bundle.rs:166` (`RuLakeBundle::new`), `crates/core/src/bundle.rs:362` (`compute_witness`), `crates/core/src/bundle.rs:340`-area (witness-fail-closed).
- Companion ADRs sharing audit / capability schema: ADR-002, ADR-003, ADR-005, ADR-006, ADR-155, ADR-156.
- Companion MCP servers on the shared audit schema: `crates/mcp-rvdna/` (commit `a66d65f`), `crates/mcp-ruqu/` (commit `6d60cf7`).
- MCP spec revisions cited as opaque identifiers: `2025-11-25` (current target), `2025-06-18`, `2025-03-26`, `2024-11-05` (retired). Upstream SDK is `modelcontextprotocol/rust-sdk` (`rmcp`); spec lives at modelcontextprotocol.io. ADR-004 does not pin URLs.
- 2025 MCP CVE timeline cited in ADR-004 §Context: CVE-2025-6514, CVE-2025-53107, CVE-2025-53818, CVE-2025-68143/68144/68145, CVE-2025-6515. Plus the Anthropic `git-mcp-server` description-injection write-up at *Infosecurity Magazine* (January 2026), Snyk Labs / Palo Alto Unit 42 deep dives. URLs are not pinned.
- R-MCP-1 audit-symmetry fix: commit `56b497b`. Full review note at `docs/research/security/shipping-substrates-v2.md` (added in commit `8ce3689`).
