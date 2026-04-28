---
description: Report the live learning metrics — hit ratio per collection, refusal rate, top-N most-recalled keys, last 10 audit rows. Tells you how well your memory is performing.
---

# /rulake-memory:memory-status

Reads the `rulake://stats` and `rulake://stats/by-backend` resources plus the audit-tail buffer. Returns a dashboard-shaped snapshot.

## Example

```text
/rulake-memory:memory-status
```

## What you get back

```jsonc
{
  "session_uptime_ms": 1834500,
  "global": {
    "queries_total": 1247,
    "cache_hits": 1085,
    "cache_misses": 162,
    "hit_ratio": 0.870,
    "refusals": 3,
    "refusal_rate": 0.0024
  },
  "by_collection": [
    {
      "collection": "agent-memory",
      "queries": 894,
      "hit_ratio": 0.91,
      "consistency_used": "Eventual{ttl_ms: 5000}",
      "warmth": "warm"
    },
    {
      "collection": "decisions",
      "queries": 218,
      "hit_ratio": 0.78,
      "consistency_used": "Fresh",
      "warmth": "cold"
    }
  ],
  "top_recalled_keys": [
    { "key": "user-preferences", "recalls": 142 },
    { "key": "current-task", "recalls": 89 },
    { "key": "decisions/v2.4-roadmap", "recalls": 41 }
  ],
  "audit_tail": [
    { "ts": 1714234500000, "code": "OK_VERIFIED_CACHE", "key": "user-preferences", "duration_ms": 0.4 },
    { "ts": 1714234499200, "code": "WITNESS_MATCH", "key": "current-task", "duration_ms": 1.1 }
  ]
}
```

## When to use

- Diagnose slow recalls — low hit ratio means the cache isn't earning its keep
- Spot drift early — refusal rate climbing means data is changing under you
- Decide what to promote — top_recalled_keys is the warm-tier candidate list
- Compare consistency settings — the `consistency_used` column shows what the planner picked per collection

This is the **input** to `/memory-tune` — `tune` reads the same metrics and outputs concrete config recommendations.
