//! ruLake — Python bindings (ADR-002).
//!
//! Wraps the public surface of `rulake` with PyO3 + numpy.
//!
//! # Design rules (from ADR-002)
//!
//! 1. **Zero-copy NumPy.** Every vector parameter takes
//!    `PyReadonlyArray1<f32>` / `PyReadonlyArray2<f32>` and calls
//!    `.as_slice()?`. Non-contiguous or wrong-dtype arrays produce a
//!    *visible* error rather than a silent copy — silent copies hide
//!    the regressions this binding exists to prevent.
//!
//! 2. **GIL release on every hot path.** Search, prime, save, warm,
//!    publish, refresh — anything that takes more than ~50 µs runs
//!    inside `py.allow_threads(...)`. Borrowed slices are converted
//!    to owned `Vec<f32>` *only when the borrow can't outlive the
//!    closure* (rare); the common case borrows directly into the
//!    GIL-released closure because `&[f32]` from a NumPy array is
//!    pinned for the call's duration.
//!
//! 3. **Errors map to a typed hierarchy** rooted at `RuLakeError`,
//!    with stdlib multi-inheritance (`LookupError`, `ValueError`,
//!    `OSError`) so `except KeyError:` / `except ValueError:` works
//!    on idiomatic Python code without learning ruLake-specific
//!    classes.

use std::path::PathBuf;
use std::sync::Arc;

use numpy::{PyArray1, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::create_exception;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyType};

use rulake::cache::Consistency as RsConsistency;
use rulake::{
    BackendAdapter, CacheStats as RsCacheStats, FsBackend as RsFs, Generation as RsGeneration,
    LocalBackend as RsLocal, PerBackendStats as RsPerBackend, RefreshResult as RsRefreshResult,
    RuLake as RsRuLake, RuLakeBundle as RsBundle, RuLakeError as RsErr, SearchResult as RsHit,
};

// ─────────────────────────────────────────────────────────────────────
// Exceptions  (ADR-002 §6)
// ─────────────────────────────────────────────────────────────────────

create_exception!(_rulake, RuLakeError, pyo3::exceptions::PyException);
create_exception!(_rulake, BackendNotFoundError, RuLakeError);
create_exception!(_rulake, CollectionNotFoundError, RuLakeError);
create_exception!(_rulake, DimensionMismatchError, RuLakeError);
create_exception!(_rulake, InvalidParameterError, RuLakeError);
create_exception!(_rulake, BackendError, RuLakeError);

/// Map a Rust `RuLakeError` to a typed Python exception.
///
/// We pick the most-specific subclass and let it inherit from
/// `RuLakeError`. The stdlib base classes (`LookupError`, `ValueError`,
/// `OSError`) are layered in via the pure-Python `__init__.py` — PyO3's
/// `create_exception!` macro doesn't support multi-inheritance directly,
/// so the Python wrapper rebinds the classes to the multi-inherited
/// versions on import.
fn map_err(e: RsErr) -> PyErr {
    let msg = e.to_string();
    match e {
        RsErr::UnknownBackend(_) => BackendNotFoundError::new_err(msg),
        RsErr::UnknownCollection { .. } => CollectionNotFoundError::new_err(msg),
        RsErr::DimensionMismatch { .. } => DimensionMismatchError::new_err(msg),
        RsErr::InvalidParameter(_) => InvalidParameterError::new_err(msg),
        RsErr::Backend { .. } => BackendError::new_err(msg),
        RsErr::Rabitq(_) => RuLakeError::new_err(msg),
    }
}

// ─────────────────────────────────────────────────────────────────────
// Consistency  (ADR-155 / cache.rs §Consistency)
// ─────────────────────────────────────────────────────────────────────

/// Consistency knob: `Consistency.fresh()`, `Consistency.eventual(ttl_ms)`,
/// `Consistency.frozen()`. Rust's tagged enum doesn't translate to a
/// clean Python class, so we expose factory classmethods + a tag/ttl
/// pair internally.
#[pyclass(name = "Consistency", module = "rulake._rulake", frozen)]
#[derive(Clone, Copy)]
struct PyConsistency {
    inner: RsConsistency,
}

