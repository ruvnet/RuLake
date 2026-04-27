//! Integration tests for `mcp-ruqu` v0.0.1 (ADR-008 §6).
//!
//! Two scenarios from the brief:
//!   1. Bell-pair end-to-end: simulate → capture witness → verify
//!      with the same circuit + witness → assert `matches == true`.
//!   2. R-2 cap enforcement: simulate with `n_qubits = 20` → assert
//!      response (here: McpError) carries `RUQU_QUBIT_CAP_EXCEEDED`.

use ruvector_rulake_mcp_ruqu::{
    RuquMcpServer, SimulateRequest, VerifyRequest,
};
use ruvector_rulake_mcp_ruqu::server::{WireCircuit, WireGate};

/// 1. Bell-pair simulate → verify round-trip.
#[tokio::test]
async fn bell_pair_simulate_then_verify_matches() {
    let server = RuquMcpServer::new("ruqu-sv");

    let circuit = WireCircuit {
        n_qubits: 2,
        gates: vec![
            WireGate::H { q: 0 },
            WireGate::Cx { control: 0, target: 1 },
        ],
        id: "bell-pair-integration".to_string(),
    };

    // Simulate.
    let sim = server
        .call_simulate(SimulateRequest {
            circuit: circuit.clone(),
            backend_tag: "sv".to_string(),
            seed: 0,
        })
        .await
        .expect("simulate ok");

    // Witness must be a 64-char hex string (SHAKE-256(32) → 64 hex).
    assert_eq!(sim.witness.len(), 64, "SHAKE-256(32) -> 64 hex chars");
    assert_eq!(sim.dim, 4);
    assert_eq!(sim.amplitudes_head.len(), 4);
    assert!(!sim.truncated);

    // Verify with the same (circuit, witness, backend_tag, seed).
    let v = server
        .call_verify(VerifyRequest {
            circuit,
            witness: sim.witness.clone(),
            backend_tag: "sv".to_string(),
            seed: 0,
        })
        .await
        .expect("verify ok");

    assert!(v.matches, "round-trip witness must match");
    assert_eq!(v.computed_witness, sim.witness);

    // Audit invariants: simulate emitted "ok" with witness_out;
    // verify emitted "ok" with witness_in == witness_out.
    let tail = server.audit().tail(8);
    let codes: Vec<String> = tail
        .iter()
        .map(|e| e["tool"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(codes.contains(&"ruqu_simulate".to_string()));
    assert!(codes.contains(&"ruqu_verify".to_string()));
}

/// 2. R-2 enforcement: simulate at `n_qubits = 20` is hard-refused.
#[tokio::test]
async fn r2_simulate_above_cap_is_refused_with_audit_code() {
    let server = RuquMcpServer::new("ruqu-sv");

    // n=20 > MAX_QUBITS_V0_0 (16) → must refuse.
    let circuit = WireCircuit {
        n_qubits: 20,
        gates: vec![],
        id: "oversized-circuit".to_string(),
    };

    let err = server
        .call_simulate(SimulateRequest {
            circuit,
            backend_tag: "sv".to_string(),
            seed: 0,
        })
        .await
        .expect_err("must refuse n=20");

    let msg = err.message.to_string();
    assert!(
        msg.contains("RUQU_QUBIT_CAP_EXCEEDED"),
        "error message must carry the RUQU_QUBIT_CAP_EXCEEDED code, got: {msg}"
    );

    // The audit row carries the same code so a downstream cost
    // pipeline can filter on it without reparsing the wire error.
    let tail = server.audit().tail(1);
    let last = tail.last().expect("audit emitted exactly one row");
    assert_eq!(last["code"], "RUQU_QUBIT_CAP_EXCEEDED");
    assert_eq!(last["outcome"], "refused");
    assert_eq!(last["tool"], "ruqu_simulate");
    // policy_decision block must be present even on refusal
    // (commit 56b497b's "fully-shaped audit" discipline).
    assert!(last["policy_decision"].is_object());
    assert_eq!(last["policy_decision"]["capability_required"], "publish");
}
