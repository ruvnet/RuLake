# rulake-ruqu

The quantum-simulation substrate plugin for ruLake. Standalone — install when your agent needs to run, verify, or plan quantum circuits with a witness chain.

## What it adds

| Surface | Detail |
|---|---|
| **MCP wire** | `ruqu-mcp` → `https://ruqu-mcp.ruv.io/` (public demo, publish-tier) |
| **5 tools** | `ruqu_simulate`, `ruqu_verify`, `ruqu_replay`, `ruqu_optimize`, `ruqu_qec_schedule` |
| **5 commands** | `/rulake-ruqu:ruqu-simulate`, `…-verify`, `…-replay`, `…-optimize`, `…-qec-schedule` |

## Install

```text
/plugin marketplace add ruvnet/RuLake          # if not already
/plugin install rulake-ruqu@rulake-marketplace
/reload-plugins
/rulake-ruqu:ruqu-simulate circuit='{"qubits":2,"gates":[{"H":0},{"CX":[0,1]}]}' shots=100
```

## Backends and the auto-pick

`ruqu_simulate` chooses the backend per the circuit's gate set + qubit count:

| Backend | When picked | Cost |
|---|---|---|
| **StateVector** | arbitrary gates, n_qubits ≤ 24 | exponential in qubits |
| **Stabilizer** | Clifford-only circuits, any n_qubits | polynomial in qubits |
| **TensorNetwork** | n_qubits > 24, low entanglement | polynomial in bond-dim |
| **Hardware** | when an external device is wired | per-device |

`ruqu_replay` runs the same circuit on a different backend and verifies the witnesses match — Stabilizer-fast vs StateVector-exact agreement is the load-bearing Clifford-correctness gate.

## Status (all 5 tools live with real upstream lib)

| Tool | Engine |
|---|---|
| `ruqu_simulate` | `ruqu_core::Simulator::run` — production SIMD-accelerated state-vector simulator (~26k LOC) |
| `ruqu_verify` | real witness re-derive |
| `ruqu_replay` | same engine as `simulate`, witness-equality check |
| `ruqu_optimize` | `ruqu_core::optimizer::fuse_gates` + `circuit_analyzer::{is_clifford, count_non_clifford}` |
| `ruqu_qec_schedule` | `ruqu_algorithms::surface_code::run_surface_code` — distance-3 rotated surface code, 9 data qubits + 8 ancillas, X/Z stabilizer measurement, lookup decoder |

n_qubits ≤ 16 in v0.0; circuit `id` MUST be content-derived (≤ 256 bytes).

## Production deploy

The public demo wires `https://ruqu-mcp.ruv.io/` (`--auth none --insecure-allow-no-auth --capabilities read,publish`). For production: deploy your own per [`docs/deploy/cloud-run.md`](../../docs/deploy/cloud-run.md), point `.mcp.json` at your URL.

## See also

- [`ruqu-v2-deep.md`](../../docs/gists/ruqu-v2-deep.md) — the deep design walkthrough (3,054 words)
- [`rulake-rvdna`](../rulake-rvdna/) — the sibling genomic substrate plugin
