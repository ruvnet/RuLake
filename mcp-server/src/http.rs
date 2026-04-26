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
use crate::ratelimit::{LayeredRateLimiter, RateLimitDecision};
use crate::replay::ReplayGuard;
use crate::server::RuLakeMcpServer;

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
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let svc = service_fn(move |req: Request<Incoming>| {
                let mcp_service = mcp_service.clone();
                let auth = auth.clone();
                let replay = Arc::clone(&replay);
                let rate_limit = Arc::clone(&rate_limit);
                async move { handle(req, mcp_service, auth, peer, replay, rate_limit).await }
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
    }
}

async fn handle(
    req: Request<Incoming>,
    mcp_service: StreamableHttpService<RuLakeMcpServer, LocalSessionManager>,
    auth: AuthMode,
    peer: SocketAddr,
    replay: Arc<ReplayGuard>,
    rate_limit: Arc<LayeredRateLimiter>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    // 1. Auth gate (where applicable). Resolves to a principal string
    // used by the rate-limit and audit layers.
    let principal = match &auth {
        AuthMode::None => format!("anon:{peer}"),
        AuthMode::Bearer(b) => match b.verify(req.headers()) {
            Ok(p) => p,
            Err(status) => {
                tracing::warn!(?peer, %status, "bearer auth failed");
                return Ok(error_response(status, "auth"));
            }
        },
        AuthMode::Jwt(j) => match j.verify(req.headers()) {
            Ok(jp) => {
                // The JWT carried scopes → capabilities. v0.4 emits a
                // log line; v0.5 will thread these into a per-request
                // CapabilitySet so per-call cap checks honor the token,
                // not the server's startup --capabilities flag.
                tracing::debug!(
                    principal = %jp.principal,
                    scopes = ?jp.scopes,
                    capabilities = ?jp.capabilities.labels(),
                    "jwt verified",
                );
                jp.principal
            }
            Err(status) => {
                tracing::warn!(?peer, %status, "jwt auth failed");
                return Ok(error_response(status, "auth"));
            }
        },
    };

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
            return Ok(error_response(
                StatusCode::TOO_MANY_REQUESTS,
                layer,
            ));
        }
    }

    // 4. Hand off to rmcp's Streamable HTTP service. The rmcp layer
    // handles `tools/call`; capability checks fire at the tool handler
    // (server.rs `require_cap`) so an unauthorized call returns the
    // typed RULAKE_CAPABILITY_REFUSED error in the JSON-RPC response.
    let response = mcp_service.handle(req).await;
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
