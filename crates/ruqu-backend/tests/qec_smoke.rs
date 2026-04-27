//! End-to-end smoke for the QEC scheduler (v0.2).
//! Validates the ADR-008 §4 contract: given a logical circuit + a
//! target code distance, return a physical-circuit plan with encoding,
//! syndrome extraction, and recovery, refusing footprint overflows.

use ruvector_rulake_ruqu::{plan_surface_code, Circuit, Gate, QecError};

fn one_qubit_logical() -> Circuit {
    let mut c = Circuit::new("logical-1q-h", 1);
    c.push(Gate::H { q: 0 });
    c
}

#[test]
fn repetition_code_at_distance_3_returns_3_qubit_plan() {
    let logical = one_qubit_logical();
    let plan = plan_surface_code(&logical, 3, 64)
        .expect("d=3 with cap=64 must succeed");

    assert_eq!(plan.code_distance, 3);
    assert_eq!(
        plan.physical_qubits, 3,
        "repetition code reserves 3 physical qubits per logical qubit"
    );
    // Encoding circuit should fan out the logical qubit into the two
    // sibling physical qubits — at least 2 CXes for 1 logical qubit.
    assert!(
        plan.encoding.gates.len() >= 2,
        "expected at least 2 encoding gates for d=3 repetition; got {}",
        plan.encoding.gates.len()
    );
    assert_eq!(plan.syndromes.len(), 1, "v0.2 emits one syndrome round");
    assert!(
        !plan.syndromes[0].gates.is_empty(),
        "syndrome round must emit at least one gate"
    );
    // Logical circuit is preserved untouched.
    assert_eq!(plan.logical.id, logical.id);
    assert_eq!(plan.logical.gates.len(), logical.gates.len());
}

#[test]
fn surface_code_distance_5_within_cap() {
    let logical = one_qubit_logical();
    let plan = plan_surface_code(&logical, 5, 64)
        .expect("d=5 with cap=64 must succeed (footprint 2·25 = 50 ≤ 64)");
    assert_eq!(plan.code_distance, 5);
    assert_eq!(plan.physical_qubits, 50);
    assert!(
        !plan.encoding.gates.is_empty(),
        "d=5 encoding must emit gates"
    );
    assert_eq!(plan.syndromes.len(), 1);
}

#[test]
fn qec_refuses_on_footprint_overflow() {
    let logical = one_qubit_logical();
    let err = plan_surface_code(&logical, 7, 10)
        .expect_err("d=7 with cap=10 must overflow the footprint");
    match err {
        QecError::DistanceTooLargeForFootprint { d, footprint, cap } => {
            assert_eq!(d, 7);
            assert_eq!(cap, 10);
            assert!(
                footprint > cap,
                "footprint must exceed the cap; got footprint={footprint}, cap={cap}"
            );
        }
        other => panic!("expected DistanceTooLargeForFootprint, got {other:?}"),
    }
}
