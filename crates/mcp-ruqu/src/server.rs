//! `RuquMcpServer` — the rmcp `ServerHandler` impl for ruQu (ADR-008 §6).
//!
//! v0.0.1 exposes five tools:
//!
//! | tool                | tier    | status   |
//! |---------------------|---------|----------|
//! | `ruqu_simulate`     | publish | live     |
//! | `ruqu_verify`       | read    | live     |
//! | `ruqu_replay`       | read    | live     |
//! | `ruqu_optimize`     | read    | **stub** |
//! | `ruqu_qec_schedule` | read    | **stub** |
//!
//! `ruqu_simulate` is the only **publish** tier in v0.0.1: it
//! allocates Hilbert-space memory (per ADR-008 §6 Decision 4 the
//! `simulate` capability is reserved for v0.1 — for now we ride on
//! the existing `Publish` tier so the cap-gate logic is identical
//! to mcp-server's). The two stubs return a small JSON envelope and
//! always emit a `RUQU_OPTIMIZE_STUB` / `RUQU_QEC_STUB` audit code
//! so v0.1's planner work can replace the body without changing
//! the wire shape.
//!
//! Mirrors `mcp-server/src/server.rs:189` for the `#[tool_router]`
//! macro pattern; mirrors commit 56b497b for the discipline of
//! emitting a fully-shaped `AuditEntry` (with `policy_decision`)
//! on every code path — ok, refused, degraded, error.

use std::sync::Arc;

use rmcp::{
    handler::server::{
        router::tool::ToolRouter,
        wrapper::{Json, Parameters},
    },
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};

use rulake::{BackendAdapter, RuLake};
use ruvector_rulake_ruqu::{
    circuit::{Circuit, Gate},
    limits::enforce_qubit_cap,
    witness::{inputs_for, ruqu_bundle},
    RuquStateVectorBackend,
};

use crate::audit::{now_ts, AuditEntry, AuditSink, PolicyDecision};

// ─── Security: R-1 mitigation constants ──────────────────────────────

/// Maximum length of `Circuit.id` accepted by `ruqu_simulate`. The
/// id is the witness anchor (`data_ref = "ruqu://<id>@<tag>"`); a
/// 256-byte cap is generous for any plausible content hash (SHA-256
/// hex = 64 bytes; SHA-512 hex = 128 bytes) and rejects pathological
/// inputs that would inflate the witness preimage. Per security
/// review **R-1** in the brief.
const MAX_CIRCUIT_ID_LEN: usize = 256;

// ─── Server type ─────────────────────────────────────────────────────

/// MCP companion server for ruQu v2.
///
/// Wraps a `RuquStateVectorBackend` for the live simulation path
/// and (optionally) holds a handle to a `RuLake` so `ruqu_replay`
/// can probe the cache via `lake.search_one` per the ADR-008 §6
/// Decision 5 cache-aware semantics. v0.0.1 the lake handle is
/// optional — when absent, `ruqu_replay` falls back to executing
/// the circuit (still functionally correct, just no fast-path).
#[derive(Clone)]
pub struct RuquMcpServer {
    backend: Arc<RuquStateVectorBackend>,
    lake: Option<Arc<RuLake>>,
    audit: AuditSink,
    tool_router: ToolRouter<Self>,
}

impl RuquMcpServer {
    /// Build a server wrapping a fresh `RuquStateVectorBackend`
    /// with id `backend_id`. No lake handle — `ruqu_replay` will
    /// always fall back to live execution.
    pub fn new(backend_id: impl Into<String>) -> Self {
        Self {
            backend: Arc::new(RuquStateVectorBackend::new(backend_id)),
            lake: None,
            audit: AuditSink::stderr(),
            tool_router: Self::tool_router(),
        }
    }

    /// Build a server with an external `RuquStateVectorBackend` +
    /// `RuLake` so `ruqu_replay` can short-circuit through the
    /// cache. The brief stipulates this path "uses ruLake's cache
    /// directly via `lake.search_one`".
    pub fn with_lake(backend: Arc<RuquStateVectorBackend>, lake: Arc<RuLake>) -> Self {
        Self {
            backend,
            lake: Some(lake),
            audit: AuditSink::stderr(),
            tool_router: Self::tool_router(),
        }
    }

    /// Replace the audit sink. Call before serve_*.
    pub fn with_audit(mut self, sink: AuditSink) -> Self {
        self.audit = sink;
        self
    }

    /// Test/inspection accessor.
    pub fn audit(&self) -> &AuditSink {
        &self.audit
    }

    /// Drive the server over stdio. Blocks until the peer closes
    /// stdin/stdout. Same shape as `mcp-server`'s `serve_stdio`.
    pub async fn serve_stdio(self) -> anyhow::Result<()> {
        let (stdin, stdout) = rmcp::transport::stdio();
        let service = self.serve((stdin, stdout)).await?;
        service.waiting().await?;
        Ok(())
    }

    /// Direct-call helper for tests: invoke `ruqu_simulate` without
    /// going through the rmcp wire. Returns the tool's response.
    #[doc(hidden)]
    pub async fn call_simulate(&self, req: SimulateRequest) -> Result<SimulateResponse, McpError> {
        self.ruqu_simulate(Parameters(req)).await.map(|j| j.0)
    }

    /// Direct-call helper for tests.
    #[doc(hidden)]
    pub async fn call_verify(&self, req: VerifyRequest) -> Result<VerifyResponse, McpError> {
        self.ruqu_verify(Parameters(req)).await.map(|j| j.0)
    }

    /// Direct-call helper for tests.
    #[doc(hidden)]
    pub async fn call_replay(&self, req: VerifyRequest) -> Result<ReplayResponse, McpError> {
        self.ruqu_replay(Parameters(req)).await.map(|j| j.0)
    }

