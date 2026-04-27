//! Streamable HTTP transport (ADR-004 §3 + §5).
//!
//! Wraps `rmcp::transport::streamable_http_server::tower::StreamableHttpService`
//! in a tower middleware stack that:
//! - validates the bearer token (returns 401 if absent/wrong) when
//!   the operator chose `--auth bearer`
//! - emits an audit line per request
//!
//! DNS-rebinding protection is handled by rmcp itself via
//! `StreamableHttpServerConfig::allowed_hosts` (defaults to loopback
//! only — see ADR-004 §5 threat model).

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};

use crate::auth::{BearerAuth, JwtAuth};
use crate::mtls::{MtlsConfig, build_acceptor as build_mtls_acceptor};
use crate::policy::{CapabilitySet, REQUEST_CAPS};
use crate::ratelimit::{LayeredRateLimiter, RateLimitDecision};
use crate::replay::ReplayGuard;
use crate::server::RuLakeMcpServer;
use crate::sessions::{SessionBindings, SessionDecision};

use tokio_rustls::TlsAcceptor;

/// Auth mode selected at startup.
#[derive(Clone)]
pub enum AuthMode {
    /// `--auth none` (loopback-only by default; `--insecure-allow-no-auth`
    /// required for any non-loopback bind).
    None,
    /// `--auth bearer <token-file>`. Dev-only path per ADR-004 §5.
    Bearer(BearerAuth),
    /// `--auth jwt`. SOTA path per ADR-004 §5: validates JWS signature,
    /// iss, aud (RFC 8707 Resource Indicators), exp, and maps the
    /// `scope` / `scp` claim to the request's CapabilitySet via
    /// `mcp:rulake:read|publish|admin`.
    Jwt(JwtAuth),
    /// `--auth mtls` — TLS termination at rulake-mcp itself. The
    /// server presents `server_cert` and requires + validates the
    /// client cert against `client_ca`. Principal becomes
    /// `mtls:<cert-fingerprint-prefix>`. Pairs with `--auth jwt`
    /// transparently: the client cert SHA-256 still flows into the
    /// session-binding tuple, but the JWT principal wins for the
    /// audit identity in that combo.
    Mtls(Arc<MtlsConfig>),
}

/// Whether the operator opted into binding bearer-mode on a public
/// interface. Embarrassing-flag default per ADR-004.
#[derive(Clone, Copy)]
pub struct AllowBearerOnPublic(pub bool);

/// Whether the operator opted into binding `--auth none` on a public
/// interface. Embarrassing-flag default per ADR-004.
#[derive(Clone, Copy)]
pub struct InsecureAllowNoAuth(pub bool);

pub async fn serve(
    server: RuLakeMcpServer,
    bind: SocketAddr,
    auth: AuthMode,
    allow_bearer_on_public: AllowBearerOnPublic,
    insecure_allow_no_auth: InsecureAllowNoAuth,
) -> anyhow::Result<()> {
    serve_with_guards(
        server,
        bind,
        auth,
        allow_bearer_on_public,
        insecure_allow_no_auth,
        Arc::new(ReplayGuard::new()),
        Arc::new(LayeredRateLimiter::default()),
        Arc::new(SessionBindings::new()),
    )
    .await
}

