//! `RuLakeMcpServer` — the rmcp `ServerHandler` impl.
//!
//! v0.1 exposes:
//! - `rulake_query` (intent: "search") — public decision-layer tool
//! - `rulake_list_backends` — internal kernel probe (gated by
//!   `--capabilities internal` in v0.2; in v0.1 it's always on for
//!   demo/debug since the only transport is stdio + parent-trust)

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError,
    ServerHandler, ServiceExt,
    handler::server::{
        router::tool::ToolRouter,
        wrapper::{Json, Parameters},
    },
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};

use ruvector_rulake::{LocalBackend, RuLake, BackendAdapter, FsBackend};

use crate::config::{BackendConfig, McpConfig};
use crate::planner::{PlanError, Planner, QueryRequest, QueryResponse};
use crate::workers::WorkerPool;

#[derive(Clone)]
pub struct RuLakeMcpServer {
    planner: Arc<Planner>,
    tool_router: ToolRouter<Self>,
}

impl RuLakeMcpServer {
    pub fn new(config: McpConfig) -> anyhow::Result<Self> {
        let consistency = config.consistency.into_runtime();
        let consistency_label = format!("{consistency:?}");

        let lake = RuLake::new(config.rerank_factor, config.rotation_seed)
            .with_consistency(consistency);

        let mut backend_ids = Vec::with_capacity(config.backends.len());
        for backend in &config.backends {
            match backend {
                BackendConfig::Local { id } => {
                    let be = Arc::new(LocalBackend::new(id.clone()));
                    let dyn_be: Arc<dyn BackendAdapter> = be;
                    lake.register_backend(dyn_be)
                        .map_err(|e| anyhow::anyhow!("register {id}: {e}"))?;
                    backend_ids.push(id.clone());
                }
                BackendConfig::Fs { id, root, collections } => {
                    let fs = FsBackend::new(id.clone(), root)
                        .map_err(|e| anyhow::anyhow!("fs {id}: {e}"))?;
                    for c in collections {
                        fs.register(c.name.clone(), c.filename.clone())
                            .map_err(|e| anyhow::anyhow!("fs register {id}/{}: {e}", c.name))?;
                    }
                    let arc = Arc::new(fs);
                    let dyn_be: Arc<dyn BackendAdapter> = arc;
                    lake.register_backend(dyn_be)
                        .map_err(|e| anyhow::anyhow!("register {id}: {e}"))?;
                    backend_ids.push(id.clone());
                }
            }
        }

        let workers = WorkerPool::new(config.workers, config.max_inflight)?;

        Ok(Self {
            planner: Arc::new(Planner {
                lake: Arc::new(lake),
                workers,
                backend_ids,
                consistency_label,
            }),
            tool_router: Self::tool_router(),
        })
    }

    /// Smoke-test helper: hand a built `RuLake` directly so a test can
    /// pre-populate a `LocalBackend` before serving. Skips the
    /// config-driven registration path entirely.
    #[doc(hidden)]
    pub fn from_lake(
        lake: Arc<RuLake>,
        consistency_label: String,
        backend_ids: Vec<String>,
        max_inflight: usize,
    ) -> anyhow::Result<Self> {
        let workers = WorkerPool::new(0, max_inflight)?;
        Ok(Self {
            planner: Arc::new(Planner {
                lake,
                workers,
                backend_ids,
                consistency_label,
            }),
            tool_router: Self::tool_router(),
        })
    }

    /// Direct planner access — used by tests to call the planner
    /// without going through the MCP wire.
    #[doc(hidden)]
    pub fn planner(&self) -> &Planner {
        &self.planner
    }

    /// Drive the server over stdio. Blocks until the peer (Claude
    /// Desktop / Cursor / Cline / Inspector) closes stdin/stdout.
    pub async fn serve_stdio(self) -> anyhow::Result<()> {
        let (stdin, stdout) = rmcp::transport::stdio();
        let service = self.serve((stdin, stdout)).await?;
        service.waiting().await?;
        Ok(())
    }
}