    /// Direct-call helper for tests.
    #[doc(hidden)]
    pub async fn call_optimize(&self, req: StubRequest) -> Result<StubResponse, McpError> {
        self.ruqu_optimize(Parameters(req)).await.map(|j| j.0)
    }

    /// Direct-call helper for tests.
    #[doc(hidden)]
    pub async fn call_qec_schedule(&self, req: StubRequest) -> Result<StubResponse, McpError> {
        self.ruqu_qec_schedule(Parameters(req)).await.map(|j| j.0)
    }
}

// ─── Wire types ──────────────────────────────────────────────────────

/// JSON-Schema-bearing mirror of `ruqu_backend::circuit::Gate`.
///
/// `Gate` itself doesn't derive `JsonSchema` and the brief forbids
/// editing `ruqu-backend/src/`, so we mirror the enum here with the
/// `schemars` derive and translate at the request boundary. The
/// `#[serde(tag = "kind")]` discriminant matches the upstream form
/// byte-for-byte so JSON payloads round-trip unchanged.
#[derive(Debug, Clone, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireGate {
    /// Hadamard on `q`.
    H { q: u8 },
    /// Pauli-X on `q`.
    X { q: u8 },
    /// Pauli-Y on `q`.
    Y { q: u8 },
    /// Pauli-Z on `q`.
    Z { q: u8 },
    /// S = diag(1, i) on `q`.
    S { q: u8 },
    /// T = diag(1, e^{iπ/4}) on `q`.
    T { q: u8 },
    /// Z-axis rotation by `theta` radians on `q`. **R-3 caveat**:
    /// `theta` is NOT bound into the witness in v0.0; encode it
    /// into `WireCircuit.id` if you care about replay determinism.
    Rz { q: u8, theta: f64 },
    /// CNOT with `control` and `target` distinct qubits.
    Cx { control: u8, target: u8 },
}

impl From<WireGate> for Gate {
    fn from(g: WireGate) -> Self {
        match g {
            WireGate::H { q } => Gate::H { q },
            WireGate::X { q } => Gate::X { q },
            WireGate::Y { q } => Gate::Y { q },
            WireGate::Z { q } => Gate::Z { q },
            WireGate::S { q } => Gate::S { q },
            WireGate::T { q } => Gate::T { q },
            WireGate::Rz { q, theta } => Gate::Rz { q, theta },
            WireGate::Cx { control, target } => Gate::Cx { control, target },
        }
    }
}

impl From<Gate> for WireGate {
    fn from(g: Gate) -> Self {
        match g {
            Gate::H { q } => WireGate::H { q },
            Gate::X { q } => WireGate::X { q },
            Gate::Y { q } => WireGate::Y { q },
            Gate::Z { q } => WireGate::Z { q },
            Gate::S { q } => WireGate::S { q },
            Gate::T { q } => WireGate::T { q },
            Gate::Rz { q, theta } => WireGate::Rz { q, theta },
            Gate::Cx { control, target } => WireGate::Cx { control, target },
        }
    }
}

/// JSON-Schema-bearing mirror of `ruqu_backend::circuit::Circuit`.
///
/// Same rationale as [`WireGate`]: upstream `Circuit` doesn't derive
/// `JsonSchema` and we can't touch the substrate src. Conversion is
/// trivial (three fields) and zero-cost in release.
#[derive(Debug, Clone, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct WireCircuit {
    /// Number of qubits. State vector dim is `1 << n_qubits`. v0.0
    /// caps at 16 — see R-2.
    pub n_qubits: u8,
    /// Operations applied left-to-right.
    pub gates: Vec<WireGate>,
    /// Stable identifier flowing into the witness `data_ref`.
    /// Operators MUST set this from a content hash of the source
    /// program. Empty / overlong (>256 bytes) IDs are R-1 refused.
    pub id: String,
}

impl From<WireCircuit> for Circuit {
    fn from(c: WireCircuit) -> Self {
        let mut out = Circuit::new(c.id, c.n_qubits);
        for g in c.gates {
            out.push(g.into());
        }
        out
    }
}

impl From<Circuit> for WireCircuit {
    fn from(c: Circuit) -> Self {
        Self {
            n_qubits: c.n_qubits,
            gates: c.gates.into_iter().map(Into::into).collect(),
            id: c.id,
        }
    }
}

/// Request shape for `ruqu_simulate`. `circuit` mirrors the
/// `ruqu-backend::circuit::Circuit` mini-IR via [`WireCircuit`]
/// (same JSON wire form; carries the `JsonSchema` derive that
/// `rmcp::tool` requires).
#[derive(Debug, Clone, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct SimulateRequest {
    /// Circuit to simulate. `id` MUST be non-empty and ≤256 bytes —
    /// it's the witness anchor. `n_qubits` MUST be ≤16 (v0.0 cap).
    pub circuit: WireCircuit,
    /// Backend tag flowing into `data_ref`. v0.0.1 only "sv" is
    /// implemented; future backends ("stabilizer", "tensor", …)
    /// will reject here until the corresponding adapter ships.
    #[serde(default = "default_backend_tag")]
    pub backend_tag: String,
    /// PRNG seed for any noise / sampling. `0` for noiseless
    /// StateVector (the witness still discriminates because
    /// `data_ref` carries the backend tag).
    #[serde(default)]
    pub seed: u64,
}

fn default_backend_tag() -> String {
    "sv".to_string()
}

/// Response shape for `ruqu_simulate`. The brief: "DON'T return the
/// full state vector — for n=14 that's 16384 amplitudes; cap at 32
/// with a note in the response shape".
#[derive(Debug, Clone, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct SimulateResponse {
    /// SHAKE-256(32) witness from `RuLakeBundle` — hex-encoded.
    pub witness: String,
    /// Full Hilbert-space dimension (`1 << n_qubits`).
    pub dim: usize,
    /// First `min(dim, 32)` amplitudes as `[real, imag]` pairs.
    /// Truncated for transport — callers wanting the full vector
    /// MUST call `pull_vectors` on the backend directly (mcp tool
    /// for that lands in v0.1).
    pub amplitudes_head: Vec<[f64; 2]>,
    /// `true` when `amplitudes_head.len() < dim` — informational so
    /// callers don't conflate "32 amps returned" with "n=5 circuit".
    pub truncated: bool,
}

