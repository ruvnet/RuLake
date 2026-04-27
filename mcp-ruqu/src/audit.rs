//! JSONL audit emission for `mcp-ruqu` (ADR-008 §6 Decision 4).
//!
//! Mirrors the shape of `mcp-server/src/audit.rs` so an operator's
//! existing audit-ingestion pipeline (file watch on `audit.jsonl`,
//! ELK / Loki / whatever) sees coherent rows from both servers. The
//! ADR pins this: "audit codes use disjoint prefixes (`RUQU_*` vs
//! `RULAKE_*`)".
//!
//! v0.0.1 ships only the stderr sink + an in-memory tail ring. The
//! file sink and the `audit-only` feature flag on `mcp-server` (so
//! both crates share `AuditEntry` from one place) are deferred to
//! mcp-ruqu v0.1 — at that point the ADR §6 Decision 4 unification
//! ("shared `mcp_server::audit::AuditEntry`") can land without
//! reshaping any v0.0.1 callers.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;

/// Bounded ring buffer of recently-emitted audit lines. Same cap as
/// `mcp-server`'s `rulake://audit/tail` resource so future cross-
/// server tooling sees identical buffering semantics.
const TAIL_CAPACITY: usize = 256;

/// Audit sink. Cheap to `Clone`; safe to share across rmcp tool
/// handlers via `Arc`-internal mutability.
#[derive(Clone)]
pub struct AuditSink {
    inner: Arc<Inner>,
    tail: Arc<Mutex<VecDeque<Value>>>,
}

/// Backing store for an [`AuditSink`]. v0.0.1 shipped only `Stderr`;
/// v0.1 adds `File` so the `--audit-file` CLI flag (mcp-ruqu's `http`
/// + `stdio` subcommands) can persist JSONL the same way mcp-rvdna and
/// mcp-server do. Schema unchanged.
enum Inner {
    Stderr,
    File {
        file: Mutex<Option<std::fs::File>>,
        path: PathBuf,
    },
}

impl Default for AuditSink {
    fn default() -> Self {
        Self::stderr()
    }
}

impl AuditSink {
    /// Stderr sink — every emit also pushes onto the in-memory tail.
    pub fn stderr() -> Self {
        Self {
            inner: Arc::new(Inner::Stderr),
            tail: Arc::new(Mutex::new(VecDeque::with_capacity(TAIL_CAPACITY))),
        }
    }

    /// JSONL file sink. Creates parents if missing; opens the file in
    /// append mode so multiple server runs accumulate cleanly. Mirrors
    /// `mcp-rvdna::audit::AuditSink::open_file` so operator tooling can
    /// treat both audit streams interchangeably.
    pub fn open_file(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(Self {
            inner: Arc::new(Inner::File {
                file: Mutex::new(Some(file)),
                path,
            }),
            tail: Arc::new(Mutex::new(VecDeque::with_capacity(TAIL_CAPACITY))),
        })
    }

    /// Snapshot the last `n` audit entries (newest last). Used by
    /// the integration tests to assert audit codes without grepping
    /// stderr. Cheap — clones the bounded ring once.
    pub fn tail(&self, n: usize) -> Vec<Value> {
        let cap = n.min(TAIL_CAPACITY);
        let guard = match self.tail.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let len = guard.len();
        let start = len.saturating_sub(cap);
        guard.iter().skip(start).cloned().collect()
    }

    /// Path of the underlying file, when the sink is file-backed.
    /// Useful for diagnostics ("audit -> JSONL file <path>" log line).
    pub fn path(&self) -> Option<&std::path::Path> {
        match &*self.inner {
            Inner::File { path, .. } => Some(path),
            Inner::Stderr => None,
        }
    }

