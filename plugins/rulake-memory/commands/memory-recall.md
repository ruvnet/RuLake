---
description: Search the witnessed memory for what's similar to a query. Returns hits plus a decision trace (witness, freshness, substrates, latency, cost).
---

# /rulake-memory:memory-recall

Wraps `rulake_query` with `intent=search`, defaulting to the agent-memory collection. Returns ranked memories + the receipt for each.

## Inputs

- **query** (required) — what to recall (natural language or a sample)
- **collection** (optional, default `agent-memory`) — where to look
- **k** (optional, default 5) — top-k memories to return
- **memory_class** (optional) — filter to `working` / `episodic` / `semantic` / `procedural`

## Example

```text
/rulake-memory:memory-recall query="what did we decide about the mincut step?" k=3
```

## What you get back

```jsonc
{
  "memories": [
    {
      "key": "decisions/v2.4-roadmap",
      "content": "ship the mincut prune step in v2.4",
      "score": 0.91,
      "witness": "b1f4...",
      "memory_class": "semantic",
      "stored_at": "2026-04-27T12:00:00Z"
    }
  ],
  "decision_trace": {
    "chosen_path": "deterministic-retrieval-path-v0.1",
    "cache": { "hit": true, "hit_ratio_session": 0.87 },
    "witness": { "match": true },
    "latency": { "total_ms": 1.02 }
  }
}
```

## When to use

- At the start of a task: "have I dealt with this before?"
- Before answering a user question: "did I already establish this fact?"
- During reasoning: "what did I just decide that's relevant here?"
- Before a tool call: "did I cache the same call recently?"

## Self-learning note

Each `/memory-recall` updates the cache statistics. Frequently-recalled keys get auto-promoted to the warm tier — that's where the "memory that gets faster the more it's used" property comes from. Run `/memory-status` to see the current hit-ratio.