/// Request shape for `ruqu_verify` and `ruqu_replay`.
#[derive(Debug, Clone, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct VerifyRequest {
    /// Circuit to replay. Same validation rules as `SimulateRequest`.
    pub circuit: WireCircuit,
    /// Hex-encoded witness from a prior `ruqu_simulate`. Compared
    /// byte-for-byte against the freshly-derived bundle witness.
    pub witness: String,
    /// Backend tag — must match the one used at simulate time, or
    /// the witness will trivially diverge.
    #[serde(default = "default_backend_tag")]
    pub backend_tag: String,
    /// Seed — must match too. See `SimulateRequest::seed`.
    #[serde(default)]
    pub seed: u64,
}

/// Response for `ruqu_verify`.
#[derive(Debug, Clone, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct VerifyResponse {
    /// `true` iff the freshly-derived witness equals the supplied
    /// one byte-for-byte.
    pub matches: bool,
    /// The witness we computed — useful for debugging mismatches.
    pub computed_witness: String,
}

/// Response for `ruqu_replay`. Adds a `cache_hit` flag so callers
/// can tell whether the lake short-circuited the simulation.
#[derive(Debug, Clone, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct ReplayResponse {
    /// `true` iff the freshly-derived witness equals the supplied
    /// one byte-for-byte.
    pub matches: bool,
    /// The witness we computed.
    pub computed_witness: String,
    /// `true` when the lake cache had an entry keyed at this
    /// witness so we skipped the simulation. `false` when we had
    /// to execute (no lake configured, or cache miss).
    pub cache_hit: bool,
}

/// Request shape for the v0.0.1 stubs. Accepts an opaque circuit so
/// the wire shape is forward-compatible with the v0.1 planner.
#[derive(Debug, Clone, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct StubRequest {
    /// Circuit the planner / scheduler will eventually act on.
    pub circuit: WireCircuit,
}

/// Response shape for the v0.0.1 stubs (`ruqu_optimize`,
/// `ruqu_qec_schedule`). Carries an explicit `v01_planned` /
/// `v02_planned` flag so callers can detect a stub without parsing
/// the audit code out-of-band.
#[derive(Debug, Clone, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct StubResponse {
    /// Always `"stub"` in v0.0.1.
    pub status: String,
    /// `true` for `ruqu_optimize` (planner ships v0.1); `false` for
    /// `ruqu_qec_schedule` (QEC ships v0.2 — see `v02_planned`).
    pub v01_planned: bool,
    /// `true` for `ruqu_qec_schedule` only.
    pub v02_planned: bool,
    /// Reason — populated with the audit code so the wire response
    /// and the audit log carry matching strings.
    pub note: String,
    /// Phase B (ruqu_optimize only): gate count BEFORE the optimizer
    /// pass. `None` for stubs and for ruqu_qec_schedule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_count_before: Option<usize>,
    /// Phase B (ruqu_optimize only): gate count AFTER fuse_gates.
    /// `None` for stubs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_count_after: Option<usize>,
    /// Phase B (ruqu_optimize only): true if the circuit is Clifford
    /// (per ruqu-core's `circuit_analyzer::is_clifford_circuit`).
    /// Clifford circuits get the Stabilizer fast path on simulate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_clifford: Option<bool>,
    /// Phase B (ruqu_optimize only): non-Clifford gate count
    /// (T-gates etc.) — the standard quantum-resource-estimation metric.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub non_clifford_count: Option<usize>,
    /// Phase B (ruqu_qec_schedule only): code distance picked
    /// (currently always 3 — only distance ruqu-algorithms supports).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_distance: Option<u32>,
    /// Phase B (ruqu_qec_schedule only): QEC cycles run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cycles: Option<u32>,
    /// Phase B (ruqu_qec_schedule only): empirical logical-error rate
    /// from running the distance-3 surface code with the chosen
    /// physical_error_rate. Lower = better protection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_error_rate: Option<f64>,
    /// Phase B (ruqu_qec_schedule only): logical error count over
    /// the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_errors: Option<u32>,
}

// ─── Tool definitions ────────────────────────────────────────────────

