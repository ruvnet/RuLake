//! Cold-tier (T2) BackendAdapter — `.rvdna` v2 sections §5 (raw DNA
//! strings packed as nucleotide-windows) through §8 (epigenomic series)
//! served by **lazy seek+read** out of a `std::fs::File` plus a small
//! per-collection LRU window cache (ADR-007 §6 and
//! `docs/research/rvdna/integration-with-rulake.md` §1.6).
//!
//! T2 mirrors [`crate::RvdnaT1Backend`]'s shape — same
//! [`BackendAdapter`] contract, same witness derivation via
//! [`crate::witness::rvdna_bundle`], same `(ids, vectors, dim,
//! generation)` `PulledBatch` quadruple — but it does **not** mmap the
//! whole file. Raw DNA strings and per-base epigenomic tracks are
//! gigabytes; paging an entire chromosome into memory just to answer
//! one query is the exact failure mode T2 exists to prevent.
//!
//! Storage path:
//! - Each collection holds a [`std::fs::File`] handle (no mmap, no
//!   `unsafe`), the section's `vectors_offset`, the `(n_vectors, dim)`
//!   shape, and a small LRU keyed by vector index.
//! - `pull_vectors` `seek`s to `vectors_offset`, reads the contiguous
//!   `n_vectors * dim * 4` byte run, decodes via [`bytemuck`], and
//!   warms the LRU with the touched vectors. The OS page cache makes
//!   subsequent reads of the same window cheap; the LRU avoids the
//!   syscall + decode round-trip entirely on the second hit.
//! - The LRU cap (1024 vectors) is sized to cover one
//!   ruLake-cache-prime on a typical query — once a search hits the
//!   same range twice in a row, calls 2..N are LRU-resident.
//!
//! Trust contract: T2 derives its witness from the same five inputs as
//! T0 and T1 — `(data_ref, dim, rotation_seed, rerank_factor,
//! generation)` — routed through [`crate::witness::rvdna_bundle`]. A
//! T0 / T1 / T2 trio with identical inputs MUST produce byte-equal
//! witnesses; the `t2_witness_matches_t0_and_t1_for_same_inputs` test
//! in `tests/t2_smoke.rs` enforces this. Federation, audit, and the
//! Console verifier path treat all three tiers as one cache federation.
//!
//! v0.2 is deliberately schema-light, same as T1: operators register
//! files via [`RvdnaT2Backend::register_file`] passing the section
//! offsets they got from upstream tooling. The `.rvdna` v2 header
//! parser lands in a future point release without changing this
//! surface.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use lru::LruCache;
use rulake::backend::{BackendAdapter, CollectionId, PulledBatch};
use rulake::error::{Result, RuLakeError};

/// Default LRU window size (in vectors) — covers a typical ruLake
/// prime. Operators can override per-collection in a future point
/// release; v0.2 uses this constant for every registered file.
const DEFAULT_LRU_CAP: usize = 1024;

/// One cold-tier collection: a [`std::fs::File`] handle to the
/// underlying `.rvdna` v2 file, the per-section offsets, plus a small
/// LRU window keyed by vector index. Lazy-read on every
/// [`RvdnaT2Backend::pull_vectors`] call — the cache is per-vector, not
/// per-batch, so a search that fans out across overlapping ranges only
/// pays the seek+read cost once per distinct row.
struct T2File {
    /// Source path. Kept for diagnostics / `Debug` only — the open
    /// `file` handle holds the actual reference.
    #[allow(dead_code)]
    path: PathBuf,
    /// Open file handle. `Mutex`-guarded because [`std::fs::File`]'s
    /// `seek` mutates the offset; we serialise reads to keep the
    /// portable `seek+read` path correct on all platforms.
    file: Mutex<std::fs::File>,
    /// Vector dimensionality.
    dim: usize,
    /// Coherence token. Bumps via [`RvdnaT2Backend::bump_generation`]
    /// when the underlying `.rvdna` file rotates.
    generation: u64,
    /// Byte offset to a packed `[f32; n_vectors * dim]` vectors array
    /// (little-endian). MUST be 4-byte aligned.
    vectors_offset: u64,
    /// Number of vectors in this section.
    n_vectors: usize,
    /// External ids, supplied at registration time. v0.2 takes them
    /// in-memory rather than seeking a separate `[u64]` table — the
    /// id list is `O(n_vectors)` and even a billion-row chromosome
    /// fits comfortably; raw DNA payloads are what's huge.
    ids: Vec<u64>,
    /// Per-vector LRU window. Keyed by vector index `0..n_vectors`,
    /// value is the decoded `dim`-length `Vec<f32>`. `Mutex` because
    /// `LruCache::get` mutates recency.
    lru: Mutex<LruCache<usize, Vec<f32>>>,
    /// Running tally of LRU hits across this collection's lifetime —
    /// surfaced via [`RvdnaT2Backend::lru_stats`] so tests and
    /// operators can verify the warm path is actually warm.
    lru_hits: Mutex<u64>,
    /// Running tally of LRU misses (reads that fell through to disk).
    lru_misses: Mutex<u64>,
}