#[pymethods]
impl PyConsistency {
    /// Per-query coherence check. Strongest staleness guarantee, lowest
    /// QPS on remote backends.
    #[classmethod]
    fn fresh(_cls: &Bound<'_, PyType>) -> Self {
        Self { inner: RsConsistency::Fresh }
    }

    /// Skip the coherence check for `ttl_ms` after the last one.
    /// Recommended for RAG / search / recommendation workloads.
    #[classmethod]
    fn eventual(_cls: &Bound<'_, PyType>, ttl_ms: u64) -> Self {
        Self { inner: RsConsistency::Eventual { ttl_ms } }
    }

    /// Pin the cache forever — never re-check the backend. For
    /// witness-sealed audit-tier snapshots.
    #[classmethod]
    fn frozen(_cls: &Bound<'_, PyType>) -> Self {
        Self { inner: RsConsistency::Frozen }
    }

    fn __repr__(&self) -> String {
        match self.inner {
            RsConsistency::Fresh => "Consistency.fresh()".to_string(),
            RsConsistency::Eventual { ttl_ms } => format!("Consistency.eventual({ttl_ms})"),
            RsConsistency::Frozen => "Consistency.frozen()".to_string(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// SearchResult — small data class
// ─────────────────────────────────────────────────────────────────────

#[pyclass(name = "SearchResult", module = "rulake._rulake", frozen)]
#[derive(Clone)]
struct PySearchResult {
    #[pyo3(get)]
    backend: String,
    #[pyo3(get)]
    collection: String,
    #[pyo3(get)]
    id: u64,
    #[pyo3(get)]
    score: f32,
}

impl From<RsHit> for PySearchResult {
    fn from(h: RsHit) -> Self {
        Self {
            backend: h.backend,
            collection: h.collection,
            id: h.id,
            score: h.score,
        }
    }
}

#[pymethods]
impl PySearchResult {
    fn __repr__(&self) -> String {
        format!(
            "SearchResult(backend={:?}, collection={:?}, id={}, score={:.6})",
            self.backend, self.collection, self.id, self.score
        )
    }

    /// Tuple form for unpacking: `(backend, collection, id, score)`.
    fn as_tuple(&self) -> (String, String, u64, f32) {
        (self.backend.clone(), self.collection.clone(), self.id, self.score)
    }
}

// ─────────────────────────────────────────────────────────────────────
// CacheStats / PerBackendStats — frozen value classes
// ─────────────────────────────────────────────────────────────────────

#[pyclass(name = "CacheStats", module = "rulake._rulake", frozen)]
#[derive(Clone)]
struct PyCacheStats {
    #[pyo3(get)] hits: u64,
    #[pyo3(get)] misses: u64,
    #[pyo3(get)] primes: u64,
    #[pyo3(get)] invalidations: u64,
    #[pyo3(get)] shared_hits: u64,
    #[pyo3(get)] total_prime_ms: f64,
    #[pyo3(get)] last_prime_ms: f64,
    #[pyo3(get)] warm_installs: u64,
}

impl From<RsCacheStats> for PyCacheStats {
    fn from(s: RsCacheStats) -> Self {
        Self {
            hits: s.hits,
            misses: s.misses,
            primes: s.primes,
            invalidations: s.invalidations,
            shared_hits: s.shared_hits,
            total_prime_ms: s.total_prime_ms,
            last_prime_ms: s.last_prime_ms,
            warm_installs: s.warm_installs,
        }
    }
}

#[pymethods]
impl PyCacheStats {
    /// `hits / (hits + misses)`, or `None` if no checked searches yet.
    /// The headline cache-first KPI from ADR-155 §M1.5 (target ≥ 0.95).
    fn hit_rate(&self) -> Option<f64> {
        let total = self.hits + self.misses;
        if total == 0 {
            None
        } else {
            Some(self.hits as f64 / total as f64)
        }
    }

    /// Mean prime duration (ms), or `None` if no primes have run.
    fn avg_prime_ms(&self) -> Option<f64> {
        if self.primes == 0 {
            None
        } else {
            Some(self.total_prime_ms / self.primes as f64)
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "CacheStats(hits={}, misses={}, primes={}, hit_rate={:?})",
            self.hits, self.misses, self.primes, self.hit_rate()
        )
    }
}

#[pyclass(name = "PerBackendStats", module = "rulake._rulake", frozen)]
#[derive(Clone)]
struct PyPerBackendStats {
    #[pyo3(get)] hits: u64,
    #[pyo3(get)] misses: u64,
    #[pyo3(get)] primes: u64,
    #[pyo3(get)] invalidations: u64,
    #[pyo3(get)] shared_hits: u64,
}

impl From<RsPerBackend> for PyPerBackendStats {
    fn from(s: RsPerBackend) -> Self {
        Self {
            hits: s.hits,
            misses: s.misses,
            primes: s.primes,
            invalidations: s.invalidations,
            shared_hits: s.shared_hits,
        }
    }
}

#[pymethods]
impl PyPerBackendStats {
    fn hit_rate(&self) -> Option<f64> {
        let total = self.hits + self.misses;
        if total == 0 { None } else { Some(self.hits as f64 / total as f64) }
    }
}

// ─────────────────────────────────────────────────────────────────────
// RuLakeBundle — table.rulake.json sidecar (witness-anchored)
// ─────────────────────────────────────────────────────────────────────

#[pyclass(name = "Bundle", module = "rulake._rulake", frozen)]
struct PyBundle {
    inner: RsBundle,
}

#[pymethods]
impl PyBundle {
    /// Build a bundle and compute its SHAKE-256(32) witness from the
    /// fields. `generation` may be an `int` (Num variant) or a `str`
    /// (Opaque variant).
    #[new]
    #[pyo3(signature = (data_ref, dim, rotation_seed, rerank_factor, generation))]
    fn new(
        data_ref: &str,
        dim: usize,
        rotation_seed: u64,
        rerank_factor: usize,
        generation: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        let gen = if let Ok(n) = generation.extract::<u64>() {
            RsGeneration::Num(n)
        } else if let Ok(s) = generation.extract::<String>() {
            RsGeneration::Opaque(s)
        } else {
            return Err(PyValueError::new_err(
                "generation must be int (Num) or str (Opaque)",
            ));
        };
        Ok(Self {
            inner: RsBundle::new(data_ref, dim, rotation_seed, rerank_factor, gen),
        })
    }

    #[getter] fn data_ref(&self) -> &str { &self.inner.data_ref }
    #[getter] fn dim(&self) -> usize { self.inner.dim }
    #[getter] fn rotation_seed(&self) -> u64 { self.inner.rotation_seed }
    #[getter] fn rerank_factor(&self) -> usize { self.inner.rerank_factor }
    #[getter] fn rvf_witness(&self) -> &str { &self.inner.rvf_witness }
    #[getter] fn format_version(&self) -> u32 { self.inner.format_version }
    #[getter] fn pii_policy(&self) -> Option<&str> { self.inner.pii_policy.as_deref() }
    #[getter] fn lineage_id(&self) -> Option<&str> { self.inner.lineage_id.as_deref() }
    #[getter] fn memory_class(&self) -> Option<&str> { self.inner.memory_class.as_deref() }

    /// Generation as `int` (Num variant) or `str` (Opaque variant).
    #[getter]
    fn generation(&self, py: Python<'_>) -> PyObject {
        match &self.inner.generation {
            RsGeneration::Num(n) => n.into_py(py),
            RsGeneration::Opaque(s) => s.clone().into_py(py),
        }
    }

    /// Recompute and compare the witness. Returns `True` if intact.
    fn verify_witness(&self) -> bool { self.inner.verify_witness() }

    /// Serialize to the canonical `table.rulake.json` JSON form.
    fn to_json(&self) -> PyResult<String> { self.inner.to_json().map_err(map_err) }

    /// Parse a `table.rulake.json` body. Enforces the 64 KiB body cap,
    /// 4 KiB per-field cap, and 128-char witness cap (DoS hardening).
    #[classmethod]
    fn from_json(_cls: &Bound<'_, PyType>, s: &str) -> PyResult<Self> {
        Ok(Self { inner: RsBundle::from_json(s).map_err(map_err)? })
    }

    /// Atomic write to `dir/table.rulake.json` (temp + rename).
    fn write_to_dir(&self, py: Python<'_>, dir: PathBuf) -> PyResult<PathBuf> {
        py.allow_threads(|| self.inner.write_to_dir(&dir).map_err(map_err))
    }

    /// Read `dir/table.rulake.json` and verify its witness.
    #[classmethod]
    fn read_from_dir(_cls: &Bound<'_, PyType>, py: Python<'_>, dir: PathBuf) -> PyResult<Self> {
        py.allow_threads(|| Ok(Self { inner: RsBundle::read_from_dir(&dir).map_err(map_err)? }))
    }

    /// Tag with a PII policy handle. Returns a new bundle (the witness
    /// does not depend on this field).
    fn with_pii_policy(&self, p: &str) -> Self {
        Self { inner: self.inner.clone().with_pii_policy(p) }
    }
    fn with_lineage_id(&self, id: &str) -> Self {
        Self { inner: self.inner.clone().with_lineage_id(id) }
    }
    fn with_memory_class(&self, class: &str) -> Self {
        Self { inner: self.inner.clone().with_memory_class(class) }
    }

    fn __repr__(&self) -> String {
        let w = &self.inner.rvf_witness;
        let prefix: &str = &w[..w.len().min(12)];
        format!(
            "Bundle(data_ref={:?}, dim={}, witness={}…)",
            self.inner.data_ref, self.inner.dim, prefix,
        )
    }
}

// ─────────────────────────────────────────────────────────────────────
// LocalBackend — in-memory adapter (NumPy-ingested)
// ─────────────────────────────────────────────────────────────────────

#[pyclass(name = "LocalBackend", module = "rulake._rulake")]
struct PyLocalBackend {
    inner: Arc<RsLocal>,
}

#[pymethods]
impl PyLocalBackend {
    #[new]
    fn new(id: &str) -> Self {
        Self { inner: Arc::new(RsLocal::new(id)) }
    }

    #[getter]
    fn id(&self) -> &str { self.inner.id() }

    /// Insert a collection wholesale.
    ///
    /// `ids` must be `np.ndarray[uint64]` of length N.
    /// `vectors` must be `np.ndarray[float32]` of shape (N, dim), C-contiguous.
    fn put_collection(
        &self,
        py: Python<'_>,
        name: &str,
        ids: PyReadonlyArray1<'_, u64>,
        vectors: PyReadonlyArray2<'_, f32>,
    ) -> PyResult<()> {
        let ids_slice: &[u64] = ids.as_slice().map_err(|_| {
            PyValueError::new_err("ids must be a contiguous uint64 ndarray")
        })?;
        let view = vectors.as_array();
        let shape = view.shape();
        if shape.len() != 2 {
            return Err(PyValueError::new_err("vectors must be 2-D"));
        }
        let n = shape[0];
        let dim = shape[1];
        if ids_slice.len() != n {
            return Err(PyValueError::new_err(format!(
                "len(ids)={} != vectors.shape[0]={}", ids_slice.len(), n
            )));
        }
        let vec_slice: &[f32] = vectors.as_slice().map_err(|_| {
            PyValueError::new_err("vectors must be a C-contiguous float32 ndarray")
        })?;
        // Convert (N, dim) flat slice → Vec<Vec<f32>> while we still hold
        // the GIL, then drop into allow_threads for the actual put.
        // Allocation is unavoidable here because the underlying API
        // takes Vec<Vec<f32>>; this matches `LocalBackend::put_collection`
        // in the Rust crate.
        let mut owned: Vec<Vec<f32>> = Vec::with_capacity(n);
        for i in 0..n {
            owned.push(vec_slice[i * dim..(i + 1) * dim].to_vec());
        }
        let owned_ids: Vec<u64> = ids_slice.to_vec();
        let backend = Arc::clone(&self.inner);
        let name = name.to_string();
        py.allow_threads(|| backend.put_collection(name, dim, owned_ids, owned).map_err(map_err))
    }

    /// Append a single vector. Bumps the backend generation.
    fn append(
        &self,
        py: Python<'_>,
        collection: &str,
        id: u64,
        vector: PyReadonlyArray1<'_, f32>,
    ) -> PyResult<()> {
        let v: Vec<f32> = vector
            .as_slice()
            .map_err(|_| PyValueError::new_err("vector must be a contiguous float32 ndarray"))?
            .to_vec();
        let backend = Arc::clone(&self.inner);
        let collection = collection.to_string();
        py.allow_threads(|| backend.append(collection, id, v).map_err(map_err))
    }
}

// ─────────────────────────────────────────────────────────────────────
// FsBackend — filesystem-backed adapter (`ruvec1` files)
// ─────────────────────────────────────────────────────────────────────

#[pyclass(name = "FsBackend", module = "rulake._rulake")]
struct PyFsBackend {
    inner: Arc<RsFs>,
}

#[pymethods]
impl PyFsBackend {
    #[new]
    fn new(id: &str, root: PathBuf) -> PyResult<Self> {
        let inner = RsFs::new(id, &root).map_err(map_err)?;
        Ok(Self { inner: Arc::new(inner) })
    }

    fn register(&self, collection: &str, filename: &str) -> PyResult<()> {
        self.inner.register(collection, filename).map_err(map_err)
    }

    /// Write a collection to disk in `ruvec1` format. Returns the path
    /// of the written file. Path-traversal-safe (rejects `..`, `/`,
    /// drive letters, control bytes — see `FsBackend::validate_filename`).
    fn write(
        &self,
        py: Python<'_>,
        collection: &str,
        filename: &str,
        ids: PyReadonlyArray1<'_, u64>,
        vectors: PyReadonlyArray2<'_, f32>,
    ) -> PyResult<PathBuf> {
        let view = vectors.as_array();
        let shape = view.shape();
        if shape.len() != 2 {
            return Err(PyValueError::new_err("vectors must be 2-D"));
        }
        let n = shape[0];
        let dim = shape[1];
        let ids_slice: &[u64] = ids.as_slice().map_err(|_| {
            PyValueError::new_err("ids must be a contiguous uint64 ndarray")
        })?;
        let vec_slice: &[f32] = vectors.as_slice().map_err(|_| {
            PyValueError::new_err("vectors must be a C-contiguous float32 ndarray")
        })?;
        if ids_slice.len() != n {
            return Err(PyValueError::new_err("len(ids) != vectors.shape[0]"));
        }
        let mut owned: Vec<Vec<f32>> = Vec::with_capacity(n);
        for i in 0..n {
            owned.push(vec_slice[i * dim..(i + 1) * dim].to_vec());
        }
        let owned_ids: Vec<u64> = ids_slice.to_vec();
        let backend = Arc::clone(&self.inner);
        let collection = collection.to_string();
        let filename = filename.to_string();
        py.allow_threads(|| {
            backend
                .write(collection, filename, dim, &owned_ids, &owned)
                .map_err(map_err)
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
// RuLake — the entry point
// ─────────────────────────────────────────────────────────────────────

#[pyclass(name = "RuLake", module = "rulake._rulake")]
struct PyRuLake {
    inner: Arc<RsRuLake>,
}

#[pymethods]
impl PyRuLake {
    /// Build a fresh ruLake. `rerank_factor=20` matches the BENCHMARK.md
    /// recall@10 ≥ 0.90 gate. `rotation_seed` controls RaBitQ's Haar
    /// rotation (deterministic; same seed → same compressed codes →
    /// same witness across processes).
    #[new]
    #[pyo3(signature = (rerank_factor=20, rotation_seed=42))]
    fn new(rerank_factor: usize, rotation_seed: u64) -> Self {
        Self {
            inner: Arc::new(RsRuLake::new(rerank_factor, rotation_seed)),
        }
    }

    /// Set the consistency mode. Returns a new `RuLake` (the underlying
    /// Rust API consumes self).
    fn with_consistency(&self, c: PyConsistency) -> Self {
        // The Rust API consumes self by value, so we have to clone the
        // inner Arc-bag-of-state and recompute. The Rust impl supports
        // it because `RuLake` is `Clone` (Arc-based).
        let cloned: RsRuLake = (*self.inner).clone();
        let next = cloned.with_consistency(c.inner);
        Self { inner: Arc::new(next) }
    }

    /// Cap the cache at `n` distinct compressed entries (LRU eviction
    /// over unpinned entries).
    fn with_max_cache_entries(&self, n: usize) -> Self {
        let cloned: RsRuLake = (*self.inner).clone();
        let next = cloned.with_max_cache_entries(n);
        Self { inner: Arc::new(next) }
    }

    /// Register a backend. Accepts `LocalBackend` or `FsBackend`
    /// instances (and, eventually, any binding-shipped backend that
    /// owns an `Arc<dyn BackendAdapter>`). A Python-implemented
    /// `BackendAdapter` is v2 (ADR-002 §F).
    fn register_backend(&self, backend: &Bound<'_, PyAny>) -> PyResult<()> {
        // Arc<ConcreteBackend> → Arc<dyn BackendAdapter>: needs an
        // explicit conversion (CoerceUnsized only works in let-bindings,
        // not match arms across types).
        let dyn_arc: Arc<dyn BackendAdapter> =
            if let Ok(b) = backend.downcast::<PyLocalBackend>() {
                let local: Arc<RsLocal> = Arc::clone(&b.borrow().inner);
                local
            } else if let Ok(b) = backend.downcast::<PyFsBackend>() {
                let fs: Arc<RsFs> = Arc::clone(&b.borrow().inner);
                fs
            } else {
                return Err(PyValueError::new_err(
                    "register_backend: expected a LocalBackend or FsBackend instance",
                ));
            };
        self.inner.register_backend(dyn_arc).map_err(map_err)
    }

    fn backend_ids(&self) -> Vec<String> { self.inner.backend_ids() }

    fn cache_stats(&self) -> PyCacheStats {
        self.inner.cache_stats().into()
    }

    fn cache_stats_by_backend<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let stats = self.inner.cache_stats_by_backend();
        let d = PyDict::new_bound(py);
        for (k, v) in stats {
            // #[pyclass] types don't implement ToPyObject directly in
            // PyO3 0.22 — wrap in Py<T> to get a PyObject.
            let py_v = Py::new(py, PyPerBackendStats::from(v))?;
            d.set_item(k, py_v)?;
        }
        Ok(d)
    }

    fn cache_entry_count(&self) -> usize { self.inner.cache_entry_count() }

    fn cache_witness_of(&self, backend: &str, collection: &str) -> Option<String> {
        self.inner
            .cache_witness_of(&(backend.to_string(), collection.to_string()))
    }

    fn invalidate_cache(&self, backend: &str, collection: &str) {
        self.inner
            .invalidate_cache(&(backend.to_string(), collection.to_string()))
    }

    /// Search a single (backend, collection) pair. Returns top-k hits
    /// sorted by ascending L2² score.
    #[pyo3(signature = (backend, collection, query, k))]
    fn search_one(
        &self,
        py: Python<'_>,
        backend: &str,
        collection: &str,
        query: PyReadonlyArray1<'_, f32>,
        k: usize,
    ) -> PyResult<Vec<PySearchResult>> {
        let q: Vec<f32> = query
            .as_slice()
            .map_err(|_| PyValueError::new_err(
                "query must be a contiguous float32 ndarray",
            ))?
            .to_vec();
        let backend = backend.to_string();
        let collection = collection.to_string();
        let lake = Arc::clone(&self.inner);
        let hits = py.allow_threads(|| {
            lake.search_one(&backend, &collection, &q, k).map_err(map_err)
        })?;
        Ok(hits.into_iter().map(PySearchResult::from).collect())
    }

    /// Federated search across multiple `(backend, collection)` targets.
    /// `targets` is a list of 2-tuples.
    #[pyo3(signature = (targets, query, k))]
    fn search_federated(
        &self,
        py: Python<'_>,
        targets: Vec<(String, String)>,
        query: PyReadonlyArray1<'_, f32>,
        k: usize,
    ) -> PyResult<Vec<PySearchResult>> {
        let q: Vec<f32> = query
            .as_slice()
            .map_err(|_| PyValueError::new_err(
                "query must be a contiguous float32 ndarray",
            ))?
            .to_vec();
        let lake = Arc::clone(&self.inner);
        let hits = py.allow_threads(|| {
            let refs: Vec<(&str, &str)> = targets
                .iter()
                .map(|(b, c)| (b.as_str(), c.as_str()))
                .collect();
            lake.search_federated(&refs, &q, k).map_err(map_err)
        })?;
        Ok(hits.into_iter().map(PySearchResult::from).collect())
    }

    /// Batched single-collection search. `queries` is float32 (N, dim).
    /// Returns a list of lists; `result[i]` is the top-k for query i.
    #[pyo3(signature = (backend, collection, queries, k))]
    fn search_batch(
        &self,
        py: Python<'_>,
        backend: &str,
        collection: &str,
        queries: PyReadonlyArray2<'_, f32>,
        k: usize,
    ) -> PyResult<Vec<Vec<PySearchResult>>> {
        let view = queries.as_array();
        let shape = view.shape();
        if shape.len() != 2 {
            return Err(PyValueError::new_err("queries must be 2-D float32"));
        }
        let n = shape[0];
        let dim = shape[1];
        let flat: &[f32] = queries.as_slice().map_err(|_| {
            PyValueError::new_err("queries must be a C-contiguous float32 ndarray")
        })?;
        let mut owned: Vec<Vec<f32>> = Vec::with_capacity(n);
        for i in 0..n {
            owned.push(flat[i * dim..(i + 1) * dim].to_vec());
        }
        let backend = backend.to_string();
        let collection = collection.to_string();
        let lake = Arc::clone(&self.inner);
        let raw = py.allow_threads(|| {
            lake.search_batch(&backend, &collection, &owned, k).map_err(map_err)
        })?;
        Ok(raw
            .into_iter()
            .map(|v| v.into_iter().map(PySearchResult::from).collect())
            .collect())
    }

    /// High-k array variant (ADR-002 §Open question 3): returns
    /// `(ids: np.ndarray[uint64], scores: np.ndarray[float32])` when the
    /// per-row dict cost is the bottleneck (k > ~500).
    #[pyo3(signature = (backend, collection, query, k))]
    fn search_one_arrays<'py>(
        &self,
        py: Python<'py>,
        backend: &str,
        collection: &str,
        query: PyReadonlyArray1<'_, f32>,
        k: usize,
    ) -> PyResult<(Bound<'py, PyArray1<u64>>, Bound<'py, PyArray1<f32>>)> {
        let q: Vec<f32> = query
            .as_slice()
            .map_err(|_| PyValueError::new_err("query must be float32, contiguous"))?
            .to_vec();
        let backend = backend.to_string();
        let collection = collection.to_string();
        let lake = Arc::clone(&self.inner);
        let hits = py.allow_threads(|| {
            lake.search_one(&backend, &collection, &q, k).map_err(map_err)
        })?;
        let ids: Vec<u64> = hits.iter().map(|h| h.id).collect();
        let scores: Vec<f32> = hits.iter().map(|h| h.score).collect();
        Ok((
            PyArray1::from_vec_bound(py, ids),
            PyArray1::from_vec_bound(py, scores),
        ))
    }

    /// Atomically publish `dir/table.rulake.json` for the entry under
    /// `(backend, collection)`. Returns the written path.
    fn publish_bundle(
        &self,
        py: Python<'_>,
        backend: &str,
        collection: &str,
        dir: PathBuf,
    ) -> PyResult<PathBuf> {
        let key = (backend.to_string(), collection.to_string());
        let lake = Arc::clone(&self.inner);
        py.allow_threads(|| lake.publish_bundle(&key, &dir).map_err(map_err))
    }

    /// Refresh from `dir/table.rulake.json`. Returns a string tag:
    /// `"up_to_date"`, `"invalidated"`, or `"bundle_missing"`.
    fn refresh_from_bundle_dir(
        &self,
        py: Python<'_>,
        backend: &str,
        collection: &str,
        dir: PathBuf,
    ) -> PyResult<&'static str> {
        let key = (backend.to_string(), collection.to_string());
        let lake = Arc::clone(&self.inner);
        let r = py.allow_threads(|| lake.refresh_from_bundle_dir(&key, &dir).map_err(map_err))?;
        Ok(match r {
            RsRefreshResult::UpToDate => "up_to_date",
            RsRefreshResult::Invalidated => "invalidated",
            RsRefreshResult::BundleMissing => "bundle_missing",
        })
    }

    /// Persist the cache for `(backend, collection)` to disk
    /// (RaBitQ-native `index.rbpx` format). Pair with a `publish_bundle`
    /// of the same key for warm-restart byte-for-byte identical to the
    /// original prime.
    fn save_cache_to_dir(
        &self,
        py: Python<'_>,
        backend: &str,
        collection: &str,
        dir: PathBuf,
    ) -> PyResult<PathBuf> {
        let key = (backend.to_string(), collection.to_string());
        let lake = Arc::clone(&self.inner);
        py.allow_threads(|| lake.save_cache_to_dir(&key, &dir).map_err(map_err))
    }

    /// Warm the cache from a previously-saved snapshot directory. Returns
    /// the number of vectors loaded.
    fn warm_from_dir(
        &self,
        py: Python<'_>,
        backend: &str,
        collection: &str,
        dir: PathBuf,
    ) -> PyResult<usize> {
        let key = (backend.to_string(), collection.to_string());
        let lake = Arc::clone(&self.inner);
        py.allow_threads(|| lake.warm_from_dir(&key, &dir).map_err(map_err))
    }
}

// ─────────────────────────────────────────────────────────────────────
// Module init
// ─────────────────────────────────────────────────────────────────────

#[pymodule]
fn _rulake(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyConsistency>()?;
    m.add_class::<PySearchResult>()?;
    m.add_class::<PyCacheStats>()?;
    m.add_class::<PyPerBackendStats>()?;
    m.add_class::<PyBundle>()?;
    m.add_class::<PyLocalBackend>()?;
    m.add_class::<PyFsBackend>()?;
    m.add_class::<PyRuLake>()?;

    m.add("RuLakeError", py.get_type_bound::<RuLakeError>())?;
    m.add("BackendNotFoundError", py.get_type_bound::<BackendNotFoundError>())?;
    m.add(
        "CollectionNotFoundError",
        py.get_type_bound::<CollectionNotFoundError>(),
    )?;
    m.add(
        "DimensionMismatchError",
        py.get_type_bound::<DimensionMismatchError>(),
    )?;
    m.add(
        "InvalidParameterError",
        py.get_type_bound::<InvalidParameterError>(),
    )?;
    m.add("BackendError", py.get_type_bound::<BackendError>())?;

    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

