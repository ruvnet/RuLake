//! `RvdnaMcpServer` — the rmcp `ServerHandler` impl.
//!
//! v0.0.1 exposes five read-tier tools (ADR-007 §6 acceptance criteria):
//! - `rvdna_find`           — kNN over a pinned collection (stub + witness)
//! - `rvdna_call_variants`  — variants in a region (stub + witness)
//! - `rvdna_translate`      — DNA → protein (stub + witness)
//! - `rvdna_score`          — model-derived score (stub + witness)
//! - `rvdna_lineage`        — bundle witness chain (live, end-to-end)
//!
//! Every tool funnels through `check_witness` (see `registry.rs`) so a
//! drifted backend triggers `RVDNA_WITNESS_DRIFT` before any payload is
//! constructed. That is the R-1 mitigation per
//! `docs/research/security/v0.0-substrates.md`.
//!
//! The audit pattern follows `mcp-server::server::audit_mutation`
//! (commit 56b497b): every tool emits an `AuditEntry` with
//! `policy_decision` populated regardless of outcome
//! (`ok` | `refused` | `degraded` | `error`).

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

use rulake::backend::BackendAdapter;

use crate::audit::{AuditEntry, AuditSink, PolicyDecision, now_ts};
use crate::policy::{Capability, CapabilitySet};
use crate::registry::{RvdnaRegistry, WitnessCheckError};

#[derive(Clone)]
pub struct RvdnaMcpServer {
    registry: Arc<RvdnaRegistry>,
    capabilities: Arc<CapabilitySet>,
    audit: AuditSink,
    tool_router: ToolRouter<Self>,
}

impl RvdnaMcpServer {
    /// Build a fresh server with an empty registry + read-only caps.
    /// The operator wires backends in via `register_collection` before
    /// `serve_*`.
    pub fn new() -> Self {
        Self {
            registry: Arc::new(RvdnaRegistry::new()),
            capabilities: Arc::new(CapabilitySet::default()),
            audit: AuditSink::stderr(),
            tool_router: Self::tool_router(),
        }
    }

    pub fn with_capabilities(mut self, caps: CapabilitySet) -> Self {
        self.capabilities = Arc::new(caps);
        self
    }

    pub fn with_audit(mut self, sink: AuditSink) -> Self {
        self.audit = sink;
        self
    }

    pub fn audit(&self) -> &AuditSink {
        &self.audit
    }

    pub fn registry(&self) -> &Arc<RvdnaRegistry> {
        &self.registry
    }

    /// Register a collection at the operator level. NOT exposed as an
    /// MCP tool — v0.0.1 is read-only on the wire (R-1 §"mutation
    /// surface zero").
    pub fn register_collection(
        &self,
        backend: Arc<dyn BackendAdapter>,
        collection: impl Into<String>,
        rotation_seed: u64,
        rerank_factor: usize,
    ) -> anyhow::Result<()> {
        self.registry
            .register(backend, collection, rotation_seed, rerank_factor)?;
        Ok(())
    }

    /// Test-only mirror of `list_tools`'s capability filter.
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

    pub async fn serve_stdio(self) -> anyhow::Result<()> {
        let (stdin, stdout) = rmcp::transport::stdio();
        let service = self.serve((stdin, stdout)).await?;
        service.waiting().await?;
        Ok(())
    }
}

