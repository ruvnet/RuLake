---
description: rvDNA T0 — DNA → protein translation for a genomic region in a pinned collection. Returns the protein string + witness. v0.0.1 stub; codon table + ORF logic lands in v0.1.
---

# /rulake-substrates:rvdna-translate

Wraps the `rvdna_translate` MCP tool. Translates a DNA region to its protein sequence.

## Inputs

- **collection** (required)
- **chrom** (required)
- **start** (required)
- **end** (required)

## Example

```text
/rulake-substrates:rvdna-translate collection=hg38-chr22 chrom=chr22 start=10000000 end=10000600
```

## What you get back

```jsonc
{
  "protein": "",                           // v0.0.1 stub — populated in v0.1
  "witness": "b1f4...32-byte-hex...",
  "elapsed_ms": 0.3,
  "consistency": "Frozen"
}
```
