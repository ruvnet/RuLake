---
name: rulake-bundle-info
description: Pretty-print every field of a `table.rulake.json` bundle without recomputing the witness. Use for inspection / debugging.
---

# /rulake-bundle-info

```text
/rulake-bundle-info path/to/table.rulake.json
```

Walks the bundle's structure: `data_ref`, `dim`, `rotation_seed`, `rerank_factor`, `generation`, `pii_policy`, `lineage_id`, `memory_class`, plus the embedded witness hex.

For verification, use [`/rulake-verify`](./rulake-verify.md). For "is this bundle still valid against the IPFS network's CID?", boot the `rulake-substrates` plugin and let `ipfs-backend` answer over MCP.
