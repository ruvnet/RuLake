//! ruLake MCP server library (ADR-004 v0.1).
//!
//! v0.1 ships:
//! - `rulake_query` (intent: "search") — the public decision-layer tool
//! - `rulake_list_backends` — internal kernel probe
//! - bounded worker pool (rayon + flume) so RuLake CPU work is isolated
//!   from the tokio reactor that owns the wire
//! - JSONL audit log to stderr with the decision-trace shape from
//!   ADR-004 §7
//! - witness-fail-closed propagated from `RuLakeBundle::read_from_dir`
//!
//! Out of scope for v0.1 (tracked in ADR-004 §Open questions):
//! - Streamable HTTP transport, OAuth/mTLS, replay protection (v0.2)
//! - intents `verify` / `explain` / `refresh` (v0.2)
//! - mutation tools (publish / admin capability tiers) (v0.3)
//! - resources (`rulake://stats`, `rulake://bundle/...`) (v0.2)
//! - layered rate limiting (v0.2)

pub mod audit;
pub mod auth;
pub mod config;
pub mod http;
pub mod planner;
pub mod policy;
pub mod server;
pub mod workers;

pub use audit::{AuditEntry, AuditSink};
pub use auth::BearerAuth;
pub use config::McpConfig;
pub use http::{AllowBearerOnPublic, AuthMode, InsecureAllowNoAuth};
pub use policy::{Capability, CapabilitySet};
pub use server::RuLakeMcpServer;
pub use workers::WorkerPool;
