# `rulake-mcp` — ruLake MCP server

Implements [ADR-004](../docs/adrs/sdk/ADR-004-rulake-mcp-server.md) — the
control-plane / decision-layer over [`ruvector-rulake`](..) for
agent-callable governed memory.

## Status: v0.2 (deployable to remote agents)

v0.1 landed the architecture; v0.2 makes it deployable to remote
agents (Streamable HTTP transport + bearer auth with the
embarrassing-flag dev-only guards from ADR-004 §5).

| Capability | v0.1 | v0.2 | v0.3 |
|---|:---:|:---:|:---:|
| stdio transport                              | ✅ | ✅ | ✅ |
| `rulake_query` (intent: `search`)            | ✅ | ✅ | ✅ |
| `rulake_list_backends`                       | ✅ | ✅ | ✅ |
| Bounded worker pool (`flume` + `rayon`)      | ✅ | ✅ | ✅ |
| Decision trace (`reason_code`, `decision.*`) | ✅ | ✅ | ✅ |
| TOML config (`mcp.toml`) — backends, consistency, workers | ✅ | ✅ | ✅ |
| Witness-fail-closed for bundle reads         | ✅ | ✅ | ✅ |
| Streamable HTTP transport                    |    | ✅ | ✅ |
| Bearer-token auth (dev-only, embarrassing-flag for public bind) | | ✅ | ✅ |
| DNS-rebinding guard (rmcp `allowed_hosts`)   |    | ✅ | ✅ |
| OAuth 2.1 + mTLS                             |    |    | ✅ |
| Replay protection (`MCP-Request-Id` + session binding) | | | ✅ |
| Layered rate limiting (`governor`)           |    |    | ✅ |
| Intents `verify` / `explain`                 |    |    | ✅ |
| Resources (`rulake://stats`, `rulake://bundle/...`) | | | ✅ |
| Mutation tools (`publish`, `admin` capabilities) |  |    | ✅ |
| `intent: "refresh"`                          |    |    | ✅ |
| JSONL audit file (full §7 schema)            |    |    | ✅ |

## Build

```bash
git clone --recurse-submodules https://github.com/ruvnet/RuLake
cd RuLake/mcp-server
cargo build --release
./target/release/rulake-mcp --help
```

Per [ADR-001](../docs/adrs/ADR-001-standalone-repo-strategy.md) this is
a sibling Cargo package — no root workspace; the parent
`ruvector-rulake` is reached via `path = ".."`.

## Run

```bash
# stdio (parent-process trust, default).
./target/release/rulake-mcp stdio --config tests/fixtures/mcp.toml

# Streamable HTTP on loopback, no auth (default for local dev).
./target/release/rulake-mcp http --bind 127.0.0.1:7440 --auth none

# Streamable HTTP with bearer auth (still loopback by default).
echo "my-dev-token" > /tmp/rulake-token
./target/release/rulake-mcp http --bind 127.0.0.1:7440 \
    --auth bearer --bearer-token-file /tmp/rulake-token

# Bind public — refused unless explicitly opted-in (ADR-004 §5):
./target/release/rulake-mcp http --bind 0.0.0.0:7440 --auth none
# Error: refusing to bind 0.0.0.0:7440 with --auth none — pass
#        --insecure-allow-no-auth or use --bind 127.0.0.1:* (ADR-004 §5)

./target/release/rulake-mcp http --bind 0.0.0.0:7440 \
    --auth bearer --bearer-token-file /tmp/rulake-token
# Error: refusing to bind 0.0.0.0:7440 with --auth bearer — bearer is
#        dev-only (static tokens leak once → permanent access). Pass
#        --allow-bearer-on-public to override or migrate to OAuth (ADR-004 §5)
```

Wire to an MCP client over stdio (e.g. Claude Desktop's
`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "rulake": {
      "command": "/path/to/rulake-mcp",
      "args": ["stdio", "--config", "/path/to/mcp.toml"]
    }
  }
}
```

Wire to a remote MCP client over Streamable HTTP (Cursor / Cline /
Continue with `http_url`; agentic-flow with the `streamable-http`
transport):

```json
{
  "mcpServers": {
    "rulake": {
      "transport": "streamable-http",
      "url": "https://rulake.example.com/mcp",
      "headers": { "Authorization": "Bearer <your-token>" }
    }
  }
}
```

## Test

```bash
cargo test --release
# 11/11 pass:
#   tests/smoke.rs:
#     search_one_returns_top_k_via_planner
#     refuses_on_unknown_backend_with_policy_refused_allowlist
#     budget_max_results_caps_k
#     decision_trace_has_required_fields_for_audit
#   tests/http_e2e.rs:
#     http_serve_starts_on_loopback_with_no_auth
#     http_refuses_bearer_on_public_without_override
#     http_refuses_no_auth_on_public_without_override
#     bearer_auth_accepts_correct_token
#     bearer_auth_rejects_wrong_token
#     bearer_auth_rejects_missing_header
#     bearer_auth_rejects_wrong_scheme
```

## Design

The MCP server is the **decision-making layer over ruLake**, not a
thin transport. The split:

| Layer       | Job                                                  |
|-------------|------------------------------------------------------|
| ruLake      | storage, cache, bundles, search, witness validation  |
| MCP server  | policy, routing, trust decisions, degradation, audit |
| Agent       | intent, reasoning, task execution                    |

The public surface is **one tool**: `rulake_query`. The agent submits
an intent (`search` / `verify` / `explain` / `refresh`) with target,
risk, freshness budget, and policy; the server picks where to search,
how strict to be, whether to refuse, and emits a decision trace
alongside the answer. See ADR-004 §4a for the full schema.

Three distinguishing characteristics:

1. **Bounded worker pool** (`rayon` of size `cores * 2`, submit via
   bounded `flume` channel of capacity `max_inflight`). Submit past
   capacity returns `Degraded` immediately — never unbounded queueing.
   This isolates RuLake CPU work from the tokio reactor that owns the
   wire and bounds worst-case scan-thread count regardless of MCP-call
   burstiness. ADR-004 §6.
2. **Decision trace on every response** — `reason_code` (closed enum:
   `CACHE_HIT_FRESH`, `STALE_CACHE_REMOTE_VALID`,
   `WITNESS_MISMATCH_REFUSED`, `POLICY_REFUSED_*`, …) plus
   `chosen_action`, `backends_used`, `budget_used_ms`, `refusals`.
   Every audit line "explains itself" — this is the load-bearing
   prerequisite for the M4 governance story (ADR-155 §M4) the server
   is the prototype consumer of. ADR-004 §7.
3. **Witness-fail-closed** for every disk-touching tool (propagating
   the existing posture from `RuLakeBundle::read_from_dir` at
   `src/bundle.rs:349`). v0.1 doesn't yet expose any disk-touching
   tools but the contract is in place for v0.2's `rulake_warm_from_dir`.
   ADR-004 §4 + §5 threat model.

## License

MIT OR Apache-2.0, matching the parent crate.