    /// Emit one audit line. Failures (lock poisoned, serializer broken,
    /// disk full) are logged via `tracing` and otherwise swallowed —
    /// auditing a tool call must never crash the request path.
    pub fn emit(&self, entry: AuditEntry) {
        let value: Value = match serde_json::to_value(&entry) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "ruqu audit serialize failed");
                return;
            }
        };
        if let Ok(mut buf) = self.tail.lock() {
            if buf.len() == TAIL_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(value.clone());
        }
        let line = match serde_json::to_string(&value) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "ruqu audit stringify failed");
                return;
            }
        };
        match &*self.inner {
            Inner::Stderr => {
                tracing::info!(target: "ruqu.audit", "{line}");
            }
            Inner::File { file, path } => {
                let mut guard = match file.lock() {
                    Ok(g) => g,
                    Err(p) => {
                        tracing::error!("ruqu audit lock poisoned at {}", path.display());
                        p.into_inner()
                    }
                };
                if let Some(f) = guard.as_mut() {
                    if let Err(e) = writeln!(f, "{line}") {
                        tracing::warn!(error = %e, "ruqu audit write failed");
                    } else if let Err(e) = f.flush() {
                        tracing::warn!(error = %e, "ruqu audit flush failed");
                    }
                }
            }
        }
    }
}

/// One audit line. Schema mirrors `mcp-server`'s `AuditEntry`
/// verbatim so the two JSONL streams interleave cleanly. Fields are
/// `Option` where the call site doesn't have data; the only ruQu-
/// specific addition is the `RUQU_*` family of `code` values
/// emitted by the five tools.
#[derive(Debug, Serialize, Clone)]
pub struct AuditEntry {
    pub ts: String,
    pub transport: String,
    pub principal: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,

    pub tool: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,

    pub outcome: String, // "ok" | "refused" | "degraded" | "error"

    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_size: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust_level: Option<String>,

    pub duration_ms: f64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub witness_in: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub witness_out: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,

    /// Policy-decision block. Always populated by the v0.0.1 tool
    /// handlers regardless of outcome — matches mcp-server commit
    /// 56b497b's "fully-shaped audit entry" discipline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision: Option<PolicyDecision>,
}

/// Policy-decision block embedded in [`AuditEntry`]. Same shape as
/// `mcp-server`'s `PolicyDecision`.
#[derive(Debug, Serialize, Clone)]
pub struct PolicyDecision {
    pub capability_required: String,
    pub capability_granted: Vec<String>,
}

/// ISO-8601 UTC timestamp — same hand-rolled formatter as
/// `mcp-server::audit::now_ts` (avoids pulling chrono in just for
/// a timestamp). Year 9999 is fine.
pub fn now_ts() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let millis = now.subsec_millis();
    let (year, month, day, hour, minute, second) = epoch_to_ymdhms(secs);
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    )
}

fn epoch_to_ymdhms(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;
    // Civil-from-days (Howard Hinnant, public domain).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + (if m <= 2 { 1 } else { 0 });
    (y as u32, m as u32, d as u32, hour, minute, second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_format_is_iso8601() {
        let ts = now_ts();
        assert_eq!(ts.len(), 24, "len = {} ({ts})", ts.len());
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[19..20], ".");
        assert_eq!(&ts[23..24], "Z");
    }

    #[test]
    fn tail_returns_recent_entries_in_emit_order() {
        let sink = AuditSink::stderr();
        for i in 0..5 {
            sink.emit(AuditEntry {
                ts: i.to_string(),
                transport: "stdio".into(),
                principal: "test".into(),
                session: None,
                request_id: None,
                tool: "ruqu_simulate".into(),
                intent: None,
                outcome: "ok".into(),
                result_size: None,
                trust_level: None,
                duration_ms: 0.0,
                witness_in: None,
                witness_out: None,
                code: None,
                policy_decision: None,
            });
        }
        let last_3 = sink.tail(3);
        assert_eq!(last_3.len(), 3);
        assert_eq!(last_3[0]["ts"], "2");
        assert_eq!(last_3[2]["ts"], "4");
    }
}