pub async fn serve_with_guards(
    server: RuLakeMcpServer,
    bind: SocketAddr,
    auth: AuthMode,
    allow_bearer_on_public: AllowBearerOnPublic,
    insecure_allow_no_auth: InsecureAllowNoAuth,
    replay: Arc<ReplayGuard>,
    rate_limit: Arc<LayeredRateLimiter>,
    sessions: Arc<SessionBindings>,
) -> anyhow::Result<()> {
    // Refuse to bind a non-loopback interface unless the operator
    // explicitly opted in for the chosen auth mode.
    let is_loopback = bind.ip().is_loopback();
    match (&auth, is_loopback) {
        (AuthMode::None, false) if !insecure_allow_no_auth.0 => {
            anyhow::bail!(
                "refusing to bind {bind} with --auth none — pass \
                 --insecure-allow-no-auth or use --bind 127.0.0.1:* (ADR-004 §5)"
            );
        }
        (AuthMode::Bearer(_), false) if !allow_bearer_on_public.0 => {
            anyhow::bail!(
                "refusing to bind {bind} with --auth bearer — bearer is dev-only \
                 (static tokens leak once → permanent access). \
                 Pass --allow-bearer-on-public to override or use --auth jwt (ADR-004 §5)"
            );
        }
        // JWT and any other future auth mode are safe on public bind.
        _ => {}
    }
    tracing::info!(rate_limit = %rate_limit.config_summary(), "rate-limit policy");
    tracing::info!(replay_window_used = replay.window_used(), "replay-guard initialized");

    // Build the rmcp Streamable HTTP service. We hand it a service
    // factory so each session gets its own RuLakeMcpServer clone (the
    // underlying state is Arc'd so cloning is cheap).
    let server_for_factory = server.clone();
    let session_manager = Arc::new(LocalSessionManager::default());
    let mut config = StreamableHttpServerConfig::default();
    // Loopback by default (DNS-rebinding guard); operators who bind
    // a public interface AND opt into bearer-on-public can add hosts.
    if !is_loopback {
        // Permit the bound IP literal as a Host header.
        config.allowed_hosts.push(bind.to_string());
        config.allowed_hosts.push(bind.ip().to_string());
        // Operators behind reverse proxies / managed runtimes (Cloud
        // Run, Fly.io, Render, etc.) terminate TLS at the edge and
        // forward the request with a Host header that's the user-
        // facing domain, not the bind address. They can pass that
        // domain in via RULAKE_ALLOWED_HOSTS (comma-separated). Empty
        // entries are ignored. v0.11 will lift this to a CLI flag.
        if let Ok(extras) = std::env::var("RULAKE_ALLOWED_HOSTS") {
            for h in extras.split(',') {
                let h = h.trim();
                if !h.is_empty() {
                    config.allowed_hosts.push(h.to_string());
                }
            }
        }
    }

    let mcp_service: StreamableHttpService<RuLakeMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(server_for_factory.clone()),
            session_manager,
            config,
        );

    // Wrap in a tower middleware that does the bearer-auth gate, then
    // delegates to the rmcp service.
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(?bind, mode = ?auth_label(&auth), "rulake-mcp HTTP listening");

    // mTLS: build the TlsAcceptor once at startup (cert load is fail-fast).
    let tls_acceptor: Option<TlsAcceptor> = match &auth {
        AuthMode::Mtls(cfg) => Some(build_mtls_acceptor(cfg)?),
        _ => None,
    };
    if tls_acceptor.is_some() {
        tracing::info!("mTLS enabled — TLS termination at rulake-mcp");
    }

    if matches!(auth, AuthMode::Bearer(_)) && !is_loopback {
        // ADR-004 §5: noisy reminder every 60s while bearer-on-public.
        let warn_loop = tokio::spawn(async {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            tick.tick().await;
            loop {
                tick.tick().await;
                tracing::warn!(
                    "bearer auth on public interface — DEV ONLY. Static tokens \
                     leak once → permanent access. Migrate to --auth oauth."
                );
            }
        });
        // Detach; the loop ends when the process does.
        std::mem::forget(warn_loop);
    }

    loop {
        let (stream, peer) = listener.accept().await?;
        let mcp_service = mcp_service.clone();
        let auth = auth.clone();
        let replay = Arc::clone(&replay);
        let rate_limit = Arc::clone(&rate_limit);
        let sessions = Arc::clone(&sessions);
        let tls_acceptor = tls_acceptor.clone();
        tokio::spawn(async move {
            // mTLS path: complete the TLS handshake + extract the
            // client cert SHA-256. The fingerprint flows into the
            // per-request handler via a connection-level Arc that
            // each request reads from.
            let mtls_fingerprint: Option<String> = match tls_acceptor {
                Some(acceptor) => {
                    match acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            // Pull client cert chain out of the
                            // server-side connection. With WebPkiClientVerifier::builder
                            // (no allow_unauthenticated), rustls
                            // guarantees ≥1 cert.
                            let (_io, conn) = tls_stream.get_ref();
                            let fp = conn
                                .peer_certificates()
                                .and_then(|c| c.first())
                                .map(|c| crate::mtls::cert_sha256_hex(c.as_ref()));
                            if fp.is_none() {
                                tracing::warn!(?peer, "mTLS: no client cert");
                                return;
                            }
                            // Continue with the TLS-wrapped stream.
                            let io = TokioIo::new(tls_stream);
                            let mtls_fp = fp.clone();
                            let svc = service_fn(move |req: Request<Incoming>| {
                                let mcp_service = mcp_service.clone();
                                let auth = auth.clone();
                                let replay = Arc::clone(&replay);
                                let rate_limit = Arc::clone(&rate_limit);
                                let sessions = Arc::clone(&sessions);
                                let mtls_fp = mtls_fp.clone();
                                async move {
                                    handle(
                                        req, mcp_service, auth, peer, replay, rate_limit,
                                        sessions, mtls_fp,
                                    )
                                    .await
                                }
                            });
                            if let Err(e) = auto::Builder::new(TokioExecutor::new())
                                .serve_connection(io, svc)
                                .await
                            {
                                tracing::debug!(?peer, error = %e, "tls connection ended");
                            }
                            return;
                        }
                        Err(e) => {
                            tracing::warn!(?peer, error = %e, "mTLS handshake failed");
                            return;
                        }
                    }
                }
                None => None,
            };

            // Plain TCP path (--auth none / bearer / jwt without mTLS).
            let io = TokioIo::new(stream);
            let svc = service_fn(move |req: Request<Incoming>| {
                let mcp_service = mcp_service.clone();
                let auth = auth.clone();
                let replay = Arc::clone(&replay);
                let rate_limit = Arc::clone(&rate_limit);
                let sessions = Arc::clone(&sessions);
                let mtls_fp = mtls_fingerprint.clone();
                async move {
                    handle(
                        req, mcp_service, auth, peer, replay, rate_limit, sessions, mtls_fp,
                    )
                    .await
                }
            });
            if let Err(e) = auto::Builder::new(TokioExecutor::new())
                .serve_connection(io, svc)
                .await
            {
                tracing::debug!(?peer, error = %e, "connection ended");
            }
        });
    }
}

