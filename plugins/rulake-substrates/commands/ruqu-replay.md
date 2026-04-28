---
description: ruQu — replay a circuit on a different backend with the same seed and verify the witness chain holds. Useful for confirming Stabilizer-fast vs StateVector-exact agree on a Clifford circuit.
---

# /rulake-substrates:ruqu-replay

Wraps the `ruqu_replay` MCP tool. Cross-backend verification.

## Inputs

- **original_witness** (required) — the witness from a prior `ruqu_simulate` call
- **circuit** (required)
- **target_backend** (required) — the backend to replay on
- **shots** (required)
- **seed** (required)

## Example

```text
/rulake-substrates:ruqu-replay original_witness=b1f4... circuit='{"qubits":4,"gates":[{"H":0},{"CX":[0,1]},{"CX":[1,2]},{"CX":[2,3]}]}' target_backend=statevector shots=100 seed=42
```

## What you get back

```jsonc
{
  "original_backend": "stabilizer",
  "target_backend": "statevector",
  "witness_match": true,                   // both backends agree ⇒ Clifford-correctness gate
  "elapsed_original_ms": 0.6,
  "elapsed_target_ms": 12.3
}
```

`witness_match: true` on a Clifford circuit means the Stabilizer simulator's polynomial-time output is byte-identical to the StateVector exact simulation — the design contract from ADR-008 §6.
