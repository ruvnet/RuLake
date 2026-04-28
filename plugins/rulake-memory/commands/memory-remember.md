---
description: Pin a fact / decision / result into the witness-anchored ruLake cache. Subsequent /memory-recall calls for the same key return in about 1 ms with a verifiable receipt.
---

# /rulake-memory:memory-remember

Wraps `rulake_publish_bundle` with a memory-shaped UX. The thing being remembered gets a SHAKE-256 fingerprint pinned to its content; that fingerprint is the receipt the next `/memory-recall` checks against.

## Inputs

- **key** (required) — a stable identifier for the memory (e.g. `decisions/v2.4-roadmap`, `facts/customer-X-prefers-eventual`)
- **content** (required) — the thing to remember (string, JSON, or a path to a file)
- **collection** (optional, default `agent-memory`) — namespace this memory under
- **memory_class** (optional, default `working`) — `working` | `episodic` | `semantic` | `procedural`

## Example

```text
/rulake-memory:memory-remember key=decisions/v2.4-roadmap content="ship the mincut prune step in v2.4" memory_class=semantic
```

## What you get back

```jsonc
{
  "key": "decisions/v2.4-roadmap",
  "collection": "agent-memory",
  "memory_class": "semantic",
  "witness": "b1f4...32-byte-hex...",       // the receipt
  "stored_bytes": 38,
  "ttl_ms": null,                            // null = pinned until explicit forget
  "consistency": "Frozen"
}
```

The witness is what makes this memory verifiable — two agents on two machines that re-run the same `/memory-remember` get the same witness, byte-for-byte. The next `/memory-recall` returns the same content + the same receipt.

## When to use

- After making an important decision the agent shouldn't have to re-derive
- When learning a fact about the user that should persist across sessions
- After a successful tool call whose result is expensive to recompute
- Whenever the agent says "I've decided X" or "I've learned Y" — pin it
