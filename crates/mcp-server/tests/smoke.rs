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

use rulake::{LocalBackend, RuLake, BackendAdapter};
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

// ─── v0.7: structured backpressure response (ADR-004 §6) ─────────────

#[tokio::test]
async fn backpressure_response_carries_advice_block() {
    use ruvector_rulake_mcp::planner::BackpressureReason;
    let (lake, _, _) = make_lake(50, 16);
    let server = RuLakeMcpServer::from_lake(
        Arc::clone(&lake),
        "Fresh".to_string(),
        vec!["local".to_string()],
        64,
    )
    .unwrap();
    let resp = server.planner().build_degraded_response(
        "search",
        BackpressureReason::RateLimitCollection,
        100,
    );
    assert!(resp.data.is_empty(), "degraded → empty data");
    assert!(resp.decision.degraded, "decision.degraded = true");
    let advice = resp.decision.degraded_advice.as_ref().expect("advice block present");
    assert_eq!(advice.reason, "rate_limit_collection");
    assert!(advice.retry_after_ms > 0);
    assert!(
        advice.hints.iter().any(|h| h == "narrow_target"),
        "rate_limit_collection should suggest narrow_target, got: {:?}",
        advice.hints
    );
}

#[tokio::test]
async fn backpressure_inflight_cap_carries_inflight_numbers() {
    use ruvector_rulake_mcp::planner::BackpressureReason;
    let (lake, _, _) = make_lake(50, 16);
    let server = RuLakeMcpServer::from_lake(
        Arc::clone(&lake),
        "Fresh".to_string(),
        vec!["local".to_string()],
        64,
    )
    .unwrap();
    let resp = server.planner().build_degraded_response(
        "search",
        BackpressureReason::InflightCap { inflight: 64, cap: 64 },
        100,
    );
    assert!(resp.decision.degraded);
    let advice = resp.decision.degraded_advice.unwrap();
    assert!(advice.reason.contains("inflight=64"));
    assert!(advice.reason.contains("cap=64"));
}

// ─── v0.4: tools/list capability filter ───────────────────────────────

#[tokio::test]
async fn tools_list_filtered_by_capability_set() {
    use rulake::{LocalBackend, RuLake, BackendAdapter};
    use ruvector_rulake_mcp::CapabilitySet;

    let lake = RuLake::new(20, 42);
    let be = std::sync::Arc::new(LocalBackend::new("local"));
    let dyn_be: std::sync::Arc<dyn BackendAdapter> = be;
    lake.register_backend(dyn_be).unwrap();
    let lake = std::sync::Arc::new(lake);

    // Read-only mode: should see only rulake_query + rulake_list_backends.
    let read_only = RuLakeMcpServer::from_lake_with_caps(
        std::sync::Arc::clone(&lake),
        "Fresh".into(),
        vec!["local".into()],
        64,
        CapabilitySet::default(),
    )
    .unwrap();
    let names = list_tool_names_via_handler(&read_only).await;
    // v0.9 added rulake_list_collections — also a Read-tier tool.
    assert_eq!(
        names,
        vec!["rulake_list_backends", "rulake_list_collections", "rulake_query"],
    );

    // read,publish: + 2 publish tools.
    let publish = RuLakeMcpServer::from_lake_with_caps(
        std::sync::Arc::clone(&lake),
        "Fresh".into(),
        vec!["local".into()],
        64,
        CapabilitySet::from_csv("read,publish").unwrap(),
    )
    .unwrap();
    let names = list_tool_names_via_handler(&publish).await;
    assert!(names.contains(&"rulake_publish_bundle".to_string()));
    assert!(names.contains(&"rulake_refresh_from_bundle_dir".to_string()));
    assert!(!names.contains(&"rulake_invalidate_cache".to_string()), "admin tool must stay hidden");

    // read,publish,admin: all 7.
    let admin = RuLakeMcpServer::from_lake_with_caps(
        std::sync::Arc::clone(&lake),
        "Fresh".into(),
        vec!["local".into()],
        64,
        CapabilitySet::from_csv("read,publish,admin").unwrap(),
    )
    .unwrap();
    let names = list_tool_names_via_handler(&admin).await;
    assert_eq!(names.len(), 8, "admin sees all 8 tools, got {names:?}");
    for required in &[
        "rulake_query", "rulake_list_backends", "rulake_list_collections",
        "rulake_publish_bundle", "rulake_refresh_from_bundle_dir",
        "rulake_save_cache_to_dir", "rulake_warm_from_dir",
        "rulake_invalidate_cache",
    ] {
        assert!(names.iter().any(|n| n == required), "missing {required}");
    }

    async fn list_tool_names_via_handler(s: &RuLakeMcpServer) -> Vec<String> {
        // Probe the filter via the test-only accessor that mirrors
        // the ServerHandler::list_tools implementation. The wire-
        // level path is also verified at the binary smoke step.
        s.list_tools_filtered()
    }
}

