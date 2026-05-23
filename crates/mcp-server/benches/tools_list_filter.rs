//! `tools/list` capability filter bench (mcp-server v0.8 effective-caps
//! intersection).
//!
//! What the filter does (`server.rs::list_tools_filtered`, mirroring
//! the wire-level `ServerHandler::list_tools`):
//!   1. compute `effective_caps()` = server-wide ∩ per-request JWT
//!      scopes (the per-request task-local is unset in this bench →
//!      collapses to server-wide)
//!   2. enumerate every registered tool
//!   3. filter by `effective.has(required_cap_for_tool(name))`
//!   4. sort the resulting names
//!
//! This bench exists because v0.8's per-call effective-caps lookup
//! lives on the hot path — every `tools/list` request from a client
//! pays for it. We exercise the filter at three scope-count tiers:
//!
//!   - 1 cap   (read-only — the default; 3 tools visible)
//!   - 4 caps  (read+publish+admin+internal — all 8 tools visible)
//!
//! The CSV → CapabilitySet parse is set up once outside the loop;
//! each iter exercises only the filter pass.

use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rulake::{BackendAdapter, LocalBackend, RuLake};
use ruvector_rulake_mcp::{CapabilitySet, RuLakeMcpServer};

fn server_with(caps_csv: &str) -> RuLakeMcpServer {
    let lake = RuLake::new(20, 42);
    let be: Arc<dyn BackendAdapter> = Arc::new(LocalBackend::new("local"));
    lake.register_backend(be).unwrap();
    let lake = Arc::new(lake);

    let caps = if caps_csv.is_empty() {
        CapabilitySet::default()
    } else {
        CapabilitySet::from_csv(caps_csv).unwrap()
    };
    RuLakeMcpServer::from_lake_with_caps(
        lake,
        "Eventual { ttl_ms: 60000 }".into(),
        vec!["local".into()],
        64,
        caps,
    )
    .expect("build server")
}

fn bench_tools_list_filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("mcp_tools_list_filter");

    // Three regimes that span the realistic operator deployments:
    //   - "read"            — agent-facing default (read-only token)
    //   - "read,publish"    — cache-publishing operator
    //   - "read,publish,admin" — full operator console
    // We don't bench the empty-caps case — that one is a hard
    // failure shape (no tools visible) and isn't an operating point.
    for csv in ["read", "read,publish", "read,publish,admin"] {
        let server = server_with(csv);
        // `len()` of the CSV gives a quick cardinality stand-in for
        // the bench id without rebuilding the set.
        let label = format!("caps={csv}");
        group.bench_with_input(BenchmarkId::from_parameter(label), csv, |b, _| {
            b.iter(|| {
                let names = server.list_tools_filtered();
                black_box(names);
            });
        });
    }
    group.finish();
}

fn bench_capabilityset_from_csv(c: &mut Criterion) {
    // Per-request setup cost — every JWT verify path lands in
    // `scopes_to_caps` which calls `CapabilitySet::from_csv`. This
    // is the `1 / 8 / 64 scopes` micro-cost the brief asks for
    // (capability tier maxes at 4 known labels; we pad with unknown
    // scopes for the 8 / 64 cases since the parser ignores them at
    // the JWT layer — here we test the CSV parser shape directly).
    let mut group = c.benchmark_group("mcp_capset_from_csv");
    let csvs: [(&str, String); 3] = [
        ("1", "read".into()),
        (
            "8",
            "read,publish,admin,internal,read,publish,admin,internal".into(),
        ),
        (
            "64",
            (0..64)
                .map(|i| {
                    // Mix in the four real labels at fixed slots, garbage
                    // labels would error out — instead repeat the four real
                    // ones to get a 64-token CSV that still parses.
                    ["read", "publish", "admin", "internal"][i % 4]
                })
                .collect::<Vec<_>>()
                .join(","),
        ),
    ];
    for (label, csv) in &csvs {
        group.bench_with_input(BenchmarkId::from_parameter(label), csv, |b, csv| {
            b.iter(|| {
                let set = CapabilitySet::from_csv(black_box(csv)).expect("parse");
                black_box(set);
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_tools_list_filter,
    bench_capabilityset_from_csv
);
criterion_main!(benches);
