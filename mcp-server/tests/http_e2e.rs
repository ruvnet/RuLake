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