// ─── v0.4: per-collection RBAC (allow-list) ───────────────────────────

#[tokio::test]
async fn rbac_denies_unallowed_collection() {
    use rulake::{LocalBackend, RuLake, BackendAdapter};
    use ruvector_rulake_mcp::AllowList;
    use ruvector_rulake_mcp::config::AllowBlock;
    use ruvector_rulake_mcp::planner::Planner;
    use ruvector_rulake_mcp::WorkerPool;

    let lake = RuLake::new(20, 42);
    let be = std::sync::Arc::new(LocalBackend::new("local"));
    be.put_collection("docs", 8, vec![0u64], vec![vec![0.0_f32; 8]]).unwrap();
    be.put_collection("secret", 8, vec![0u64], vec![vec![0.0_f32; 8]]).unwrap();
    let dyn_be: std::sync::Arc<dyn BackendAdapter> = be;
    lake.register_backend(dyn_be).unwrap();

    // Allow only `docs`, NOT `secret`.
    let allow = AllowList::from_blocks(&[AllowBlock {
        backend: "local".into(),
        collection: "docs".into(),
        caps: vec!["read".into()],
    }])
    .unwrap();

    let workers = WorkerPool::new(0, 64).unwrap();
    let planner = Planner {
        lake: std::sync::Arc::new(lake),
        workers,
        backend_ids: vec!["local".into()],
        consistency_label: "Fresh".into(),
        allow,
    };

    // 1. Allowed collection works.
    let req_ok = serde_json::from_value(serde_json::json!({
        "intent": "search",
        "target": { "collection": "docs" },
        "search": { "vector": vec![0.0_f32; 8], "k": 1 }
    }))
    .unwrap();
    let r1 = planner.handle(req_ok).await.expect("ok");
    assert!(!r1.data.is_empty(), "allowed route returns data");
    assert_ne!(r1.decision.chosen_action, "refused");

    // 2. Denied collection refuses with reason_code = PolicyRefusedAllowlist.
    let req_denied = serde_json::from_value(serde_json::json!({
        "intent": "search",
        "target": { "routes": [["local", "secret"]] },
        "search": { "vector": vec![0.0_f32; 8], "k": 1 }
    }))
    .unwrap();
    let r2 = planner.handle(req_denied).await.expect("planner returns refusal as Ok");
    assert!(r2.data.is_empty(), "denied route → empty data");
    let code = format!("{:?}", r2.decision.reason_code);
    assert!(
        code.contains("PolicyRefusedAllowlist"),
        "expected PolicyRefusedAllowlist, got {code}"
    );
    assert!(
        r2.decision.refusals.iter().any(|r| r.code.contains("ALLOWLIST_DENIED")),
        "refusals must name the rule that denied"
    );
}

// ─── v0.3d: capability gating ─────────────────────────────────────────

#[tokio::test]
async fn capability_intersect_keeps_only_both_grants() {
    use ruvector_rulake_mcp::{Capability, CapabilitySet};
    let server = CapabilitySet::from_csv("read,publish,admin").unwrap();
    let token = CapabilitySet::from_csv("read,publish").unwrap();
    let effective = server.intersect(&token);
    assert!(effective.has(Capability::Read));
    assert!(effective.has(Capability::Publish));
    assert!(!effective.has(Capability::Admin), "admin not in token → not in intersection");
}

