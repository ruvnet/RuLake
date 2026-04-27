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
    model::{
        Implementation, ListResourcesResult, ListToolsResult, PaginatedRequestParams, RawResource,
        ReadResourceRequestParams, ReadResourceResult, ResourceContents, ServerCapabilities,
        ServerInfo,
    },
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};

use rulake::{LocalBackend, RuLake, BackendAdapter, FsBackend};

use crate::allow::AllowList;
use crate::audit::{AuditEntry, AuditSink, PolicyDecision, now_ts};
use crate::config::{BackendConfig, McpConfig};
use crate::planner::{PlanError, Planner, QueryRequest, QueryResponse};
use crate::policy::{Capability, CapabilitySet};
use crate::workers::WorkerPool;

#[derive(Clone)]
pub struct RuLakeMcpServer {
    planner: Arc<Planner>,
    capabilities: Arc<CapabilitySet>,
    audit: AuditSink,
    tool_router: ToolRouter<Self>,
}

impl RuLakeMcpServer {
    pub fn new_with_caps(config: McpConfig, capabilities: CapabilitySet) -> anyhow::Result<Self> {
        let mut me = Self::new(config)?;
        me.capabilities = Arc::new(capabilities);
        Ok(me)
    }

    /// Replace the audit sink. Call before serve_*.
    pub fn with_audit(mut self, sink: AuditSink) -> Self {
        self.audit = sink;
        self
    }

    pub fn audit(&self) -> &AuditSink {
        &self.audit
    }

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
        let allow = AllowList::from_blocks(&config.allow)?;
        if !allow.is_empty() {
            tracing::info!(blocks = config.allow.len(), "RBAC allow-list active");
        }

