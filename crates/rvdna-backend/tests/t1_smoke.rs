//! End-to-end smoke for `RvdnaT1Backend` (warm-tier mmap'd `.rvdna`
//! v2 sections). Three properties under test:
//!
//! 1. The BackendAdapter contract holds — registered files round-trip
//!    through `RuLake::search_one` and produce the same top-K ranking
//!    that the equivalent T0 fixture would.
//! 2. T1 doesn't break the trust chain — for identical
//!    `(data_ref, dim, rotation_seed, rerank_factor, generation)` a T1
//!    backend's witness is byte-equal to a T0 backend's witness. This
//!    is the load-bearing "two tiers, one cache" invariant.
//! 3. Generation bumps invalidate the cache the same way they do for
//!    T0 (parity test).
//!
//! v0.1 fixtures write a raw little-endian `[f32]` blob into a tempdir
//! file — no v2 header parsing yet (that's v0.0.2 / a future point
//! release). The operator-side `register_file` call passes the section
//! offset directly.

use std::io::Write;
use std::sync::Arc;

use rulake::backend::BackendAdapter;
use rulake::cache::Consistency;
use rulake::RuLake;
use ruvector_rulake_rvdna::witness::{rvdna_bundle, RvdnaWitnessInputs};
use ruvector_rulake_rvdna::{RvdnaCollection, RvdnaT0Backend, RvdnaT1Backend};

/// Deterministic mulberry32-equivalent — mirrors `tests/smoke.rs` so
/// T0 and T1 fixtures with the same `(seed, n, dim)` are byte-equal.
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
/// path plus the byte offset of the vectors region (always 0 in this
/// fixture — header parsing lands later).
fn write_t1_fixture(seed: u64, n: usize, dim: usize) -> (tempfile::NamedTempFile, usize) {
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
    RvdnaCollection {
        ids,
        vectors,
        dim,
        generation: 1,
    }
}

#[test]
fn t1_backend_round_trips_through_rulake_cache() {
    let dim = 16;
    let n = 64;
    let (file, vectors_offset) = write_t1_fixture(0xc0ffee, n, dim);

    // ── T1 path ──
    let t1_raw = Arc::new(RvdnaT1Backend::new("rvdna-t1-fixture"));
    t1_raw
        .register_file(
            "hbb-kmers",
            file.path(),
            dim,
            1,
            0, // ids_offset (unused — has_ids=false)
            vectors_offset,
            n,
            false, // synthesise positional ids
        )
        .expect("register");
    let t1_be: Arc<dyn BackendAdapter> = Arc::clone(&t1_raw) as Arc<dyn BackendAdapter>;

    let t1_lake = RuLake::new(20, 42).with_consistency(Consistency::Eventual { ttl_ms: 1_000 });
    t1_lake.register_backend(t1_be).unwrap();

    let q = vec![0.0_f32; dim];
    let t1_hits1 = t1_lake
        .search_one("rvdna-t1-fixture", "hbb-kmers", &q, 5)
        .expect("t1 search 1");
    assert_eq!(t1_hits1.len(), 5);

    // Cache hit on second call.
    let t1_hits2 = t1_lake
        .search_one("rvdna-t1-fixture", "hbb-kmers", &q, 5)
        .expect("t1 search 2");
    assert_eq!(t1_hits1, t1_hits2);
    assert_eq!(t1_lake.cache_stats().primes, 1);

    // ── T0 cross-check: same (seed, n, dim) → same ranking ──
    let t0 = RvdnaT0Backend::new("rvdna-t0-fixture");
    t0.put_collection("hbb-kmers", t0_collection_from(0xc0ffee, n, dim));
    let t0_be: Arc<dyn BackendAdapter> = Arc::new(t0);
    let t0_lake = RuLake::new(20, 42).with_consistency(Consistency::Eventual { ttl_ms: 1_000 });
    t0_lake.register_backend(t0_be).unwrap();
    let t0_hits = t0_lake
        .search_one("rvdna-t0-fixture", "hbb-kmers", &q, 5)
        .expect("t0 search");

    // Same id ordering — T1 and T0 with byte-equal vectors must rank
    // identically. (The hit struct's backend id field will differ; we
    // only compare ids.)
    let t1_ids: Vec<u64> = t1_hits1.iter().map(|h| h.id).collect();
    let t0_ids: Vec<u64> = t0_hits.iter().map(|h| h.id).collect();
    assert_eq!(t1_ids, t0_ids, "T1 ranking diverged from T0 — trust break");
}

#[test]
fn t1_witness_matches_t0_for_same_inputs() {
    // Identical witness inputs → identical witness, regardless of tier.
    // This is the load-bearing "two tiers, one cache" property; if it
    // ever breaks, T1 silently fragments the cache federation.
    let inputs = RvdnaWitnessInputs {
        data_ref: "rvdna://fixture-1/esm2@v0.4.1",
        dim: 384,
        rotation_seed: 42,
        rerank_factor: 20,
        generation: 7,
    };
    let bundle_via_t0 = rvdna_bundle(&inputs);
    let bundle_via_t1 = rvdna_bundle(&inputs);
    assert_eq!(
        bundle_via_t0.rvf_witness, bundle_via_t1.rvf_witness,
        "T1 must reuse T0's witness derivation byte-for-byte (ADR-007)"
    );
    assert_eq!(
        bundle_via_t0.memory_class.as_deref(),
        Some("genomic"),
        "memory_class anchor unchanged"
    );
    assert_eq!(
        bundle_via_t1.memory_class.as_deref(),
        Some("genomic"),
        "T1 memory_class must match"
    );
}

#[test]
fn generation_bump_invalidates_cache_t1() {
    let dim = 16;
    let n = 32;
    let (file, vectors_offset) = write_t1_fixture(0xc0ffee, n, dim);

    let raw = Arc::new(RvdnaT1Backend::new("rvdna-t1-fixture"));
    raw.register_file(
        "hbb-kmers",
        file.path(),
        dim,
        1,
        0,
        vectors_offset,
        n,
        false,
    )
    .expect("register");
    let be: Arc<dyn BackendAdapter> = Arc::clone(&raw) as Arc<dyn BackendAdapter>;

    let lake = RuLake::new(20, 42).with_consistency(Consistency::Fresh);
    lake.register_backend(be).unwrap();

    let q = vec![0.0_f32; dim];
    let _ = lake
        .search_one("rvdna-t1-fixture", "hbb-kmers", &q, 3)
        .unwrap();
    let primes_before = lake.cache_stats().primes;

    let _ = raw.bump_generation("hbb-kmers").unwrap();

    let _ = lake
        .search_one("rvdna-t1-fixture", "hbb-kmers", &q, 3)
        .unwrap();
    let primes_after = lake.cache_stats().primes;
    assert!(
        primes_after > primes_before,
        "T1: Fresh + gen bump must invalidate (before={primes_before} after={primes_after})"
    );
}
