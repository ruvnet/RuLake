# rulake-memory

**Self-learning, self-optimizing agentic memory** built on the witness-anchored ruLake cache.

Where `rulake-core` exposes the raw retrieval surface (`rulake_query`, `rulake_publish_bundle`, etc.), `rulake-memory` wraps those into a **working-memory surface** for agents — with the verbs operators actually want (`remember`, `recall`, `forget`) and an opinionated learning loop on top (hit-ratio tuning, drift detection, audit-driven optimization).

## What "self-learning" means here

Three feedback loops, all running against the live ruLake cache:

1. **Cache learning** — every `recall` records what was asked. Frequently-recalled keys get auto-promoted to the warm-cache tier; rare keys age out.
2. **Consistency learning** — `tune` analyzes the cache hit ratio per collection and recommends `Fresh` / `Eventual{ttl_ms}` / `Frozen` based on what the access pattern actually needs (not what was guessed at config time).
3. **Drift learning** — `replay` re-runs past queries against current state and detects when the underlying data has changed. The witness chain catches it; the plugin surfaces it as actionable reports.

## Install

```text
/plugin marketplace add ruvnet/RuLake          # if not already
/plugin install rulake-memory@rulake-marketplace
/reload-plugins
/rulake-memory:memory-status
```

## The eight slash commands

| Command | What it does |
|---|---|
| `/rulake-memory:memory-remember` | Pin a fact / decision / result into the cache with a witness. The next `/recall` of the same key returns in ~1 ms. |
| `/rulake-memory:memory-recall` | Search the witnessed memory for what's similar to a query. Returns hits + the decision trace (witness, freshness, substrates, latency). |
| `/rulake-memory:memory-forget` | Drop a key from the cache. The next access re-primes from the backend. |
| `/rulake-memory:memory-status` | Show learning metrics: hit ratio per collection, refusal rate, top-N most-recalled keys, audit-row tail. |
| `/rulake-memory:memory-promote` | Force a collection into the warm cache (pre-prime). Useful before a high-traffic agent session starts. |
| `/rulake-memory:memory-tune` | Read the audit ledger + cache stats. Recommend `Consistency` setting per collection based on observed access patterns. |
| `/rulake-memory:memory-replay` | Replay the last N queries against current state. Surface the diff: which witnesses still match, which have drifted, which would now refuse. |
| `/rulake-memory:memory-compact` | Compact the audit ledger into a "long-term memory" bundle — pinned witness + summary statistics. The pinned bundle survives restarts. |

## The auto-invokable skill

This plugin also ships a [`memory`](skills/memory/SKILL.md) skill that Claude can invoke without an explicit slash command. When an agent encounters a new fact worth remembering, the skill calls `/memory-remember`; when the agent needs to recall, it calls `/memory-recall`. The skill turns ruLake into the agent's working memory transparently.

## How it composes with the rest

| Plugin | Relationship |
|---|---|
| `rulake-core` | The underlying retrieval surface. `rulake-memory` wraps it with memory verbs. Install `rulake-core` if you also want the raw `rulake_query` API. |
| `rulake-witness` | Used by `/memory-replay` to verify each replay-query's witness chain. |
| `rulake-rvdna`, `rulake-ruqu` | If installed, the memory surface federates over them too — recall a quantum simulation result by witness, remember a genomic search by region. |
| `rulake-loop-vector` | The memory plugin's `/memory-tune` recommendations can be wired into a `/loop` worker for continuous auto-tuning. |

## How to wire the MCP

`rulake-memory` ships **commands only** (no bundled `.mcp.json`) — same shape as `rulake-core`, for the same reason: when both this plugin and `rulake-stack` are installed, the duplicate `rulake` MCP server name would emit a warning. Pick one of:

- **Install `rulake-stack`** (recommended) — bundles the wire
- Add the `rulake` MCP server to your `~/.claude.json` global config
- For production: deploy your own MCP per [`docs/deploy/cloud-run.md`](../../docs/deploy/cloud-run.md) and point your config at it

## See also

- [`docs/userguide/`](../../docs/userguide/) — Console walkthrough (the visual companion to these CLI commands)
- [`memory-substrate-deep.md`](../../docs/gists/memory-substrate-deep.md) — the deep design walkthrough on what makes ruLake a good memory layer for agents
- [`mcp-server-deep.md`](../../docs/gists/mcp-server-deep.md) — the underlying MCP wire