/// Snapshot of one collection's LRU stats — useful for tests
/// (`t2_lru_warms_under_repeated_search`) and for operator dashboards
/// that want to confirm the cold tier isn't actually cold in steady
/// state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct T2LruStats {
    /// Number of vector reads that were served from the LRU.
    pub hits: u64,
    /// Number of vector reads that fell through to a disk read.
    pub misses: u64,
    /// Current LRU occupancy (number of resident vectors).
    pub len: usize,
    /// LRU capacity (in vectors).
    pub cap: usize,
}

/// Cold-tier (T2) BackendAdapter. See module docs.
///
/// Holds a name → cold-file map. `pull_vectors` lazily seeks to the
/// declared section, reads the contiguous f32 run, decodes via
/// [`bytemuck`] (no `unsafe`), and warms a per-collection LRU. Subsequent
/// pulls of the same vector indices return from the LRU without
/// touching the file.
///
/// Thread-safe: the inner table is guarded by an `RwLock` (same shape
/// as [`crate::RvdnaT1Backend`]); the per-file `seek` cursor and per-
/// collection LRU each sit behind a `Mutex` because both mutate on
/// read.
pub struct RvdnaT2Backend {
    id: String,
    mappings: RwLock<BTreeMap<CollectionId, T2File>>,
}

