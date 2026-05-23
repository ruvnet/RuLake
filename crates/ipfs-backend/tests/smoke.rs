//! Smoke tests for `IpfsBackend`.
//!
//! Two paths:
//! - **Offline (always runs):** uses a tiny tokio-spawned HTTP server
//!   that mocks kubo's `add` / `cat` / `pin/add` endpoints.
//! - **Live (gated on `RULAKE_IPFS_LIVE_TEST=1`):** talks to a real
//!   kubo at `RULAKE_IPFS_KUBO_API` (default `http://127.0.0.1:5001`).
//!
//! Plain `#[test]` (not `#[tokio::test]`) so the test thread isn't
//! inside a tokio context — `IpfsBackend` runs its own internal
//! current-thread runtime via `block_on`, and nested `block_on`s
//! panic on drop. The mock server runs on a separate long-lived
//! multi-thread runtime owned by the test.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;

use rulake::backend::BackendAdapter;
use rulake::bundle::{Generation, RuLakeBundle};
use ruvector_rulake_ipfs::{IpfsBackend, IpfsCollection, KuboApiUrl, Mode};

const FAKE_CID: &str = "bafybeifaketestcidforipfsbackendsmokeonlyy";

#[derive(Default, Clone)]
struct MockState {
    blocks: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>,
    pins: Arc<Mutex<std::collections::HashSet<String>>>,
}

/// Spawns a mock kubo HTTP server on the given runtime, returns the
/// listening port. Server task lives until the runtime is dropped.
async fn spawn_mock_kubo(state: MockState) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let state = state.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                    let state = state.clone();
                    async move { handle(req, state).await }
                });
                let _ = auto::Builder::new(TokioExecutor::new())
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });
    port
}

