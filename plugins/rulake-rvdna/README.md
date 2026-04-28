# rulake-rvdna

The genomic substrate plugin for ruLake. Standalone — install when your agent needs DNA / variants / protein / scoring against a registered `.rvdna` v2 collection.

## What it adds

| Surface | Detail |
|---|---|
| **MCP wire** | `rvdna-mcp` → `https://rvdna-mcp.ruv.io/` (public demo, read-only) |
| **5 tools** | `rvdna_find`, `rvdna_call_variants`, `rvdna_translate`, `rvdna_score`, `rvdna_lineage` |
| **5 commands** | `/rulake-rvdna:rvdna-find`, `…-call-variants`, `…-translate`, `…-score`, `…-lineage` |

## Install

```text
/plugin marketplace add ruvnet/RuLake          # if not already
/plugin install rulake-rvdna@rulake-marketplace
/reload-plugins
/rulake-rvdna:rvdna-find collection=hg38-chr22 query_seq=ACGT k=5
```

## v0.0.1 status

- **Live + verified**: every response carries a real SHAKE-256(32) witness pinned to `(collection, query_args, generation)`. The witness chain is end-to-end working today.
- **Stub data plane**: the 4 read tools (`find`, `call_variants`, `translate`, `score`) return witnessed-but-empty results in v0.0.1. The RaBitQ kNN loop, precomputed variant index, codon table + ORF logic, and scorer model wiring all land in v0.1.
- **Internal-tier `rvdna_lineage`**: live + end-to-end. Returns assembly version, source URI, build epoch. Requires the rvdna-mcp server to be wired with `--capabilities read,internal`.

## Production deploy

The public demo wire ships `--auth none --insecure-allow-no-auth --capabilities read`. For production with real genomic data: deploy your own per [`docs/deploy/cloud-run.md`](../../docs/deploy/cloud-run.md), then point this plugin's `.mcp.json` at your URL.

## See also

- [`rvdna-v2-deep.md`](../../docs/gists/rvdna-v2-deep.md) — the deep design walkthrough (2,804 words)
- [`rulake-ruqu`](../rulake-ruqu/) — the sibling quantum substrate plugin
