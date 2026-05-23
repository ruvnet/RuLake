//! Session binding (ADR-004 §5).
//!
//! When the operator runs `--auth jwt` and an MCP client opens a
//! Streamable HTTP session, we record the binding
//! `(session_id) → (principal, client_id, mTLS-cert-fingerprint)`
//! at first sighting. Every subsequent request on that session must
//! present the same tuple — a stolen token replayed from a different
//! client_id is rejected with 401.
//!
//! Two pieces:
//! - `SessionBindings` — the in-memory store. ~10k LRU.
//! - `SessionDecision` — the per-request outcome enum the HTTP
//!   handler turns into 200/401.
//!
//! ADR-004 §5 also mentions `(mTLS-cert)` as part of the tuple.
//! v0.5 ships JWT-side only; mTLS lands separately and adds the
//! cert fingerprint into the tuple at that point — the binding
//! function already takes the cert as an `Option<String>` so the
//! mTLS work doesn't need a schema change here.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

const WINDOW: usize = 10_000;

/// In-memory session binding store. Cheap to clone (Arc-wrapped state).
#[derive(Default)]
pub struct SessionBindings {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Most-recent insertion order — first member of the pair is the
    /// session id; we evict the oldest when the window fills.
    seen: VecDeque<String>,
    bindings: HashMap<String, BindingTuple>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BindingTuple {
    principal: String,
    client_id: Option<String>,
    /// SHA-256 hex of the mTLS client certificate, if mTLS is in use.
    /// `None` for JWT-only or bearer-only deployments.
    mtls_fingerprint: Option<String>,
}

impl SessionBindings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a session id against the binding store.
    /// Outcomes:
    /// - `Allowed { first_sighting: true }` — first time we've seen
    ///   this session id; the binding is recorded for next time.
    /// - `Allowed { first_sighting: false }` — session is known and
    ///   the presented tuple matches.
    /// - `Mismatch { .. }` — session is known but the tuple differs.
    ///   This is the "stolen token replayed elsewhere" case; the
    ///   HTTP handler returns 401.
    pub fn check_or_bind(
        &self,
        session_id: &str,
        principal: &str,
        client_id: Option<&str>,
        mtls_fingerprint: Option<&str>,
    ) -> SessionDecision {
        if session_id.is_empty() {
            // No session id (initialize call etc.) — pass through.
            return SessionDecision::Allowed {
                first_sighting: false,
            };
        }
        let want = BindingTuple {
            principal: principal.to_string(),
            client_id: client_id.map(str::to_string),
            mtls_fingerprint: mtls_fingerprint.map(str::to_string),
        };
        let mut inner = self.inner.lock().unwrap();
        if let Some(existing) = inner.bindings.get(session_id) {
            if existing == &want {
                return SessionDecision::Allowed {
                    first_sighting: false,
                };
            }
            return SessionDecision::Mismatch {
                expected_principal: existing.principal.clone(),
                presented_principal: principal.to_string(),
            };
        }
        // Evict oldest if window full.
        if inner.seen.len() >= WINDOW {
            if let Some(evict) = inner.seen.pop_front() {
                inner.bindings.remove(&evict);
            }
        }
        inner.seen.push_back(session_id.to_string());
        inner.bindings.insert(session_id.to_string(), want);
        SessionDecision::Allowed {
            first_sighting: true,
        }
    }

    pub fn binding_count(&self) -> usize {
        self.inner.lock().unwrap().bindings.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionDecision {
    Allowed {
        /// `true` when the binding was just recorded — useful for
        /// audit attribution.
        first_sighting: bool,
    },
    Mismatch {
        expected_principal: String,
        presented_principal: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_session_id_passes() {
        let s = SessionBindings::new();
        let d = s.check_or_bind("", "alice", None, None);
        assert_eq!(
            d,
            SessionDecision::Allowed {
                first_sighting: false
            }
        );
    }

    #[test]
    fn first_sighting_records_binding() {
        let s = SessionBindings::new();
        let d = s.check_or_bind("sess-1", "alice", Some("cursor"), None);
        assert_eq!(
            d,
            SessionDecision::Allowed {
                first_sighting: true
            }
        );
        assert_eq!(s.binding_count(), 1);
    }

    #[test]
    fn second_sighting_with_same_tuple_passes() {
        let s = SessionBindings::new();
        s.check_or_bind("sess-1", "alice", Some("cursor"), None);
        let d = s.check_or_bind("sess-1", "alice", Some("cursor"), None);
        assert_eq!(
            d,
            SessionDecision::Allowed {
                first_sighting: false
            }
        );
    }

    #[test]
    fn principal_change_is_rejected() {
        let s = SessionBindings::new();
        s.check_or_bind("sess-1", "alice", Some("cursor"), None);
        let d = s.check_or_bind("sess-1", "mallory", Some("cursor"), None);
        match d {
            SessionDecision::Mismatch {
                expected_principal,
                presented_principal,
            } => {
                assert_eq!(expected_principal, "alice");
                assert_eq!(presented_principal, "mallory");
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    #[test]
    fn client_id_change_is_rejected() {
        let s = SessionBindings::new();
        s.check_or_bind("sess-1", "alice", Some("cursor"), None);
        let d = s.check_or_bind("sess-1", "alice", Some("EVIL_CLIENT"), None);
        assert!(matches!(d, SessionDecision::Mismatch { .. }));
    }

    #[test]
    fn mtls_fingerprint_change_is_rejected() {
        let s = SessionBindings::new();
        s.check_or_bind("sess-1", "alice", Some("cursor"), Some("aabb..."));
        let d = s.check_or_bind("sess-1", "alice", Some("cursor"), Some("ccdd..."));
        assert!(matches!(d, SessionDecision::Mismatch { .. }));
    }

    #[test]
    fn evicts_after_window_full() {
        let s = SessionBindings::new();
        for i in 0..(WINDOW + 1) {
            s.check_or_bind(&format!("sess-{i}"), "u", None, None);
        }
        assert_eq!(s.binding_count(), WINDOW);
        // The first session aged out — re-binding it counts as first sighting.
        let d = s.check_or_bind("sess-0", "u", None, None);
        assert_eq!(
            d,
            SessionDecision::Allowed {
                first_sighting: true
            }
        );
    }
}