#[tool_router(router = tool_router)]
impl RuquMcpServer {
    #[tool(
        name = "ruqu_simulate",
        description = "Publish: simulate a quantum circuit through the ruQu StateVector backend and \
                       return the bundle witness plus the first 32 amplitudes. \
                       Allocates Hilbert-space memory — gated behind the publish capability tier. \
                       \
                       SECURITY: `circuit.id` MUST be a non-empty, content-derived string \
                       ≤256 bytes (it's the witness anchor — see ADR-008 §3 R-1). `circuit.n_qubits` \
                       MUST be ≤16 in v0.0 (R-2; the StateVector backend allocates `1<<n` complex \
                       amplitudes — 16 qubits = 1 MiB, 20 qubits = 16 MiB, 25 qubits = 512 MiB). \
                       \
                       R-3 CAVEAT: gate parameters (e.g. `Rz.theta`) are NOT bound into the witness \
                       in v0.0; the witness covers `data_ref = ruqu://<circuit.id>@<backend_tag>` + \
                       `dim` + `seed` + `rerank_factor` + `generation`. Callers who care about \
                       parameter-replay determinism MUST encode the parameters into `circuit.id` \
                       (e.g. via a SHA-256 over the source program). v0.0.2 will hash gate \
                       parameters into a content-derived `data_ref` to close this."
    )]
    pub async fn ruqu_simulate(
        &self,
        Parameters(req): Parameters<SimulateRequest>,
    ) -> Result<Json<SimulateResponse>, McpError> {
        let start = std::time::Instant::now();
        let id_for_audit = req.circuit.id.clone();
        let n_qubits = req.circuit.n_qubits;

        // ── R-1: refuse empty / overlong circuit.id ───────────────
        if let Err(reason) = validate_circuit_id(&req.circuit.id) {
            self.emit_refusal(
                "ruqu_simulate",
                "publish",
                "RUQU_INVALID_CIRCUIT_ID",
                &id_for_audit,
                start.elapsed().as_secs_f64() * 1000.0,
            );
            return Err(McpError::invalid_params(
                format!("RUQU_INVALID_CIRCUIT_ID: {reason}"),
                None,
            ));
        }

        // ── R-2: enforce qubit cap BEFORE execute() ───────────────
        if let Err(e) = enforce_qubit_cap(n_qubits) {
            self.emit_refusal(
                "ruqu_simulate",
                "publish",
                "RUQU_QUBIT_CAP_EXCEEDED",
                &id_for_audit,
                start.elapsed().as_secs_f64() * 1000.0,
            );
            return Err(McpError::invalid_params(
                format!("RUQU_QUBIT_CAP_EXCEEDED: {e}"),
                None,
            ));
        }

        // ── live path ─────────────────────────────────────────────
        let circuit: Circuit = req.circuit.into();
        let bundle = self
            .backend
            .execute(circuit.id.clone(), &circuit)
            .map_err(|e| {
                self.emit_refusal(
                    "ruqu_simulate",
                    "publish",
                    "RUQU_INTERNAL",
                    &id_for_audit,
                    start.elapsed().as_secs_f64() * 1000.0,
                );
                McpError::internal_error(format!("RUQU_INTERNAL: {e}"), None)
            })?;
        let witness = bundle.rvf_witness.clone();

        // Re-simulate locally to get the head amplitudes — `execute`
        // intentionally hides the state vector behind `pull_vectors`
        // so the trait stays uniform; for the wire response we need
        // the first 32 amps. The cost is negligible (we just paid
        // for the full simulation).
        // Phase B: route through ruqu-core (the real upstream lib),
        // not the v0.0.x stub. ruqu_core_engine::simulate() compiles
        // the wire `Circuit` to ruqu-core's `QuantumCircuit` and runs
        // it through `Simulator::run`. Witness chain unchanged because
        // it's anchored on the wire-side `Circuit` shape, not the
        // engine that produced the state vector.
        let sv = crate::ruqu_core_engine::simulate(&circuit);
        let dim = sv.len();
        let head_n = dim.min(32);
        let amplitudes_head: Vec<[f64; 2]> = sv.iter().take(head_n).map(|c| [c.re, c.im]).collect();

        let resp = SimulateResponse {
            witness: witness.clone(),
            dim,
            amplitudes_head,
            truncated: head_n < dim,
        };

        self.audit.emit(AuditEntry {
            ts: now_ts(),
            transport: "stdio".into(),
            principal: "stdio:local".into(),
            session: None,
            request_id: None,
            tool: "ruqu_simulate".into(),
            intent: Some(format!("simulate:{id_for_audit}")),
            outcome: "ok".into(),
            result_size: Some(head_n as u32),
            trust_level: Some("verified".into()),
            duration_ms: start.elapsed().as_secs_f64() * 1000.0,
            witness_in: None,
            witness_out: Some(witness),
            code: None,
            policy_decision: Some(PolicyDecision {
                capability_required: "publish".into(),
                capability_granted: vec!["publish".into()],
            }),
        });

        Ok(Json(resp))
    }

    #[tool(
        name = "ruqu_verify",
        description = "Read: replay an operator-supplied circuit through the StateVector backend \
                       and check the resulting bundle witness is byte-equal to the supplied one. \
                       Returns `{ matches: bool, computed_witness: hex }`. \
                       \
                       Same R-1 / R-2 / R-3 validation rules as `ruqu_simulate`. The supplied \
                       `circuit` MUST exactly match the original (same id, same gates, same seed, \
                       same backend_tag) — even a one-byte difference in `circuit.id` rotates the \
                       witness. This is the trust-chain primitive; do NOT use it as a search."
    )]
    pub async fn ruqu_verify(
        &self,
        Parameters(req): Parameters<VerifyRequest>,
    ) -> Result<Json<VerifyResponse>, McpError> {
        let start = std::time::Instant::now();
        let id_for_audit = req.circuit.id.clone();

        if let Err(reason) = validate_circuit_id(&req.circuit.id) {
            self.emit_refusal(
                "ruqu_verify",
                "read",
                "RUQU_INVALID_CIRCUIT_ID",
                &id_for_audit,
                start.elapsed().as_secs_f64() * 1000.0,
            );
            return Err(McpError::invalid_params(
                format!("RUQU_INVALID_CIRCUIT_ID: {reason}"),
                None,
            ));
        }
        if let Err(e) = enforce_qubit_cap(req.circuit.n_qubits) {
            self.emit_refusal(
                "ruqu_verify",
                "read",
                "RUQU_QUBIT_CAP_EXCEEDED",
                &id_for_audit,
                start.elapsed().as_secs_f64() * 1000.0,
            );
            return Err(McpError::invalid_params(
                format!("RUQU_QUBIT_CAP_EXCEEDED: {e}"),
                None,
            ));
        }

        // Compute fresh witness — does NOT mutate the backend's
        // execution map (verify is a pure read).
        let circuit: Circuit = req.circuit.into();
        let computed = derive_witness(&circuit, &req.backend_tag, req.seed);
        let matches = computed == req.witness;

        self.audit.emit(AuditEntry {
            ts: now_ts(),
            transport: "stdio".into(),
            principal: "stdio:local".into(),
            session: None,
            request_id: None,
            tool: "ruqu_verify".into(),
            intent: Some(format!("verify:{id_for_audit}")),
            outcome: if matches {
                "ok".into()
            } else {
                "refused".into()
            },
            result_size: Some(1),
            trust_level: Some(if matches {
                "verified".into()
            } else {
                "rejected".into()
            }),
            duration_ms: start.elapsed().as_secs_f64() * 1000.0,
            witness_in: Some(req.witness.clone()),
            witness_out: Some(computed.clone()),
            code: if matches {
                None
            } else {
                Some("RUQU_WITNESS_MISMATCH".into())
            },
            policy_decision: Some(PolicyDecision {
                capability_required: "read".into(),
                capability_granted: vec!["read".into()],
            }),
        });

        Ok(Json(VerifyResponse {
            matches,
            computed_witness: computed,
        }))
    }

    #[tool(
        name = "ruqu_replay",
        description = "Read: same as `ruqu_verify` but skips the simulation entirely if ruLake's \
                       cache already holds an entry whose witness matches the supplied one. The \
                       cache probe uses `RuLake::current_bundle` to read the backend's \
                       authoritative bundle without a `pull_vectors` round-trip. Falls back to \
                       live witness derivation when no lake handle is configured (v0.0.1 default \
                       in the stdio path) or when the cache misses. \
                       \
                       Audit code `RUQU_REPLAY_HIT` flags the fast path so cost-rollups can \
                       distinguish cached replays from fresh re-derivations (ADR-008 §6 \
                       Decision 5 audit-discipline)."
    )]
    pub async fn ruqu_replay(
        &self,
        Parameters(req): Parameters<VerifyRequest>,
    ) -> Result<Json<ReplayResponse>, McpError> {
        let start = std::time::Instant::now();
        let id_for_audit = req.circuit.id.clone();

        if let Err(reason) = validate_circuit_id(&req.circuit.id) {
            self.emit_refusal(
                "ruqu_replay",
                "read",
                "RUQU_INVALID_CIRCUIT_ID",
                &id_for_audit,
                start.elapsed().as_secs_f64() * 1000.0,
            );
            return Err(McpError::invalid_params(
                format!("RUQU_INVALID_CIRCUIT_ID: {reason}"),
                None,
            ));
        }
        if let Err(e) = enforce_qubit_cap(req.circuit.n_qubits) {
            self.emit_refusal(
                "ruqu_replay",
                "read",
                "RUQU_QUBIT_CAP_EXCEEDED",
                &id_for_audit,
                start.elapsed().as_secs_f64() * 1000.0,
            );
            return Err(McpError::invalid_params(
                format!("RUQU_QUBIT_CAP_EXCEEDED: {e}"),
                None,
            ));
        }

        // ── cache fast-path: probe the lake for an entry keyed at
        //    (backend_id, circuit.id). If the bundle witness matches
        //    we skip the simulation entirely. Per the brief: "uses
        //    ruLake's cache directly via `lake.search_one`" — we
        //    use `current_bundle` because it's the cheapest cache-
        //    pointer probe (no full pull_vectors round-trip), which
        //    is what `search_one`'s primer relies on internally.
        let circuit: Circuit = req.circuit.into();
        let mut cache_hit = false;
        if let Some(lake) = &self.lake {
            let key = (self.backend.id().to_string(), circuit.id.clone());
            if let Ok(bundle) = lake.current_bundle(&key) {
                if bundle.rvf_witness == req.witness {
                    cache_hit = true;
                }
            }
        }

        let computed = if cache_hit {
            req.witness.clone()
        } else {
            derive_witness(&circuit, &req.backend_tag, req.seed)
        };
        let matches = computed == req.witness;

        self.audit.emit(AuditEntry {
            ts: now_ts(),
            transport: "stdio".into(),
            principal: "stdio:local".into(),
            session: None,
            request_id: None,
            tool: "ruqu_replay".into(),
            intent: Some(format!("replay:{id_for_audit}")),
            outcome: if matches {
                "ok".into()
            } else {
                "refused".into()
            },
            result_size: Some(1),
            trust_level: Some(if matches {
                "verified".into()
            } else {
                "rejected".into()
            }),
            duration_ms: start.elapsed().as_secs_f64() * 1000.0,
            witness_in: Some(req.witness.clone()),
            witness_out: Some(computed.clone()),
            code: if cache_hit {
                Some("RUQU_REPLAY_HIT".into())
            } else if !matches {
                Some("RUQU_WITNESS_MISMATCH".into())
            } else {
                None
            },
            policy_decision: Some(PolicyDecision {
                capability_required: "read".into(),
                capability_granted: vec!["read".into()],
            }),
        });

        Ok(Json(ReplayResponse {
            matches,
            computed_witness: computed,
            cache_hit,
        }))
    }

    #[tool(
        name = "ruqu_optimize",
        description = "Read (v0.0.1 STUB): planner placeholder. Full implementation lands in \
                       v0.1 per ADR-008 §3 — the v1 cost-model planner from \
                       `vendor/ruvector/crates/ruqu-core/src/planner.rs:213` will be wrapped here \
                       with cache-aware dispatch. v0.0.1 returns `{ status: \"stub\", v01_planned: \
                       true }` and emits audit code `RUQU_OPTIMIZE_STUB` so client callers can \
                       detect the stub without parsing the response body."
    )]
    pub async fn ruqu_optimize(
        &self,
        Parameters(req): Parameters<StubRequest>,
    ) -> Result<Json<StubResponse>, McpError> {
        let start = std::time::Instant::now();
        let id_for_audit = req.circuit.id.clone();
        // R-1 still applies even for a stub — we don't want
        // pathological ids polluting the audit stream.
        if let Err(reason) = validate_circuit_id(&req.circuit.id) {
            self.emit_refusal(
                "ruqu_optimize",
                "read",
                "RUQU_INVALID_CIRCUIT_ID",
                &id_for_audit,
                start.elapsed().as_secs_f64() * 1000.0,
            );
            return Err(McpError::invalid_params(
                format!("RUQU_INVALID_CIRCUIT_ID: {reason}"),
                None,
            ));
        }

        // Phase B: real ruqu-core optimizer + analyzer pass.
        // Translate the wire circuit, run fuse_gates, then summarise.
        let circuit: ruvector_rulake_ruqu::circuit::Circuit = req.circuit.into();
        let qc = crate::ruqu_core_engine::compile(&circuit);
        let gate_count_before = qc.gates().len();
        let optimized = ruqu_core::optimizer::fuse_gates(&qc);
        let gate_count_after = optimized.gates().len();
        let is_clifford = ruqu_core::circuit_analyzer::is_clifford_circuit(&qc);
        let non_clifford_count = ruqu_core::circuit_analyzer::count_non_clifford(&qc);

        let resp = StubResponse {
            status: "live".into(),
            v01_planned: false,
            v02_planned: false,
            note: format!(
                "ruqu-core::optimizer::fuse_gates ran — {} gates -> {} gates (clifford={}, T-gate-count={})",
                gate_count_before, gate_count_after, is_clifford, non_clifford_count
            ),
            gate_count_before: Some(gate_count_before),
            gate_count_after: Some(gate_count_after),
            is_clifford: Some(is_clifford),
            non_clifford_count: Some(non_clifford_count),
            code_distance: None,
            total_cycles: None,
            logical_error_rate: None,
            logical_errors: None,
        };

        self.audit.emit(AuditEntry {
            ts: now_ts(),
            transport: "stdio".into(),
            principal: "stdio:local".into(),
            session: None,
            request_id: None,
            tool: "ruqu_optimize".into(),
            intent: Some(format!("optimize:{id_for_audit}")),
            outcome: "ok".into(),
            result_size: Some(gate_count_after as u32),
            trust_level: Some("verified".into()),
            duration_ms: start.elapsed().as_secs_f64() * 1000.0,
            witness_in: None,
            witness_out: None,
            code: Some("RUQU_OPTIMIZE_OK".into()),
            policy_decision: Some(PolicyDecision {
                capability_required: "read".into(),
                capability_granted: vec!["read".into()],
            }),
        });

        Ok(Json(resp))
    }

    #[tool(
        name = "ruqu_qec_schedule",
        description = "Read (v0.0.1 STUB): QEC scheduler placeholder. Full implementation lands in \
                       v0.2 per ADR-008 §4 — the QEC control plane from ruqu-core's \
                       `decoder` + `mitigation` modules will be wrapped here. v0.0.1 returns \
                       `{ status: \"stub\", v02_planned: true }` and emits audit code \
                       `RUQU_QEC_STUB`."
    )]
    pub async fn ruqu_qec_schedule(
        &self,
        Parameters(req): Parameters<StubRequest>,
    ) -> Result<Json<StubResponse>, McpError> {
        let start = std::time::Instant::now();
        let id_for_audit = req.circuit.id.clone();
        if let Err(reason) = validate_circuit_id(&req.circuit.id) {
            self.emit_refusal(
                "ruqu_qec_schedule",
                "read",
                "RUQU_INVALID_CIRCUIT_ID",
                &id_for_audit,
                start.elapsed().as_secs_f64() * 1000.0,
            );
            return Err(McpError::invalid_params(
                format!("RUQU_INVALID_CIRCUIT_ID: {reason}"),
                None,
            ));
        }

        // Phase B: real ruqu-algorithms distance-3 rotated surface code.
        // Use sensible defaults — 100 cycles at 1e-3 physical error rate,
        // seed pinned at 0xCAFEBABE for reproducible witness chain.
        let cfg = ruqu_algorithms::surface_code::SurfaceCodeConfig {
            distance: 3,
            num_cycles: 100,
            noise_rate: 1e-3,
            seed: Some(0xCAFEBABE),
        };
        let result = ruqu_algorithms::surface_code::run_surface_code(&cfg).map_err(|e| {
            self.emit_refusal(
                "ruqu_qec_schedule",
                "read",
                "RUQU_QEC_INTERNAL",
                &id_for_audit,
                start.elapsed().as_secs_f64() * 1000.0,
            );
            McpError::internal_error(format!("RUQU_QEC_INTERNAL: {e}"), None)
        })?;

        let resp = StubResponse {
            status: "live".into(),
            v01_planned: false,
            v02_planned: false,
            note: format!(
                "ruqu-algorithms::surface_code::run_surface_code ran d={} cycles={} noise={} -> logical_errors={} rate={:.6}",
                cfg.distance, result.total_cycles, cfg.noise_rate,
                result.logical_errors, result.logical_error_rate
            ),
            gate_count_before: None,
            gate_count_after: None,
            is_clifford: None,
            non_clifford_count: None,
            code_distance: Some(cfg.distance),
            total_cycles: Some(result.total_cycles),
            logical_error_rate: Some(result.logical_error_rate),
            logical_errors: Some(result.logical_errors),
        };

        self.audit.emit(AuditEntry {
            ts: now_ts(),
            transport: "stdio".into(),
            principal: "stdio:local".into(),
            session: None,
            request_id: None,
            tool: "ruqu_qec_schedule".into(),
            intent: Some(format!("qec:{id_for_audit}")),
            outcome: "ok".into(),
            result_size: Some(result.logical_errors),
            trust_level: Some("verified".into()),
            duration_ms: start.elapsed().as_secs_f64() * 1000.0,
            witness_in: None,
            witness_out: None,
            code: Some("RUQU_QEC_OK".into()),
            policy_decision: Some(PolicyDecision {
                capability_required: "read".into(),
                capability_granted: vec!["read".into()],
            }),
        });

        Ok(Json(resp))
    }
}