fn auth_label(a: &AuthMode) -> &'static str {
    match a {
        AuthMode::None => "none",
        AuthMode::Bearer(_) => "bearer",
        AuthMode::Jwt(_) => "jwt",
        AuthMode::Mtls(_) => "mtls",
    }
}

async fn handle(
    req: Request<Incoming>,
    mcp_service: StreamableHttpService<RuLakeMcpServer, LocalSessionManager>,
    auth: AuthMode,
    peer: SocketAddr,
    replay: Arc<ReplayGuard>,
    rate_limit: Arc<LayeredRateLimiter>,
    sessions: Arc<SessionBindings>,
    mtls_fingerprint: Option<String>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    // 0. CORS for browser callers (the ruLake Console / any web app).
    // ADR-006 §"server-side gaps". The mcp-server is loopback-friendly
    // by default, so we echo the requesting Origin (or `*` if absent),
    // expose the MCP-specific headers, and short-circuit OPTIONS.
    // Tightening to an allow-list is a v0.9 follow-up.
    let origin = req
        .headers()
        .get("origin")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("*")
        .to_string();
    if req.method() == http::Method::OPTIONS {
        let resp = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header("access-control-allow-origin", &origin)
            .header("access-control-allow-methods", "GET, POST, OPTIONS, DELETE")
            .header(
                "access-control-allow-headers",
                "authorization, content-type, accept, mcp-session-id, mcp-request-id, mcp-protocol-version",
            )
            .header("access-control-expose-headers", "mcp-session-id, mcp-protocol-version")
            .header("access-control-max-age", "600")
            .body(BodyExt::boxed(Full::new(Bytes::new())))
            .unwrap();
        return Ok(resp);
    }
    // 1. Auth gate (where applicable). Resolves to a principal string
    // + optional client_id that flow into the rate-limit, session-
    // binding, and audit layers.
    //
    // mTLS principal is the per-connection cert fingerprint (already
    // computed at TLS handshake time). When mTLS combines with another
    // mode (e.g. mTLS + JWT — transport-level identity + app-level
    // scopes), the JWT principal wins for capability mapping but the
    // mTLS fingerprint still flows into the session-binding tuple.
    let principal: String;
    let client_id: Option<String>;
    let mut request_caps: Option<CapabilitySet> = None;
    match &auth {
        AuthMode::None => {
            principal = format!("anon:{peer}");
            client_id = None;
        }
        AuthMode::Bearer(b) => match b.verify(req.headers()) {
            Ok(p) => {
                principal = p;
                client_id = None;
            }
            Err(status) => {
                tracing::warn!(?peer, %status, "bearer auth failed");
                return Ok(error_response(status, "auth"));
            }
        },
        AuthMode::Jwt(j) => match j.verify(req.headers()) {
            Ok(jp) => {
                tracing::debug!(
                    principal = %jp.principal,
                    scopes = ?jp.scopes,
                    capabilities = ?jp.capabilities.labels(),
                    "jwt verified",
                );
                principal = jp.principal;
                client_id = jp.client_id;
                // v0.8: JWT scopes drive a per-request CapabilitySet.
                // Server-wide caps stay the upper bound; per-request
                // caps are intersected at require_cap time.
                request_caps = Some(jp.capabilities);
            }
            Err(status) => {
                tracing::warn!(?peer, %status, "jwt auth failed");
                return Ok(error_response(status, "auth"));
            }
        },
        AuthMode::Mtls(_) => {
            // The TLS handshake already ran in the listener loop. The
            // client cert SHA-256 was extracted there and threaded in
            // as `mtls_fingerprint`. Build the mTLS principal from it.
            principal = mtls_fingerprint
                .as_deref()
                .map(|fp| format!("mtls:{}", &fp[..16.min(fp.len())]))
                .unwrap_or_else(|| format!("mtls:no-fp:{peer}"));
            client_id = None;
        }
    }

    // 1b. Session binding (ADR-004 §5). Streamable HTTP carries the
    // session id in `mcp-session-id` after the initialize handshake.
    // First sighting records (session_id) → (principal, client_id);
    // subsequent requests must present the same tuple.
    let session_id = req
        .headers()
        .get("mcp-session-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    match sessions.check_or_bind(
        &session_id,
        &principal,
        client_id.as_deref(),
        mtls_fingerprint.as_deref(),
    ) {
        SessionDecision::Allowed { first_sighting } => {
            if first_sighting && !session_id.is_empty() {
                tracing::info!(
                    session = %session_id,
                    principal = %principal,
                    "session binding recorded"
                );
            }
        }
        SessionDecision::Mismatch { expected_principal, presented_principal } => {
            tracing::warn!(
                ?peer,
                session = %session_id,
                expected = %expected_principal,
                presented = %presented_principal,
                "session binding mismatch — token reused with different principal"
            );
            return Ok(error_response(StatusCode::UNAUTHORIZED, "session"));
        }
    }

    // 2. Replay protection — `MCP-Request-Id` nonce dedup
    // (ADR-004 §5). Empty id (stdio's case) bypasses; here we only
    // see HTTP requests so a missing id is a real omission for any
    // non-initialize call.
    let request_id = req
        .headers()
        .get("mcp-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    if let Err(replay_err) = replay.check(&request_id) {
        tracing::warn!(?peer, principal = %principal, %replay_err, "replay rejected");
        return Ok(error_response(
            StatusCode::CONFLICT,
            "replay",
        ));
    }

    // 3. Per-process rate-limit gate. Per-(principal,backend,collection)
    // is enforced inside the planner where the routes are known; the
    // HTTP layer enforces per-process and per-principal-transport.
    match rate_limit.check("http", &principal, "_", "_") {
        RateLimitDecision::Allowed => {}
        RateLimitDecision::Denied { layer } => {
            tracing::warn!(?peer, principal = %principal, layer, "rate limit denied");
            // v0.8b: structured backpressure response. The HTTP layer
            // returns 429 + Retry-After + a JSON body that mirrors
            // the planner's DegradedAdvice. Framework-aware agents
            // read the body; legacy agents see the 429 + Retry-After
            // header and back off blindly. Both work.
            let retry_after_ms = match layer {
                "process" => 1000u32,
                "principal" => 500,
                "collection" => 500,
                _ => 500,
            };
            let advice = serde_json::json!({
                "error": {
                    "code": "RULAKE_DEGRADED",
                    "layer": layer,
                    "retry_after_ms": retry_after_ms,
                    "hints": match layer {
                        "process"    => vec!["wait"],
                        "principal"  => vec!["wait", "use_cached_consistency"],
                        "collection" => vec!["wait", "narrow_target"],
                        _ => vec!["wait"],
                    },
                }
            });
            return Ok(degraded_response(retry_after_ms, advice.to_string()));
        }
    }

    // 4. Hand off to rmcp's Streamable HTTP service. The rmcp layer
    // handles `tools/call`; capability checks fire at the tool handler
    // (server.rs `require_cap`) so an unauthorized call returns the
    // typed RULAKE_CAPABILITY_REFUSED error in the JSON-RPC response.
    //
    // v0.8: thread the per-request CapabilitySet (from JWT scopes)
    // into the task-local before forwarding. require_cap reads this
    // first; falls back to server-wide for non-JWT auth modes.
    let mut response = match request_caps {
        Some(caps) => REQUEST_CAPS.scope(caps, mcp_service.handle(req)).await,
        None => mcp_service.handle(req).await,
    };
    // CORS response header — pair to the OPTIONS preflight above. The
    // browser's strict-mode fetch needs the response to carry the
    // origin echo; same `*` semantics for non-CORS callers.
    let h = response.headers_mut();
    if let Ok(v) = http::HeaderValue::from_str(&origin) {
        h.insert("access-control-allow-origin", v);
    }
    h.insert(
        "access-control-expose-headers",
        http::HeaderValue::from_static("mcp-session-id, mcp-protocol-version"),
    );
    Ok(response)
}

fn error_response(status: StatusCode, msg: &str) -> Response<BoxBody<Bytes, Infallible>> {
    let body = Full::new(Bytes::from(msg.to_string())).map_err(|e| match e {}).boxed();
    Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "text/plain")
        .body(body)
        .unwrap()
}

/// 429 + Retry-After + a JSON body mirroring the planner's
/// `DegradedAdvice`. The standard HTTP signal (status code + header)
/// satisfies legacy agents that don't parse the body; the body
/// satisfies framework-aware agents per ADR-004 §6.
fn degraded_response(
    retry_after_ms: u32,
    json_body: String,
) -> Response<BoxBody<Bytes, Infallible>> {
    let body = Full::new(Bytes::from(json_body)).map_err(|e| match e {}).boxed();
    let retry_after_secs = (retry_after_ms.div_ceil(1000)).max(1);
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::RETRY_AFTER, retry_after_secs.to_string())
        .body(body)
        .unwrap()
}
