//! `mcp-rvdna` — MCP companion server for the rvDNA v2 substrate
//! (ADR-007 §6).
//!
//! v0.0.1 ships:
//! - `rvdna_find` — k-NN search over an `RvdnaT0Backend` collection
//!   (Read tier). v0.0.1 returns a witnessed stub; the kNN inner loop
//!   lands in v0.1 alongside a real RaBitQ index over the hot tier.
//! - `rvdna_call_variants` — variants for a region (Read tier). Stub +
//!   live witness in v0.0.1.
//! - `rvdna_translate` — DNA → protein (Read tier). Stub + live witness.
//! - `rvdna_score` — model-derived score for a region (Read tier). Stub
//!   + live witness.
//! - `rvdna_lineage` — bundle witness + provenance for the registered
//!   collection (Internal tier). End-to-end live in v0.0.1; this is
//!   the trust-anchor demo per ADR-007 §6 acceptance criteria.
//!
//! ## Witness discipline (R-1 mitigation)
//!
//! The rvdna-backend witness is derived from inputs (data_ref, dim,
//! rotation_seed, rerank_factor, generation) — NOT the vector content.
//! See `docs/research/security/v0.0-substrates.md` R-1: a pinned
//! lineage record can disagree with the backend's *current* witness
//! after operator-driven mutation (e.g. `put_collection` replacing a
//! collection in-place).
//!
//! Every read tool here re-calls `BackendAdapter::current_bundle()` and
//! compares the result to whatever the operator pinned at register
//! time. Disagreement triggers a `RVDNA_WITNESS_DRIFT` refusal — the
//! tool returns nothing and audit emits the code so federation /
//! Console can flag the deployment as drifted.
//!
//! Mutation surface is **zero** in v0.0.1: there is no put_collection
//! tool over the wire. Operators wire `RvdnaT0Backend`s in at process
//! start (`RvdnaMcpServer::register_collection`); attackers cannot
//! introduce a collection without operator action.

pub mod audit;
pub mod http;
pub mod policy;
pub mod real_compute;
pub mod registry;
pub mod server;

pub use audit::{AuditEntry, AuditSink, PolicyDecision};
pub use http::{AllowBearerOnPublic, AuthMode, BearerAuth, InsecureAllowNoAuth};
pub use policy::{Capability, CapabilitySet};
pub use registry::{PinnedCollection, RvdnaRegistry};
pub use server::RvdnaMcpServer;
