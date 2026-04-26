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