#[tokio::test]
async fn capability_intersect_empty_token_yields_empty() {
    use ruvector_rulake_mcp::CapabilitySet;
    let server = CapabilitySet::from_csv("read,publish,admin").unwrap();
    let token = CapabilitySet::default(); // read-only
    let effective = server.intersect(&token);
    // Both have read; intersection is just read.
    assert_eq!(effective.labels(), vec!["read"]);
}

#[tokio::test]
async fn effective_caps_falls_through_to_server_wide_when_task_local_unset() {
    use ruvector_rulake_mcp::{Capability, CapabilitySet};
    use ruvector_rulake_mcp::policy::effective_caps;
    let server = CapabilitySet::from_csv("read,publish").unwrap();
    // Outside any REQUEST_CAPS scope — fall through.
    let effective = effective_caps(&server);
    assert!(effective.has(Capability::Read));
    assert!(effective.has(Capability::Publish));
}

#[tokio::test]
async fn effective_caps_intersects_inside_task_local_scope() {
    use ruvector_rulake_mcp::{Capability, CapabilitySet};
    use ruvector_rulake_mcp::policy::{REQUEST_CAPS, effective_caps};
    let server = CapabilitySet::from_csv("read,publish,admin").unwrap();
    let token = CapabilitySet::from_csv("read").unwrap();
    let result = REQUEST_CAPS.scope(token, async move {
        let effective = effective_caps(&server);
        effective.has(Capability::Admin)
    }).await;
    assert!(!result, "token-restricted scope must downgrade server's admin to read");
}

#[tokio::test]
async fn capability_set_default_excludes_publish_and_admin() {
    use ruvector_rulake_mcp::{Capability, CapabilitySet};
    let cs = CapabilitySet::default();
    assert!(cs.has(Capability::Read));
    assert!(!cs.has(Capability::Publish));
    assert!(!cs.has(Capability::Admin));
    assert!(!cs.has(Capability::Internal));
}

#[tokio::test]
async fn capability_set_publish_implies_read() {
    use ruvector_rulake_mcp::{Capability, CapabilitySet};
    let cs = CapabilitySet::from_csv("publish").unwrap();
    assert!(cs.has(Capability::Read), "publish must implicitly grant read");
    assert!(cs.has(Capability::Publish));
    assert!(!cs.has(Capability::Admin));
}

#[tokio::test]
async fn capability_set_rejects_unknown_label() {
    use ruvector_rulake_mcp::CapabilitySet;
    let err = CapabilitySet::from_csv("foo").unwrap_err();
    assert!(format!("{err:#}").contains("unknown capability"));
}

// ─── v0.3a: verify + explain intents ─────────────────────────────────

#[tokio::test]
async fn explain_intent_returns_cache_stats_rollup() {
    let (lake, _, _) = make_lake(500, 16);
    let server = RuLakeMcpServer::from_lake(
        Arc::clone(&lake),
        "Eventual".to_string(),
        vec!["local".to_string()],
        64,
    )
    .unwrap();

    let req = serde_json::from_value(serde_json::json!({
        "intent": "explain",
        "target": { "collection": "docs" }
    }))
    .unwrap();
    let resp = server.planner().handle(req).await.expect("ok");
    assert_eq!(resp.decision.intent, "explain");
    assert_eq!(resp.decision.chosen_action, "cache_stats_rollup");
    assert!(
        resp.decision.reason.contains("hit_rate"),
        "reason should carry the rollup string"
    );
}