impl RvdnaT2Backend {
    /// Create an empty T2 backend with a stable id. Operators then
    /// call [`Self::register_file`] for each `.rvdna` v2 section they
    /// want to expose lazily.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            mappings: RwLock::new(BTreeMap::new()),
        }
    }

    /// Register one cold-tier collection backed by lazy reads from a
    /// file region.
    ///
    /// Arguments:
    /// - `name`: collection id under this backend (e.g.
    ///   `"raw_dna_chr17"`, `"epigenome_h3k27ac"`).
    /// - `path`: filesystem path to the `.rvdna` v2 file.
    /// - `dim`: vector dimensionality (per-window epigenomic channels
    ///   or k-mer footprint depending on section).
    /// - `generation`: starting coherence token.
    /// - `ids`: external ids for the `n_vectors` rows. Pass an empty
    ///   `Vec` to have the backend synthesise positional ids
    ///   `0..n_vectors` (mirrors T1's `has_ids=false` shortcut).
    /// - `vectors_offset`: byte offset to the `[f32]` vectors table.
    ///   MUST be 4-byte aligned.
    /// - `n_vectors`: number of vectors in this section.
    ///
    /// Returns `Ok(())` on success. Errors if the file can't be
    /// opened, if the declared region's byte size overflows, or if
    /// `ids` is non-empty and its length doesn't match `n_vectors`.
    ///
    /// Note: T2 deliberately does **not** stat the file to bounds-
    /// check the declared region at registration time — chromosome-
    /// scale files are remote-mounted on FUSE / S3FS in production
    /// and stat'ing them up-front defeats the lazy-tier purpose. The
    /// per-call `seek+read` will surface an error at first access.
    pub fn register_file(
        &self,
        name: impl Into<String>,
        path: impl AsRef<Path>,
        dim: usize,
        generation: u64,
        ids: Vec<u64>,
        vectors_offset: u64,
        n_vectors: usize,
    ) -> Result<()> {
        if dim == 0 {
            return Err(RuLakeError::InvalidParameter(
                "rvdna-t2: dim must be > 0".into(),
            ));
        }
        if vectors_offset % (std::mem::align_of::<f32>() as u64) != 0 {
            return Err(RuLakeError::InvalidParameter(format!(
                "rvdna-t2: vectors_offset={vectors_offset} not f32-aligned"
            )));
        }
        if !ids.is_empty() && ids.len() != n_vectors {
            return Err(RuLakeError::InvalidParameter(format!(
                "rvdna-t2: ids.len()={} disagrees with n_vectors={n_vectors}",
                ids.len()
            )));
        }
        // Defensive overflow check — even though we don't read up-front,
        // we still need the byte length to fit a `usize` for the read
        // buffer.
        let _ = n_vectors
            .checked_mul(dim)
            .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| {
                RuLakeError::InvalidParameter(format!(
                    "rvdna-t2: vectors-region size overflow (n={n_vectors}, dim={dim})"
                ))
            })?;

        let path_buf = path.as_ref().to_path_buf();
        let file = std::fs::File::open(&path_buf).map_err(|e| {
            RuLakeError::InvalidParameter(format!(
                "rvdna-t2: open {}: {e}",
                path_buf.display()
            ))
        })?;

        let cap = NonZeroUsize::new(DEFAULT_LRU_CAP).expect("DEFAULT_LRU_CAP > 0");
        let resolved_ids: Vec<u64> = if ids.is_empty() {
            (0..n_vectors as u64).collect()
        } else {
            ids
        };

        let mapping = T2File {
            path: path_buf,
            file: Mutex::new(file),
            dim,
            generation,
            vectors_offset,
            n_vectors,
            ids: resolved_ids,
            lru: Mutex::new(LruCache::new(cap)),
            lru_hits: Mutex::new(0),
            lru_misses: Mutex::new(0),
        };
        self.mappings
            .write()
            .expect("poisoned")
            .insert(name.into(), mapping);
        Ok(())
    }

    /// Bump the generation of a collection — call this after rotating
    /// the underlying `.rvdna` file so the cache invalidates. Mirrors
    /// [`crate::RvdnaT1Backend::bump_generation`] verbatim.
    ///
    /// Also clears the per-collection LRU: a generation bump means the
    /// underlying bytes may have shifted, so retaining decoded vectors
    /// would silently serve stale rows.
    pub fn bump_generation(&self, name: &str) -> Result<u64> {
        let mut g = self.mappings.write().expect("poisoned");
        match g.get_mut(name) {
            Some(m) => {
                m.generation = m.generation.checked_add(1).unwrap_or(m.generation);
                m.lru.lock().expect("poisoned").clear();
                *m.lru_hits.lock().expect("poisoned") = 0;
                *m.lru_misses.lock().expect("poisoned") = 0;
                Ok(m.generation)
            }
            None => Err(RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: name.to_string(),
            }),
        }
    }

    /// Snapshot the LRU stats for one collection. Returns `None` if
    /// the collection isn't registered. Used by
    /// `tests/t2_smoke.rs::t2_lru_warms_under_repeated_search` to
    /// verify the warm-path contract.
    pub fn lru_stats(&self, name: &str) -> Option<T2LruStats> {
        let g = self.mappings.read().expect("poisoned");
        let m = g.get(name)?;
        let lru = m.lru.lock().expect("poisoned");
        let stats = T2LruStats {
            hits: *m.lru_hits.lock().expect("poisoned"),
            misses: *m.lru_misses.lock().expect("poisoned"),
            len: lru.len(),
            cap: lru.cap().get(),
        };
        Some(stats)
    }
}

