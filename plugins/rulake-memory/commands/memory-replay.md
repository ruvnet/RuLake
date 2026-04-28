---
description: Replay the last N queries against current state. Surface the witness diff — which queries still return the same answer, which have drifted, which now refuse.
---

# /rulake-memory:memory-replay

Pulls the last N audit rows, re-issues each query against current state, compares the new witness to the original. The diff is your drift report.

## Inputs

- **n** (optional, default 100) — how many recent queries to replay
- **collection** (optional) — restrict to one collection
- **since_ms** (optional) — only replay queries newer than this many ms

## Example

```text
/rulake-memory:memory-replay n=50
```

## What you get back

```jsonc
{
  "replayed": 50,
  "still_match": 44,                       // witness identical
  "drifted": 5,                            // witness changed (data updated)
  "now_refuse": 1,                         // backend gone or witness chain broken
  "drift_details": [
    {
      "query": "what was the v2.3 ship date?",
      "original_witness": "b1f4...",
      "current_witness": "9d8e...",
      "code": "STALE_CACHE_REMOTE_VALID",
      "drift_age_ms": 3601000
    }
  ],
  "refuse_details": [
    {
      "query": "what's in the temp-collection?",
      "code": "WITNESS_MISMATCH_REFUSED",
      "reason": "backend collection 'temp-2026-04-27' no longer registered"
    }
  ],
  "elapsed_ms": 87
}
```

## When to use

- Before relying on a cached answer that's old: did the underlying data change?
- After a known data update: which previously-cached answers are now stale?
- For correctness audits: prove that the witness chain catches drift
- During incident review: replay last hour of queries to scope what was affected by a bad publish

## Self-learning loop

`/memory-replay` is the **drift detector** in the learning loop. Schedule it (via `rulake-loop-vector`) on a per-collection cadence calibrated to that collection's observed update rate. If `drifted + now_refuse > 0`, fire `/memory-tune` to reconsider the consistency setting.
