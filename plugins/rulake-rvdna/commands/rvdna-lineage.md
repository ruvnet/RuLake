---
description: rvDNA — internal capability tier. Reveals lineage / provenance metadata for a registered rvDNA collection (e.g. assembly version, source URI, build epoch). Gated by --capabilities internal.
---

# /rulake-substrates:rvdna-lineage

Wraps the `rvdna_lineage` MCP tool. Internal-tier — requires the rvdna-mcp server to be wired with `--capabilities read,internal`.

## Inputs

- **collection** (required)

## Example

```text
/rulake-substrates:rvdna-lineage collection=hg38-chr22
```

## What you get back

```jsonc
{
  "assembly": "GRCh38.p14",
  "source_uri": "gs://my-bucket/hg38-chr22.rvdna",
  "build_epoch_ms": 1709251200000,
  "witness": "b1f4...32-byte-hex...",
  "consistency": "Frozen"
}
```

The public demo (`https://rvdna-mcp.ruv.io/`) ships with `--capabilities read` only — `rvdna_lineage` will refuse there. For full lineage, deploy your own per [`docs/deploy/cloud-run.md`](../../../docs/deploy/cloud-run.md).
