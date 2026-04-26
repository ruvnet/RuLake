//! Capability flags + per-call policy gate (ADR-004 §4 + §5).
//!
//! The four tiers from ADR-004 §4b:
//! - `read`     — default; exposes `rulake_query` (and `rulake_list_backends`)
//! - `internal` — operator-only (no OAuth scope); exposes the internal
//!                kernel tools that `rulake_query` composes
//! - `publish`  — adds `rulake_publish_bundle` + `rulake_refresh_from_bundle_dir`
//!                + enables `intent: "refresh"`
//! - `admin`    — adds `rulake_save_cache_to_dir` + `rulake_warm_from_dir` +
//!                `rulake_invalidate_cache`
//!
//! v0.3 enforces capability membership at tool-call time. OAuth-scope
//! mapping (mcp:rulake:read|publish|admin) lands in v0.4 with the
//! OAuth auth mode itself.

use std::collections::HashSet;

tokio::task_local! {
    /// Per-request CapabilitySet — set by the HTTP middleware from
    /// the validated JWT's scope claim before calling into rmcp.
    /// `effective_caps()` reads this first; falls back to the
    /// server-wide set when unset (stdio path; non-tools/call HTTP
    /// requests; legacy auth modes).
    pub static REQUEST_CAPS: CapabilitySet;
}

/// Resolve the effective capability set for the current call.
/// Server-wide caps act as the **upper bound** (operator policy
/// ceiling). Per-request caps from the JWT act as the lower bound
/// (what the IdP granted this token). The intersection is what
/// gates the actual tool call. This is the v0.8 SOTA shape.
pub fn effective_caps(server_wide: &CapabilitySet) -> CapabilitySet {
    match REQUEST_CAPS.try_with(|r| r.clone()) {
        Ok(per_request) => server_wide.intersect(&per_request),
        Err(_) => server_wide.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Read,
    Internal,
    Publish,
    Admin,
}

impl Capability {
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Internal => "internal",
            Self::Publish => "publish",
            Self::Admin => "admin",
        }
    }

    /// Parse a single capability label. Unknown labels error.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "read" => Ok(Self::Read),
            "internal" => Ok(Self::Internal),
            "publish" => Ok(Self::Publish),
            "admin" => Ok(Self::Admin),
            other => anyhow::bail!("unknown capability {other:?} — expected read|internal|publish|admin"),
        }
    }
}

/// The set of capabilities granted to this server instance. Set once
/// at startup from `--capabilities` and never mutated thereafter.
#[derive(Debug, Clone)]
pub struct CapabilitySet {
    granted: HashSet<Capability>,
}

impl Default for CapabilitySet {
    fn default() -> Self {
        // Default = read-only. `rulake_query` is the only tool an
        // agent sees on the wire.
        let mut granted = HashSet::new();
        granted.insert(Capability::Read);
        Self { granted }
    }
}

impl CapabilitySet {
    pub fn from_csv(csv: &str) -> anyhow::Result<Self> {
        let mut granted = HashSet::new();
        for tok in csv.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            granted.insert(Capability::parse(tok)?);
        }
        // Read is implicit when any other capability is granted —
        // there's no use case for "publish but not read".
        if !granted.is_empty() {
            granted.insert(Capability::Read);
        }
        Ok(Self { granted })
    }

    pub fn has(&self, cap: Capability) -> bool {
        self.granted.contains(&cap)
    }

    pub fn require(&self, cap: Capability) -> Result<(), CapabilityRefused> {
        if self.has(cap) {
            Ok(())
        } else {
            Err(CapabilityRefused { required: cap })
        }
    }

    pub fn labels(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = self.granted.iter().map(|c| c.label()).collect();
        v.sort();
        v
    }

    /// Intersect with another set — what BOTH grant. Used by
    /// `effective_caps()` to combine the server-wide ceiling with the
    /// per-request token's grant. Read is always preserved if either
    /// side has it (consistent with `from_csv` which implicitly
    /// grants Read alongside any other capability).
    pub fn intersect(&self, other: &CapabilitySet) -> CapabilitySet {
        let mut granted: HashSet<Capability> = self
            .granted
            .iter()
            .copied()
            .filter(|c| other.granted.contains(c))
            .collect();
        if !granted.is_empty() && (self.has(Capability::Read) || other.has(Capability::Read)) {
            granted.insert(Capability::Read);
        }
        CapabilitySet { granted }
    }
}

#[derive(Debug)]
pub struct CapabilityRefused {
    pub required: Capability,
}

impl std::fmt::Display for CapabilityRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RULAKE_CAPABILITY_REFUSED: tool requires `{}` capability — \
             pass --capabilities {} (or higher) at startup",
            self.required.label(),
            self.required.label(),
        )
    }
}
impl std::error::Error for CapabilityRefused {}