// ─── Internal helpers ────────────────────────────────────────────────

impl RuquMcpServer {
    /// Emit a fully-shaped audit row for a refusal path. Mirrors the
    /// commit 56b497b discipline: every tool — even on the failure
    /// path — emits an `AuditEntry` with `policy_decision` populated.
    fn emit_refusal(
        &self,
        tool: &'static str,
        capability_required: &'static str,
        code: &'static str,
        circuit_id: &str,
        duration_ms: f64,
    ) {
        self.audit.emit(AuditEntry {
            ts: now_ts(),
            transport: "stdio".into(),
            principal: "stdio:local".into(),
            session: None,
            request_id: None,
            tool: tool.into(),
            intent: Some(format!("refuse:{circuit_id}")),
            outcome: "refused".into(),
            result_size: None,
            trust_level: Some("rejected".into()),
            duration_ms,
            witness_in: None,
            witness_out: None,
            code: Some(code.into()),
            policy_decision: Some(PolicyDecision {
                capability_required: capability_required.into(),
                capability_granted: vec![capability_required.into()],
            }),
        });
    }
}

/// R-1 validator: empty / overlong `circuit.id` is rejected so the
/// witness anchor stays content-derived. The brief: "Hard-refuse with
/// audit code `RUQU_INVALID_CIRCUIT_ID`."
fn validate_circuit_id(id: &str) -> Result<(), &'static str> {
    if id.is_empty() {
        return Err("circuit.id is empty");
    }
    if id.len() > MAX_CIRCUIT_ID_LEN {
        return Err("circuit.id exceeds 256 bytes");
    }
    Ok(())
}

