# rulake-witness

Trust-chain ergonomics for ruLake bundles. The witness is a SHAKE-256(32) hash over `(data_ref, dim, rotation_seed, rerank_factor, generation)` — recompute it locally, compare to the substrate-supplied witness, refuse on mismatch.

## Commands

| Command | Purpose |
|---|---|
| `/rulake-verify <bundle-path>` | Recompute the witness over a `table.rulake.json` sidecar and compare to the embedded one. Prints `WITNESS_MATCH` or `WITNESS_MISMATCH_REFUSED` plus the diff. |
| `/rulake-bundle-info <bundle-path>` | Pretty-print every field of a bundle (data_ref, dim, rotation_seed, rerank_factor, generation, pii_policy, lineage_id, memory_class) without recomputing the witness. |

## Why it's a separate plugin

Verification is the load-bearing trust commitment of ruLake. Operators who want to audit a bundle they pulled from IPFS / GCS / a colleague's USB stick should be able to do it **without** booting an MCP server, registering backends, or running the cache. This plugin wraps the underlying `rulake::witness::compute_witness` + `rulake::bundle::RuLakeBundle::verify` calls into two CLI commands.

## See also

- [ADR-005 §R-IPFS-1](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md) — the data_ref ↔ CID mismatch hard-refuse
- [`docs/userguide/06-bundle-witness-viewer.md`](https://github.com/ruvnet/RuLake/blob/main/docs/userguide/06-bundle-witness-viewer.md) — the Console's witness viewer walkthrough
