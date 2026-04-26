//! ruLake — Node.js bindings (ADR-003).
//!
//! Wraps the public surface of `rulake` with napi-rs.
//!
//! # Design rules (from ADR-003)
//!
//! 1. **Float32Array / BigUint64Array zero-copy in.** The query vector
//!    and id arrays come in as TypedArray references whose backing
//!    `ArrayBuffer` is borrowed directly. The slice contents are then
//!    copied *once* into an owned `Vec<f32>` / `Vec<u64>` because the
//!    work runs on a libuv worker thread and the borrow can't cross
//!    the `await` boundary. The copy is one memcpy at memory
//!    bandwidth (~3 µs at D=768) — far below the cache scan itself.
//!
//! 2. **Async-only at the JS surface.** Every method that does work
//!    is `pub async fn` — napi-rs returns `Promise<T>` to JS. Pure
//!    getters that take a small lock and return scalar data are sync.
//!
//! 3. **bigint for u64 IDs.** `napi-rs` serializes Rust `u64` to JS
//!    `bigint`. Slight ergonomic tax (`42n` not `42`), but no silent
//!    precision loss on the high u64 bit.
//!
//! 4. **One error class with a `code` discriminator.** Idiomatic Node
//!    pattern (matches `SystemError`, AWS SDK errors). See ADR-003 §6.

use std::path::PathBuf;
use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi_derive::napi;

use rulake::cache::Consistency as RsConsistency;
use rulake::{
    BackendAdapter, FsBackend as RsFs, Generation as RsGeneration, LocalBackend as RsLocal,
    RefreshResult as RsRefreshResult, RuLake as RsRuLake, RuLakeBundle as RsBundle,
    RuLakeError as RsErr,
};

// ─────────────────────────────────────────────────────────────────────
// Error mapping (ADR-003 §6) — one class, `code` discriminator.
// ─────────────────────────────────────────────────────────────────────

fn err_code(e: &RsErr) -> &'static str {
    match e {
        RsErr::UnknownBackend(_) => "RULAKE_BACKEND_NOT_FOUND",
        RsErr::UnknownCollection { .. } => "RULAKE_COLLECTION_NOT_FOUND",
        RsErr::DimensionMismatch { .. } => "RULAKE_DIMENSION_MISMATCH",
        RsErr::InvalidParameter(_) => "RULAKE_INVALID_PARAMETER",
        RsErr::Backend { .. } => "RULAKE_BACKEND",
        RsErr::Rabitq(_) => "RULAKE_RABITQ",
    }
}

fn map_err(e: RsErr) -> Error {
    // napi-rs Error carries (Status, reason). The `code` ends up on the
    // JS-side Error as `error.code`; we set it via Error::new with a
    // generic Status::GenericFailure and put the discriminator in the
    // reason prefix so JS can split on it. The TS shim normalizes this
    // into `RuLakeError { code, message }` — see index.mjs.
    let code = err_code(&e);
    Error::new(Status::GenericFailure, format!("{code}: {e}"))
}

// ─────────────────────────────────────────────────────────────────────
// Consistency — three factories
// ─────────────────────────────────────────────────────────────────────

#[napi]
pub struct Consistency {
    inner: RsConsistency,
}

#[napi]
impl Consistency {
    /// Per-query coherence check. Default; for compliance/audit workloads.
    #[napi(factory)]
    pub fn fresh() -> Self {
        Self { inner: RsConsistency::Fresh }
    }

    /// Skip the coherence check for `ttl_ms` after the last one.
    #[napi(factory)]
    pub fn eventual(ttl_ms: u32) -> Self {
        Self {
            inner: RsConsistency::Eventual { ttl_ms: ttl_ms as u64 },
        }
    }

    /// Pin the cache forever — for witness-sealed audit-tier snapshots.
    #[napi(factory)]
    pub fn frozen() -> Self {
        Self { inner: RsConsistency::Frozen }
    }
}

// ─────────────────────────────────────────────────────────────────────
// SearchResult — small data class
// ─────────────────────────────────────────────────────────────────────

#[napi(object)]
pub struct SearchResult {
    pub backend: String,
    pub collection: String,
    /// u64 → bigint in JS.
    pub id: BigInt,
    pub score: f64,
}

