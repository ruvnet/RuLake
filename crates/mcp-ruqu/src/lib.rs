//! `mcp-ruqu` — MCP companion server for ruQu v2 (ADR-008 §6 v0.0.1).
//!
//! Sibling crate of `mcp-server/`; mirrors its `#[tool_router]` shape
//! and its `AuditEntry` schema so an operator's existing audit
//! pipeline (file watch on `audit.jsonl`, ELK / Loki / whatever) sees
//! coherent rows from both servers. Audit codes use the disjoint
//! `RUQU_*` prefix per ADR-008 §6 Decision 4.
//!
//! v0.0.1 ships the five tools the ADR's acceptance criteria pin —
//! see `server.rs` for the table — with the R-1 / R-2 / R-3 security
//! mitigations enforced at the entry of every tool handler.
//!
//! v0.1 adds the Streamable HTTP transport (lifted near-verbatim from
//! `mcp-rvdna::http`, including the iter 32 CORS layer). Tool surface
//! is unchanged.
//!
//! Out of scope for v0.1 (deferred to mcp-ruqu v0.2):
//! - mTLS / JWT / JWKS / replay guard — mcp-server already provides
//!   those building blocks; the v0.1 HTTP `serve()` signature already
//!   carries the `AuthMode` / `AllowBearerOnPublic` /
//!   `InsecureAllowNoAuth` types so the v0.2 lift is mechanical.
//! - The `simulate` / `hardware` / `verify` capability tier additions
//!   from ADR-008 §6 Decision 4 — for now we ride on mcp-server's
//!   existing `Read` / `Publish` tiers so the policy module doesn't
//!   need to fork before mcp-server agrees.
//! - `mcp-server::audit::AuditEntry` unification via a shared
//!   `audit-only` feature on `mcp-server` — v0.1 still ships its own
//!   structurally-identical type so we don't gate this PR on a
//!   change to a sibling crate.

#![deny(unsafe_code)]
// rmcp's `#[tool]` macro strips the doc-comment off the method it
// rewrites, so module-level `warn(missing_docs)` would yield ~5 false
// positives per release. The wire types and free functions still
// document themselves; we just relax the lint.
#![allow(missing_docs)]

pub mod audit;
pub mod http;
pub mod ruqu_core_engine;
pub mod server;

pub use audit::{AuditEntry, AuditSink, PolicyDecision};
pub use http::{AllowBearerOnPublic, AuthMode, BearerAuth, InsecureAllowNoAuth};
pub use server::{
    ReplayResponse, RuquMcpServer, SimulateRequest, SimulateResponse, StubRequest,
    StubResponse, VerifyRequest, VerifyResponse,
};