#[tokio::test]
async fn verify_intent_refuses_on_missing_bundle() {
    let (lake, _, _) = make_lake(100, 16);
    let server = RuLakeMcpServer::from_lake(
        Arc::clone(&lake),
        "Fresh".to_string(),
        vec!["local".to_string()],
        64,
    )
    .unwrap();

    let req = serde_json::from_value(serde_json::json!({
        "intent": "verify",
        "target": { "collection": "docs" },
        "verify": { "bundle_dir": "/nonexistent/path" }
    }))
    .unwrap();
    let resp = server.planner().handle(req).await.expect("ok");
    assert!(resp.data.is_empty(), "verify refusal → empty data");
    let code = format!("{:?}", resp.decision.reason_code);
    assert!(
        code.contains("WitnessMismatchRefused"),
        "missing bundle should map to WitnessMismatchRefused (fail-closed), got {code}"
    );
}

#[tokio::test]
async fn verify_intent_via_backend_returns_real_dim_and_witness() {
    // v0.7: with via_backend=true, verify reaches through to the
    // route's BackendAdapter::current_bundle (no disk path required).
    // For LocalBackend this hands back the in-memory bundle with the
    // real dim + the canonical Generation::Num generation.
    let (lake, _, _) = make_lake(100, 32);
    let server = RuLakeMcpServer::from_lake(
        Arc::clone(&lake),
        "Fresh".to_string(),
        vec!["local".to_string()],
        64,
    )
    .unwrap();
    let req = serde_json::from_value(serde_json::json!({
        "intent": "verify",
        "target": { "collection": "docs" },
        "verify": { "via_backend": true }
    }))
    .unwrap();
    let resp = server.planner().handle(req).await.expect("ok");
    assert_eq!(resp.decision.intent, "verify");
    assert!(
        resp.provenance.witness_verified,
        "BackendAdapter::current_bundle returns a freshly-built bundle whose witness verifies"
    );
    let witness = resp.provenance.witness.as_ref().expect("witness present");
    assert_eq!(witness.len(), 64, "SHAKE-256(32) hex");
}

#[tokio::test]
async fn verify_intent_succeeds_on_valid_bundle() {
    use rulake::RuLakeBundle;
    use tempfile::TempDir;

    let (lake, _, _) = make_lake(100, 16);
    let server = RuLakeMcpServer::from_lake(
        Arc::clone(&lake),
        "Fresh".to_string(),
        vec!["local".to_string()],
        64,
    )
    .unwrap();

    // Write a fresh bundle to a tempdir.
    let dir = TempDir::new().unwrap();
    let bundle = RuLakeBundle::new(
        "test://verify",
        16,
        42,
        20,
        rulake::Generation::Num(1),
    );
    bundle.write_to_dir(dir.path()).unwrap();

    let req = serde_json::from_value(serde_json::json!({
        "intent": "verify",
        "target": { "collection": "docs" },
        "verify": { "bundle_dir": dir.path().to_string_lossy() }
    }))
    .unwrap();
    let resp = server.planner().handle(req).await.expect("ok");
    assert_eq!(resp.decision.intent, "verify");
    assert!(
        resp.provenance.witness_verified,
        "valid bundle must verify"
    );
    assert_eq!(
        resp.provenance.witness.as_ref().unwrap(),
        &bundle.rvf_witness,
        "verified witness must match the bundle"
    );
}

#[tokio::test]
async fn refresh_intent_returns_bundle_missing_when_dir_empty() {
    use tempfile::TempDir;
    let (lake, _, _) = make_lake(50, 16);
    let server = RuLakeMcpServer::from_lake(
        Arc::clone(&lake),
        "Fresh".to_string(),
        vec!["local".to_string()],
        64,
    )
    .unwrap();
    let dir = TempDir::new().unwrap();
    let req = serde_json::from_value(serde_json::json!({
        "intent": "refresh",
        "target": { "collection": "docs" },
        "refresh": { "bundle_dir": dir.path().to_string_lossy() }
    }))
    .unwrap();
    let resp = server.planner().handle(req).await.expect("planner ok");
    assert_eq!(resp.decision.intent, "refresh");
    assert_eq!(resp.decision.chosen_action, "refresh_from_bundle_dir");
    // Empty directory → BundleMissing for the route — reason carries the outcome string.
    assert!(
        resp.decision.reason.contains("BundleMissing"),
        "expected BundleMissing in reason, got: {}",
        resp.decision.reason
    );
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
