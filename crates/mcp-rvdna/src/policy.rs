//! Capability tiers + per-call policy gate (ADR-007 §D4).
//!
//! Mirrors `mcp-server::policy` but with rvdna scope vocabulary:
//! - `read`     — exposes `rvdna_find`, `rvdna_call_variants`,
//!   `rvdna_translate`, `rvdna_score`
//! - `internal` — adds `rvdna_lineage` (operator/SRE/Console-trust)
//! - `clinical` — reserved for `rvdna_call_variants` PHI flow (v0.1)
//! - `admin`    — reserved (v0.1+); no admin tools in v0.0.1
//!
//! v0.0.1 enforces capability membership at tool-call time. JWT
//! scope→cap mapping (`mcp:rvdna:read|internal|clinical|admin`) lands
//! in v0.0.2 alongside the JwtAuth port from mcp-server.

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Read,
    Internal,
    Clinical,
    Admin,
}

impl Capability {
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Internal => "internal",
            Self::Clinical => "clinical",
            Self::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "read" => Ok(Self::Read),
            "internal" => Ok(Self::Internal),
            "clinical" => Ok(Self::Clinical),
            "admin" => Ok(Self::Admin),
            other => anyhow::bail!(
                "unknown capability {other:?} — expected read|internal|clinical|admin"
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CapabilitySet {
    granted: HashSet<Capability>,
}

impl Default for CapabilitySet {
    fn default() -> Self {
        // Default = read-only. The four read-tier rvdna tools are the
        // only ones an agent sees on the wire.
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
        // Read is implicit when any other capability is granted — same
        // policy as mcp-server: no use case for "internal but not read".
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
            "RVDNA_CAPABILITY_REFUSED: tool requires `{}` capability — \
             pass --capabilities {} (or higher) at startup",
            self.required.label(),
            self.required.label(),
        )
    }
}
impl std::error::Error for CapabilityRefused {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_grants_read_only() {
        let c = CapabilitySet::default();
        assert!(c.has(Capability::Read));
        assert!(!c.has(Capability::Internal));
        assert!(!c.has(Capability::Clinical));
        assert!(!c.has(Capability::Admin));
    }

    #[test]
    fn from_csv_implies_read() {
        let c = CapabilitySet::from_csv("internal").unwrap();
        assert!(c.has(Capability::Read));
        assert!(c.has(Capability::Internal));
    }

    #[test]
    fn require_rejects_missing_tier() {
        let c = CapabilitySet::default();
        assert!(c.require(Capability::Internal).is_err());
    }
}