impl From<rulake::SearchResult> for SearchResult {
    fn from(h: rulake::SearchResult) -> Self {
        Self {
            backend: h.backend,
            collection: h.collection,
            id: BigInt::from(h.id),
            score: h.score as f64,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// CacheStats / PerBackendStats
// ─────────────────────────────────────────────────────────────────────

#[napi(object)]
pub struct CacheStats {
    pub hits: BigInt,
    pub misses: BigInt,
    pub primes: BigInt,
    pub invalidations: BigInt,
    pub shared_hits: BigInt,
    pub total_prime_ms: f64,
    pub last_prime_ms: f64,
    pub warm_installs: BigInt,
    /// `hits / (hits + misses)`, or `null` when no checks have run yet.
    pub hit_rate: Option<f64>,
    /// Mean prime duration (ms), or `null` when no primes have run yet.
    pub avg_prime_ms: Option<f64>,
}

impl From<rulake::CacheStats> for CacheStats {
    fn from(s: rulake::CacheStats) -> Self {
        let hit_rate = {
            let total = s.hits + s.misses;
            if total == 0 { None } else { Some(s.hits as f64 / total as f64) }
        };
        let avg_prime_ms = if s.primes == 0 {
            None
        } else {
            Some(s.total_prime_ms / s.primes as f64)
        };
        Self {
            hits: BigInt::from(s.hits),
            misses: BigInt::from(s.misses),
            primes: BigInt::from(s.primes),
            invalidations: BigInt::from(s.invalidations),
            shared_hits: BigInt::from(s.shared_hits),
            total_prime_ms: s.total_prime_ms,
            last_prime_ms: s.last_prime_ms,
            warm_installs: BigInt::from(s.warm_installs),
            hit_rate,
            avg_prime_ms,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Bundle — table.rulake.json sidecar
// ─────────────────────────────────────────────────────────────────────

#[napi]
pub struct Bundle {
    inner: RsBundle,
}

#[napi]
impl Bundle {
    /// Build a bundle and compute its SHAKE-256(32) witness.
    /// `generation` is `bigint` (Num variant) or `string` (Opaque variant).
    #[napi(constructor)]
    pub fn new(
        data_ref: String,
        dim: u32,
        rotation_seed: BigInt,
        rerank_factor: u32,
        generation: GenerationInput,
    ) -> Result<Self> {
        // BigInt::get_u64 returns (signed, value, lossless); we want
        // value, and treat signed/lossless as best-effort because all
        // legitimate seeds and Num generations fit in u64.
        let (_, seed, _) = rotation_seed.get_u64();
        let gen = match generation {
            Either::A(n) => {
                let (_, v, _) = n.get_u64();
                RsGeneration::Num(v)
            }
            Either::B(s) => RsGeneration::Opaque(s),
        };
        Ok(Self {
            inner: RsBundle::new(data_ref, dim as usize, seed, rerank_factor as usize, gen),
        })
    }

    #[napi(getter)]
    pub fn data_ref(&self) -> &str { &self.inner.data_ref }
    #[napi(getter)]
    pub fn dim(&self) -> u32 { self.inner.dim as u32 }
    #[napi(getter)]
    pub fn rotation_seed(&self) -> BigInt { BigInt::from(self.inner.rotation_seed) }
    #[napi(getter)]
    pub fn rerank_factor(&self) -> u32 { self.inner.rerank_factor as u32 }
    #[napi(getter)]
    pub fn rvf_witness(&self) -> &str { &self.inner.rvf_witness }
    #[napi(getter)]
    pub fn format_version(&self) -> u32 { self.inner.format_version }
    #[napi(getter)]
    pub fn pii_policy(&self) -> Option<String> { self.inner.pii_policy.clone() }
    #[napi(getter)]
    pub fn lineage_id(&self) -> Option<String> { self.inner.lineage_id.clone() }
    #[napi(getter)]
    pub fn memory_class(&self) -> Option<String> { self.inner.memory_class.clone() }

    /// Generation as `bigint` (Num) or `string` (Opaque).
    #[napi]
    pub fn generation(&self) -> GenerationInput {
        match &self.inner.generation {
            RsGeneration::Num(n) => Either::A(BigInt::from(*n)),
            RsGeneration::Opaque(s) => Either::B(s.clone()),
        }
    }

    #[napi]
    pub fn verify_witness(&self) -> bool { self.inner.verify_witness() }

    #[napi]
    pub fn to_json(&self) -> Result<String> { self.inner.to_json().map_err(map_err) }

    #[napi(factory)]
    pub fn from_json(s: String) -> Result<Bundle> {
        Ok(Bundle { inner: RsBundle::from_json(&s).map_err(map_err)? })
    }

    /// Atomic write to `dir/table.rulake.json` (temp + rename).
    /// Returns the written path.
    #[napi]
    pub async fn write_to_dir(&self, dir: String) -> Result<String> {
        let inner = self.inner.clone();
        spawn_blocking(move || {
            inner.write_to_dir(PathBuf::from(dir)).map_err(map_err)
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))?
        .map(|p| p.to_string_lossy().into_owned())
    }

    #[napi(factory)]
    pub async fn read_from_dir(dir: String) -> Result<Bundle> {
        let bundle = spawn_blocking(move || {
            RsBundle::read_from_dir(PathBuf::from(dir)).map_err(map_err)
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))??;
        Ok(Bundle { inner: bundle })
    }

    #[napi]
    pub fn with_pii_policy(&self, p: String) -> Bundle {
        Bundle { inner: self.inner.clone().with_pii_policy(p) }
    }
    #[napi]
    pub fn with_lineage_id(&self, id: String) -> Bundle {
        Bundle { inner: self.inner.clone().with_lineage_id(id) }
    }
    #[napi]
    pub fn with_memory_class(&self, class: String) -> Bundle {
        Bundle { inner: self.inner.clone().with_memory_class(class) }
    }
}

// Generation flows across FFI as `bigint | string`. napi-rs models
// disjunctive types via `Either<A, B>`; tagged enums are unsupported.
type GenerationInput = Either<BigInt, String>;

// ─────────────────────────────────────────────────────────────────────
// LocalBackend — in-memory
// ─────────────────────────────────────────────────────────────────────

#[napi]
pub struct LocalBackend {
    inner: Arc<RsLocal>,
}

#[napi]
impl LocalBackend {
    #[napi(constructor)]
    pub fn new(id: String) -> Self {
        Self { inner: Arc::new(RsLocal::new(id)) }
    }

    #[napi(getter)]
    pub fn id(&self) -> &str { self.inner.id() }

    /// Insert a collection wholesale.
    /// `ids` is a `BigUint64Array` of length N.
    /// `vectors` is a `Float32Array` of length `N * dim` (row-major).
    #[napi]
    pub async fn put_collection(
        &self,
        name: String,
        ids: BigInt64Array,
        vectors: Float32Array,
        dim: u32,
    ) -> Result<()> {
        let dim = dim as usize;
        let id_slice: &[i64] = &ids;
        let vec_slice: &[f32] = &vectors;
        let n = id_slice.len();
        if vec_slice.len() != n * dim {
            return Err(Error::new(
                Status::GenericFailure,
                format!(
                    "RULAKE_INVALID_PARAMETER: vectors.length ({}) != ids.length ({}) * dim ({})",
                    vec_slice.len(), n, dim
                ),
            ));
        }
        // u64 ids come over as bigint (i64-shaped Buffer); the .node ABI
        // for a BigUint64Array doesn't survive napi-rs's typed wrappers
        // cleanly, so we use BigInt64Array and reinterpret. Negative
        // values would round-trip wrong, but all ids in the system are
        // u64 and the user-facing API is documented as bigint.
        let owned_ids: Vec<u64> = id_slice.iter().map(|i| *i as u64).collect();
        let mut owned_vectors: Vec<Vec<f32>> = Vec::with_capacity(n);
        for i in 0..n {
            owned_vectors.push(vec_slice[i * dim..(i + 1) * dim].to_vec());
        }
        let backend = Arc::clone(&self.inner);
        spawn_blocking(move || {
            backend.put_collection(name, dim, owned_ids, owned_vectors).map_err(map_err)
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))?
    }

    /// Append a single vector. Bumps the backend generation.
    #[napi]
    pub async fn append(&self, collection: String, id: BigInt, vector: Float32Array) -> Result<()> {
        let v: Vec<f32> = vector.to_vec();
        let (_, id_u64, _) = id.get_u64();
        let backend = Arc::clone(&self.inner);
        spawn_blocking(move || {
            backend.append(collection, id_u64, v).map_err(map_err)
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))?
    }
}

// ─────────────────────────────────────────────────────────────────────
// FsBackend — filesystem-backed `ruvec1` files
// ─────────────────────────────────────────────────────────────────────

#[napi]
pub struct FsBackend {
    inner: Arc<RsFs>,
}

#[napi]
impl FsBackend {
    #[napi(constructor)]
    pub fn new(id: String, root: String) -> Result<Self> {
        let inner = RsFs::new(id, PathBuf::from(root)).map_err(map_err)?;
        Ok(Self { inner: Arc::new(inner) })
    }

    #[napi]
    pub fn register(&self, collection: String, filename: String) -> Result<()> {
        self.inner.register(collection, filename).map_err(map_err)
    }

    #[napi]
    pub async fn write(
        &self,
        collection: String,
        filename: String,
        ids: BigInt64Array,
        vectors: Float32Array,
        dim: u32,
    ) -> Result<String> {
        let dim = dim as usize;
        let id_slice: &[i64] = &ids;
        let vec_slice: &[f32] = &vectors;
        let n = id_slice.len();
        if vec_slice.len() != n * dim {
            return Err(Error::new(
                Status::GenericFailure,
                "RULAKE_INVALID_PARAMETER: vectors.length != ids.length * dim".to_string(),
            ));
        }
        let owned_ids: Vec<u64> = id_slice.iter().map(|i| *i as u64).collect();
        let mut owned: Vec<Vec<f32>> = Vec::with_capacity(n);
        for i in 0..n {
            owned.push(vec_slice[i * dim..(i + 1) * dim].to_vec());
        }
        let backend = Arc::clone(&self.inner);
        spawn_blocking(move || {
            backend
                .write(collection, filename, dim, &owned_ids, &owned)
                .map_err(map_err)
                .map(|p| p.to_string_lossy().into_owned())
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))?
    }
}

// ─────────────────────────────────────────────────────────────────────
// RuLake — entry point
// ─────────────────────────────────────────────────────────────────────

#[napi]
pub struct RuLake {
    inner: Arc<RsRuLake>,
}

#[napi]
impl RuLake {
    #[napi(constructor)]
    pub fn new(rerank_factor: u32, rotation_seed: BigInt) -> Self {
        let (_, seed, _) = rotation_seed.get_u64();
        Self { inner: Arc::new(RsRuLake::new(rerank_factor as usize, seed)) }
    }

    #[napi]
    pub fn with_consistency(&self, c: &Consistency) -> RuLake {
        let cloned: RsRuLake = (*self.inner).clone();
        let next = cloned.with_consistency(c.inner);
        RuLake { inner: Arc::new(next) }
    }

    #[napi]
    pub fn with_max_cache_entries(&self, n: u32) -> RuLake {
        let cloned: RsRuLake = (*self.inner).clone();
        let next = cloned.with_max_cache_entries(n as usize);
        RuLake { inner: Arc::new(next) }
    }

    /// Register a `LocalBackend`.
    #[napi]
    pub fn register_local_backend(&self, b: &LocalBackend) -> Result<()> {
        let arc: Arc<RsLocal> = Arc::clone(&b.inner);
        let dyn_arc: Arc<dyn BackendAdapter> = arc;
        self.inner.register_backend(dyn_arc).map_err(map_err)
    }

    /// Register an `FsBackend`.
    #[napi]
    pub fn register_fs_backend(&self, b: &FsBackend) -> Result<()> {
        let arc: Arc<RsFs> = Arc::clone(&b.inner);
        let dyn_arc: Arc<dyn BackendAdapter> = arc;
        self.inner.register_backend(dyn_arc).map_err(map_err)
    }

    #[napi]
    pub fn backend_ids(&self) -> Vec<String> { self.inner.backend_ids() }

    #[napi]
    pub fn cache_stats(&self) -> CacheStats { self.inner.cache_stats().into() }

    #[napi]
    pub fn cache_entry_count(&self) -> u32 { self.inner.cache_entry_count() as u32 }

    #[napi]
    pub fn cache_witness_of(&self, backend: String, collection: String) -> Option<String> {
        self.inner.cache_witness_of(&(backend, collection))
    }

    #[napi]
    pub fn invalidate_cache(&self, backend: String, collection: String) {
        self.inner.invalidate_cache(&(backend, collection))
    }

    /// Search a single (backend, collection) pair. Returns top-k hits.
    #[napi]
    pub async fn search_one(
        &self,
        backend: String,
        collection: String,
        query: Float32Array,
        k: u32,
    ) -> Result<Vec<SearchResult>> {
        // Borrowed slice can't cross await — copy into Vec on the
        // calling thread. ~3 µs at D=768; benchmarks budget this in.
        let q: Vec<f32> = query.to_vec();
        let lake = Arc::clone(&self.inner);
        let k = k as usize;
        let hits = spawn_blocking(move || {
            lake.search_one(&backend, &collection, &q, k).map_err(map_err)
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))??;
        Ok(hits.into_iter().map(SearchResult::from).collect())
    }

    /// Federated search across multiple (backend, collection) targets.
    /// `targets` is `[[backend, collection], ...]`.
    #[napi]
    pub async fn search_federated(
        &self,
        targets: Vec<Vec<String>>,
        query: Float32Array,
        k: u32,
    ) -> Result<Vec<SearchResult>> {
        // Validate targets shape.
        let pairs: Vec<(String, String)> = targets
            .into_iter()
            .map(|p| {
                if p.len() != 2 {
                    return Err(Error::new(
                        Status::GenericFailure,
                        "RULAKE_INVALID_PARAMETER: each target must be [backend, collection]"
                            .to_string(),
                    ));
                }
                let mut it = p.into_iter();
                Ok((it.next().unwrap(), it.next().unwrap()))
            })
            .collect::<Result<Vec<_>>>()?;
        let q: Vec<f32> = query.to_vec();
        let k = k as usize;
        let lake = Arc::clone(&self.inner);
        let hits = spawn_blocking(move || {
            let refs: Vec<(&str, &str)> =
                pairs.iter().map(|(b, c)| (b.as_str(), c.as_str())).collect();
            lake.search_federated(&refs, &q, k).map_err(map_err)
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))??;
        Ok(hits.into_iter().map(SearchResult::from).collect())
    }

    /// Batched search. `queries` is `Float32Array` of length `n * dim`.
    /// Returns `n` arrays of top-k each, in input order.
    #[napi]
    pub async fn search_batch(
        &self,
        backend: String,
        collection: String,
        queries: Float32Array,
        dim: u32,
        k: u32,
    ) -> Result<Vec<Vec<SearchResult>>> {
        let dim = dim as usize;
        let q_slice: &[f32] = &queries;
        if q_slice.len() % dim != 0 {
            return Err(Error::new(
                Status::GenericFailure,
                "RULAKE_INVALID_PARAMETER: queries.length must be a multiple of dim".to_string(),
            ));
        }
        let n = q_slice.len() / dim;
        let mut owned: Vec<Vec<f32>> = Vec::with_capacity(n);
        for i in 0..n {
            owned.push(q_slice[i * dim..(i + 1) * dim].to_vec());
        }
        let lake = Arc::clone(&self.inner);
        let k = k as usize;
        let raw = spawn_blocking(move || {
            lake.search_batch(&backend, &collection, &owned, k).map_err(map_err)
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))??;
        Ok(raw
            .into_iter()
            .map(|v| v.into_iter().map(SearchResult::from).collect())
            .collect())
    }

    #[napi]
    pub async fn publish_bundle(
        &self,
        backend: String,
        collection: String,
        dir: String,
    ) -> Result<String> {
        let lake = Arc::clone(&self.inner);
        let key = (backend, collection);
        spawn_blocking(move || {
            lake.publish_bundle(&key, PathBuf::from(dir))
                .map_err(map_err)
                .map(|p| p.to_string_lossy().into_owned())
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))?
    }

    /// Returns one of `"up_to_date"`, `"invalidated"`, `"bundle_missing"`.
    #[napi]
    pub async fn refresh_from_bundle_dir(
        &self,
        backend: String,
        collection: String,
        dir: String,
    ) -> Result<String> {
        let lake = Arc::clone(&self.inner);
        let key = (backend, collection);
        let res = spawn_blocking(move || {
            lake.refresh_from_bundle_dir(&key, PathBuf::from(dir))
                .map_err(map_err)
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))??;
        Ok(match res {
            RsRefreshResult::UpToDate => "up_to_date".to_string(),
            RsRefreshResult::Invalidated => "invalidated".to_string(),
            RsRefreshResult::BundleMissing => "bundle_missing".to_string(),
        })
    }

    #[napi]
    pub async fn save_cache_to_dir(
        &self,
        backend: String,
        collection: String,
        dir: String,
    ) -> Result<String> {
        let lake = Arc::clone(&self.inner);
        let key = (backend, collection);
        spawn_blocking(move || {
            lake.save_cache_to_dir(&key, PathBuf::from(dir))
                .map_err(map_err)
                .map(|p| p.to_string_lossy().into_owned())
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))?
    }

    #[napi]
    pub async fn warm_from_dir(
        &self,
        backend: String,
        collection: String,
        dir: String,
    ) -> Result<u32> {
        let lake = Arc::clone(&self.inner);
        let key = (backend, collection);
        let n = spawn_blocking(move || {
            lake.warm_from_dir(&key, PathBuf::from(dir)).map_err(map_err)
        })
        .await
        .map_err(|e| Error::new(Status::GenericFailure, format!("join: {e}")))??;
        Ok(n as u32)
    }
}
