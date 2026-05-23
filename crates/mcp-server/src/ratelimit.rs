//! Layered rate limiter (ADR-004 §6).
//!
//! Three buckets per ADR:
//! - `(transport, principal)`        — per-agent fairness
//! - `(principal, backend, collection)` — per-collection isolation
//!   (one hot collection cannot starve a peer for the same principal)
//! - `(transport)`                   — process-wide DoS ceiling
//!
//! The first bucket to fail any layer is the rate-limit failure mode.
//! Per-key bookkeeping lives in a `dashmap::DashMap`; the
//! `governor::Quota` is the per-key state.

use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use governor::{clock::DefaultClock, state::keyed::DefaultKeyedStateStore, Quota, RateLimiter};

/// Configuration for one rate-limit layer. Defaults match ADR-004 §6.
#[derive(Debug, Clone, Copy)]
pub struct LayerConfig {
    pub per_second: u32,
    pub burst: u32,
}

impl LayerConfig {
    fn quota(self) -> Quota {
        Quota::per_second(std::num::NonZeroU32::new(self.per_second).unwrap())
            .allow_burst(std::num::NonZeroU32::new(self.burst).unwrap())
    }
}

#[derive(Clone)]
pub struct LayeredRateLimiter {
    inner: Arc<Inner>,
}

struct Inner {
    /// Layer 1 — `(transport, principal)`.
    per_principal: RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>,
    per_principal_cfg: LayerConfig,
    /// Layer 2 — `(principal, backend, collection)`.
    per_collection: RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>,
    per_collection_cfg: LayerConfig,
    /// Layer 3 — `(transport)` (process-wide).
    per_process: RateLimiter<&'static str, DefaultKeyedStateStore<&'static str>, DefaultClock>,
    per_process_cfg: LayerConfig,
    /// Eviction tracker — when DashMap-backed key counts grow past
    /// 10k we drop the layered limiters and rebuild. Cheaper than
    /// a real LRU and keys age out within a few minutes anyway.
    /// Field initialized but the eviction step itself is still
    /// scaffolded; v0.11 wires the rebuild trigger.
    #[allow(dead_code)]
    last_reset: std::sync::Mutex<Instant>,
    keys: DashMap<String, ()>,
}

impl Default for LayeredRateLimiter {
    fn default() -> Self {
        Self::new(
            LayerConfig {
                per_second: 60,
                burst: 120,
            },
            LayerConfig {
                per_second: 30,
                burst: 60,
            },
            LayerConfig {
                per_second: 600,
                burst: 1200,
            },
        )
    }
}

impl LayeredRateLimiter {
    pub fn new(p: LayerConfig, c: LayerConfig, proc: LayerConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                per_principal: RateLimiter::keyed(p.quota()),
                per_principal_cfg: p,
                per_collection: RateLimiter::keyed(c.quota()),
                per_collection_cfg: c,
                per_process: RateLimiter::keyed(proc.quota()),
                per_process_cfg: proc,
                last_reset: std::sync::Mutex::new(Instant::now()),
                keys: DashMap::new(),
            }),
        }
    }

    /// Check all three layers. First failure wins; result names which
    /// layer denied so the audit log can attribute correctly.
    pub fn check(
        &self,
        transport: &str,
        principal: &str,
        backend: &str,
        collection: &str,
    ) -> RateLimitDecision {
        let key_p = format!("{transport}:{principal}");
        let key_c = format!("{principal}:{backend}:{collection}");
        // Track keys for the soft eviction below.
        if self.inner.keys.len() < 10_000 {
            self.inner.keys.insert(key_p.clone(), ());
            self.inner.keys.insert(key_c.clone(), ());
        }
        if self.inner.per_process.check_key(&"_proc").is_err() {
            return RateLimitDecision::Denied { layer: "process" };
        }
        if self.inner.per_principal.check_key(&key_p).is_err() {
            return RateLimitDecision::Denied { layer: "principal" };
        }
        if self.inner.per_collection.check_key(&key_c).is_err() {
            return RateLimitDecision::Denied {
                layer: "collection",
            };
        }
        RateLimitDecision::Allowed
    }

    pub fn key_count(&self) -> usize {
        self.inner.keys.len()
    }

    pub fn config_summary(&self) -> String {
        format!(
            "per_principal={}/s burst={}; per_collection={}/s burst={}; per_process={}/s burst={}",
            self.inner.per_principal_cfg.per_second,
            self.inner.per_principal_cfg.burst,
            self.inner.per_collection_cfg.per_second,
            self.inner.per_collection_cfg.burst,
            self.inner.per_process_cfg.per_second,
            self.inner.per_process_cfg.burst,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allowed,
    /// `layer` ∈ {"process", "principal", "collection"} — matches the
    /// audit `decision.refusals[].code` taxonomy.
    Denied {
        layer: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_under_burst() {
        let rl = LayeredRateLimiter::default();
        for _ in 0..10 {
            assert_eq!(
                rl.check("stdio", "alice", "be", "docs"),
                RateLimitDecision::Allowed
            );
        }
    }

    #[test]
    fn isolates_principals() {
        let rl = LayeredRateLimiter::new(
            LayerConfig {
                per_second: 1,
                burst: 2,
            },
            LayerConfig {
                per_second: 100,
                burst: 200,
            },
            LayerConfig {
                per_second: 1000,
                burst: 2000,
            },
        );
        // Burn alice's bucket.
        rl.check("h", "alice", "be", "x");
        rl.check("h", "alice", "be", "x");
        let alice_third = rl.check("h", "alice", "be", "x");
        assert!(matches!(
            alice_third,
            RateLimitDecision::Denied { layer: "principal" }
        ));
        // Bob is unaffected.
        let bob = rl.check("h", "bob", "be", "x");
        assert_eq!(bob, RateLimitDecision::Allowed);
    }
}
