---
description: Compact the audit ledger and frequently-recalled keys into a long-term memory bundle — pinned witness + summary statistics. The pinned bundle survives restarts.
---

# /rulake-memory:memory-compact

Walks the audit ledger + the warm-cache top-K and produces a single witnessed bundle that captures "what this agent has learned." The bundle is publishable to any backend; another agent can `/memory-recall` against it and inherit the learnings.

## Inputs

- **out_collection** (optional, default `learnings/<date>`) — where to publish the compacted bundle
- **window_days** (optional, default 7) — how far back to look in the audit ledger
- **min_recalls** (optional, default 5) — only compact memories recalled at least this many times

## Example

```text
/rulake-memory:memory-compact window_days=30 min_recalls=10
```

## What you get back

```jsonc
{
  "bundle_witness": "b1f4...32-byte-hex...",
  "out_collection": "learnings/2026-04-27",
  "compacted_keys": 47,
  "compacted_bytes": 184320,
  "audit_rows_summarized": 12453,
  "summary_stats": {
    "top_recalled": [
      { "key": "user-preferences", "recalls": 142 }
    ],
    "drift_rate_observed": 0.012,
    "avg_hit_latency_ms": 0.94
  }
}
```

## When to use

- Periodic memory consolidation — daily or weekly compaction
- Before tearing down an agent session — capture what was learned for the next session
- Before sharing learnings with another agent — give them the witness, they recall against it
- For audit / explainability — the compacted bundle is a portable receipt of "what this agent knew at time T"

## Self-learning loop

`/memory-compact` closes the loop. The compacted bundle becomes the **seed** for the next session — promote it via `/memory-promote` at startup so the new agent inherits the warm-cache state. Run `/memory-status` to confirm hit ratios stay high across the session boundary.
