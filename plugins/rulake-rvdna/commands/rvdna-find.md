---
description: rvDNA T0 — kNN search over a pinned genomic collection. Returns a witnessed bundle pointer plus the matching segments. v0.0.1 stub; real RaBitQ inner loop lands in v0.1.
---

# /rulake-substrates:rvdna-find

Wraps the `rvdna_find` MCP tool against the wired `rvdna-mcp` server (`https://rvdna-mcp.ruv.io/` by default).

## Inputs

- **collection** (required) — registered rvdna T0 collection id
- **query_seq** (required) — DNA query sequence (A/C/G/T/N)
- **k** (optional, default 10) — top-k neighbors to return

## Example

```text
/rulake-substrates:rvdna-find collection=hg38-chr22 query_seq=ACGTACGTACGT k=5
```

## What you get back

```jsonc
{
  "hits": [],                              // v0.0.1 stub — populated in v0.1
  "witness": "b1f4...32-byte-hex...",      // live + verified
  "elapsed_ms": 0.8,
  "consistency": "Frozen"
}
```

The witness is real and verified; the hits array is empty in v0.0.1 so callers can wire the call shape today. Refuses with `RVDNA_WITNESS_DRIFT` if the backend mutated since registration.

## See also

- [ADR-007](../../../docs/adrs/ADR-007-rvdna-as-rulake-substrate.md) — the rvDNA design
- [rvdna-v2-deep gist](../../../docs/gists/rvdna-v2-deep.md)
