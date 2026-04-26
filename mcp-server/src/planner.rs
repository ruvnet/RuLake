//! Planner — turns `rulake_query` requests into RuLake calls and emits
//! the decision trace alongside the answer (ADR-004 §4a).
//!
//! v0.1 supports `intent: "search"` only. The planner picks
//! single-backend vs federated search based on `target.routes` /
//! `target.backends`, applies the budget cap as `max_results`, and
//! threads the witness through to `provenance.witness`. The full
//! refusal taxonomy from §4a (POLICY_REFUSED_*, BUDGET_EXCEEDED_*,
//! WITNESS_MISMATCH_REFUSED) is wired but only some branches fire in
//! v0.1 — the rest land as the missing intents and capabilities ship.

use std::sync::Arc;

use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use ruvector_rulake::{RuLake, SearchResult};

use crate::workers::{SubmitError, WorkerPool};

// ─── Wire schemas (mirror ADR-004 §4a) ────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryRequest {
    /// `search | verify | explain | refresh`. v0.1 supports `search`.
    pub intent: Intent,

    pub target: Target,

    /// Search-intent args. Required when `intent == "search"`.
    #[serde(default)]
    pub search: Option<SearchArgs>,

    /// `low | medium | high`. Shapes the budget cap and policy floor.
    #[serde(default = "default_risk")]
    pub risk: Risk,

    /// Upper bound on staleness (ms). v0.1 honours via Consistency
    /// already configured on RuLake; finer-grained per-call control
    /// lands when the planner gains its own coherence-check loop.
    #[serde(default)]
    pub freshness_ms: Option<u64>,

    #[serde(default)]
    pub budget: Budget,

    #[serde(default)]
    pub policy: Policy,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Intent {
    Search,
    Verify,
    Explain,
    Refresh,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Target {
    /// Shorthand: planner expands to one route per allowed backend.
    #[serde(default)]
    pub collection: Option<String>,
    /// Explicit (backend, collection) pairs. Wins over `collection` +
    /// `backends` when set. Maps to `RuLake::search_federated`'s
    /// `&[(&str, &str)]` shape verbatim.
    #[serde(default)]
    pub routes: Vec<[String; 2]>,
    /// Optional backend allow-list when `collection` is shorthand.
    #[serde(default)]
    pub backends: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Query vector. Length-bounded to MAX_PULLED_DIM=8192 per ADR-004 §5.
    #[schemars(length(min = 1, max = 8192))]
    pub vector: Vec<f32>,
    /// Top-k. 1..=1000 — matches the cap in ADR-004 §5.
    #[schemars(range(min = 1, max = 1000))]
    pub k: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    #[default]
    Medium,
    Low,
    High,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Budget {
    #[serde(default = "default_max_latency_ms")]
    pub max_latency_ms: u64,
    #[serde(default = "default_max_backends")]
    pub max_backends: u32,
    #[serde(default = "default_max_results")]
    pub max_results: u32,
    #[serde(default = "default_max_rerank")]
    pub max_rerank: u32,
    #[serde(default)]
    pub force_global_rerank: bool,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_latency_ms: default_max_latency_ms(),
            max_backends: default_max_backends(),
            max_results: default_max_results(),
            max_rerank: default_max_rerank(),
            force_global_rerank: false,
        }
    }
}

fn default_max_latency_ms() -> u64 { 100 }
fn default_max_backends() -> u32 { 3 }
fn default_max_results() -> u32 { 20 }
fn default_max_rerank() -> u32 { 20 }
fn default_risk() -> Risk { Risk::Medium }

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct Policy {
    #[serde(default)]
    pub witness_required: bool,
    #[serde(default)]
    pub allow_partial: bool,
    #[serde(default = "default_min_collections_hit")]
    pub min_collections_hit: u32,
}

fn default_min_collections_hit() -> u32 { 1 }

