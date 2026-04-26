//! Smoke tests for the v0.1 MCP server.
//!
//! Exercises the planner directly (so the bounded worker pool, the
//! decision-trace shape, and the search dispatch are all on the hot
//! path) without standing up the full stdio transport — that's the
//! v0.1 acceptance bar; the rmcp transport itself is rmcp's
//! responsibility, not ours.
//!
//! For an end-to-end stdio test, see `tests/stdio_e2e.rs` (v0.2).

use std::sync::Arc;

use ruvector_rulake::{LocalBackend, RuLake, BackendAdapter};
use ruvector_rulake_mcp::RuLakeMcpServer;

/// Build an in-memory ruLake with one collection of N×D vectors.
fn make_lake(n: usize, dim: usize) -> (Arc<RuLake>, Vec<Vec<f32>>, Vec<u64>) {
    let lake = RuLake::new(20, 42);
    let backend = Arc::new(LocalBackend::new("local"));
    let ids: Vec<u64> = (0..n as u64).collect();
    // Deterministic vectors so search results are predictable.
    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..dim)
                .map(|d| ((i * dim + d) as f32 * 0.001).sin())
                .collect()
        })
        .collect();
    backend
        .put_collection("docs", dim, ids.clone(), vectors.clone())
        .unwrap();
    let dyn_be: Arc<dyn BackendAdapter> = backend;
    lake.register_backend(dyn_be).unwrap();
    (Arc::new(lake), vectors, ids)
}

#[tokio::test]
async fn search_one_returns_top_k_via_planner() {
    let (lake, vectors, _ids) = make_lake(1_000, 32);

    let server = RuLakeMcpServer::from_lake(
        Arc::clone(&lake),
        "Fresh".to_string(),
        vec!["local".to_string()],
        64,
    )
    .unwrap();

    // The query is exactly vector 42 → expect id=42 in the top hit.
    let request = serde_json::json!({
        "intent": "search",
        "target": { "collection": "docs" },
        "search": { "vector": vectors[42], "k": 5 },
        "risk": "medium",
        "budget": { "max_results": 5, "max_backends": 1 }
    });
    let req = serde_json::from_value(request).unwrap();
    let resp = server.planner().handle(req).await.expect("planner ok");

    assert_eq!(resp.data.len(), 5, "expected k=5 hits");
    assert_eq!(resp.data[0].id, "42", "top-1 must be the self-vector");
    assert_eq!(resp.decision.chosen_action, "search_one");
    assert_eq!(resp.decision.backends_used, vec!["local".to_string()]);
    assert_eq!(resp.decision.degraded, false);
    assert!(resp.decision.budget_used_ms >= 0.0);
}

#[tokio::test]
async fn refuses_on_unknown_backend_with_policy_refused_allowlist() {
    let (lake, vectors, _) = make_lake(100, 16);
    let server = RuLakeMcpServer::from_lake(
        Arc::clone(&lake),
        "Fresh".to_string(),
        vec!["local".to_string()],
        64,
    )
    .unwrap();

    let request = serde_json::json!({
        "intent": "search",
        "target": { "routes": [["nope", "docs"]] },
        "search": { "vector": vectors[0], "k": 3 }
    });
    let req = serde_json::from_value(request).unwrap();
    let resp = server.planner().handle(req).await.expect("planner returns refusal as Ok");

    // Refusal is a successful planner response with empty data + reason_code.
    assert!(resp.data.is_empty(), "refused → empty data");
    assert_eq!(resp.decision.chosen_action, "refused");
    let code = format!("{:?}", resp.decision.reason_code);
    assert!(
        code.contains("PolicyRefusedAllowlist"),
        "expected PolicyRefusedAllowlist, got {code}"
    );
}

#[tokio::test]
async fn budget_max_results_caps_k() {
    let (lake, vectors, _) = make_lake(500, 16);
    let server = RuLakeMcpServer::from_lake(
        Arc::clone(&lake),
        "Fresh".to_string(),
        vec!["local".to_string()],
        64,
    )
    .unwrap();

    // Ask for k=50 but cap budget.max_results=10 — planner must clamp.
    let request = serde_json::json!({
        "intent": "search",
        "target": { "collection": "docs" },
        "search": { "vector": vectors[0], "k": 50 },
        "budget": { "max_results": 10, "max_backends": 1 }
    });
    let req = serde_json::from_value(request).unwrap();
    let resp = server.planner().handle(req).await.expect("planner ok");
    assert_eq!(resp.data.len(), 10, "budget.max_results must cap k");
}

#[tokio::test]
async fn decision_trace_has_required_fields_for_audit() {
    // Validates the §7 acceptance criterion: every audit line must
    // explain itself. The decision block is the load-bearing structure.
    let (lake, vectors, _) = make_lake(200, 16);
    let server = RuLakeMcpServer::from_lake(
        Arc::clone(&lake),
        "Eventual".to_string(),
        vec!["local".to_string()],
        64,
    )
    .unwrap();

    let req = serde_json::from_value(serde_json::json!({
        "intent": "search",
        "target": { "collection": "docs" },
        "search": { "vector": vectors[7], "k": 3 }
    }))
    .unwrap();
    let resp = server.planner().handle(req).await.expect("ok");

    assert!(!resp.decision.intent.is_empty());
    assert!(!resp.decision.chosen_action.is_empty());
    assert!(!resp.decision.reason.is_empty());
    assert!(!resp.decision.consistency_used.is_empty());
    assert!(resp.decision.budget_cap_ms > 0);
    // reason_code is enum so it's always present.
    let _ = format!("{:?}", resp.decision.reason_code);
}
