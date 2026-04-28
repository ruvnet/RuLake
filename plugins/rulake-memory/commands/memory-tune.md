---
description: Read the cache stats + audit ledger and recommend Consistency settings (Fresh / Eventual / Frozen) per collection based on observed access patterns.
---

# /rulake-memory:memory-tune

Reads `/memory-status` data and outputs a per-collection recommendation. The agent then decides whether to apply each recommendation.

## Example

```text
/rulake-memory:memory-tune
```

## What you get back

```jsonc
{
  "recommendations": [
    {
      "collection": "agent-memory",
      "current_consistency": "Fresh",
      "observed_hit_ratio": 0.91,
      "observed_drift_rate": 0.001,
      "recommended_consistency": "Eventual{ttl_ms: 30000}",
      "rationale": "high hit ratio + very low drift; per-query check is wasted work; 30 s TTL captures all observed update intervals",
      "estimated_speedup": "1.4x avg latency reduction"
    },
    {
      "collection": "live-prices",
      "current_consistency": "Eventual{ttl_ms: 5000}",
      "observed_hit_ratio": 0.42,
      "observed_drift_rate": 0.31,
      "recommended_consistency": "Fresh",
      "rationale": "high drift rate makes the TTL a stale-data factory; switching to Fresh trades 0.6 ms/query for correctness",
      "estimated_speedup": "n/a — correctness fix"
    },
    {
      "collection": "audit-archive-2024",
      "current_consistency": "Fresh",
      "observed_hit_ratio": 0.99,
      "observed_drift_rate": 0.0,
      "recommended_consistency": "Frozen",
      "rationale": "zero drift across 10k queries; pin until explicit refresh",
      "estimated_speedup": "drops the per-query coherence check entirely"
    }
  ],
  "audit_window_ms": 86400000,
  "queries_analyzed": 12453
}
```

## How to apply

The recommendations are advisory. To apply: edit your `RuLake::register_backend(...)` call's `consistency:` parameter or, for the public-demo MCP, ask the operator who deployed it to update the config and redeploy.

## Self-learning loop

Run `/memory-tune` weekly (via `rulake-loop-vector`'s `/rulake-incremental-index` cadence). Track the recommendations over time — if the same collection swings between recommendations, that's a signal its access pattern is non-stationary and you should reconsider its design.
