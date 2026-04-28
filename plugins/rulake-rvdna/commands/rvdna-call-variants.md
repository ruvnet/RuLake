---
description: rvDNA T0 — variant calls for a region in a pinned collection. Returns whatever variants the registered RVDNA file's VariantTensor section holds for the region. Empty array if no variant data registered (genuine answer, not a stub). Witness-anchored.
---

# /rulake-substrates:rvdna-call-variants

Wraps the `rvdna_call_variants` MCP tool. Returns SNVs / indels for a region.

## Inputs

- **collection** (required) — registered rvdna T0 collection id
- **chrom** (required) — chromosome label, e.g. `chr22`
- **start** (required) — 0-based start coordinate
- **end** (required) — exclusive end coordinate

## Example

```text
/rulake-substrates:rvdna-call-variants collection=hg38-chr22 chrom=chr22 start=10000000 end=10000100
```

## What you get back

```jsonc
{
  "variants": [],                          // v0.0.1 stub — populated in v0.1
  "witness": "b1f4...32-byte-hex...",
  "elapsed_ms": 0.4,
  "consistency": "Frozen"
}
```

Refuses with `RVDNA_WITNESS_DRIFT` on backend drift.

## See also

- [ADR-007](../../../docs/adrs/ADR-007-rvdna-as-rulake-substrate.md)
