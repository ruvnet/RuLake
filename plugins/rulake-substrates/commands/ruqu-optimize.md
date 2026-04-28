---
description: ruQu — circuit-optimization pass (gate cancellation, commutation, T-count reduction). Returns the optimized circuit + a new witness pinned to the optimization parameters.
---

# /rulake-substrates:ruqu-optimize

Wraps the `ruqu_optimize` MCP tool. Pre-execution simplification.

## Inputs

- **circuit** (required)
- **passes** (optional, default `["cancel","commute"]`) — array of optimization passes
- **target** (optional) — `clifford-t` | `nisq` | `surface` (shapes the gate set the optimizer reduces to)

## Example

```text
/rulake-substrates:ruqu-optimize circuit='{"qubits":4,"gates":[{"H":0},{"H":0},{"CX":[0,1]}]}' passes='["cancel","commute"]'
```

## What you get back

```jsonc
{
  "optimized_circuit": { "qubits": 4, "gates": [{"CX":[0,1]}] },
  "passes_applied": ["cancel","commute"],
  "gate_count_before": 3,
  "gate_count_after": 1,
  "witness": "b1f4..."                     // pinned on (input, passes, target)
}
```

The optimized circuit can be fed to `ruqu_simulate` directly. Witness chains across the optimize → simulate boundary so a downstream `ruqu_verify` can confirm both steps.
