---
description: ruQu — simulate a quantum circuit on the chosen backend (StateVector / Stabilizer / TensorNetwork / Hardware-stub). Returns the witnessed final state, sample counts, and decision trace.
---

# /rulake-substrates:ruqu-simulate

Wraps the `ruqu_simulate` MCP tool. Picks the backend per the circuit's gate set + qubit count.

## Inputs

- **circuit** (required) — QASM-3-ish JSON: `{ qubits, gates: [...] }`
- **backend** (optional) — `statevector` | `stabilizer` | `tensor_network` | `hardware` (default: auto)
- **shots** (optional, default 1024) — measurement repetitions

## Example

```text
/rulake-substrates:ruqu-simulate circuit='{"qubits":2,"gates":[{"H":0},{"CX":[0,1]}]}' shots=100
```

## What you get back

```jsonc
{
  "backend": "stabilizer",                 // auto-selected — Clifford-only ⇒ Stabilizer
  "samples": { "00": 51, "11": 49 },       // Bell pair
  "witness": "b1f4...32-byte-hex...",      // pinned on (circuit, backend, shots, seed)
  "elapsed_ms": 0.6,
  "consistency": "Frozen"
}
```

The auto-selector honors ADR-008 §4: Clifford-only circuits → Stabilizer (polynomial in qubits); arbitrary single-qubit + 2-qubit → StateVector up to 24 qubits, TensorNetwork past that.

## See also

- [ADR-008](../../../docs/adrs/ADR-008-ruqu-as-rulake-substrate.md) — the ruQu design
- [ruqu-v2-deep gist](../../../docs/gists/ruqu-v2-deep.md)