        Ok(Self {
            planner: Arc::new(Planner {
                lake: Arc::new(lake),
                workers,
                backend_ids,
                consistency_label,
                allow,
            }),
            capabilities: Arc::new(CapabilitySet::default()),
            audit: AuditSink::stderr(),
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
                allow: AllowList::empty(),
            }),
            capabilities: Arc::new(CapabilitySet::default()),
            audit: AuditSink::stderr(),
            tool_router: Self::tool_router(),
        })
    }

    /// Test helper: same as `from_lake` but with a custom capability set.
    #[doc(hidden)]
    pub fn from_lake_with_caps(
        lake: Arc<RuLake>,
        consistency_label: String,
        backend_ids: Vec<String>,
        max_inflight: usize,
        capabilities: CapabilitySet,
    ) -> anyhow::Result<Self> {
        let mut me = Self::from_lake(lake, consistency_label, backend_ids, max_inflight)?;
        me.capabilities = Arc::new(capabilities);
        Ok(me)
    }

    /// Direct planner access — used by tests to call the planner
    /// without going through the MCP wire.
    #[doc(hidden)]
    pub fn planner(&self) -> &Planner {
        &self.planner
    }

    /// Test-only mirror of the `list_tools` capability filter — lets a
    /// smoke test verify the filter without standing up a full MCP
    /// session. Same logic as the ServerHandler's `list_tools` method.
    #[doc(hidden)]
    pub fn list_tools_filtered(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|t| self.capabilities.has(required_cap_for_tool(&t.name)))
            .map(|t| t.name.to_string())
            .collect();
        names.sort();
        names
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
        let start = std::time::Instant::now();
        let intent = format!("{:?}", req.intent).to_ascii_lowercase();
        let result = self.planner.handle(req).await;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let cap_grant = self.capabilities.labels().iter().map(|s| s.to_string()).collect();
        match result {
            Ok(resp) => {
                let outcome = if resp.data.is_empty()
                    && format!("{:?}", resp.decision.reason_code).contains("Refused")
                {
                    "refused"
                } else {
                    "ok"
                };
                self.audit.emit(AuditEntry {
                    ts: now_ts(),
                    transport: "stdio".into(),
                    principal: "stdio:local".into(),
                    session: None,
                    request_id: None,
                    tool: "rulake_query".into(),
                    intent: Some(intent),
                    outcome: outcome.into(),
                    result_size: Some(resp.data.len() as u32),
                    trust_level: Some(format!("{:?}", resp.trust_level).to_ascii_lowercase()),
                    duration_ms: elapsed_ms,
                    witness_in: None,
                    witness_out: resp.provenance.witness.clone(),
                    code: None,
                    policy_decision: Some(PolicyDecision {
                        capability_required: "read".into(),
                        capability_granted: cap_grant,
                    }),
                    decision: serde_json::to_value(&resp.decision).ok(),
                });
                Ok(Json(resp))
            }
            Err(PlanError::Refused(resp)) => {
                self.audit.emit(AuditEntry {
                    ts: now_ts(),
                    transport: "stdio".into(),
                    principal: "stdio:local".into(),
                    session: None,
                    request_id: None,
                    tool: "rulake_query".into(),
                    intent: Some(intent),
                    outcome: "refused".into(),
                    result_size: Some(0),
                    trust_level: Some("unverified".into()),
                    duration_ms: elapsed_ms,
                    witness_in: None,
                    witness_out: None,
                    code: Some(format!("{:?}", resp.decision.reason_code)),
                    policy_decision: Some(PolicyDecision {
                        capability_required: "read".into(),
                        capability_granted: cap_grant,
                    }),
                    decision: serde_json::to_value(&resp.decision).ok(),
                });
                Ok(Json(resp))
            }
            Err(PlanError::Degraded { inflight, cap }) => {
                self.audit.emit(AuditEntry {
                    ts: now_ts(),
                    transport: "stdio".into(),
                    principal: "stdio:local".into(),
                    session: None,
                    request_id: None,
                    tool: "rulake_query".into(),
                    intent: Some(intent),
                    outcome: "degraded".into(),
                    result_size: None,
                    trust_level: None,
                    duration_ms: elapsed_ms,
                    witness_in: None,
                    witness_out: None,
                    code: Some("RULAKE_DEGRADED".into()),
                    policy_decision: Some(PolicyDecision {
                        capability_required: "read".into(),
                        capability_granted: cap_grant,
                    }),
                    decision: None,
                });
                Err(McpError::internal_error(
                    format!("RULAKE_DEGRADED: inflight={inflight} cap={cap}"),
                    None,
                ))
            }
            Err(PlanError::Internal(s)) => {
                self.audit.emit(AuditEntry {
                    ts: now_ts(),
                    transport: "stdio".into(),
                    principal: "stdio:local".into(),
                    session: None,
                    request_id: None,
                    tool: "rulake_query".into(),
                    intent: Some(intent),
                    outcome: "error".into(),
                    result_size: None,
                    trust_level: None,
                    duration_ms: elapsed_ms,
                    witness_in: None,
                    witness_out: None,
                    code: Some("RULAKE_INTERNAL".into()),
                    policy_decision: Some(PolicyDecision {
                        capability_required: "read".into(),
                        capability_granted: cap_grant,
                    }),
                    decision: None,
                });
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

    #[tool(
        name = "rulake_list_collections",
        description = "Read: list the collections (id, dim, row_count) a registered backend exposes. \
                       Closes ADR-006 server-gap #1 for the ruLake Console's Browse screen live mode. \
                       Read-tier — gated by --capabilities read."
    )]
    pub async fn rulake_list_collections(
        &self,
        Parameters(args): Parameters<ListCollectionsArgs>,
    ) -> Result<Json<ListCollectionsResponse>, McpError> {
        require_cap(&self.capabilities, Capability::Read)?;
        let lake = Arc::clone(&self.planner.lake);
        let backend = args.backend.clone();
        let collections = self
            .planner
            .workers
            .submit(move || lake.list_collections(&backend))
            .await
            .map_err(|e| McpError::internal_error(format!("RULAKE_DEGRADED: {e}"), None))?
            .map_err(|e| match e {
                rulake::error::RuLakeError::UnknownBackend(b) => {
                    McpError::invalid_params(format!("UNKNOWN_BACKEND: {b}"), None)
                }
                other => McpError::internal_error(format!("RULAKE_INTERNAL: {other}"), None),
            })?;
        Ok(Json(ListCollectionsResponse {
            backend: args.backend,
            collections: collections
                .into_iter()
                .map(|id| CollectionEntry { id })
                .collect(),
        }))
    }

    // ─── publish-tier tools ──────────────────────────────────────────

    #[tool(
        name = "rulake_publish_bundle",
        description = "Publish: atomically write the cache's witness bundle for (backend, collection) to dir/table.rulake.json. Requires --capabilities publish."
    )]
    pub async fn rulake_publish_bundle(
        &self,
        Parameters(args): Parameters<PublishBundleArgs>,
    ) -> Result<Json<PathResponse>, McpError> {
        let start = std::time::Instant::now();
        let backend = args.backend.clone();
        let collection = args.collection.clone();
        let r: Result<Json<PathResponse>, McpError> = async {
            require_cap(&self.capabilities, Capability::Publish)?;
            let lake = Arc::clone(&self.planner.lake);
            let key = (args.backend, args.collection);
            let dir = std::path::PathBuf::from(args.dir);
            let res = self
                .planner
                .workers
                .submit(move || lake.publish_bundle(&key, &dir).map(|p| p.to_string_lossy().to_string()))
                .await
                .map_err(|e| McpError::internal_error(format!("RULAKE_DEGRADED: {e}"), None))?
                .map_err(|e| McpError::internal_error(format!("RULAKE_INTERNAL: {e}"), None))?;
            Ok(Json(PathResponse { path: res }))
        }
        .await;
        self.audit_mutation(
            "rulake_publish_bundle",
            Capability::Publish,
            &backend,
            &collection,
            &r,
            start.elapsed().as_secs_f64() * 1000.0,
        );
        r
    }

    #[tool(
        name = "rulake_refresh_from_bundle_dir",
        description = "Publish: refresh cache from dir/table.rulake.json. Returns 'up_to_date' | 'invalidated' | 'bundle_missing'. Requires --capabilities publish."
    )]
    pub async fn rulake_refresh_from_bundle_dir(
        &self,
        Parameters(args): Parameters<RefreshArgs>,
    ) -> Result<Json<RefreshResponse>, McpError> {
        let start = std::time::Instant::now();
        let backend = args.backend.clone();
        let collection = args.collection.clone();
        let r: Result<Json<RefreshResponse>, McpError> = async {
            require_cap(&self.capabilities, Capability::Publish)?;
            let lake = Arc::clone(&self.planner.lake);
            let key = (args.backend, args.collection);
            let dir = std::path::PathBuf::from(args.dir);
            let res = self
                .planner
                .workers
                .submit(move || lake.refresh_from_bundle_dir(&key, &dir))
                .await
                .map_err(|e| McpError::internal_error(format!("RULAKE_DEGRADED: {e}"), None))?
                .map_err(|e| McpError::internal_error(format!("RULAKE_INTERNAL: {e}"), None))?;
            let status = match res {
                rulake::RefreshResult::UpToDate => "up_to_date",
                rulake::RefreshResult::Invalidated => "invalidated",
                rulake::RefreshResult::BundleMissing => "bundle_missing",
            };
            Ok(Json(RefreshResponse { status: status.into() }))
        }
        .await;
        self.audit_mutation(
            "rulake_refresh_from_bundle_dir",
            Capability::Publish,
            &backend,
            &collection,
            &r,
            start.elapsed().as_secs_f64() * 1000.0,
        );
        r
    }

    // ─── admin-tier tools ────────────────────────────────────────────

    #[tool(
        name = "rulake_save_cache_to_dir",
        description = "Admin: persist the primed cache for (backend, collection) to dir as a warm-restart snapshot. Requires --capabilities admin."
    )]
    pub async fn rulake_save_cache_to_dir(
        &self,
        Parameters(args): Parameters<SaveCacheArgs>,
    ) -> Result<Json<PathResponse>, McpError> {
        let start = std::time::Instant::now();
        let backend = args.backend.clone();
        let collection = args.collection.clone();
        let r: Result<Json<PathResponse>, McpError> = async {
            require_cap(&self.capabilities, Capability::Admin)?;
            let lake = Arc::clone(&self.planner.lake);
            let key = (args.backend, args.collection);
            let dir = std::path::PathBuf::from(args.dir);
            let res = self
                .planner
                .workers
                .submit(move || lake.save_cache_to_dir(&key, &dir).map(|p| p.to_string_lossy().to_string()))
                .await
                .map_err(|e| McpError::internal_error(format!("RULAKE_DEGRADED: {e}"), None))?
                .map_err(|e| McpError::internal_error(format!("RULAKE_INTERNAL: {e}"), None))?;
            Ok(Json(PathResponse { path: res }))
        }
        .await;
        self.audit_mutation(
            "rulake_save_cache_to_dir",
            Capability::Admin,
            &backend,
            &collection,
            &r,
            start.elapsed().as_secs_f64() * 1000.0,
        );
        r
    }

    #[tool(
        name = "rulake_warm_from_dir",
        description = "Admin: warm the cache for (backend, collection) from a snapshot dir. Returns the number of vectors loaded. Requires --capabilities admin."
    )]
    pub async fn rulake_warm_from_dir(
        &self,
        Parameters(args): Parameters<WarmArgs>,
    ) -> Result<Json<WarmResponse>, McpError> {
        let start = std::time::Instant::now();
        let backend = args.backend.clone();
        let collection = args.collection.clone();
        let r: Result<Json<WarmResponse>, McpError> = async {
            require_cap(&self.capabilities, Capability::Admin)?;
            let lake = Arc::clone(&self.planner.lake);
            let key = (args.backend, args.collection);
            let dir = std::path::PathBuf::from(args.dir);
            let res = self
                .planner
                .workers
                .submit(move || lake.warm_from_dir(&key, &dir))
                .await
                .map_err(|e| McpError::internal_error(format!("RULAKE_DEGRADED: {e}"), None))?
                .map_err(|e| McpError::internal_error(format!("RULAKE_INTERNAL: {e}"), None))?;
            Ok(Json(WarmResponse { vectors: res as u32 }))
        }
        .await;
        self.audit_mutation(
            "rulake_warm_from_dir",
            Capability::Admin,
            &backend,
            &collection,
            &r,
            start.elapsed().as_secs_f64() * 1000.0,
        );
        r
    }

    #[tool(
        name = "rulake_invalidate_cache",
        description = "Admin: drop the cache entry for (backend, collection). Next search re-primes. Requires --capabilities admin."
    )]
    pub async fn rulake_invalidate_cache(
        &self,
        Parameters(args): Parameters<InvalidateArgs>,
    ) -> Result<Json<EmptyResponse>, McpError> {
        let start = std::time::Instant::now();
        let r: Result<Json<EmptyResponse>, McpError> = (|| {
            require_cap(&self.capabilities, Capability::Admin)?;
            let key = (args.backend.clone(), args.collection.clone());
            self.planner.lake.invalidate_cache(&key);
            Ok(Json(EmptyResponse {}))
        })();
        self.audit_mutation(
            "rulake_invalidate_cache",
            Capability::Admin,
            &args.backend,
            &args.collection,
            &r,
            start.elapsed().as_secs_f64() * 1000.0,
        );
        r
    }

    /// R-MCP-1: emit a fully-shaped audit entry for a mutation tool
    /// so the audit log is symmetric across read (`rulake_query`) and
    /// write (publish/admin) paths. Outcome/code are derived from the
    /// `Result` so callers don't have to reshape state at the call
    /// site — the helper inspects what actually happened.
    fn audit_mutation<T>(
        &self,
        tool: &str,
        required: Capability,
        backend: &str,
        collection: &str,
        result: &Result<T, McpError>,
        elapsed_ms: f64,
    ) {
        let (outcome, code) = match result {
            Ok(_) => ("ok", None),
            Err(e) => {
                let msg = e.message.to_string();
                if msg.starts_with("RULAKE_DEGRADED") {
                    ("degraded", Some("RULAKE_DEGRADED".to_string()))
                } else if msg.starts_with("RULAKE_INTERNAL") {
                    ("error", Some("RULAKE_INTERNAL".to_string()))
                } else {
                    ("refused", Some("CAP_DENIED".to_string()))
                }
            }
        };
        self.audit.emit(AuditEntry {
            ts: now_ts(),
            transport: "stdio".into(),
            principal: "stdio:local".into(),
            session: None,
            request_id: None,
            tool: tool.into(),
            intent: Some(format!("mutate:{backend}/{collection}")),
            outcome: outcome.into(),
            result_size: None,
            trust_level: None,
            duration_ms: elapsed_ms,
            witness_in: None,
            witness_out: None,
            code,
            policy_decision: Some(PolicyDecision {
                capability_required: format!("{:?}", required).to_ascii_lowercase(),
                capability_granted: self
                    .capabilities
                    .labels()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            }),
            decision: None,
        });
    }
}