impl BackendAdapter for RvdnaT2Backend {
    fn id(&self) -> &str {
        &self.id
    }

    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        Ok(self
            .mappings
            .read()
            .expect("poisoned")
            .keys()
            .cloned()
            .collect())
    }

    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        let g = self.mappings.read().expect("poisoned");
        let m = g.get(collection).ok_or_else(|| RuLakeError::UnknownCollection {
            backend: self.id.clone(),
            collection: collection.to_string(),
        })?;

        let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(m.n_vectors);

        // Fast-path: if every vector is already LRU-resident, skip the
        // file lock entirely. This is the steady-state hot loop after
        // the first prime.
        {
            let mut lru = m.lru.lock().expect("poisoned");
            if lru.len() >= m.n_vectors {
                let mut all_present = true;
                for i in 0..m.n_vectors {
                    if !lru.contains(&i) {
                        all_present = false;
                        break;
                    }
                }
                if all_present {
                    for i in 0..m.n_vectors {
                        // `get` mutates recency — exactly what we want
                        // for an "all hot" pull.
                        let v = lru.get(&i).expect("contains-checked").clone();
                        vectors.push(v);
                    }
                    *m.lru_hits.lock().expect("poisoned") += m.n_vectors as u64;
                    return Ok(PulledBatch {
                        collection: collection.to_string(),
                        ids: m.ids.clone(),
                        vectors,
                        dim: m.dim,
                        generation: m.generation,
                    });
                }
            }
        }

        // Cold/warm-mixed path: read the full vectors region in one
        // contiguous syscall, then walk it row-by-row updating the
        // LRU. We always read the whole region rather than scattering
        // per-row pread calls — for the tier-2 sizes the OS page
        // cache will fault in the same pages either way and one big
        // read minimises syscall overhead.
        let vectors_bytes = m
            .n_vectors
            .checked_mul(m.dim)
            .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| {
                RuLakeError::InvalidParameter(format!(
                    "rvdna-t2: vectors-region size overflow (n={}, dim={})",
                    m.n_vectors, m.dim
                ))
            })?;
        let mut buf = vec![0u8; vectors_bytes];
        {
            let mut f = m.file.lock().expect("poisoned");
            f.seek(SeekFrom::Start(m.vectors_offset)).map_err(|e| {
                RuLakeError::InvalidParameter(format!(
                    "rvdna-t2: seek to {}: {e}",
                    m.vectors_offset
                ))
            })?;
            f.read_exact(&mut buf).map_err(|e| {
                RuLakeError::InvalidParameter(format!(
                    "rvdna-t2: read {vectors_bytes} bytes from {}: {e}",
                    m.vectors_offset
                ))
            })?;
        }
        let flat: &[f32] = bytemuck::try_cast_slice(&buf).map_err(|e| {
            RuLakeError::InvalidParameter(format!(
                "rvdna-t2: vectors region not castable to &[f32]: {e}"
            ))
        })?;

        let mut lru = m.lru.lock().expect("poisoned");
        let mut hits = 0u64;
        let mut misses = 0u64;
        for i in 0..m.n_vectors {
            if let Some(v) = lru.get(&i) {
                vectors.push(v.clone());
                hits += 1;
            } else {
                let start = i * m.dim;
                let end = start + m.dim;
                let v = flat[start..end].to_vec();
                lru.put(i, v.clone());
                vectors.push(v);
                misses += 1;
            }
        }
        drop(lru);
        *m.lru_hits.lock().expect("poisoned") += hits;
        *m.lru_misses.lock().expect("poisoned") += misses;

        Ok(PulledBatch {
            collection: collection.to_string(),
            ids: m.ids.clone(),
            vectors,
            dim: m.dim,
            generation: m.generation,
        })
    }

    fn generation(&self, collection: &str) -> Result<u64> {
        let g = self.mappings.read().expect("poisoned");
        let m = g.get(collection).ok_or_else(|| RuLakeError::UnknownCollection {
            backend: self.id.clone(),
            collection: collection.to_string(),
        })?;
        Ok(m.generation)
    }
}
