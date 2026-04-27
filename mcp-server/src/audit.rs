//! JSONL audit log per ADR-004 §7.
//!
//! Append-only file, one JSON object per tool invocation. Schema
//! matches the §7 outer block: `ts`, `transport`, `principal`,
//! `tool`, `intent`, `args_hash`, `args_size`, `outcome`,
//! `result_size`, `trust_level`, `duration_ms`, `witness_in/out`,
//! `code`, plus `policy_decision` and `decision` blocks when
//! available.
//!
//! v0.3 keeps it simple: one Mutex<File>, write+flush per line. At
//! this scale (<10k QPS per process) the lock contention is below
//! the syscall noise floor; v0.4 can swap in a background-flushed
//! ring buffer if a real workload says it matters.
//!
//! The fallback is the existing `tracing` JSON sink to stderr —
//! when no `--audit-file` is configured every emit goes to stderr
//! with the same shape. Operators get coherent audit either way.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;

/// Bounded ring buffer of recently-emitted audit lines. Backs the
/// `rulake://audit/tail` resource (mcp-server v0.10) — operators
/// (and the Console's Audit screen in live mode) can read the last
/// N lines without needing the audit-file path or shell access.
const TAIL_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct AuditSink {
    inner: Arc<Inner>,
    tail: Arc<Mutex<VecDeque<Value>>>,
}

enum Inner {
    Stderr,
    File {
        // None when the file is closed (e.g. shutdown); usually Some.
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

    /// Snapshot the last `n` audit entries (newest last). `n` is
    /// clamped to `TAIL_CAPACITY`. Backs the `rulake://audit/tail`
    /// MCP resource. Cheap — clones the bounded ring once.
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

    /// Emit one audit line. Failures (closed file, disk full) are
    /// logged via `tracing` and otherwise swallowed — auditing a
    /// tool call must never crash the request path.
    pub fn emit(&self, entry: AuditEntry) {
        let value: Value = match serde_json::to_value(&entry) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "audit serialize failed");
                return;
            }
        };
        // Tee into the in-memory tail ring before writing — the file/
        // stderr write path can fail (disk full, lock poisoned) but we
        // still want the resource to surface what we tried to log.
        if let Ok(mut buf) = self.tail.lock() {
            if buf.len() == TAIL_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(value.clone());
        }
        let line = match serde_json::to_string(&value) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "audit stringify failed");
                return;
            }
        };
        match &*self.inner {
            Inner::Stderr => {
                tracing::info!(target: "rulake.audit", "{line}");
            }
            Inner::File { file, path } => {
                let mut guard = match file.lock() {
                    Ok(g) => g,
                    Err(p) => {
                        tracing::error!("audit lock poisoned at {}", path.display());
                        p.into_inner()
                    }
                };
                if let Some(f) = guard.as_mut() {
                    if let Err(e) = writeln!(f, "{line}") {
                        tracing::warn!(error = %e, "audit write failed");
                    } else if let Err(e) = f.flush() {
                        tracing::warn!(error = %e, "audit flush failed");
                    }
                }
            }
        }
    }
}

/// One audit line. Schema mirrors ADR-004 §7. Fields are `Option`
/// where the call site doesn't have the data (e.g. `intent` is
/// only present for `rulake_query` calls).
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

    /// Policy-decision block (capability granted, allow-rule, etc.).
    /// v0.3 fills capability_required + capability_granted; per-rule
    /// allow-list lands in v0.4 with the path/collection allow-list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision: Option<PolicyDecision>,

    /// Decision-trace block from the planner. Present for
    /// `rulake_query` calls; `None` for direct internal-kernel calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<Value>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PolicyDecision {
    pub capability_required: String,
    pub capability_granted: Vec<String>,
}