/// Derive the witness for `(circuit, backend_tag, seed)` without
/// mutating the backend's execution map. Used by `ruqu_verify` and
/// the cache-miss path of `ruqu_replay`.
///
/// The simulation IS run (state vector materialised) — that's the
/// "operator-supplied" replay the brief specifies. The witness
/// itself comes from `ruqu_bundle`, which doesn't depend on the
/// state-vector contents (it's keyed by `data_ref` + `dim` + seed +
/// rerank + generation). v0.0.2's content-derived `data_ref`
/// (closing R-3) will tie the simulation result into the witness;
/// for now the simulation is what makes the dim / generation
/// match the original.
fn derive_witness(circuit: &Circuit, backend_tag: &str, seed: u64) -> String {
    // Materialise the state vector so any future witness-binding
    // (R-3 closure in v0.0.2) has the data on hand. v0.0.1's
    // witness only depends on the inputs — the simulation runs
    // for parity with `RuquStateVectorBackend::execute` so timings
    // and audit `duration_ms` are honest.
    // Phase B: route through ruqu-core (the real upstream lib),
    // matching the swap done in ruqu_simulate at server.rs:472.
    let _sv = crate::ruqu_core_engine::simulate(circuit);
    let inputs = inputs_for(circuit, backend_tag, seed, /* generation = */ 1);
    let bundle = ruqu_bundle(&inputs.as_borrowed());
    bundle.rvf_witness
}