fn require_cap(caps: &CapabilitySet, required: Capability) -> Result<(), McpError> {
    // v0.8: combine server-wide caps with per-request JWT scopes.
    // The intersection gates the actual tool call. If the per-request
    // task-local isn't set (stdio path; legacy auth) this falls
    // through to server-wide alone.
    let effective = crate::policy::effective_caps(caps);
    effective
        .require(required)
        .map_err(|refused| McpError::invalid_request(refused.to_string(), None))
}

/// The single source of truth for tool→capability mapping. ADR-004 §4b.
/// Used by both the per-call gate (in each tool fn) AND the
/// `tools/list` visibility filter.
fn required_cap_for_tool(name: &str) -> Capability {
    match name {
        // Public surface — read tier.
        "rulake_query" | "rulake_list_backends" | "rulake_list_collections" => Capability::Read,
        // Mutation tools — publish/admin tiers.
        "rulake_publish_bundle" | "rulake_refresh_from_bundle_dir" => Capability::Publish,
        "rulake_save_cache_to_dir"
        | "rulake_warm_from_dir"
        | "rulake_invalidate_cache" => Capability::Admin,
        // Anything else (future internal-kernel tools etc.) defaults
        // to internal — invisible by default until --capabilities
        // internal is granted. Safer than defaulting to Read.
        _ => Capability::Internal,
    }
}

