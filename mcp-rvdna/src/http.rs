//! Streamable HTTP transport for `mcp-rvdna`.
//!
//! v0.0.1 keeps the surface small: stdio + Streamable HTTP, no auth
//! beyond the loopback default + an embarrassing-flag opt-in for
//! non-loopback `--auth none`. Bearer / JWT / mTLS lift wholesale from
//! `mcp-server::http` in v0.0.2 — the request/response shape here is
//! deliberately the same so that lift is mechanical.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, combinators::BoxBody};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};

use crate::server::RvdnaMcpServer;

/// Auth mode selected at startup. v0.0.1 ships only `None` — the rest
/// of the variants exist so the wire signature is stable when the
/// JWT/mTLS lift from mcp-server lands in v0.0.2.
#[derive(Clone, Default)]
pub enum AuthMode {
    #[default]
    None,
    Bearer(BearerAuth),
}

/// Stub bearer-auth carrier — token comparison itself lifts in v0.0.2.
/// Present so the wire signature stays put.
#[derive(Clone)]
pub struct BearerAuth {
    _expected: Arc<Vec<u8>>,
}

impl BearerAuth {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            _expected: Arc::new(token.into().into_bytes()),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct AllowBearerOnPublic(pub bool);

#[derive(Clone, Copy, Default)]
pub struct InsecureAllowNoAuth(pub bool);

/// Spin up the Streamable HTTP transport. Mirrors mcp-server's `serve`
/// API so a future federation script can swap one for the other by
/// crate name only.
pub async fn serve(
    server: RvdnaMcpServer,
    bind: SocketAddr,
    auth: AuthMode,
    _allow_bearer_on_public: AllowBearerOnPublic,
    insecure_allow_no_auth: InsecureAllowNoAuth,
) -> anyhow::Result<()> {
    let is_loopback = bind.ip().is_loopback();
    if matches!(auth, AuthMode::None) && !is_loopback && !insecure_allow_no_auth.0 {
        anyhow::bail!(
            "rvdna-mcp: --auth none on a non-loopback bind ({bind}) requires \
             --insecure-allow-no-auth (operator opt-in)"
        );
    }

    let server_arc = Arc::new(server);
    let session_manager = Arc::new(LocalSessionManager::default());
    let cfg = StreamableHttpServerConfig::default();
    let svc = StreamableHttpService::new(
        {
            let s = server_arc.clone();
            move || Ok::<_, std::io::Error>((*s).clone())
        },
        session_manager,
        cfg,
    );
    let svc = Arc::new(svc);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "rvdna-mcp http listening");

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "accept failed");
                continue;
            }
        };
        let svc = svc.clone();
        let _peer = peer;
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let svc_for_conn = svc.clone();
            let h_svc = service_fn(move |req: Request<Incoming>| {
                let svc_inner = svc_for_conn.clone();
                async move {
                    let resp = StreamableHttpService::handle(svc_inner.as_ref(), req).await;
                    let resp = resp.map(|b| {
                        BodyExt::map_err(b, |e| match e {})
                            .boxed()
                            .map_err(|e: Infallible| match e {})
                            .boxed()
                    });
                    Ok::<Response<BoxBody<Bytes, Infallible>>, Infallible>(resp)
                }
            });
            if let Err(e) = auto::Builder::new(TokioExecutor::new())
                .serve_connection(io, h_svc)
                .await
            {
                tracing::debug!(error = %e, "rvdna-mcp http connection ended");
            }
        });
    }
    // unreachable — loop forever or until tokio::spawn(serve).abort()
    #[allow(unreachable_code)]
    Ok::<(), anyhow::Error>(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refuses_no_auth_on_public_without_override() {
        let bind: SocketAddr = "0.0.0.0:0".parse().unwrap();
        let server = RvdnaMcpServer::new();
        let result = serve(
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
            msg.contains("--insecure-allow-no-auth"),
            "expected refusal mentioning --insecure-allow-no-auth, got: {msg}"
        );
    }
}
