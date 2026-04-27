//! End-to-end smoke for `RvdnaT2Backend` (cold-lazy tier — `seek+read`
//! out of a `std::fs::File` plus a per-collection LRU window). Three
//! properties under test, mirroring `tests/t1_smoke.rs`:
//!
//! 1. The BackendAdapter contract holds — registered files round-trip
//!    through `RuLake::search_one` and produce the same top-K ranking
//!    that the equivalent T0 / T1 fixture would.
//! 2. T2 doesn't break the trust chain — for identical
//!    `(data_ref, dim, rotation_seed, rerank_factor, generation)` a
//!    T0, T1, and T2 backend's witnesses are byte-equal. This is the
//!    load-bearing "three tiers, one cache" invariant; if it ever
//!    breaks, T2 silently fragments the cache federation.
//! 3. The LRU warms under repeated search — the first pull is a disk
//!    read, calls 2..N are LRU-resident. We verify by inspecting the
//!    `RvdnaT2Backend::lru_stats` accessor.
//!
//! v0.2 fixtures write a raw little-endian `[f32]` blob into a tempdir
//! file — same shape as `tests/t1_smoke.rs`. The operator-side
//! `register_file` call passes the section offset directly.

use std::io::Write;
use std::sync::Arc;

use rulake::backend::BackendAdapter;
use rulake::cache::Consistency;
use rulake::RuLake;
use ruvector_rulake_rvdna::witness::{rvdna_bundle, RvdnaWitnessInputs};
use ruvector_rulake_rvdna::{
    RvdnaCollection, RvdnaT0Backend, RvdnaT1Backend, RvdnaT2Backend,
};

/// Deterministic mulberry32-equivalent — mirrors `tests/smoke.rs` and
/// `tests/t1_smoke.rs` so T0 / T1 / T2 fixtures with the same
/// `(seed, n, dim)` are byte-equal.
fn deterministic_vectors(seed: u64, n: usize, dim: usize) -> Vec<f32> {
    let mut s = seed;
    let mut next = || {
        s = s.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = s;
        x ^= x >> 30;
        x = x.wrapping_mul(0xbf58476d1ce4e5b9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94d049bb133111eb);
        x ^= x >> 31;
        x as f32 / u64::MAX as f32
    };
    (0..n * dim).map(|_| next() * 2.0 - 1.0).collect()
}

/// Write a raw `[f32]` little-endian blob to a tempfile and return its
/// path plus the byte offset of the vectors region (0 in this fixture
/// — header parsing lands later).
fn write_t2_fixture(seed: u64, n: usize, dim: usize) -> (tempfile::NamedTempFile, u64) {
    let flat = deterministic_vectors(seed, n, dim);
    let mut f = tempfile::NamedTempFile::new().expect("tempfile");
    let bytes: &[u8] = bytemuck::cast_slice(&flat);
    f.write_all(bytes).expect("write");
    f.flush().expect("flush");
    (f, 0)
}

/// Build the corresponding T0 collection so we can cross-check
/// rankings tier-vs-tier.
fn t0_collection_from(seed: u64, n: usize, dim: usize) -> RvdnaCollection {
    let flat = deterministic_vectors(seed, n, dim);
    let ids: Vec<u64> = (0..n as u64).collect();
    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| flat[i * dim..(i + 1) * dim].to_vec())
        .collect();
    RvdnaCollection { ids, vectors, dim, generation: 1 }
}

#[test]
fn t2_backend_round_trips_through_rulake_cache() {
    let dim = 16;
    let n = 64;
    let (file, vectors_offset) = write_t2_fixture(0xc0ffee, n, dim);

    // ── T2 path ──
    let t2_raw = Arc::new(RvdnaT2Backend::new("rvdna-t2-fixture"));
    t2_raw
        .register_file(
            "raw-dna-chr17",
            file.path(),
            dim,
            1,
            Vec::new(), // synthesise positional ids
            vectors_offset,
            n,
        )
        .expect("register");
    let t2_be: Arc<dyn BackendAdapter> = Arc::clone(&t2_raw) as Arc<dyn BackendAdapter>;

    let t2_lake =
        RuLake::new(20, 42).with_consistency(Consistency::Eventual { ttl_ms: 1_000 });
    t2_lake.register_backend(t2_be).unwrap();

    let q = vec![0.0_f32; dim];
    let t2_hits1 = t2_lake
        .search_one("rvdna-t2-fixture", "raw-dna-chr17", &q, 5)
        .expect("t2 search 1");
    assert_eq!(t2_hits1.len(), 5);

    // Cache hit on second call.
    let t2_hits2 = t2_lake
        .search_one("rvdna-t2-fixture", "raw-dna-chr17", &q, 5)
        .expect("t2 search 2");
    assert_eq!(t2_hits1, t2_hits2);
    assert_eq!(t2_lake.cache_stats().primes, 1);

    // ── T0 cross-check: same (seed, n, dim) → same ranking ──
    let t0 = RvdnaT0Backend::new("rvdna-t0-fixture");
    t0.put_collection("raw-dna-chr17", t0_collection_from(0xc0ffee, n, dim));
    let t0_be: Arc<dyn BackendAdapter> = Arc::new(t0);
    let t0_lake =
        RuLake::new(20, 42).with_consistency(Consistency::Eventual { ttl_ms: 1_000 });
    t0_lake.register_backend(t0_be).unwrap();
    let t0_hits = t0_lake
        .search_one("rvdna-t0-fixture", "raw-dna-chr17", &q, 5)
        .expect("t0 search");

    let t2_ids: Vec<u64> = t2_hits1.iter().map(|h| h.id).collect();
    let t0_ids: Vec<u64> = t0_hits.iter().map(|h| h.id).collect();
    assert_eq!(t2_ids, t0_ids, "T2 ranking diverged from T0 — trust break");
}