// ─── Mutation-tool arg/response shapes ────────────────────────────────

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct PublishBundleArgs {
    pub backend: String,
    pub collection: String,
    /// Directory to write `table.rulake.json` into. Atomic temp+rename.
    pub dir: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct RefreshArgs {
    pub backend: String,
    pub collection: String,
    pub dir: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct SaveCacheArgs {
    pub backend: String,
    pub collection: String,
    pub dir: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct WarmArgs {
    pub backend: String,
    pub collection: String,
    pub dir: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct InvalidateArgs {
    pub backend: String,
    pub collection: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct PathResponse {
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct RefreshResponse {
    pub status: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct WarmResponse {
    pub vectors: u32,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct EmptyResponse {}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ListBackendsResponse {
    pub backends: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ListCollectionsArgs {
    /// Backend id to enumerate. Use rulake_list_backends if you don't
    /// know the registered ids.
    pub backend: String,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct CollectionEntry {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ListCollectionsResponse {
    pub backend: String,
    pub collections: Vec<CollectionEntry>,
}

// ─── ServerHandler — emitted from #[tool_handler] ────────────────────

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RuLakeMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = Implementation::new("ruvector-rulake-mcp", env!("CARGO_PKG_VERSION"));
        info.title = Some("ruLake MCP server".into());
        info.website_url = Some("https://github.com/ruvnet/RuLake".into());

        let mut init = ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        );
        init.server_info = info;
        init.instructions = Some(
            "ruLake MCP server (ADR-004 v0.2). Public tool: rulake_query \
             (intent=search|verify|explain). Resources: rulake://stats, \
             rulake://stats/by-backend. The response carries a decision trace \
             alongside the data; resources carry the cache-first KPI rollup."
                .into(),
        );
        init
    }

    /// `tools/list` — filtered by the server's effective capability
    /// set. ADR-004 §4 ("agents see one tool; ops sees the kernel;
    /// operators see everything"). The `--capabilities read` default
    /// exposes only `rulake_query` + `rulake_list_backends`; adding
    /// `publish` reveals the publish-tier tools, and so on.
    ///
    /// Enforcement is dual: the visibility filter here, AND the
    /// per-tool `require_cap` gate inside each handler. An adversary
    /// who knows tool names cannot bypass the filter to call them.
    async fn list_tools(
        &self,
        _params: Option<PaginatedRequestParams>,
        _ctx: RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        // v0.8: tools/list visibility honors the same effective-caps
        // intersection that gates per-call require_cap. An agent
        // whose JWT only grants `read` sees only read-tier tools,
        // even on a server started with --capabilities admin.
        let effective = crate::policy::effective_caps(&self.capabilities);
        let tools: Vec<rmcp::model::Tool> = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|t| effective.has(required_cap_for_tool(&t.name)))
            .collect();
        let mut result = ListToolsResult::default();
        result.tools = tools;
        Ok(result)
    }

    /// MCP resources/list — ADR-004 §Resources. v0.6 adds per-(backend,
    /// collection) bundle resources alongside the existing stats roll-ups.
    /// The bundle resource reads through `cache_witness_of` (ADR-005 §4
    /// cheap path); it does NOT call `BackendAdapter::current_bundle`'s
    /// default impl which would do a full `pull_vectors`.
    async fn list_resources(
        &self,
        _params: Option<PaginatedRequestParams>,
        _context: RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let mut resources = vec![
            {
                let mut r = RawResource::new("rulake://stats", "cache stats (rollup)");
                r.description = Some(
                    "Roll-up cache stats: hits, misses, primes, hit_rate, avg_prime_ms.".into(),
                );
                r.mime_type = Some("application/json".into());
                rmcp::model::Annotated::new(r, None)
            },
            {
                let mut r = RawResource::new("rulake://stats/by-backend", "cache stats per backend");
                r.description = Some("Per-backend cache stats: hits/misses/primes per backend id.".into());
                r.mime_type = Some("application/json".into());
                rmcp::model::Annotated::new(r, None)
            },
            {
                // v0.10: bounded ring buffer of recent audit lines.
                // Closes ADR-006 server-gap §V #2.
                let mut r = RawResource::new("rulake://audit/tail", "audit tail (jsonl)");
                r.description = Some(
                    "Last ≤256 audit lines (newest last) as a JSON array of the JSONL emit \
                     records. Backs the ruLake Console's Audit screen live mode."
                        .into(),
                );
                r.mime_type = Some("application/json".into());
                rmcp::model::Annotated::new(r, None)
            },
        ];
        // Per-(backend, collection) bundle resources for every cached
        // collection. Reads cheaply from `cache_witness_of`.
        for (k, _) in self.planner.lake.cache_stats_by_collection() {
            let (b, c) = (k.0.clone(), k.1.clone());
            let uri = format!("rulake://bundle/{b}/{c}");
            let mut r = RawResource::new(uri, format!("bundle witness: {b}/{c}"));
            r.description = Some(format!(
                "Witness + provenance for {b}/{c}. Reads the cache pointer; no full pull."
            ));
            r.mime_type = Some("application/json".into());
            resources.push(rmcp::model::Annotated::new(r, None));
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    /// MCP resources/read — serve the JSON body for a known URI.
    async fn read_resource(
        &self,
        params: ReadResourceRequestParams,
        _context: RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let body = self.read_resource_json(&params.uri).map_err(|e| {
            McpError::invalid_params(format!("RULAKE_RESOURCE: {e}"), None)
        })?;
        let contents = ResourceContents::text(body, params.uri);
        Ok(ReadResourceResult::new(vec![contents]))
    }
}

impl RuLakeMcpServer {
    /// Synchronous, lock-only resource read. v0.2 returns fresh stats
    /// on every read; the 200 ms TTL caching from ADR-004 §Resources
    /// is a v0.3 optimization (the lock-and-clone cost is sub-microsecond
    /// today so the TTL only matters at extreme poll rates).
    fn read_resource_json(&self, uri: &str) -> Result<String, String> {
        match uri {
            "rulake://stats" => {
                let s = self.planner.lake.cache_stats();
                serde_json::to_string(&serde_json::json!({
                    "hits":           s.hits,
                    "misses":         s.misses,
                    "primes":         s.primes,
                    "invalidations":  s.invalidations,
                    "shared_hits":    s.shared_hits,
                    "total_prime_ms": s.total_prime_ms,
                    "last_prime_ms":  s.last_prime_ms,
                    "warm_installs":  s.warm_installs,
                    "hit_rate":       s.hit_rate(),
                    "avg_prime_ms":   s.avg_prime_ms(),
                }))
                .map_err(|e| e.to_string())
            }
            "rulake://stats/by-backend" => {
                let by = self.planner.lake.cache_stats_by_backend();
                let map: serde_json::Map<String, serde_json::Value> = by
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            serde_json::json!({
                                "hits":          v.hits,
                                "misses":        v.misses,
                                "primes":        v.primes,
                                "invalidations": v.invalidations,
                                "shared_hits":   v.shared_hits,
                                "hit_rate":      v.hit_rate(),
                            }),
                        )
                    })
                    .collect();
                serde_json::to_string(&serde_json::Value::Object(map)).map_err(|e| e.to_string())
            }
            "rulake://audit/tail" => {
                // v0.10: snapshot the in-memory ring (capped at 256
                // entries by the AuditSink). Returns the JSONL records
                // as a JSON array so the client doesn't have to parse
                // line-by-line.
                let entries = self.audit.tail(256);
                serde_json::to_string(&serde_json::json!({
                    "entries": entries,
                    "count":   entries.len(),
                }))
                .map_err(|e| e.to_string())
            }
            uri if uri.starts_with("rulake://bundle/") => {
                // rulake://bundle/{backend}/{collection}
                let rest = &uri["rulake://bundle/".len()..];
                let mut parts = rest.splitn(2, '/');
                let backend = parts.next().unwrap_or("").to_string();
                let collection = parts.next().unwrap_or("").to_string();
                if backend.is_empty() || collection.is_empty() {
                    return Err(format!("malformed bundle URI: {uri}"));
                }
                let key = (backend.clone(), collection.clone());
                let witness = self.planner.lake.cache_witness_of(&key);
                let entry_count = self.planner.lake.cache_entry_count();
                serde_json::to_string(&serde_json::json!({
                    "backend":          backend,
                    "collection":       collection,
                    "witness":          witness,
                    "witness_present":  witness.is_some(),
                    "cache_entries":    entry_count,
                }))
                .map_err(|e| e.to_string())
            }
            other => Err(format!("unknown resource URI: {other}")),
        }
    }
}