impl Default for RvdnaMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tool argument / response shapes ──────────────────────────────────

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct FindArgs {
    pub backend: String,
    pub collection: String,
    /// Top-K. Bounded server-side in v0.1 alongside the real index.
    pub k: u32,
    /// Query vector. v0.0.1 returns a stub; only the dim is used to
    /// shape the response.
    pub query: Vec<f32>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct FindResponse {
    pub backend: String,
    pub collection: String,
    pub witness: String,
    pub hits: Vec<FindHit>,
    /// `true` in v0.0.1 — flips false in v0.1 when the kNN inner loop
    /// lands. Lets callers gate on stub-vs-live without sniffing types.
    pub stub: bool,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct FindHit {
    pub id: u64,
    pub score: f32,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct RegionArgs {
    pub backend: String,
    pub collection: String,
    /// Region in 1-based half-open form: `chr1:1000-1100`.
    pub region: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct CallVariantsResponse {
    pub backend: String,
    pub collection: String,
    pub region: String,
    pub witness: String,
    pub variants: Vec<serde_json::Value>,
    pub stub: bool,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct TranslateResponse {
    pub backend: String,
    pub collection: String,
    pub region: String,
    pub witness: String,
    /// Single-letter amino acid string. Empty in the v0.0.1 stub.
    pub protein: String,
    pub stub: bool,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct ScoreResponse {
    pub backend: String,
    pub collection: String,
    pub region: String,
    pub witness: String,
    pub score: f64,
    pub stub: bool,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct LineageArgs {
    pub backend: String,
    pub collection: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct LineageResponse {
    pub backend: String,
    pub collection: String,
    /// Witness captured at `register_collection` time.
    pub pinned_witness: String,
    /// Witness re-derived at request time. Equal to `pinned_witness`
    /// or the request would have been refused.
    pub live_witness: String,
    /// Full pinned bundle — `data_ref`, `dim`, `rotation_seed`,
    /// `rerank_factor`, `generation`, `memory_class`. The trust
    /// anchor's full contents.
    pub pinned_bundle: serde_json::Value,
    /// `true` if pinned and live witnesses agreed at this call. Always
    /// `true` in a successful response (drift would have refused).
    pub witness_verified: bool,
}

// ─── Tool definitions ─────────────────────────────────────────────────

#[tool_router(router = tool_router)]
impl RvdnaMcpServer {
    #[tool(
        name = "rvdna_find",
        description = "Read: kNN search over a pinned rvdna T0 collection. v0.0.1 returns a \
                       witnessed stub (the witness is live + verified, the hit list is empty) \
                       so callers can wire the call shape today. The RaBitQ inner loop lands \
                       in v0.1 alongside the real T0 index. Gated by --capabilities read; \
                       refuses with RVDNA_WITNESS_DRIFT if the backend mutated since register."
    )]
    pub async fn rvdna_find(
        &self,
        Parameters(args): Parameters<FindArgs>,
    ) -> Result<Json<FindResponse>, McpError> {
        let start = std::time::Instant::now();
        let r: Result<Json<FindResponse>, McpError> = (|| {
            require_cap(&self.capabilities, Capability::Read)?;
            let (_pin, live) = self
                .registry
                .check_witness(&args.backend, &args.collection)
                .map_err(witness_error_to_mcp)?;
            Ok(Json(FindResponse {
                backend: args.backend.clone(),
                collection: args.collection.clone(),
                witness: live.rvf_witness,
                hits: Vec::new(),
                stub: true,
            }))
        })();
        self.audit_tool(
            "rvdna_find",
            Capability::Read,
            Some(format!("find:{}/{}", args.backend, args.collection)),
            &r,
            r.as_ref().ok().map(|j| j.0.hits.len() as u32),
            r.as_ref().ok().map(|j| j.0.witness.clone()),
            start.elapsed().as_secs_f64() * 1000.0,
        );
        r
    }

    #[tool(
        name = "rvdna_call_variants",
        description = "Read: variant calls for a region in a pinned rvdna T0 collection. \
                       v0.0.1 returns a witnessed stub (empty variants array); the precomputed \
                       variant index lands in v0.1. Gated by --capabilities read; refuses with \
                       RVDNA_WITNESS_DRIFT on backend drift."
    )]
    pub async fn rvdna_call_variants(
        &self,
        Parameters(args): Parameters<RegionArgs>,
    ) -> Result<Json<CallVariantsResponse>, McpError> {
        let start = std::time::Instant::now();
        let r: Result<Json<CallVariantsResponse>, McpError> = (|| {
            require_cap(&self.capabilities, Capability::Read)?;
            let (_pin, live) = self
                .registry
                .check_witness(&args.backend, &args.collection)
                .map_err(witness_error_to_mcp)?;
            Ok(Json(CallVariantsResponse {
                backend: args.backend.clone(),
                collection: args.collection.clone(),
                region: args.region.clone(),
                witness: live.rvf_witness,
                variants: Vec::new(),
                stub: true,
            }))
        })();
        self.audit_tool(
            "rvdna_call_variants",
            Capability::Read,
            Some(format!(
                "variants:{}/{}@{}",
                args.backend, args.collection, args.region
            )),
            &r,
            r.as_ref().ok().map(|j| j.0.variants.len() as u32),
            r.as_ref().ok().map(|j| j.0.witness.clone()),
            start.elapsed().as_secs_f64() * 1000.0,
        );
        r
    }

    #[tool(
        name = "rvdna_translate",
        description = "Read: DNA → protein for a region in a pinned rvdna T0 collection. \
                       v0.0.1 returns a witnessed stub (empty protein string); the codon table \
                       + ORF logic lands in v0.1. Gated by --capabilities read; refuses with \
                       RVDNA_WITNESS_DRIFT on backend drift."
    )]
    pub async fn rvdna_translate(
        &self,
        Parameters(args): Parameters<RegionArgs>,
    ) -> Result<Json<TranslateResponse>, McpError> {
        let start = std::time::Instant::now();
        let r: Result<Json<TranslateResponse>, McpError> = (|| {
            require_cap(&self.capabilities, Capability::Read)?;
            let (_pin, live) = self
                .registry
                .check_witness(&args.backend, &args.collection)
                .map_err(witness_error_to_mcp)?;
            Ok(Json(TranslateResponse {
                backend: args.backend.clone(),
                collection: args.collection.clone(),
                region: args.region.clone(),
                witness: live.rvf_witness,
                protein: String::new(),
                stub: true,
            }))
        })();
        self.audit_tool(
            "rvdna_translate",
            Capability::Read,
            Some(format!(
                "translate:{}/{}@{}",
                args.backend, args.collection, args.region
            )),
            &r,
            r.as_ref().ok().map(|j| j.0.protein.len() as u32),
            r.as_ref().ok().map(|j| j.0.witness.clone()),
            start.elapsed().as_secs_f64() * 1000.0,
        );
        r
    }

    #[tool(
        name = "rvdna_score",
        description = "Read: model-derived score (polygenic risk / pharmacogenomic dosing) for \
                       a region in a pinned rvdna T0 collection. v0.0.1 returns a witnessed \
                       stub (score=0.0); the scorer model wiring lands in v0.1. Gated by \
                       --capabilities read; refuses with RVDNA_WITNESS_DRIFT on backend drift."
    )]
    pub async fn rvdna_score(
        &self,
        Parameters(args): Parameters<RegionArgs>,
    ) -> Result<Json<ScoreResponse>, McpError> {
        let start = std::time::Instant::now();
        let r: Result<Json<ScoreResponse>, McpError> = (|| {
            require_cap(&self.capabilities, Capability::Read)?;
            let (_pin, live) = self
                .registry
                .check_witness(&args.backend, &args.collection)
                .map_err(witness_error_to_mcp)?;
            Ok(Json(ScoreResponse {
                backend: args.backend.clone(),
                collection: args.collection.clone(),
                region: args.region.clone(),
                witness: live.rvf_witness,
                score: 0.0,
                stub: true,
            }))
        })();
        self.audit_tool(
            "rvdna_score",
            Capability::Read,
            Some(format!(
                "score:{}/{}@{}",
                args.backend, args.collection, args.region
            )),
            &r,
            None,
            r.as_ref().ok().map(|j| j.0.witness.clone()),
            start.elapsed().as_secs_f64() * 1000.0,
        );
        r
    }

    #[tool(
        name = "rvdna_lineage",
        description = "Internal: bundle witness chain for a pinned rvdna T0 collection. \
                       Returns the operator-pinned bundle (data_ref, dim, rotation_seed, \
                       rerank_factor, generation) AND the live re-derived witness. Refuses \
                       with RVDNA_WITNESS_DRIFT if they disagree — that is the trust-anchor \
                       gate per ADR-007 §6. Gated by --capabilities internal."
    )]
    pub async fn rvdna_lineage(
        &self,
        Parameters(args): Parameters<LineageArgs>,
    ) -> Result<Json<LineageResponse>, McpError> {
        let start = std::time::Instant::now();
        let r: Result<Json<LineageResponse>, McpError> = (|| {
            require_cap(&self.capabilities, Capability::Internal)?;
            let (pin, live) = self
                .registry
                .check_witness(&args.backend, &args.collection)
                .map_err(witness_error_to_mcp)?;
            let pinned_bundle = serde_json::to_value(&pin.pinned_bundle).map_err(|e| {
                McpError::internal_error(format!("RVDNA_INTERNAL: serialize bundle: {e}"), None)
            })?;
            Ok(Json(LineageResponse {
                backend: args.backend.clone(),
                collection: args.collection.clone(),
                pinned_witness: pin.pinned_witness,
                live_witness: live.rvf_witness,
                pinned_bundle,
                witness_verified: true,
            }))
        })();
        self.audit_tool(
            "rvdna_lineage",
            Capability::Internal,
            Some(format!("lineage:{}/{}", args.backend, args.collection)),
            &r,
            r.as_ref().ok().map(|_| 1u32),
            r.as_ref().ok().map(|j| j.0.live_witness.clone()),
            start.elapsed().as_secs_f64() * 1000.0,
        );
        r
    }

    /// R-MCP-1: emit a fully-shaped audit line for any tool call.
    /// Outcome/code derived from the `Result` so the call site doesn't
    /// have to reshape state — mirrors `mcp-server::server::audit_mutation`
    /// (commit 56b497b).
    fn audit_tool<T>(
        &self,
        tool: &str,
        required: Capability,
        intent: Option<String>,
        result: &Result<T, McpError>,
        result_size: Option<u32>,
        witness_out: Option<String>,
        elapsed_ms: f64,
    ) {
        let (outcome, code) = match result {
            Ok(_) => ("ok", None),
            Err(e) => {
                let msg = e.message.to_string();
                if msg.contains("RVDNA_WITNESS_DRIFT") {
                    ("refused", Some("RVDNA_WITNESS_DRIFT".to_string()))
                } else if msg.contains("RVDNA_UNKNOWN_COLLECTION") {
                    ("refused", Some("RVDNA_UNKNOWN_COLLECTION".to_string()))
                } else if msg.contains("RVDNA_CAPABILITY_REFUSED") {
                    ("refused", Some("RVDNA_CAPABILITY_REFUSED".to_string()))
                } else if msg.starts_with("RVDNA_INTERNAL") {
                    ("error", Some("RVDNA_INTERNAL".to_string()))
                } else {
                    ("error", Some("RVDNA_INTERNAL".to_string()))
                }
            }
        };
        let trust_level = match outcome {
            "ok" => Some("verified".to_string()),
            "refused" => Some("unverified".to_string()),
            _ => None,
        };
        self.audit.emit(AuditEntry {
            ts: now_ts(),
            transport: "stdio".into(),
            principal: "stdio:local".into(),
            session: None,
            request_id: None,
            tool: tool.into(),
            intent,
            outcome: outcome.into(),
            result_size,
            trust_level,
            duration_ms: elapsed_ms,
            witness_in: None,
            witness_out,
            code,
            policy_decision: Some(PolicyDecision {
                capability_required: required.label().to_string(),
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
    caps.require(required)
        .map_err(|refused| McpError::invalid_request(refused.to_string(), None))
}

fn witness_error_to_mcp(e: WitnessCheckError) -> McpError {
    match e {
        WitnessCheckError::Unknown { .. } => McpError::invalid_params(e.to_string(), None),
        WitnessCheckError::Drift { .. } => McpError::invalid_request(e.to_string(), None),
        WitnessCheckError::Backend(_) => McpError::internal_error(e.to_string(), None),
    }
}

/// Single source of truth for tool→capability mapping (ADR-007 §D4).
/// Drives both the per-call `require_cap` gate AND the `tools/list`
/// visibility filter.
fn required_cap_for_tool(name: &str) -> Capability {
    match name {
        "rvdna_find" | "rvdna_call_variants" | "rvdna_translate" | "rvdna_score" => {
            Capability::Read
        }
        "rvdna_lineage" => Capability::Internal,
        // Anything else (future internal/admin tools) defaults to
        // internal — invisible by default. Safer than defaulting to Read.
        _ => Capability::Internal,
    }
}

// ─── ServerHandler — emitted from #[tool_handler] ────────────────────

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RvdnaMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = Implementation::new("ruvector-rulake-mcp-rvdna", env!("CARGO_PKG_VERSION"));
        info.title = Some("rvDNA MCP server".into());
        info.website_url = Some("https://github.com/ruvnet/RuLake".into());

        let mut init = ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        );
        init.server_info = info;
        init.instructions = Some(
            "rvDNA MCP server (ADR-007 §6, v0.0.1). Read tools: rvdna_find, \
             rvdna_call_variants, rvdna_translate, rvdna_score (all stubs + live \
             witness in v0.0.1). Internal tool: rvdna_lineage (live, end-to-end). \
             Mutation surface is intentionally zero in v0.0.1 (R-1 mitigation)."
                .into(),
        );
        init
    }

    /// `tools/list` — filtered by the server's capability set. Same
    /// dual-enforcement pattern as mcp-server: visibility AND per-call
    /// gate. An adversary who knows tool names cannot bypass the filter
    /// to call them.
    async fn list_tools(
        &self,
        _params: Option<PaginatedRequestParams>,
        _ctx: RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools: Vec<rmcp::model::Tool> = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|t| self.capabilities.has(required_cap_for_tool(&t.name)))
            .collect();
        let mut result = ListToolsResult::default();
        result.tools = tools;
        Ok(result)
    }

    async fn list_resources(
        &self,
        _params: Option<PaginatedRequestParams>,
        _context: RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let mut resources = vec![{
            // Bounded ring of recent audit lines — same shape as
            // mcp-server's rulake://audit/tail.
            let mut r = RawResource::new("rvdna://audit/tail", "audit tail (jsonl)");
            r.description = Some(
                "Last <=256 audit lines (newest last) as a JSON array of the JSONL emit \
                 records. Backs the Console's Audit screen live mode."
                    .into(),
            );
            r.mime_type = Some("application/json".into());
            rmcp::model::Annotated::new(r, None)
        }];
        // Per-(backend, collection) lineage resources for everything in
        // the registry. Cheap — reads pinned witness only, no backend
        // round-trip (use rvdna_lineage for the live re-derive).
        for (backend, collection) in self.registry.list() {
            let uri = format!("rvdna://lineage/{backend}/{collection}");
            let mut r = RawResource::new(uri, format!("lineage: {backend}/{collection}"));
            r.description = Some(format!(
                "Pinned bundle witness for {backend}/{collection}. Cheap — no backend call."
            ));
            r.mime_type = Some("application/json".into());
            resources.push(rmcp::model::Annotated::new(r, None));
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        params: ReadResourceRequestParams,
        _context: RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let body = self
            .read_resource_json(&params.uri)
            .map_err(|e| McpError::invalid_params(format!("RVDNA_RESOURCE: {e}"), None))?;
        let contents = ResourceContents::text(body, params.uri);
        Ok(ReadResourceResult::new(vec![contents]))
    }
}

impl RvdnaMcpServer {
    fn read_resource_json(&self, uri: &str) -> Result<String, String> {
        match uri {
            "rvdna://audit/tail" => {
                let entries = self.audit.tail(256);
                serde_json::to_string(&serde_json::json!({
                    "entries": entries,
                    "count":   entries.len(),
                }))
                .map_err(|e| e.to_string())
            }
            uri if uri.starts_with("rvdna://lineage/") => {
                let rest = &uri["rvdna://lineage/".len()..];
                let mut parts = rest.splitn(2, '/');
                let backend = parts.next().unwrap_or("").to_string();
                let collection = parts.next().unwrap_or("").to_string();
                if backend.is_empty() || collection.is_empty() {
                    return Err(format!("malformed lineage URI: {uri}"));
                }
                let pin = self
                    .registry
                    .get(&backend, &collection)
                    .ok_or_else(|| format!("unknown collection: {backend}/{collection}"))?;
                let bundle = serde_json::to_value(&pin.pinned_bundle)
                    .map_err(|e| format!("serialize bundle: {e}"))?;
                serde_json::to_string(&serde_json::json!({
                    "backend":        backend,
                    "collection":     collection,
                    "pinned_witness": pin.pinned_witness,
                    "pinned_bundle":  bundle,
                }))
                .map_err(|e| e.to_string())
            }
            other => Err(format!("unknown resource URI: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruvector_rulake_rvdna::{RvdnaCollection, RvdnaT0Backend};

    fn make_server() -> RvdnaMcpServer {
        let server = RvdnaMcpServer::new()
            .with_capabilities(CapabilitySet::from_csv("read,internal").unwrap());
        let be = Arc::new(RvdnaT0Backend::new("rvdna-test"));
        be.put_collection(
            "chr1",
            RvdnaCollection {
                ids: (0..4).collect(),
                vectors: (0..4).map(|i| vec![i as f32; 8]).collect(),
                dim: 8,
                generation: 1,
            },
        );
        let dyn_be: Arc<dyn BackendAdapter> = be;
        server
            .register_collection(dyn_be, "chr1", 42, 20)
            .expect("register chr1");
        server
    }

    #[tokio::test]
    async fn rvdna_find_returns_stub_with_live_witness() {
        let server = make_server();
        let resp = server
            .rvdna_find(Parameters(FindArgs {
                backend: "rvdna-test".into(),
                collection: "chr1".into(),
                k: 5,
                query: vec![0.0; 8],
            }))
            .await
            .expect("find ok");
        assert!(resp.0.stub, "v0.0.1 is a stub");
        assert!(!resp.0.witness.is_empty(), "live witness present");
        assert!(resp.0.hits.is_empty(), "stub returns no hits");
    }

    #[tokio::test]
    async fn rvdna_call_variants_returns_stub() {
        let server = make_server();
        let resp = server
            .rvdna_call_variants(Parameters(RegionArgs {
                backend: "rvdna-test".into(),
                collection: "chr1".into(),
                region: "chr1:1-100".into(),
            }))
            .await
            .expect("call_variants ok");
        assert!(resp.0.stub);
        assert!(!resp.0.witness.is_empty());
    }

    #[tokio::test]
    async fn rvdna_translate_returns_stub() {
        let server = make_server();
        let resp = server
            .rvdna_translate(Parameters(RegionArgs {
                backend: "rvdna-test".into(),
                collection: "chr1".into(),
                region: "chr1:1-100".into(),
            }))
            .await
            .expect("translate ok");
        assert!(resp.0.stub);
        assert!(resp.0.protein.is_empty());
    }

    #[tokio::test]
    async fn rvdna_score_returns_stub() {
        let server = make_server();
        let resp = server
            .rvdna_score(Parameters(RegionArgs {
                backend: "rvdna-test".into(),
                collection: "chr1".into(),
                region: "chr1:1-100".into(),
            }))
            .await
            .expect("score ok");
        assert!(resp.0.stub);
        assert_eq!(resp.0.score, 0.0);
    }

    #[tokio::test]
    async fn rvdna_lineage_round_trips_pinned_witness_end_to_end() {
        let server = make_server();
        let resp = server
            .rvdna_lineage(Parameters(LineageArgs {
                backend: "rvdna-test".into(),
                collection: "chr1".into(),
            }))
            .await
            .expect("lineage ok");
        assert_eq!(resp.0.pinned_witness, resp.0.live_witness);
        assert!(resp.0.witness_verified);
        // The bundle JSON carries the ruLake-side fields verbatim.
        let bundle = &resp.0.pinned_bundle;
        assert!(bundle.get("data_ref").is_some());
        assert!(bundle.get("rvf_witness").is_some());
    }

    #[tokio::test]
    async fn witness_drift_refuses_with_audit_code() {
        let server = RvdnaMcpServer::new()
            .with_capabilities(CapabilitySet::from_csv("read,internal").unwrap());
        let be = Arc::new(RvdnaT0Backend::new("rvdna-drift"));
        be.put_collection(
            "chr1",
            RvdnaCollection {
                ids: (0..2).collect(),
                vectors: vec![vec![0.0; 4], vec![1.0; 4]],
                dim: 4,
                generation: 1,
            },
        );
        let dyn_be: Arc<dyn BackendAdapter> = be.clone();
        server.register_collection(dyn_be, "chr1", 42, 20).unwrap();

        // Drift the backend out from under the pin.
        be.bump_generation("chr1").unwrap();

        let result = server
            .rvdna_find(Parameters(FindArgs {
                backend: "rvdna-drift".into(),
                collection: "chr1".into(),
                k: 1,
                query: vec![0.0; 4],
            }))
            .await;
        let err = match result {
            Ok(_) => panic!("must refuse on drift, got Ok"),
            Err(e) => e,
        };
        assert!(
            err.message.contains("RVDNA_WITNESS_DRIFT"),
            "drift code missing: {}",
            err.message
        );

        // The audit tail captured the refusal with the right code.
        let tail = server.audit().tail(8);
        let last = tail.last().expect("audit emitted");
        assert_eq!(last["tool"], "rvdna_find");
        assert_eq!(last["outcome"], "refused");
        assert_eq!(last["code"], "RVDNA_WITNESS_DRIFT");
    }

    #[test]
    fn list_tools_filtered_respects_capability_tier() {
        let server = RvdnaMcpServer::new(); // default = read only
        let names = server.list_tools_filtered();
        assert!(names.contains(&"rvdna_find".to_string()));
        assert!(names.contains(&"rvdna_call_variants".to_string()));
        assert!(names.contains(&"rvdna_translate".to_string()));
        assert!(names.contains(&"rvdna_score".to_string()));
        assert!(
            !names.contains(&"rvdna_lineage".to_string()),
            "lineage requires internal: {names:?}"
        );

        let server = RvdnaMcpServer::new()
            .with_capabilities(CapabilitySet::from_csv("read,internal").unwrap());
        let names = server.list_tools_filtered();
        assert!(names.contains(&"rvdna_lineage".to_string()));
    }
}
