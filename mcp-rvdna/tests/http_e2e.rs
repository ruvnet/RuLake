//! End-to-end HTTP integration test for `mcp-rvdna` v0.0.1.
//!
//! Spins up the server on a random loopback port with one synthetic
//! `RvdnaT0Backend` collection registered, drives the MCP handshake,
//! and asserts that `rvdna_lineage` round-trips the witness over the
//! wire. This is the trust-anchor demo per ADR-007 §6 acceptance
//! criteria.
//!
//! Notes:
//! - rmcp's Streamable HTTP returns SSE for tool responses, so we
//!   strip `data:` framing before JSON-parsing.
//! - The `--auth none` loopback path is used: bearer/JWT lift in
//!   v0.0.2 alongside the mcp-server port.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rulake::backend::BackendAdapter;
use ruvector_rulake_mcp_rvdna::{
    AllowBearerOnPublic, AuthMode, CapabilitySet, InsecureAllowNoAuth, RvdnaMcpServer,
};
use ruvector_rulake_rvdna::{RvdnaCollection, RvdnaT0Backend};

fn make_server() -> (RvdnaMcpServer, String) {
    let server = RvdnaMcpServer::new()
        .with_capabilities(CapabilitySet::from_csv("read,internal").unwrap());
    let be = Arc::new(RvdnaT0Backend::new("rvdna-e2e"));
    be.put_collection(
        "chr1",
        RvdnaCollection {
            ids: (0..4).collect(),
            vectors: (0..4).map(|i| vec![i as f32; 8]).collect(),
            dim: 8,
            generation: 7,
        },
    );
    let dyn_be: Arc<dyn BackendAdapter> = be;
    server
        .register_collection(dyn_be, "chr1", 42, 20)
        .expect("register chr1");
    let pinned = server
        .registry()
        .get("rvdna-e2e", "chr1")
        .expect("pinned")
        .pinned_witness;
    (server, pinned)
}

#[tokio::test(flavor = "multi_thread")]
async fn rvdna_lineage_round_trips_pinned_witness_over_http() {
    // Find a free loopback port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let bind: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    let (server, expected_witness) = make_server();

    let serve = tokio::spawn(async move {
        ruvector_rulake_mcp_rvdna::http::serve(
            server,
            bind,
            AuthMode::None,
            AllowBearerOnPublic(false),
            InsecureAllowNoAuth(false),
        )
        .await
    });

    // Give the listener a beat to bind.
    tokio::time::sleep(Duration::from_millis(150)).await;

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/");

    // ── 1. initialize ────────────────────────────────────────────────
    let init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "rvdna-e2e", "version": "0" }
        }
    });
    let init_resp = client
        .post(&url)
        .header("accept", "application/json, text/event-stream")
        .json(&init)
        .send()
        .await
        .expect("initialize sent");
    assert!(
        init_resp.status().is_success(),
        "initialize HTTP status: {}",
        init_resp.status()
    );
    let session_id = init_resp
        .headers()
        .get("mcp-session-id")
        .and_then(|h| h.to_str().ok())
        .map(str::to_string)
        .unwrap_or_default();

    // ── 2. notifications/initialized ─────────────────────────────────
    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let mut req = client
        .post(&url)
        .header("accept", "application/json, text/event-stream")
        .json(&initialized);
    if !session_id.is_empty() {
        req = req.header("mcp-session-id", &session_id);
    }
    let _ = req.send().await;

    // ── 3. tools/call rvdna_lineage ──────────────────────────────────
    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "rvdna_lineage",
            "arguments": {
                "backend": "rvdna-e2e",
                "collection": "chr1"
            }
        }
    });
    let mut req = client
        .post(&url)
        .header("accept", "application/json, text/event-stream")
        .json(&call);
    if !session_id.is_empty() {
        req = req.header("mcp-session-id", &session_id);
    }
    let resp = req.send().await.expect("tools/call sent");
    let status = resp.status();
    let body = resp.text().await.expect("read body");
    serve.abort();

    assert!(
        status.is_success(),
        "tools/call HTTP status: {status}, body: {body}"
    );

    // The body is either bare JSON or SSE-wrapped JSON. Strip `data: `
    // framing if present and grab the first non-empty JSON-RPC payload —
    // rmcp's SSE stream emits a leading empty `data:` keepalive before
    // the actual response, so we have to skip blanks.
    let json_str = body
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .find(|s| !s.trim().is_empty())
        .unwrap_or(body.as_str());
    let v: serde_json::Value =
        serde_json::from_str(json_str).unwrap_or_else(|e| panic!("parse JSON: {e}\nbody:\n{body}"));

    // tools/call returns { result: { content: [ { type:"text", text:"<json>" } ], structuredContent: {...} } }
    // The Json<T> wrapper from rmcp serializes into `structuredContent`
    // (and into the text content as JSON). Read structuredContent first
    // for cleanliness; fall back to parsing the text content if not.
    let result = v.get("result").unwrap_or_else(|| {
        panic!("no `result` in JSON-RPC response: {v}");
    });
    let payload = if let Some(sc) = result.get("structuredContent") {
        sc.clone()
    } else {
        let text = result
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|first| first.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or_else(|| panic!("no text content in result: {result}"));
        serde_json::from_str(text).expect("parse text payload")
    };

    let live = payload
        .get("live_witness")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let pinned = payload
        .get("pinned_witness")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert_eq!(
        pinned, expected_witness,
        "pinned witness must match what register_collection captured"
    );
    assert_eq!(
        live, expected_witness,
        "live re-derived witness must equal pinned (no drift)"
    );
    assert_eq!(
        payload
            .get("witness_verified")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        true,
        "witness_verified must be true on a successful lineage call"
    );
}
