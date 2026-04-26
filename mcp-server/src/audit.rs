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

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;

#[derive(Clone)]
pub struct AuditSink {
    inner: Arc<Inner>,
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
        })
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
}
