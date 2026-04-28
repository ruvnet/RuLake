---
name: rulake-query
description: Query the witness-anchored ruLake cache — search, verify, explain, or refresh. Returns ranked results plus a decision_trace block with cost, latency, witness match, and substrates used.
---

# /rulake-query

Wraps the `rulake_query` MCP tool with sensible defaults so a fresh
operator can issue their first query without thinking about the
intent / risk / freshness / target axes.

## Inputs (all optional except the question)

- **question** (required): natural-language query
- **intent**: `search` | `verify` | `explain` | `refresh` (default: `search`)
- **freshness_ms**: tolerance for stale cache hits (default: 5000)
- **risk**: `low` | `medium` | `high` (default: `low`)
- **target_collection**: limit to a specific backend.collection (default: federated across all)

## What you get back

```jsonc
{
  "result": [ /* ranked retrieval */ ],
  "decision_trace": {
    "chosen_path": "deterministic-retrieval-path-v0.1",
    "intent": "search",
    "freshness": { "budget_ms": 5000, "actual_ms": 1031 },
    "cache":  { "hit": true, "hit_ratio_session": 0.87 },
    "substrates_used": ["gcs-backend", "rvdna-backend"],
    "kernel": { "id": "cpu-naive", "deterministic": true },
    "witness": { "match": true },
    "cost":    { "compute_kernel": 0.0, "backend_fetch": 0.0, "cache_hit_discount": -1.0 },
    "latency": { "total_ms": 1.02, "cache_ms": 0.4, "witness_ms": 0.6 },
    "refusals": []
  }
}
```

The `decision_trace` block is the new contract from ADR-009 — it lets a calling agent negotiate cost vs trust vs latency without reaching around the abstraction. **Note:** the trace shape ships in v2.4 — until then, responses carry the data only and the trace is in the JSONL audit log on the server side.

## Examples

```text
/rulake-query "what does ADR-157 commit to?"

/rulake-query --intent verify --freshness_ms 0 "is bundle b1f4...3d valid?"

/rulake-query --target_collection adrs.rulake "list all merged ADRs"
```

## See also

- [ADR-004 §4b](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/sdk/ADR-004-rulake-mcp-server.md) — the eight tool tier matrix
- [ADR-009](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/sdk/ADR-009-rulake-plugin-marketplace.md) — the deterministic retrieval path + decision_trace shape