// ─── Tool definitions ─────────────────────────────────────────────────

#[tool_router(router = tool_router)]
impl RuLakeMcpServer {
    #[tool(
        name = "rulake_query",
        description = "Public ruLake decision-layer tool. Submit an intent (search/verify/explain/refresh) \
                       with target (collection or routes), risk, freshness budget, and policy. \
                       Returns the answer plus a decision trace (chosen_action, reason_code, \
                       backends_used, refusals). v0.1 supports intent=search; other intents \
                       return RULAKE_INTERNAL until v0.2."
    )]
    pub async fn rulake_query(
        &self,
        Parameters(req): Parameters<QueryRequest>,
    ) -> Result<Json<QueryResponse>, McpError> {
        emit_audit_start("rulake_query");
        match self.planner.handle(req).await {
            Ok(resp) => {
                emit_audit_ok("rulake_query", &resp);
                Ok(Json(resp))
            }
            Err(PlanError::Refused(resp)) => {
                emit_audit_refused("rulake_query", &resp);
                Ok(Json(resp))
            }
            Err(PlanError::Degraded { inflight, cap }) => {
                emit_audit_degraded("rulake_query", inflight, cap);
                Err(McpError::internal_error(
                    format!("RULAKE_DEGRADED: inflight={inflight} cap={cap}"),
                    None,
                ))
            }
            Err(PlanError::Internal(s)) => {
                Err(McpError::internal_error(format!("RULAKE_INTERNAL: {s}"), None))
            }
        }
    }

    #[tool(
        name = "rulake_list_backends",
        description = "Internal: list registered backend ids. Always available in v0.1; v0.2 \
                       gates this behind --capabilities internal."
    )]
    pub async fn rulake_list_backends(&self) -> Result<Json<ListBackendsResponse>, McpError> {
        Ok(Json(ListBackendsResponse {
            backends: self.planner.backend_ids.clone(),
        }))
    }
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ListBackendsResponse {
    pub backends: Vec<String>,
}

// ─── ServerHandler — emitted from #[tool_handler] ────────────────────

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RuLakeMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = Implementation::new("ruvector-rulake-mcp", env!("CARGO_PKG_VERSION"));
        info.title = Some("ruLake MCP server".into());
        info.website_url = Some("https://github.com/ruvnet/RuLake".into());

        let mut init = ServerInfo::new(ServerCapabilities::builder().enable_tools().build());
        init.server_info = info;
        init.instructions = Some(
            "ruLake MCP server (ADR-004 v0.1). One public tool: rulake_query \
             (intent=search). Pass {target.collection|routes, search:{vector,k}, ...}. \
             The response carries a decision trace alongside the data."
                .into(),
        );
        init
    }
}

// ─── Audit helpers ────────────────────────────────────────────────────
//
// v0.1 emits to stderr via `tracing` JSON. v0.2 lands the per-line
// JSONL audit file with the full schema from ADR-004 §7. The fields
// emitted here match the §7 schema's outer shape so downstream
// log shippers don't need to be retold the schema.

fn emit_audit_start(tool: &str) {
    tracing::info!(tool, event = "start");
}

fn emit_audit_ok(tool: &str, resp: &QueryResponse) {
    tracing::info!(
        tool,
        event = "ok",
        result_size = resp.data.len(),
        reason_code = ?resp.decision.reason_code,
        backends_used = ?resp.decision.backends_used,
        budget_used_ms = resp.decision.budget_used_ms,
    );
}

fn emit_audit_refused(tool: &str, resp: &QueryResponse) {
    tracing::warn!(
        tool,
        event = "refused",
        reason_code = ?resp.decision.reason_code,
        reason = %resp.decision.reason,
    );
}

fn emit_audit_degraded(tool: &str, inflight: usize, cap: usize) {
    tracing::warn!(tool, event = "degraded", inflight, cap);
}