// ─── Response (mirror ADR-004 §4a) ────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
pub struct QueryResponse {
    pub data: Vec<HitJson>,
    pub provenance: Provenance,
    pub trust_level: TrustLevel,
    pub decision: Decision,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HitJson {
    pub backend: String,
    pub collection: String,
    /// u64 stringified — JSON numbers can't carry the high bit safely.
    pub id: String,
    pub score: f32,
}

impl From<SearchResult> for HitJson {
    fn from(h: SearchResult) -> Self {
        Self {
            backend: h.backend,
            collection: h.collection,
            id: h.id.to_string(),
            score: h.score,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Provenance {
    pub witness: Option<String>,
    pub witness_verified: bool,
    pub consistency: String,
    pub served_from: Vec<String>,
    pub lineage_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TrustLevel {
    Verified,
    Unverified,
    Partial,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Decision {
    pub intent: String,
    pub chosen_action: String,
    pub reason_code: ReasonCode,
    pub reason: String,
    pub backends_planned: Vec<String>,
    pub backends_used: Vec<String>,
    pub consistency_used: String,
    pub budget_used_ms: f64,
    pub budget_cap_ms: u64,
    pub degraded: bool,
    pub refusals: Vec<Refusal>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Refusal {
    pub route: [String; 2],
    pub code: String,
}

/// Closed enum from ADR-004 §4a. Wire schema never grows; ADR bumps when new values appear.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReasonCode {
    CacheHitFresh,
    CacheHitEventual,
    StaleCacheRemoteValid,
    ColdPrimeThenServe,
    WitnessMismatchRefused,
    BudgetExceededFallbackCache,
    BudgetExceededRefused,
    PolicyRefusedRisk,
    PolicyRefusedAllowlist,
    PolicyRefusedPath,
    PartialFederation,
    /// v0.1 placeholder — covers cases the v0.1 planner doesn't yet
    /// classify finely. Promoted to a specific variant in v0.2.
    Other,
}

// ─── Errors ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum PlanError {
    Refused(QueryResponse),
    /// Backpressure — the response shape is wrapped at the call site
    /// into the `_meta.rulake.degraded` block per ADR-004 §6.
    Degraded { inflight: usize, cap: usize },
    /// Internal — bubble up as a tool-level isError.
    Internal(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(_) => write!(f, "RULAKE_REFUSED"),
            Self::Degraded { inflight, cap } => {
                write!(f, "RULAKE_DEGRADED: inflight={inflight} cap={cap}")
            }
            Self::Internal(s) => write!(f, "RULAKE_INTERNAL: {s}"),
        }
    }
}
impl std::error::Error for PlanError {}

// ─── The planner itself ───────────────────────────────────────────────

pub struct Planner {
    pub lake: Arc<RuLake>,
    pub workers: WorkerPool,
    pub backend_ids: Vec<String>,
    pub consistency_label: String,
}

impl Planner {
    pub async fn handle(&self, req: QueryRequest) -> Result<QueryResponse, PlanError> {
        let start = std::time::Instant::now();
        match req.intent {
            Intent::Search => self.handle_search(req, start).await,
            Intent::Verify | Intent::Explain | Intent::Refresh => {
                Err(PlanError::Internal(format!(
                    "intent {:?} is not implemented in v0.1 (ADR-004 §Open questions)",
                    req.intent
                )))
            }
        }
    }

    async fn handle_search(
        &self,
        req: QueryRequest,
        start: std::time::Instant,
    ) -> Result<QueryResponse, PlanError> {
        let SearchArgs { vector, k } = req
            .search
            .ok_or_else(|| PlanError::Internal("intent=search requires `search` block".into()))?;

        // Risk floor: low forces witness_required.
        let mut policy = req.policy;
        if matches!(req.risk, Risk::Low) {
            policy.witness_required = true;
        }

        // Resolve target → routes. A refusal here is a successful
        // planner outcome (empty data + populated refusals) per
        // ADR-004 §4a, NOT an error.
        let routes = match self.resolve_routes(&req.target, req.budget.max_backends as usize) {
            Ok(r) => r,
            Err(PlanError::Refused(resp)) => return Ok(resp),
            Err(e) => return Err(e),
        };
        if routes.is_empty() {
            return Ok(self.build_refusal(
                "no allowed routes",
                ReasonCode::PolicyRefusedAllowlist,
                vec![],
                vec![],
                start,
                req.budget.max_latency_ms,
            ));
        }

        // Per ADR-004 §4a budget: max_results caps k.
        let effective_k = (k as usize).min(req.budget.max_results as usize).max(1);

        let lake = Arc::clone(&self.lake);
        let routes_for_run = routes.clone();
        let submit = self
            .workers
            .submit(move || -> Result<Vec<SearchResult>, ruvector_rulake::RuLakeError> {
                if routes_for_run.len() == 1 {
                    let (b, c) = &routes_for_run[0];
                    lake.search_one(b, c, &vector, effective_k)
                } else {
                    let refs: Vec<(&str, &str)> = routes_for_run
                        .iter()
                        .map(|(b, c)| (b.as_str(), c.as_str()))
                        .collect();
                    lake.search_federated(&refs, &vector, effective_k)
                }
            })
            .await;

        let hits = match submit {
            Ok(Ok(hits)) => hits,
            Ok(Err(e)) => return Err(PlanError::Internal(format!("ruLake: {e}"))),
            Err(SubmitError::Degraded { inflight, cap }) => {
                return Err(PlanError::Degraded { inflight, cap });
            }
            Err(e) => return Err(PlanError::Internal(format!("worker: {e}"))),
        };

        let backends_used: Vec<String> = {
            let mut set: Vec<String> = hits.iter().map(|h| h.backend.clone()).collect();
            set.sort();
            set.dedup();
            set
        };
        let backends_planned: Vec<String> =
            routes.iter().map(|(b, _)| b.clone()).collect();

        // v0.1 doesn't actually verify witnesses on the search path —
        // that's plumbed via Consistency in the Rust crate. trust_level
        // therefore reflects what Consistency was configured to do.
        let witness = if let Some((b, c)) = routes.first() {
            self.lake
                .cache_witness_of(&(b.clone(), c.clone()))
        } else {
            None
        };
        let witness_verified = matches!(
            self.consistency_label.as_str(),
            "Fresh" | "Frozen"
        );
        let trust_level = if policy.witness_required && !witness_verified {
            // ADR-004 precedence: witness > freshness > budget. Refusal
            // is a successful planner outcome (Ok), not a PlanError.
            return Ok(self.build_refusal(
                "policy.witness_required: true but consistency does not verify on every call",
                ReasonCode::PolicyRefusedRisk,
                backends_planned,
                vec![],
                start,
                req.budget.max_latency_ms,
            ));
        } else if witness_verified {
            TrustLevel::Verified
        } else {
            TrustLevel::Unverified
        };

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok(QueryResponse {
            data: hits.into_iter().map(HitJson::from).collect(),
            provenance: Provenance {
                witness,
                witness_verified,
                consistency: self.consistency_label.clone(),
                served_from: backends_used.clone(),
                lineage_id: None,
            },
            trust_level,
            decision: Decision {
                intent: "search".into(),
                chosen_action: if routes.len() == 1 {
                    "search_one".into()
                } else {
                    "search_federated".into()
                },
                reason_code: ReasonCode::CacheHitFresh,
                reason: "served from cache (v0.1 planner does not yet inspect cache state)".into(),
                backends_planned,
                backends_used,
                consistency_used: self.consistency_label.clone(),
                budget_used_ms: elapsed_ms,
                budget_cap_ms: req.budget.max_latency_ms,
                degraded: false,
                refusals: vec![],
            },
        })
    }

    fn resolve_routes(
        &self,
        target: &Target,
        max_backends: usize,
    ) -> Result<Vec<(String, String)>, PlanError> {
        let routes: Vec<(String, String)> = if !target.routes.is_empty() {
            target
                .routes
                .iter()
                .take(max_backends.max(1))
                .map(|p| (p[0].clone(), p[1].clone()))
                .collect()
        } else if let Some(coll) = &target.collection {
            let backends: Vec<&String> = if !target.backends.is_empty() {
                target
                    .backends
                    .iter()
                    .filter(|b| self.backend_ids.contains(b))
                    .collect()
            } else {
                self.backend_ids.iter().collect()
            };
            backends
                .into_iter()
                .take(max_backends.max(1))
                .map(|b| (b.clone(), coll.clone()))
                .collect()
        } else {
            return Err(PlanError::Internal(
                "target requires either `routes` or `collection`".into(),
            ));
        };

        for (b, _) in &routes {
            if !self.backend_ids.contains(b) {
                return Err(PlanError::Refused(self.build_refusal(
                    &format!("backend {b:?} not registered"),
                    ReasonCode::PolicyRefusedAllowlist,
                    vec![b.clone()],
                    vec![],
                    std::time::Instant::now(),
                    100,
                )));
            }
        }
        Ok(routes)
    }

    fn build_refusal(
        &self,
        reason: &str,
        code: ReasonCode,
        backends_planned: Vec<String>,
        refusals: Vec<Refusal>,
        start: std::time::Instant,
        budget_cap_ms: u64,
    ) -> QueryResponse {
        QueryResponse {
            data: vec![],
            provenance: Provenance {
                witness: None,
                witness_verified: false,
                consistency: self.consistency_label.clone(),
                served_from: vec![],
                lineage_id: None,
            },
            trust_level: TrustLevel::Unverified,
            decision: Decision {
                intent: "search".into(),
                chosen_action: "refused".into(),
                reason_code: code,
                reason: reason.to_string(),
                backends_planned,
                backends_used: vec![],
                consistency_used: self.consistency_label.clone(),
                budget_used_ms: start.elapsed().as_secs_f64() * 1000.0,
                budget_cap_ms,
                degraded: false,
                refusals,
            },
        }
    }
}
