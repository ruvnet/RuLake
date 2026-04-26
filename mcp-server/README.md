# `rulake-mcp` — ruLake MCP server

Implements [ADR-004](../docs/adrs/sdk/ADR-004-rulake-mcp-server.md) — the
control-plane / decision-layer over [`ruvector-rulake`](..) for
agent-callable governed memory.

## Status: v0.1 (skeleton)

The first commit lands the architecture and the public-surface tool;
later versions fill in the rest of the ADR scope.

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
| OAuth 2.1 + bearer + mTLS                    |    | ✅ | ✅ |
| Replay protection (`MCP-Request-Id` + session binding) | | ✅ | ✅ |
| Layered rate limiting (`governor`)           |    | ✅ | ✅ |
| Intents `verify` / `explain`                 |    | ✅ | ✅ |
| Resources (`rulake://stats`, `rulake://bundle/...`) | | ✅ | ✅ |
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
# v0.1 — stdio only.
./target/release/rulake-mcp stdio --config tests/fixtures/mcp.toml
```

Wire to an MCP client (e.g. Claude Desktop's `claude_desktop_config.json`):

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

## Test

```bash
cargo test --release
# 4/4 pass:
#   search_one_returns_top_k_via_planner
#   refuses_on_unknown_backend_with_policy_refused_allowlist
#   budget_max_results_caps_k
#   decision_trace_has_required_fields_for_audit
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