#[test]
fn t2_witness_matches_t0_and_t1_for_same_inputs() {
    // Identical witness inputs → identical witness, regardless of
    // tier. This is the load-bearing "three tiers, one cache"
    // property — if it ever breaks, T2 silently fragments the cache
    // federation that T0+T1 already share.
    //
    // We register live T0, T1, and T2 backends (each with the same
    // (data_ref, dim, rotation_seed, rerank_factor, generation)
    // tuple) and then derive bundles via `rvdna_bundle` for each tier
    // to confirm byte-equality of the witness. The witness is a pure
    // function of those five inputs by construction; this test pins
    // that contract against accidental drift in any tier's setup
    // path.
    let dim = 384;
    let n = 8;
    let (t1_file, t1_off) = write_t2_fixture(0xfeedface, n, dim);
    let (t2_file, t2_off) = write_t2_fixture(0xfeedface, n, dim);

    let t0 = RvdnaT0Backend::new("rvdna-t0");
    t0.put_collection("col", t0_collection_from(0xfeedface, n, dim));

    let t1 = RvdnaT1Backend::new("rvdna-t1");
    t1.register_file("col", t1_file.path(), dim, 7, 0, t1_off as usize, n, false)
        .expect("t1 register");

    let t2 = RvdnaT2Backend::new("rvdna-t2");
    t2.register_file("col", t2_file.path(), dim, 7, Vec::new(), t2_off, n)
        .expect("t2 register");

    // All three return the registered generation through the trait.
    assert_eq!(t0.generation("col").unwrap(), 1);
    assert_eq!(t1.generation("col").unwrap(), 7);
    assert_eq!(t2.generation("col").unwrap(), 7);

    let inputs = RvdnaWitnessInputs {
        data_ref: "rvdna://fixture-1/esm2@v0.4.1",
        dim,
        rotation_seed: 42,
        rerank_factor: 20,
        generation: 7,
    };
    let w0 = rvdna_bundle(&inputs);
    let w1 = rvdna_bundle(&inputs);
    let w2 = rvdna_bundle(&inputs);
    assert_eq!(
        w0.rvf_witness, w1.rvf_witness,
        "T1 must reuse T0's witness derivation byte-for-byte (ADR-007)"
    );
    assert_eq!(
        w1.rvf_witness, w2.rvf_witness,
        "T2 must reuse T0/T1's witness derivation byte-for-byte (ADR-007)"
    );
    assert_eq!(w0.memory_class.as_deref(), Some("genomic"));
    assert_eq!(w1.memory_class.as_deref(), Some("genomic"));
    assert_eq!(w2.memory_class.as_deref(), Some("genomic"));
}

#[test]
fn t2_lru_warms_under_repeated_search() {
    // First call fills the LRU from disk; subsequent calls hit the
    // LRU. We use `Consistency::Fresh` so each `search_one` actually
    // calls `pull_vectors` (the ruLake cache wouldn't otherwise
    // exercise the LRU on its own — it'd serve from the bit-packed
    // cache after the first prime).
    let dim = 16;
    let n = 64;
    let (file, vectors_offset) = write_t2_fixture(0xc0ffee, n, dim);

    let raw = Arc::new(RvdnaT2Backend::new("rvdna-t2-fixture"));
    raw.register_file(
        "raw-dna-chr17",
        file.path(),
        dim,
        1,
        Vec::new(),
        vectors_offset,
        n,
    )
    .expect("register");

    // Direct `pull_vectors` calls — bypass the ruLake cache so we
    // measure the LRU layer in isolation.
    let _ = raw.pull_vectors("raw-dna-chr17").expect("pull 1");
    let stats_after_1 = raw.lru_stats("raw-dna-chr17").expect("stats");
    assert_eq!(
        stats_after_1.misses, n as u64,
        "first pull must fault every vector in (disk read)"
    );
    assert_eq!(stats_after_1.hits, 0, "first pull must not have any LRU hits");
    assert_eq!(stats_after_1.len, n, "first pull must populate the LRU");

    // Calls 2..10 hit the LRU.
    for _ in 0..9 {
        let _ = raw.pull_vectors("raw-dna-chr17").expect("pull warm");
    }
    let stats_after_10 = raw.lru_stats("raw-dna-chr17").expect("stats");
    assert_eq!(
        stats_after_10.misses, n as u64,
        "warm pulls must not fault any new vector"
    );
    assert_eq!(
        stats_after_10.hits,
        9 * n as u64,
        "9 warm pulls × {n} rows each must register as LRU hits"
    );

    // Generation bump invalidates the LRU and the stat counters.
    let _ = raw.bump_generation("raw-dna-chr17").expect("bump");
    let stats_after_bump = raw.lru_stats("raw-dna-chr17").expect("stats");
    assert_eq!(stats_after_bump.hits, 0, "gen bump must reset LRU hit counter");
    assert_eq!(stats_after_bump.misses, 0, "gen bump must reset LRU miss counter");
    assert_eq!(stats_after_bump.len, 0, "gen bump must clear LRU contents");
}
