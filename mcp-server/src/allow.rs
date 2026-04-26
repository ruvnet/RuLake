//! Per-collection allow-list (RBAC) — ADR-004 §5.
//!
//! Compiled from `[[allow]]` blocks in `mcp.toml` at startup. Per
//! tool call, the planner asks: "is this (backend, collection,
//! capability) tuple allowed?" — fast deny-by-default unless a
//! matching block authorises it.
//!
//! Empty allow-list (the v0.3 default) bypasses this layer entirely;
//! only the capability-tier gate from `policy.rs` fires. This keeps
//! existing v0.3 deployments behaviour-compatible — RBAC is opt-in.

use regex::Regex;

use crate::config::AllowBlock;
use crate::policy::Capability;

#[derive(Debug, Clone)]
pub struct AllowList {
    blocks: Vec<CompiledBlock>,
}

#[derive(Debug, Clone)]
struct CompiledBlock {
    backend: String,
    collection_re: Regex,
    caps: Vec<Capability>,
    /// Original pattern, for audit log (`allow_rule_matched`).
    pattern: String,
}

impl AllowList {
    pub fn empty() -> Self {
        Self { blocks: Vec::new() }
    }

    pub fn from_blocks(raw: &[AllowBlock]) -> anyhow::Result<Self> {
        let mut blocks = Vec::with_capacity(raw.len());
        for b in raw {
            // Anchor the regex implicitly so `docs.*` doesn't match
            // `secret-docs.public`. Operators write the pattern they
            // expect to match the whole collection name.
            let anchored = format!("^(?:{})$", b.collection);
            let re = Regex::new(&anchored).map_err(|e| {
                anyhow::anyhow!(
                    "[[allow]] block backend={:?} collection={:?}: bad regex: {}",
                    b.backend, b.collection, e
                )
            })?;
            let caps: Result<Vec<Capability>, _> =
                b.caps.iter().map(|s| Capability::parse(s)).collect();
            blocks.push(CompiledBlock {
                backend: b.backend.clone(),
                collection_re: re,
                caps: caps?,
                pattern: format!("{}/{}", b.backend, b.collection),
            });
        }
        Ok(Self { blocks })
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Returns the matching rule's pattern when the tuple is allowed,
    /// or `Err(AllowDenied { ... })` with the reason. `Read` is
    /// implicitly granted everywhere a block matches, so a caller
    /// asking for `Read` succeeds if any block matches the
    /// (backend, collection) pair regardless of its `caps` list.
    pub fn check(
        &self,
        backend: &str,
        collection: &str,
        cap: Capability,
    ) -> Result<MatchedRule, AllowDenied> {
        if self.blocks.is_empty() {
            // Empty list = unrestricted (v0.3 backwards-compat).
            return Ok(MatchedRule {
                pattern: "<unrestricted>".into(),
            });
        }
        let mut backend_match_seen = false;
        for b in &self.blocks {
            if b.backend != backend {
                continue;
            }
            if !b.collection_re.is_match(collection) {
                continue;
            }
            backend_match_seen = true;
            // Read is implicitly granted by any matching block.
            if cap == Capability::Read || b.caps.contains(&cap) {
                return Ok(MatchedRule {
                    pattern: b.pattern.clone(),
                });
            }
        }
        Err(AllowDenied {
            backend: backend.to_string(),
            collection: collection.to_string(),
            cap,
            reason: if backend_match_seen {
                AllowDeniedReason::CollectionAllowedButCapNotGranted
            } else {
                AllowDeniedReason::NoMatchingRule
            },
        })
    }
}

#[derive(Debug, Clone)]
pub struct MatchedRule {
    pub pattern: String,
}

#[derive(Debug, Clone)]
pub struct AllowDenied {
    pub backend: String,
    pub collection: String,
    pub cap: Capability,
    pub reason: AllowDeniedReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowDeniedReason {
    /// No `[[allow]]` block named this (backend, collection).
    NoMatchingRule,
    /// A block matched but didn't grant the requested capability.
    CollectionAllowedButCapNotGranted,
}

impl std::fmt::Display for AllowDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.reason {
            AllowDeniedReason::NoMatchingRule => write!(
                f,
                "RULAKE_ALLOWLIST_DENIED: no [[allow]] block matches {}/{}",
                self.backend, self.collection
            ),
            AllowDeniedReason::CollectionAllowedButCapNotGranted => write!(
                f,
                "RULAKE_ALLOWLIST_DENIED: {}/{} matched but cap `{}` not granted",
                self.backend, self.collection, self.cap.label()
            ),
        }
    }
}
impl std::error::Error for AllowDenied {}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(backend: &str, collection: &str, caps: &[&str]) -> AllowBlock {
        AllowBlock {
            backend: backend.into(),
            collection: collection.into(),
            caps: caps.iter().map(|s| (*s).into()).collect(),
        }
    }

    #[test]
    fn empty_list_grants_everything() {
        let al = AllowList::empty();
        assert!(al.check("any", "any", Capability::Admin).is_ok());
    }

    #[test]
    fn single_block_grants_read_implicitly() {
        let al = AllowList::from_blocks(&[block("be", "docs", &["read"])]).unwrap();
        assert!(al.check("be", "docs", Capability::Read).is_ok());
        assert!(al.check("be", "docs", Capability::Publish).is_err());
    }

    #[test]
    fn regex_matches_anchored_to_full_collection() {
        let al = AllowList::from_blocks(&[block("be", "docs.*", &["read"])]).unwrap();
        assert!(al.check("be", "docs", Capability::Read).is_ok());
        assert!(al.check("be", "docs.public", Capability::Read).is_ok());
        // Anchored to the whole string — not a substring match.
        let denied = al.check("be", "secret-docs", Capability::Read);
        assert!(denied.is_err(), "anchored regex should not match prefix");
    }

    #[test]
    fn publish_cap_grants_read() {
        let al = AllowList::from_blocks(&[block("be", "x", &["read", "publish"])]).unwrap();
        assert!(al.check("be", "x", Capability::Read).is_ok());
        assert!(al.check("be", "x", Capability::Publish).is_ok());
        assert!(al.check("be", "x", Capability::Admin).is_err());
    }

    #[test]
    fn unknown_backend_denied_with_specific_reason() {
        let al = AllowList::from_blocks(&[block("be", ".*", &["read"])]).unwrap();
        let err = al.check("nope", "x", Capability::Read).unwrap_err();
        assert_eq!(err.reason, AllowDeniedReason::NoMatchingRule);
    }
}