async fn handle(
    req: hyper::Request<hyper::body::Incoming>,
    state: MockState,
) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let uri = req.uri().clone();
    let path = uri.path();
    let query = uri.query().unwrap_or("");

    if path == "/api/v0/add" {
        let body = req.collect().await.unwrap().to_bytes();
        let s = String::from_utf8_lossy(&body);
        if let (Some(start), Some(end)) = (s.find('{'), s.rfind('}')) {
            let payload = body[start..=end].to_vec();
            state
                .blocks
                .lock()
                .unwrap()
                .insert(FAKE_CID.into(), payload);
        }
        let json = format!(r#"{{"Name":"bundle.json","Hash":"{FAKE_CID}","Size":"256"}}"#);
        return Ok(json_response(json));
    }
    if path == "/api/v0/cat" {
        let cid = parse_arg(query).unwrap_or_default();
        if let Some(body) = state.blocks.lock().unwrap().get(&cid) {
            return Ok(bytes_response(body.clone()));
        }
        return Ok(empty_404());
    }
    if path == "/api/v0/pin/add" {
        let cid = parse_arg(query).unwrap_or_default();
        state.pins.lock().unwrap().insert(cid);
        return Ok(json_response(r#"{"Pins":["pinned"]}"#.into()));
    }
    Ok(empty_404())
}

fn parse_arg(query: &str) -> Option<String> {
    for kv in query.split('&') {
        if let Some(rest) = kv.strip_prefix("arg=") {
            return Some(rest.to_string());
        }
    }
    None
}

fn json_response(body: String) -> hyper::Response<Full<Bytes>> {
    hyper::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}
fn bytes_response(body: Vec<u8>) -> hyper::Response<Full<Bytes>> {
    hyper::Response::builder()
        .status(200)
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}
fn empty_404() -> hyper::Response<Full<Bytes>> {
    hyper::Response::builder()
        .status(404)
        .body(Full::new(Bytes::new()))
        .unwrap()
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

// ─── Tests ───────────────────────────────────────────────────────────

#[test]
fn publish_then_fetch_roundtrip_offline() {
    let runtime = rt();
    let state = MockState::default();
    let port = runtime.block_on(spawn_mock_kubo(state.clone()));

    let backend = IpfsBackend::new(
        "ipfs",
        Mode::Kubo {
            api: KuboApiUrl(format!("http://127.0.0.1:{port}")),
            api_token: None,
        },
    )
    .unwrap();
    backend
        .register(IpfsCollection {
            name: "memories".into(),
            cid: None,
            dim: None,
        })
        .unwrap();

    let bundle = RuLakeBundle::new(
        format!("ipfs://{FAKE_CID}"),
        128,
        42,
        20,
        Generation::Opaque(FAKE_CID.into()),
    );
    let cid = backend.publish_bundle("memories", &bundle).unwrap();
    assert_eq!(cid, FAKE_CID);
    assert!(state.pins.lock().unwrap().contains(FAKE_CID));

    let fetched = backend.fetch_bundle("memories").unwrap();
    assert_eq!(fetched.dim, 128);
    assert_eq!(fetched.rvf_witness, bundle.rvf_witness);
    assert!(fetched.verify_witness());
}

#[test]
fn current_bundle_fast_path_skips_network() {
    // Operator pre-set CID + dim → cheap-path. No mock server needed.
    let backend = IpfsBackend::new(
        "ipfs",
        Mode::Kubo {
            api: KuboApiUrl("http://127.0.0.1:1".into()),
            api_token: None,
        },
    )
    .unwrap();
    backend
        .register(IpfsCollection {
            name: "wit".into(),
            cid: Some(FAKE_CID.into()),
            dim: Some(64),
        })
        .unwrap();

    let bundle = backend.current_bundle("wit", 42, 20).unwrap();
    assert_eq!(bundle.dim, 64);
    assert_eq!(bundle.data_ref, format!("ipfs://{FAKE_CID}"));
    assert!(bundle.verify_witness());
    match bundle.generation {
        Generation::Opaque(s) => assert_eq!(s, FAKE_CID),
        other => panic!("expected Opaque(cid), got {other:?}"),
    }
}

#[test]
fn pull_vectors_errors_with_bundle_only_message() {
    let backend = IpfsBackend::kubo_local("ipfs").unwrap();
    let err = backend.pull_vectors("anything").unwrap_err();
    let s = err.to_string();
    assert!(
        s.contains("bundle-only") && s.contains("ADR-005 §1"),
        "expected ADR-005 §1 bundle-only error, got: {s}"
    );
}

#[test]
fn gateway_only_mode_refuses_publish() {
    let backend = IpfsBackend::gateway_only("ipfs-gw", "https://ipfs.io").unwrap();
    let bundle = RuLakeBundle::new("ipfs://x", 8, 42, 20, Generation::Opaque("x".into()));
    let err = backend.publish_bundle("c", &bundle).unwrap_err();
    let s = err.to_string();
    assert!(
        s.contains("Mode::Gateway is read-only"),
        "expected gateway-read-only error, got: {s}"
    );
}

#[test]
fn list_collections_reflects_register() {
    let backend = IpfsBackend::kubo_local("ipfs").unwrap();
    backend
        .register(IpfsCollection {
            name: "a".into(),
            cid: None,
            dim: None,
        })
        .unwrap();
    backend
        .register(IpfsCollection {
            name: "b".into(),
            cid: Some(FAKE_CID.into()),
            dim: Some(8),
        })
        .unwrap();
    let mut collections = backend.list_collections().unwrap();
    collections.sort();
    assert_eq!(collections, vec!["a".to_string(), "b".to_string()]);
}

// ─── Live test (gated) ──────────────────────────────────────────────

#[test]
#[ignore]
fn ipfs_live_publish_and_fetch() {
    if std::env::var("RULAKE_IPFS_LIVE_TEST").is_err() {
        eprintln!("skip: RULAKE_IPFS_LIVE_TEST not set");
        return;
    }
    let api =
        std::env::var("RULAKE_IPFS_KUBO_API").unwrap_or_else(|_| "http://127.0.0.1:5001".into());

    let backend = IpfsBackend::new(
        "ipfs-live",
        Mode::Kubo {
            api: KuboApiUrl(api),
            api_token: std::env::var("RULAKE_IPFS_TOKEN").ok(),
        },
    )
    .unwrap();
    backend
        .register(IpfsCollection {
            name: "live".into(),
            cid: None,
            dim: None,
        })
        .unwrap();

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros();
    let bundle = RuLakeBundle::new(
        format!("ipfs://placeholder-live-{nonce}"),
        128,
        42,
        20,
        Generation::Opaque(format!("nonce-{nonce}")),
    );
    let cid = backend.publish_bundle("live", &bundle).unwrap();
    eprintln!("live: published CID = {cid}");

    let fetched = backend.fetch_bundle("live").unwrap();
    assert_eq!(fetched.rvf_witness, bundle.rvf_witness);
    assert!(fetched.verify_witness());
}
