# rulake-substrates

The four ruLake backend adapters and their two companion MCP servers.

## Adapters

| Crate | ADR | Surface |
|---|---|---|
| `gcs-backend` | ADR-155 | GCS Parquet bundles via `BackendAdapter` |
| `ipfs-backend` | ADR-005 | IPFS CID-anchored bundles with R-IPFS-1 hard-refuse on data_ref↔CID mismatch |
| `rvdna-backend` | ADR-007 | `.rvdna` v2 genomic format — T0/T1/T2 tier roadmap |
| `ruqu-backend` | ADR-008 | Five quantum simulation backends — StateVector / Stabilizer / TensorNetwork / Hardware / QEC |

## Companion MCPs

Two of the four substrates also ship with their own MCP server (the genomic and quantum surfaces are large enough to want their own tool namespace):

| MCP | Tools | Default wire |
|---|---|---|
| `rvdna-mcp` | `rvdna_call_variants`, `rvdna_find`, `rvdna_lineage`, `rvdna_score`, `rvdna_translate` | `https://rvdna-mcp.ruv.io/` |
| `ruqu-mcp` | `ruqu_optimize`, `ruqu_qec_schedule`, `ruqu_replay`, `ruqu_simulate`, `ruqu_verify` | `https://ruqu-mcp.ruv.io/` |

Both demos are read-only with no auth. For production, deploy your own per [`docs/deploy/cloud-run.md`](https://github.com/ruvnet/RuLake/blob/main/docs/deploy/cloud-run.md).

## Wiring backends into RuLake

Adapters live in-process — they're registered against a `RuLake` instance via `register_backend(Arc<dyn BackendAdapter>)`. The companion MCP servers expose them over the wire; this plugin's MCP wires hit those.

## See also

- [`rulake-core`](../rulake-core/) — the witness-anchored cache that consumes these adapters
- [ADR-007](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/ADR-007-rvdna-as-rulake-substrate.md) — rvDNA design
- [ADR-008](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/ADR-008-ruqu-as-rulake-substrate.md) — ruQu design
