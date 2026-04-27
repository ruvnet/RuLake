//! `AuditSink::emit()` throughput bench at the 256-entry ring boundary.
//!
//! v0.10 added a bounded `VecDeque<Value>` (cap = `TAIL_CAPACITY`,
//! 256) behind the audit sink so `rulake://audit/tail` can serve the
//! recent N lines without re-parsing the JSONL file. Every `emit()`
//! call pushes into that ring AND writes one line to the configured
//! sink (stderr-via-tracing, or appended to a file under
//! `Mutex<Option<File>>`).
//!
//! The push-pop logic flips a branch when the ring fills up
//! (`pop_front` then `push_back` instead of just `push_back`). This
//! bench exercises both regimes:
//!   - `below_capacity` — only push_back, ring growing.
//!   - `at_capacity`    — push_back + pop_front every iter.
//!   - `tail_snapshot`  — read path: clone the last 256 entries.
//!
//! The file/stderr write side is intentionally pinned to stderr;
//! the test runner already captures stderr via `tracing` so we
//! don't pollute it with millions of lines, and the file write
//! latency would otherwise dominate this micro-bench.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use ruvector_rulake_mcp::{AuditEntry, AuditSink};

fn entry(i: u64) -> AuditEntry {
    AuditEntry {
        ts: format!("2026-04-26T00:00:{:02}.000Z", i % 60),
        transport: "http".into(),
        principal: "bearer:abc123".into(),
        session: None,
        request_id: Some(format!("req-{i}")),
        tool: "rulake_query".into(),
        intent: Some("search".into()),
        outcome: "ok".into(),
        result_size: Some(5),
        trust_level: Some("verified".into()),
        duration_ms: 1.7,
        witness_in: None,
        witness_out: Some("fc01".into()),
        code: None,
        policy_decision: None,
        decision: None,
    }
}

fn bench_emit_below_capacity(c: &mut Criterion) {
    // Only push_back — ring grows from empty toward 256. Each iter
    // builds a fresh sink so the steady-state isn't masked by the
    // first few cheap pushes. Sample size kept low because the sink
    // alloc dominates an otherwise trivial body.
    let mut group = c.benchmark_group("mcp_audit_emit_below_cap");
    group.bench_function("push_only", |b| {
        b.iter_with_setup(
            AuditSink::stderr,
            |sink| {
                // 100 emits — well under the 256 cap.
                for i in 0..100u64 {
                    sink.emit(entry(i));
                }
                black_box(sink);
            },
        );
    });
    group.finish();
}

fn bench_emit_at_capacity(c: &mut Criterion) {
    // Sink is pre-filled to 256, then we measure the steady-state
    // push_back + pop_front cost per emit.
    let mut group = c.benchmark_group("mcp_audit_emit_at_cap");
    let sink = AuditSink::stderr();
    for i in 0..256u64 {
        sink.emit(entry(i));
    }
    let mut counter: u64 = 1_000;
    group.bench_function("push_pop", |b| {
        b.iter(|| {
            sink.emit(entry(counter));
            counter = counter.wrapping_add(1);
        });
    });
    group.finish();
}

fn bench_tail_snapshot(c: &mut Criterion) {
    // Read path. The `rulake://audit/tail` resource clones the last
    // n entries (n clamped to 256). At a full ring this is the
    // clone of 256 serde_json::Value objects.
    let sink = AuditSink::stderr();
    for i in 0..256u64 {
        sink.emit(entry(i));
    }
    let mut group = c.benchmark_group("mcp_audit_tail_snapshot");
    for n in &[16usize, 64, 256] {
        group.bench_with_input(BenchmarkId::from_parameter(n), n, |b, &n| {
            b.iter(|| {
                let snap = sink.tail(black_box(n));
                black_box(snap);
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_emit_below_capacity,
    bench_emit_at_capacity,
    bench_tail_snapshot
);
criterion_main!(benches);
