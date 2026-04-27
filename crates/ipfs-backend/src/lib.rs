//! ruLake IPFS backend (ADR-005).
//!
//! Stores `table.rulake.json` bundles on IPFS, addressing them by
//! CIDv1 (base32). The cache-key witness is still SHAKE-256(32) hex
//! anchored on the bundle fields — the CID is a network-level
//! address that lands in `Generation::Opaque` (per ADR-005 §3:
//! "two anchors, one cache key").
//!
//! # What v0.1 ships
//!
//! - Three operating modes selected at construction:
//!   - `Mode::Kubo { api_url }` — talks to a kubo HTTP RPC at
//!     `http://127.0.0.1:5001` (typical), supports both pin+publish
//!     and read.
//!   - `Mode::Gateway { gateway_url }` — read-only, fetches blocks
//!     through a public IPFS gateway (ipfs.io / dweb.link / w3s.link).
//!     CI / sandbox / no-network-egress shape.
//!   - `Mode::KuboWithGatewayFallback { ... }` — kubo first, fall
//!     back to gateway on read miss. Audit-loud per ADR-005 §2.
//! - **Cheap `current_bundle()` override** — single-block `cat` on
//!   the CID; bundles are ≤ 64 KiB by `RuLakeBundle::from_json`'s
//!   cap (`src/bundle.rs:218`), so this is one round trip with
//!   bounded payload. Honours the ADR-004 §Resources contract.
//! - **Witness-fail-closed** — `read_from_dir` runs through
//!   `RuLakeBundle::read_from_dir`'s witness check at
//!   `src/bundle.rs:349`. A maliciously-pinned bundle whose body
//!   doesn't hash to its claimed witness is rejected automatically.
//!
//! # Auth
//!
//! Bearer-token via kubo's native `API.Authorizations` `HTTPAuthSecrets`
//! support is operator-side config (set in kubo's daemon), surfaced
//! into the client via `Mode::Kubo { api_token, .. }`.
//!
//! # Out of scope for v0.1 (tracked in ADR-005)
//!
//! - Pinning-service mode (Storacha/Pinata/Filebase) — v0.2
//! - AES-256-GCM envelope for private bundles — v0.2 (operator can
//!   today encrypt the bundle JSON themselves before publish)
//! - DAG-walked retrieval for bundles > 64 KiB — never (the existing
//!   cap means bundles always fit in a single block)

pub mod backend;

pub use backend::{
    GatewayUrl, IpfsBackend, IpfsCollection, IpfsCollectionConfig, KuboApiUrl, Mode,
};