/// ISO-8601 UTC timestamp — matches the ADR-004 §7 example.
pub fn now_ts() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let millis = now.subsec_millis();
    // Hand-roll an ISO-8601 string to avoid pulling chrono into the
    // dep tree just for a timestamp. Year 9999 is fine.
    let (year, month, day, hour, minute, second) = epoch_to_ymdhms(secs);
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    )
}

fn epoch_to_ymdhms(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    // Days since 1970-01-01.
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;

    // Civil-from-days algorithm (Howard Hinnant, public domain).
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
        // Matches YYYY-MM-DDTHH:MM:SS.MMMZ
        assert!(ts.len() == 24, "len = {} ({ts})", ts.len());
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[19..20], ".");
        assert_eq!(&ts[23..24], "Z");
    }

    #[test]
    fn epoch_2026_01_01() {
        // 2026-01-01 00:00:00 UTC = 1767225600
        let (y, m, d, _, _, _) = epoch_to_ymdhms(1767225600);
        assert_eq!((y, m, d), (2026, 1, 1));
    }

    #[test]
    fn write_and_read_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("audit.jsonl");
        let sink = AuditSink::open_file(&path).unwrap();
        sink.emit(AuditEntry {
            ts: now_ts(),
            transport: "stdio".into(),
            principal: "test".into(),
            session: None,
            request_id: None,
            tool: "rulake_query".into(),
            intent: Some("search".into()),
            outcome: "ok".into(),
            result_size: Some(5),
            trust_level: Some("verified".into()),
            duration_ms: 1.7,
            witness_in: None,
            witness_out: Some("fc01".into()),
            code: None,
            policy_decision: Some(PolicyDecision {
                capability_required: "read".into(),
                capability_granted: vec!["read".into()],
            }),
            decision: None,
        });
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("\"tool\":\"rulake_query\""));
        assert!(body.contains("\"outcome\":\"ok\""));
        assert!(body.ends_with('\n'));
    }

    fn entry(tool: &'static str, ts: u64) -> AuditEntry {
        AuditEntry {
            ts: ts.to_string(),
            transport: "http".into(),
            principal: "anon:test".into(),
            session: None,
            request_id: None,
            tool: tool.into(),
            intent: None,
            outcome: "ok".into(),
            result_size: None,
            trust_level: None,
            duration_ms: 0.1,
            witness_in: None,
            witness_out: None,
            code: None,
            policy_decision: None,
            decision: None,
        }
    }

    #[test]
    fn tail_returns_recent_entries_in_emit_order() {
        let sink = AuditSink::stderr();
        for i in 0..5 {
            sink.emit(entry("rulake_query", i));
        }
        let last_3 = sink.tail(3);
        assert_eq!(last_3.len(), 3);
        // Newest last — entries with ts strings "2","3","4".
        assert_eq!(last_3[0]["ts"], "2");
        assert_eq!(last_3[2]["ts"], "4");
        assert_eq!(last_3[2]["tool"], "rulake_query");
    }

    #[test]
    fn tail_buffer_caps_at_capacity() {
        let sink = AuditSink::stderr();
        let cap = super::TAIL_CAPACITY as u64;
        for i in 0..(cap + 50) {
            sink.emit(entry("rulake_query", i));
        }
        let all = sink.tail(super::TAIL_CAPACITY * 2);
        assert_eq!(all.len(), super::TAIL_CAPACITY);
        // Oldest surviving = index 50; newest = the last emitted.
        assert_eq!(all[0]["ts"], "50");
        assert_eq!(all.last().unwrap()["ts"], (cap + 49).to_string());
    }

    #[test]
    fn tail_records_even_when_file_write_path_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let sink = AuditSink::open_file(&path).unwrap();
        sink.emit(entry("rulake_publish_bundle", 7));
        sink.emit(entry("rulake_invalidate_cache", 9));
        let snap = sink.tail(10);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0]["tool"], "rulake_publish_bundle");
        assert_eq!(snap[1]["tool"], "rulake_invalidate_cache");
        // File path also got the lines.
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body.lines().count(), 2);
    }
}
