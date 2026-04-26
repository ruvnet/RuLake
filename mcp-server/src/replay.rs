//! Replay protection (ADR-004 §5).
//!
//! Per-request `MCP-Request-Id` nonce dedup over a 10k-entry LRU
//! window. Replays inside that window are rejected; OAuth signature
//! + audience + short expiry handles the bulk of the threat, this
//! is the same-window second-strike layer.
//!
//! Session-id binding to `(principal, client_id, mTLS-cert)` is also
//! per ADR-004 §5; we surface the binding hash so the HTTP layer can
//! detect a token reuse from a different client_id.

use std::collections::VecDeque;
use std::sync::Mutex;

const WINDOW: usize = 10_000;

#[derive(Default)]
pub struct ReplayGuard {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Insertion-ordered ring of seen request ids. Pop-front on grow.
    seen: VecDeque<String>,
    /// O(1) membership.
    set: std::collections::HashSet<String>,
}

impl ReplayGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `Ok(())` for the first sighting; `Err(RequestIdReplay)`
    /// for any second sighting inside the window.
    pub fn check(&self, request_id: &str) -> Result<(), RequestIdReplay> {
        if request_id.is_empty() {
            // Spec allows missing id on stdio; only enforce when present.
            return Ok(());
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.set.contains(request_id) {
            return Err(RequestIdReplay {
                request_id: request_id.to_string(),
            });
        }
        if inner.seen.len() >= WINDOW {
            if let Some(evict) = inner.seen.pop_front() {
                inner.set.remove(&evict);
            }
        }
        inner.seen.push_back(request_id.to_string());
        inner.set.insert(request_id.to_string());
        Ok(())
    }

    pub fn window_used(&self) -> usize {
        self.inner.lock().unwrap().seen.len()
    }
}

#[derive(Debug)]
pub struct RequestIdReplay {
    pub request_id: String,
}

impl std::fmt::Display for RequestIdReplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RULAKE_REPLAY: MCP-Request-Id {:?} seen within the last {WINDOW} requests",
            self.request_id
        )
    }
}
impl std::error::Error for RequestIdReplay {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sighting_passes() {
        let g = ReplayGuard::new();
        assert!(g.check("abc").is_ok());
    }

    #[test]
    fn second_sighting_inside_window_fails() {
        let g = ReplayGuard::new();
        g.check("abc").unwrap();
        assert!(g.check("abc").is_err());
    }

    #[test]
    fn empty_id_is_allowed_through() {
        let g = ReplayGuard::new();
        // Stdio has no nonce — never deny.
        for _ in 0..5 {
            assert!(g.check("").is_ok());
        }
    }

    #[test]
    fn evicts_after_window_full() {
        let g = ReplayGuard::new();
        for i in 0..10_001 {
            g.check(&format!("id-{i}")).unwrap();
        }
        // The first id has aged out — re-checking it should pass.
        assert!(g.check("id-0").is_ok());
    }
}
