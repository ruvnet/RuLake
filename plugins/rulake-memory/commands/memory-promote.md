---
description: Force a collection into the warm cache by pre-priming. Useful before a high-traffic agent session starts. Wraps rulake_warm_from_dir.
---

# /rulake-memory:memory-promote

Pre-load a cold collection into the cache so the first `/memory-recall` of the session returns in ~1 ms instead of paying the cold-start tax.

## Inputs

- **collection** (required) — the collection to warm
- **bundle_dir** (optional) — directory containing the snapshot to warm from. Defaults to the configured snapshot dir.

## Example

```text
/rulake-memory:memory-promote collection=user-preferences
```

## What you get back

```jsonc
{
  "collection": "user-preferences",
  "vectors_loaded": 1842,
  "warm_from_dir_ms": 89,
  "warm_size_bytes": 472576,
  "witness": "b1f4..."             // the warm bundle's witness
}
```

## When to use

- At session start: warm the top-N collections from `/memory-status.top_recalled_keys`
- Before a known high-traffic operation (e.g. a user-presentation flow)
- After a server restart — warm before agents start hitting it

## Self-learning loop

Combine with `/memory-status`:

```
/memory-status                                       # see top_recalled_keys
/memory-promote collection=user-preferences          # warm the top one
/memory-promote collection=current-task              # warm the second
/memory-status                                       # confirm hit_ratio jumps
```

A `/loop`-driven worker (see `rulake-loop-vector`) can automate this — promote the top-3 daily.
