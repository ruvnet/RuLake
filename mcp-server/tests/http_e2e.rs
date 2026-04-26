//! End-to-end HTTP smoke tests for the v0.2 Streamable HTTP transport.
//!
//! Spawns the server on a random loopback port, sends a real HTTP
//! request, asserts the response. Covers:
//! - happy path: GET /.well-known/* (just verifies the listener is up)
//! - bearer-on-public refusal at startup
//! - bearer-on-loopback acceptance at startup
//!
//! Full MCP-over-Streamable-HTTP wire validation is left to v0.3 when
//! we add the OAuth-handshake fixtures (the rmcp Inspector tool covers
//! it interactively today).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ruvector_rulake::{LocalBackend, RuLake, BackendAdapter};
use ruvector_rulake_mcp::{
    AllowBearerOnPublic, AuthMode, BearerAuth, InsecureAllowNoAuth, RuLakeMcpServer,
};

fn make_server() -> RuLakeMcpServer {
    let lake = RuLake::new(20, 42);
    let be = Arc::new(LocalBackend::new("local"));
    be.put_collection(
        "docs",
        8,
        (0..50).collect(),
        (0..50).map(|i| vec![i as f32 * 0.01; 8]).collect(),
    )
    .unwrap();
    let dyn_be: Arc<dyn BackendAdapter> = be;
    lake.register_backend(dyn_be).unwrap();
    RuLakeMcpServer::from_lake(
        Arc::new(lake),
        "Fresh".into(),
        vec!["local".into()],
        64,
    )
    .unwrap()
}

#[tokio::test]
async fn http_serve_starts_on_loopback_with_no_auth() {
    let server = make_server();
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    // Use TcpListener to grab a free port, then close it before serve binds.
    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let bind: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    let serve = tokio::spawn(async move {
        ruvector_rulake_mcp::http::serve(
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

    // Hit the root with a plain HTTP GET. The rmcp Streamable HTTP
    // service rejects non-JSON-RPC requests with a 4xx but the TCP
    // listener accepting at all proves the transport is up.
    let mut conn = tokio::net::TcpStream::connect(bind).await.unwrap();
    use tokio::io::AsyncWriteExt;
    conn.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
    use tokio::io::AsyncReadExt;
    let mut buf = vec![0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(2), conn.read(&mut buf))
        .await
        .expect("response within 2s")
        .expect("read ok");
    assert!(n > 0, "got an HTTP response");
    let head = String::from_utf8_lossy(&buf[..n.min(64)]).to_string();
    assert!(
        head.starts_with("HTTP/1.1 ") || head.starts_with("HTTP/1.0 "),
        "looks like an HTTP response: {head}"
    );

    serve.abort();
}

#[tokio::test]
async fn http_refuses_bearer_on_public_without_override() {
    let server = make_server();
    let bind: SocketAddr = "0.0.0.0:0".parse().unwrap();
    let bearer = BearerAuth::new("dev-token-xyz");

    let result = ruvector_rulake_mcp::http::serve(
        server,
        bind,
        AuthMode::Bearer(bearer),
        AllowBearerOnPublic(false),
        InsecureAllowNoAuth(false),
    )
    .await;

    let err = result.unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("bearer is dev-only") || msg.contains("--allow-bearer-on-public"),
        "expected refusal mentioning --allow-bearer-on-public, got: {msg}"
    );
}

#[tokio::test]
async fn http_refuses_no_auth_on_public_without_override() {
    let server = make_server();
    let bind: SocketAddr = "0.0.0.0:0".parse().unwrap();

    let result = ruvector_rulake_mcp::http::serve(
        server,
        bind,
        AuthMode::None,
        AllowBearerOnPublic(false),
        InsecureAllowNoAuth(false),
    )
    .await;

    let err = result.unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("--insecure-allow-no-auth") || msg.contains("auth none"),
        "expected refusal mentioning --insecure-allow-no-auth, got: {msg}"
    );
}

#[tokio::test]
async fn bearer_auth_accepts_correct_token() {
    use http::HeaderMap;
    let bearer = BearerAuth::new("correct-horse-battery-staple");
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        "Bearer correct-horse-battery-staple".parse().unwrap(),
    );
    let principal = bearer.verify(&headers).expect("auth ok");
    assert!(
        principal.starts_with("bearer:"),
        "principal is bearer:<fingerprint>, got: {principal}"
    );
}

