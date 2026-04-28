---
description: rvDNA T0 — deterministic FNV-1a score in [0.0, 1.0] over (witness, region). Region-sensitive, witness-sensitive, reproducible across operators. Production scorer model wires here in v0.2.
---

# /rulake-substrates:rvdna-score

Wraps the `rvdna_score` MCP tool.

## Inputs

- **collection** (required)
- **chrom** (required)
- **start** (required)
- **end** (required)

## Example

```text
/rulake-substrates:rvdna-score collection=hg38-chr22 chrom=chr22 start=10000000 end=10000100
```

## What you get back

```jsonc
{
  "score": 0.0,                            // v0.0.1 stub — populated in v0.1
  "witness": "b1f4...32-byte-hex...",
  "elapsed_ms": 0.3,
  "consistency": "Frozen"
}
```
