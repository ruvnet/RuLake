---
description: ruQu — recompute the witness for a previously-recorded simulation result and compare. Refuses on mismatch with `RUQU_WITNESS_MISMATCH`.
---

# /rulake-substrates:ruqu-verify

Wraps the `ruqu_verify` MCP tool. Local re-verification of a `ruqu_simulate` result.

## Inputs

- **result_witness** (required) — the SHAKE-256 hex from a prior `ruqu_simulate` call
- **circuit** (required) — the same circuit JSON
- **backend** (required) — the same backend
- **shots** (required) — the same shot count
- **seed** (required) — the same seed

## Example

```text
/rulake-substrates:ruqu-verify result_witness=b1f4... circuit='{...}' backend=stabilizer shots=100 seed=42
```

## What you get back

```jsonc
{
  "match": true,
  "expected": "b1f4...",
  "computed": "b1f4...",
  "elapsed_ms": 0.4
}
```

Returns `match: false` + a diff if any input drifted. This is how cross-host quantum-result reproducibility is gated — two operators on two machines re-running the same circuit get the same witness or an honest refusal.
