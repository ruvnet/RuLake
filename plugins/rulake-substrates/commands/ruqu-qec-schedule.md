---
description: ruQu QEC — surface-code planner. Compute the syndrome-extraction schedule, code-distance overhead, and logical-qubit footprint for a given circuit and noise budget.
---

# /rulake-substrates:ruqu-qec-schedule

Wraps the `ruqu_qec_schedule` MCP tool. Plans surface-code QEC overhead before you commit to running on noisy hardware.

## Inputs

- **circuit** (required) — the *logical* circuit to protect
- **target_logical_error_rate** (required) — e.g. `1e-12`
- **physical_error_rate** (optional, default `1e-3`) — assumed per-gate physical error
- **code** (optional, default `"surface"`) — currently only `"surface"`

## Example

```text
/rulake-substrates:ruqu-qec-schedule circuit='{"qubits":2,"gates":[{"H":0},{"CX":[0,1]}]}' target_logical_error_rate=1e-12 physical_error_rate=1e-3
```

## What you get back

```jsonc
{
  "code_distance": 17,
  "physical_qubits_per_logical": 578,
  "total_physical_qubits": 1156,           // 2 logical × 578
  "syndrome_rounds": 50,
  "estimated_runtime_us": 5000,
  "witness": "b1f4..."
}
```

This is **planning**, not execution — the answer tells you whether the circuit fits in the QEC overhead budget your hardware can sustain. The full execution path lands when `ruqu_simulate --backend hardware` is wired to a real device.

## See also

- [ADR-008 §QEC](../../../docs/adrs/ADR-008-ruqu-as-rulake-substrate.md)
