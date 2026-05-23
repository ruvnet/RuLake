//! End-to-end smoke for `RvdnaT0Backend`. Confirms the BackendAdapter
//! contract holds: registered collections enumerate, pull_vectors round-
//! trips through ruLake, and a generation bump invalidates the cache.

use std::sync::Arc;

use rulake::backend::BackendAdapter;
use rulake::cache::Consistency;
use rulake::RuLake;
use ruvector_rulake_rvdna::{RvdnaCollection, RvdnaT0Backend};

fn fixture_collection(seed: u64, n: usize, dim: usize) -> RvdnaCollection {
    // Tiny deterministic fixture — mulberry32-equivalent.
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
    let ids: Vec<u64> = (0..n as u64).collect();
    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|_| (0..dim).map(|_| next() * 2.0 - 1.0).collect())
        .collect();
    RvdnaCollection {
        ids,
        vectors,
        dim,
        generation: 1,
    }
}

#[test]
fn t0_backend_adapter_contract_holds() {
    let be = RvdnaT0Backend::new("rvdna-fixture");
    be.put_collection("hbb-kmers", fixture_collection(0xc0ffee, 32, 16));
    be.put_collection("brca1-kmers", fixture_collection(0xdeadbeef, 24, 16));

    // id
    assert_eq!(be.id(), "rvdna-fixture");

    // list_collections
    let mut cols = be.list_collections().unwrap();
    cols.sort();
    assert_eq!(
        cols,
        vec!["brca1-kmers".to_string(), "hbb-kmers".to_string()]
    );

    // pull_vectors
    let batch = be.pull_vectors("hbb-kmers").unwrap();
    assert_eq!(batch.collection, "hbb-kmers");
    assert_eq!(batch.dim, 16);
    assert_eq!(batch.ids.len(), 32);
    assert_eq!(batch.vectors.len(), 32);
    assert_eq!(batch.vectors[0].len(), 16);
    assert_eq!(batch.generation, 1);

    // generation
    assert_eq!(be.generation("brca1-kmers").unwrap(), 1);
}

#[test]
fn t0_backend_round_trips_through_rulake_cache() {
    let be: Arc<dyn BackendAdapter> = Arc::new({
        let b = RvdnaT0Backend::new("rvdna-fixture");
        b.put_collection("hbb-kmers", fixture_collection(0xc0ffee, 64, 16));
        b
    });
    let lake = RuLake::new(20, 42).with_consistency(Consistency::Eventual { ttl_ms: 1_000 });
    lake.register_backend(Arc::clone(&be)).unwrap();

    // First search primes from the backend.
    let q = vec![0.0_f32; 16];
    let hits1 = lake
        .search_one("rvdna-fixture", "hbb-kmers", &q, 5)
        .unwrap();
    assert_eq!(hits1.len(), 5, "got {} hits", hits1.len());

    // Second search hits the cache (faster, but we just verify it
    // returns the same byte-exact ranking).
    let hits2 = lake
        .search_one("rvdna-fixture", "hbb-kmers", &q, 5)
        .unwrap();
    assert_eq!(hits1, hits2);

    let stats = lake.cache_stats();
    assert_eq!(stats.primes, 1, "second call must hit the cache");
}

#[test]
fn generation_bump_invalidates_cache() {
    // Production pattern: keep a concrete handle alongside the
    // BackendAdapter trait object so operators can call ops the trait
    // doesn't expose (here, bump_generation). `Arc<RvdnaT0Backend>`
    // upcasts via Arc::clone to satisfy the BackendAdapter slot.
    let raw = Arc::new({
        let b = RvdnaT0Backend::new("rvdna-fixture");
        b.put_collection("hbb-kmers", fixture_collection(0xc0ffee, 32, 16));
        b
    });
    let be: Arc<dyn BackendAdapter> = Arc::clone(&raw) as Arc<dyn BackendAdapter>;

    let lake = RuLake::new(20, 42).with_consistency(Consistency::Fresh);
    lake.register_backend(be).unwrap();

    let q = vec![0.0_f32; 16];
    let _ = lake
        .search_one("rvdna-fixture", "hbb-kmers", &q, 3)
        .unwrap();
    let primes_before = lake.cache_stats().primes;

    // Bump the underlying generation via the concrete handle. Next
    // Fresh search must re-prime.
    let _ = raw.bump_generation("hbb-kmers").unwrap();

    let _ = lake
        .search_one("rvdna-fixture", "hbb-kmers", &q, 3)
        .unwrap();
    let primes_after = lake.cache_stats().primes;
    assert!(
        primes_after > primes_before,
        "Fresh + gen bump must invalidate"
    );
}