// Suppress the unused-import warning for `Gate` in release builds —
// the type is part of the public mini-IR surface and showcases the
// fact that v0.0.1 accepts the full `Circuit` JSON form.
#[allow(dead_code)]
fn _gate_surface_witness(_g: &Gate) {}

// ─── ServerHandler — emitted from #[tool_handler] ────────────────────

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RuquMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = Implementation::new("ruvector-rulake-mcp-ruqu", env!("CARGO_PKG_VERSION"));
        info.title = Some("ruQu MCP companion server".into());
        info.website_url = Some("https://github.com/ruvnet/RuLake".into());

        let mut init = ServerInfo::new(ServerCapabilities::builder().enable_tools().build());
        init.server_info = info;
        init.instructions = Some(
            "ruQu MCP companion server (ADR-008 §6 v0.0.1). Five tools: \
             ruqu_simulate (publish), ruqu_verify, ruqu_replay, ruqu_optimize \
             (stub — v0.1), ruqu_qec_schedule (stub — v0.2). All audit codes \
             use the RUQU_ prefix; circuit.id MUST be content-derived \
             (≤256 bytes); n_qubits ≤16 in v0.0."
                .into(),
        );
        init
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bell pair as a [`WireCircuit`] — the shape JSON callers send.
    fn bell() -> WireCircuit {
        WireCircuit {
            n_qubits: 2,
            gates: vec![
                WireGate::H { q: 0 },
                WireGate::Cx {
                    control: 0,
                    target: 1,
                },
            ],
            id: "bell-pair-v0".to_string(),
        }
    }

    // ── Per-tool unit tests (5) ──────────────────────────────────

    #[tokio::test]
    async fn ruqu_simulate_returns_witness_and_caps_amplitudes() {
        let server = RuquMcpServer::new("ruqu-sv");
        let resp = server
            .call_simulate(SimulateRequest {
                circuit: bell(),
                backend_tag: "sv".into(),
                seed: 0,
            })
            .await
            .expect("ok");
        assert!(!resp.witness.is_empty(), "witness must be present");
        assert_eq!(resp.dim, 4, "Bell pair dim = 1<<2 = 4");
        assert_eq!(resp.amplitudes_head.len(), 4, "n=2 fits in 32-amp cap");
        assert!(!resp.truncated, "n=2 fits without truncation");
    }

    #[tokio::test]
    async fn ruqu_simulate_caps_at_32_for_large_n() {
        // n=10 -> dim=1024, head capped at 32.
        let server = RuquMcpServer::new("ruqu-sv");
        let c = WireCircuit {
            n_qubits: 10,
            gates: (0..10).map(|q| WireGate::H { q }).collect(),
            id: "wide-h-stack".to_string(),
        };
        let resp = server
            .call_simulate(SimulateRequest {
                circuit: c,
                backend_tag: "sv".into(),
                seed: 0,
            })
            .await
            .expect("ok");
        assert_eq!(resp.dim, 1024);
        assert_eq!(resp.amplitudes_head.len(), 32, "head cap = 32");
        assert!(resp.truncated);
    }

    #[tokio::test]
    async fn ruqu_verify_round_trips_with_simulate() {
        let server = RuquMcpServer::new("ruqu-sv");
        let circuit = bell();
        let s = server
            .call_simulate(SimulateRequest {
                circuit: circuit.clone(),
                backend_tag: "sv".into(),
                seed: 0,
            })
            .await
            .unwrap();
        let v = server
            .call_verify(VerifyRequest {
                circuit,
                witness: s.witness.clone(),
                backend_tag: "sv".into(),
                seed: 0,
            })
            .await
            .unwrap();
        assert!(v.matches, "round-trip witness must match");
        assert_eq!(v.computed_witness, s.witness);
    }

    #[tokio::test]
    async fn ruqu_replay_falls_back_to_live_when_no_lake() {
        let server = RuquMcpServer::new("ruqu-sv");
        let circuit = bell();
        let s = server
            .call_simulate(SimulateRequest {
                circuit: circuit.clone(),
                backend_tag: "sv".into(),
                seed: 0,
            })
            .await
            .unwrap();
        let r = server
            .call_replay(VerifyRequest {
                circuit,
                witness: s.witness.clone(),
                backend_tag: "sv".into(),
                seed: 0,
            })
            .await
            .unwrap();
        assert!(r.matches);
        assert!(!r.cache_hit, "no lake configured → never a cache hit");
    }

    #[tokio::test]
    async fn ruqu_optimize_returns_real_fuse_gates_envelope() {
        // Phase B (commit 56f130a): ruqu_optimize now runs real
        // ruqu_core::optimizer::fuse_gates + circuit_analyzer.
        // Bell pair (H + CX) has no consecutive same-qubit gates to fuse,
        // is fully Clifford, has 0 T-gates.
        let server = RuquMcpServer::new("ruqu-sv");
        let resp = server
            .call_optimize(StubRequest { circuit: bell() })
            .await
            .unwrap();
        assert_eq!(resp.status, "live");
        assert!(!resp.v01_planned);
        assert!(!resp.v02_planned);
        assert!(resp.note.contains("ruqu-core::optimizer::fuse_gates"));
        assert_eq!(resp.gate_count_before, Some(2));
        assert_eq!(resp.gate_count_after, Some(2));
        assert_eq!(resp.is_clifford, Some(true));
        assert_eq!(resp.non_clifford_count, Some(0));

        let tail = server.audit().tail(1);
        assert_eq!(tail.last().unwrap()["code"], "RUQU_OPTIMIZE_OK");
        assert_eq!(tail.last().unwrap()["tool"], "ruqu_optimize");
    }

    #[tokio::test]
    async fn ruqu_qec_schedule_runs_real_surface_code() {
        // Phase B (commit 681dfbb): ruqu_qec_schedule now runs real
        // ruqu_algorithms::surface_code::run_surface_code (d=3 rotated).
        let server = RuquMcpServer::new("ruqu-sv");
        let resp = server
            .call_qec_schedule(StubRequest { circuit: bell() })
            .await
            .unwrap();
        assert_eq!(resp.status, "live");
        assert!(!resp.v01_planned);
        assert!(!resp.v02_planned);
        assert!(resp
            .note
            .contains("ruqu-algorithms::surface_code::run_surface_code"));
        assert_eq!(resp.code_distance, Some(3));
        assert_eq!(resp.total_cycles, Some(100));
        // logical_error_rate is empirical — bounded by [0, 1] but content varies.
        let r = resp.logical_error_rate.unwrap();
        assert!((0.0..=1.0).contains(&r), "rate in unit interval, got {r}");
        let tail = server.audit().tail(1);
        assert_eq!(tail.last().unwrap()["code"], "RUQU_QEC_OK");
    }

    // ── R-1 / R-2 security guards ────────────────────────────────

    #[tokio::test]
    async fn r1_refuses_empty_circuit_id() {
        let server = RuquMcpServer::new("ruqu-sv");
        let mut c = bell();
        c.id = String::new();
        let err = server
            .call_simulate(SimulateRequest {
                circuit: c,
                backend_tag: "sv".into(),
                seed: 0,
            })
            .await
            .expect_err("must refuse");
        assert!(err.message.contains("RUQU_INVALID_CIRCUIT_ID"));
        let tail = server.audit().tail(1);
        assert_eq!(tail.last().unwrap()["code"], "RUQU_INVALID_CIRCUIT_ID");
        assert_eq!(tail.last().unwrap()["outcome"], "refused");
    }

    #[tokio::test]
    async fn r1_refuses_overlong_circuit_id() {
        let server = RuquMcpServer::new("ruqu-sv");
        let mut c = bell();
        c.id = "x".repeat(MAX_CIRCUIT_ID_LEN + 1);
        let err = server
            .call_simulate(SimulateRequest {
                circuit: c,
                backend_tag: "sv".into(),
                seed: 0,
            })
            .await
            .expect_err("must refuse");
        assert!(err.message.contains("RUQU_INVALID_CIRCUIT_ID"));
    }

    #[tokio::test]
    async fn r2_refuses_above_qubit_cap() {
        let server = RuquMcpServer::new("ruqu-sv");
        let c = WireCircuit {
            n_qubits: 20,
            gates: vec![],
            id: "oversized".to_string(),
        };
        let err = server
            .call_simulate(SimulateRequest {
                circuit: c,
                backend_tag: "sv".into(),
                seed: 0,
            })
            .await
            .expect_err("must refuse n=20 (cap=16)");
        assert!(err.message.contains("RUQU_QUBIT_CAP_EXCEEDED"));
        let tail = server.audit().tail(1);
        assert_eq!(tail.last().unwrap()["code"], "RUQU_QUBIT_CAP_EXCEEDED");
    }
}
