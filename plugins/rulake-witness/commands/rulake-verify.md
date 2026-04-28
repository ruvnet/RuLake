---
name: rulake-verify
description: Recompute the SHAKE-256(32) witness over a `table.rulake.json` bundle and compare to the embedded witness. Refuses on mismatch with `WITNESS_MISMATCH_REFUSED`.
---

# /rulake-verify

Local-only — no MCP server, no backend registration, no cache. Just reads the bundle, recomputes the witness preimage, and compares.

```text
/rulake-verify path/to/table.rulake.json
```

Sample output (match):

```text
✓ WITNESS_MATCH
  data_ref:        gcs://my-bucket/segment-001.parquet
  dim:             384
  rotation_seed:   42
  rerank_factor:   20
  generation:      17
  expected:        b1f4a2c8e3d5...
  computed:        b1f4a2c8e3d5...
```

Sample output (mismatch):

```text
✗ WITNESS_MISMATCH_REFUSED
  expected:  b1f4a2c8e3d5...
  computed:  9d8e7c6b5a4f...
  diff:      data_ref differs (bundle was rewritten, generation tag bumped, or someone tampered)
```

Returns exit code 0 on match, 1 on mismatch — useful in CI gates.