#[tokio::test]
async fn bearer_auth_rejects_wrong_token() {
    use http::HeaderMap;
    let bearer = BearerAuth::new("right");
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        "Bearer wrong".parse().unwrap(),
    );
    let err = bearer.verify(&headers).unwrap_err();
    assert_eq!(err, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bearer_auth_rejects_missing_header() {
    use http::HeaderMap;
    let bearer = BearerAuth::new("any");
    let headers = HeaderMap::new();
    let err = bearer.verify(&headers).unwrap_err();
    assert_eq!(err, http::StatusCode::UNAUTHORIZED);
}

// ─── v0.8c: end-to-end JWT scope downgrade on the wire ────────────────
//
// Status: scaffolded but `#[ignore]`d. The v0.8a scope-downgrade contract
// itself is already verified at the unit-level by
// `tests/smoke.rs::tools_list_filtered_by_capability_set`. This e2e test
// would prove the same contract holds across the rmcp Streamable HTTP
// transport — but consuming the response correctly requires reading the
// SSE stream until the JSON-RPC response arrives, then closing it. A bare
// `resp.text().await` hangs on the rmcp keepalive (`data:` empty events
// every ~3s with no terminating chunk). To unblock this we'd need an SSE
// consumer (`reqwest_eventsource` / `eventsource-stream`) or a server-side
// knob that returns a JSON body when the request can be served
// synchronously. Scaffolding kept (token mint, MCP handshake order,
// replay-guard-aware MCP-Request-Id seeds) — that part is right.

#[ignore = "rmcp Streamable HTTP SSE response stays open on keepalives; needs a proper SSE consumer to read the tools/list result. Contract verified by tools_list_filtered_by_capability_set."]
#[tokio::test(flavor = "multi_thread")]
async fn jwt_scope_downgrade_filters_tools_list_on_the_wire() {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use ruvector_rulake_mcp::auth::JwtKey;
    use ruvector_rulake_mcp::{
        AllowBearerOnPublic, CapabilitySet, InsecureAllowNoAuth, JwtAuth, JwtConfig,
    };

    let server = make_server();

    // Server starts with the maximum capability set — admin is the
    // ceiling. Per-call grants come from the token (the v0.8a contract).
    let server = ruvector_rulake_mcp::RuLakeMcpServer::from_lake_with_caps(
        std::sync::Arc::new(make_lake()),
        "Fresh".into(),
        vec!["local".into()],
        64,
        CapabilitySet::from_csv("read,publish,admin").unwrap(),
    )
    .unwrap();
    drop(server); // unused intermediate; keep make_server() result below.

    // (The duplicate `make_server()` above is intentional — the
    // `from_lake_with_caps` form is what the wire-test asserts; the
    // make_server() helper is the existing fixture.)
    let server_with_admin_caps = make_admin_server();

    // Find a free loopback port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let bind: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    // HMAC-shared secret for both ends.
    let secret = b"e2e-test-secret-32bytes-or-longer".to_vec();
    let issuer = "https://idp.test".to_string();
    let audience = "https://rulake.test/mcp".to_string();

    let jwt = JwtAuth::new(JwtConfig {
        key: JwtKey::Hmac(secret.clone()),
        issuer: issuer.clone(),
        audience: audience.clone(),
        algorithms: vec![Algorithm::HS256],
    });

    let serve = tokio::spawn(async move {
        ruvector_rulake_mcp::http::serve(
            server_with_admin_caps,
            bind,
            ruvector_rulake_mcp::AuthMode::Jwt(jwt),
            AllowBearerOnPublic(false),
            InsecureAllowNoAuth(false),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mint = |scope: &str| {
        let claims = serde_json::json!({
            "sub":   "alice",
            "iss":   issuer,
            "aud":   audience,
            "exp":   now + 60,
            "scope": scope,
        });
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(&secret),
        )
        .unwrap()
    };
    let read_token = mint("mcp:rulake:read");
    let admin_token = mint("mcp:rulake:read mcp:rulake:publish mcp:rulake:admin");

    // The MCP wire requires initialize → notifications/initialized →
    // tools/list. We do the minimal handshake then ask for tools/list,
    // varying the bearer.
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/");

    let n_tools_for = |token: String, seed_base: u32| {
        let url = url.clone();
        let client = client.clone();
        async move {
            // Send the three messages in one batch via a single
            // POST per the Streamable HTTP spec — initialize first
            // to negotiate the session, then notifications/initialized
            // (notification, no response), then tools/list. Each call
            // uses a fresh MCP-Request-Id triple — the replay guard
            // returns 409 on a duplicate id, so the second token must
            // not reuse seeds 1/2/3.
            let init = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": { "name": "e2e", "version": "0" }
                }
            });
            let init_resp = client
                .post(&url)
                .bearer_auth(&token)
                .header("MCP-Request-Id", uuid_like(seed_base))
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
            // Pull the session id from the response header.
            let session_id = init_resp
                .headers()
                .get("mcp-session-id")
                .and_then(|h| h.to_str().ok())
                .map(str::to_string)
                .unwrap_or_default();

            let initialized = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            });
            let mut req = client
                .post(&url)
                .bearer_auth(&token)
                .header("MCP-Request-Id", uuid_like(seed_base + 1))
                .header("accept", "application/json, text/event-stream")
                .json(&initialized);
            if !session_id.is_empty() {
                req = req.header("mcp-session-id", &session_id);
            }
            let _ = req.send().await;

            let list = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/list"
            });
            let mut req = client
                .post(&url)
                .bearer_auth(&token)
                .header("MCP-Request-Id", uuid_like(seed_base + 2))
                .header("accept", "application/json, text/event-stream")
                .json(&list);
            if !session_id.is_empty() {
                req = req.header("mcp-session-id", &session_id);
            }
            let resp = req.send().await.expect("tools/list sent");
            let status = resp.status();
            let body = resp.text().await.unwrap();
            eprintln!("[DEBUG seed_base={seed_base}] tools/list status={status} body={body}");
            // The body is either JSON or SSE-wrapped JSON. Strip
            // SSE framing if present.
            let json_str = body
                .lines()
                .find_map(|l| l.strip_prefix("data: "))
                .unwrap_or(body.as_str());
            let v: serde_json::Value = serde_json::from_str(json_str)
                .unwrap_or_else(|_| serde_json::json!({}));
            v.get("result")
                .and_then(|r| r.get("tools"))
                .and_then(|t| t.as_array())
                .map(|a| a.len())
                .unwrap_or(0)
        }
    };

    let n_read = n_tools_for(read_token, 1).await;
    let n_admin = n_tools_for(admin_token, 100).await;

    serve.abort();

    // The read-only token must see strictly fewer tools than the admin
    // token. Exact counts depend on session behaviour; the contract we
    // care about is the downgrade actually fires on the wire.
    assert!(
        n_admin > n_read,
        "v0.8a contract: admin token must see more tools than read-only \
         (read={n_read}, admin={n_admin})"
    );
    assert!(n_read >= 1, "read token sees ≥1 tool (rulake_query)");
}

