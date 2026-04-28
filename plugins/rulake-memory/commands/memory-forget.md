---
description: Drop a key from the witnessed cache. The next access re-primes from the backend. Useful for invalidating stale memories or retiring obsolete decisions.
---

# /rulake-memory:memory-forget

Wraps `rulake_invalidate_cache`. Removes the cached entry without deleting the underlying bundle — the next `/memory-recall` for this key will re-pull from the backend (and re-verify the witness).

## Inputs

- **key** (required) — the memory key to drop
- **collection** (optional, default `agent-memory`)

## Example

```text
/rulake-memory:memory-forget key=decisions/v2.4-roadmap
```

## What you get back

```jsonc
{
  "key": "decisions/v2.4-roadmap",
  "dropped": true,
  "had_cache_entry": true
}
```

## When to use

- When you've explicitly retired a decision ("we changed our minds on X")
- When the underlying data is known to have changed and you want a fresh pull
- During testing — to force a cold path
- Before a `/memory-replay` cycle to ensure the replay reads from the source, not the cache

## Note

`/memory-forget` does NOT delete the underlying bundle from the backend. Use `rulake_publish_bundle` with empty content (or the substrate's own delete primitive) for actual data removal. The forget is cache-only.
