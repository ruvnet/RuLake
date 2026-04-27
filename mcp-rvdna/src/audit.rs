//! JSONL audit log — wire-compatible with `mcp-server::audit::AuditEntry`
//! (commit 56b497b).
//!
//! Two MCP servers (`mcp-server` for ruLake bundles, `mcp-rvdna` for
//! genomic substrate) share the same audit shape so an operator can
//! `cat rulake.jsonl rvdna.jsonl | jq` and get a coherent timeline.
//! The only field that diverges across servers is `tool` — `rvdna_*`
//! prefixes are reserved for this crate, `rulake_*` for mcp-server.
//!
//! v0.0.1 keeps it minimal: a 256-entry tail buffer + one Mutex<File>
//! (or stderr fallback). No bg-flushed ring / no rotation — `mcp-server`
//! v0.10 ships the same shape; if that contention floor matters here
//! we'll lift its bg sink wholesale rather than reinvent.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;

/// Bounded ring buffer of recently-emitted audit lines. Same capacity
/// as mcp-server so the two crates' tail resources behave identically.
const TAIL_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct AuditSink {
    inner: Arc<Inner>,
    tail: Arc<Mutex<VecDeque<Value>>>,
}

enum Inner {
    Stderr,
    File {
        file: Mutex<Option<std::fs::File>>,
        path: PathBuf,
    },
}

impl AuditSink {
    pub fn stderr() -> Self {
        Self {
            inner: Arc::new(Inner::Stderr),
            tail: Arc::new(Mutex::new(VecDeque::with_capacity(TAIL_CAPACITY))),
        }
    }

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

    /// Snapshot the last `n` audit entries (newest last). Cheap — clones
    /// the bounded ring once.
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

    pub fn path(&self) -> Option<&std::path::Path> {
        match &*self.inner {
            Inner::File { path, .. } => Some(path),
            Inner::Stderr => None,
        }
    }

    /// Emit one audit line. Failures (closed file, disk full) get
    /// logged via tracing and otherwise swallowed — auditing a tool
    /// call must never crash the request path.
    pub fn emit(&self, entry: AuditEntry) {
        let value: Value = match serde_json::to_value(&entry) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "rvdna audit serialize failed");
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
                tracing::warn!(error = %e, "rvdna audit stringify failed");
                return;
            }
        };
        match &*self.inner {
            Inner::Stderr => {
                tracing::info!(target: "rvdna.audit", "{line}");
            }
            Inner::File { file, path } => {
                let mut guard = match file.lock() {
                    Ok(g) => g,
                    Err(p) => {
                        tracing::error!("rvdna audit lock poisoned at {}", path.display());
                        p.into_inner()
                    }
                };
                if let Some(f) = guard.as_mut() {
                    if let Err(e) = writeln!(f, "{line}") {
                        tracing::warn!(error = %e, "rvdna audit write failed");
                    } else if let Err(e) = f.flush() {
                        tracing::warn!(error = %e, "rvdna audit flush failed");
                    }
                }
            }
        }
    }
}

/// One audit line. Schema is byte-isomorphic to
/// `mcp-server::audit::AuditEntry`, by design (commit 56b497b).
#[derive(Debug, Serialize, Clone)]
pub struct AuditEntry {
    pub ts: String,
    pub transport: String,
    pub principal: String,
    pub session: Option<String>,
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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision: Option<PolicyDecision>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<Value>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PolicyDecision {
    pub capability_required: String,
    pub capability_granted: Vec<String>,
}

/// ISO-8601 UTC timestamp — matches the mcp-server formatter so audit
/// timelines from both crates sort identically.
pub fn now_ts() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let millis = now.subsec_millis();
    let (year, month, day, hour, minute, second) = epoch_to_ymdhms(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn epoch_to_ymdhms(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;
    // Civil-from-days (Howard Hinnant, public domain) — same as mcp-server.
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
        assert_eq!(ts.len(), 24);
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[23..24], "Z");
    }

    #[test]
    fn tail_records_what_emit_writes() {
        let sink = AuditSink::stderr();
        sink.emit(AuditEntry {
            ts: now_ts(),
            transport: "stdio".into(),
            principal: "stdio:local".into(),
            session: None,
            request_id: None,
            tool: "rvdna_lineage".into(),
            intent: None,
            outcome: "ok".into(),
            result_size: Some(1),
            trust_level: Some("verified".into()),
            duration_ms: 0.5,
            witness_in: None,
            witness_out: Some("deadbeef".into()),
            code: None,
            policy_decision: Some(PolicyDecision {
                capability_required: "internal".into(),
                capability_granted: vec!["read".into(), "internal".into()],
            }),
            decision: None,
        });
        let snap = sink.tail(8);
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0]["tool"], "rvdna_lineage");
        assert_eq!(snap[0]["witness_out"], "deadbeef");
    }
}