fn make_admin_server() -> ruvector_rulake_mcp::RuLakeMcpServer {
    use ruvector_rulake_mcp::CapabilitySet;
    ruvector_rulake_mcp::RuLakeMcpServer::from_lake_with_caps(
        std::sync::Arc::new(make_lake()),
        "Fresh".into(),
        vec!["local".into()],
        64,
        CapabilitySet::from_csv("read,publish,admin").unwrap(),
    )
    .unwrap()
}

fn make_lake() -> ruvector_rulake::RuLake {
    use ruvector_rulake::{LocalBackend, RuLake, BackendAdapter};
    let lake = RuLake::new(20, 42);
    let be = std::sync::Arc::new(LocalBackend::new("local"));
    be.put_collection(
        "docs",
        8,
        (0..50).collect(),
        (0..50).map(|i| vec![i as f32 * 0.01; 8]).collect(),
    )
    .unwrap();
    let dyn_be: std::sync::Arc<dyn BackendAdapter> = be;
    lake.register_backend(dyn_be).unwrap();
    lake
}

fn uuid_like(seed: u32) -> String {
    // Deterministic 16-byte hex per request — the replay guard wants
    // each MCP-Request-Id distinct.
    format!("{:016x}", (seed as u64).wrapping_mul(0x9e3779b97f4a7c15))
}

#[tokio::test]
async fn bearer_auth_rejects_wrong_scheme() {
    use http::HeaderMap;
    let bearer = BearerAuth::new("any");
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        "Basic dXNlcjpwYXNz".parse().unwrap(),
    );
    let err = bearer.verify(&headers).unwrap_err();
    assert_eq!(err, http::StatusCode::UNAUTHORIZED);
}
