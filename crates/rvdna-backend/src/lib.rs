//! rvDNA v2 BackendAdapter for ruLake (ADR-007).
//!
//! v0.0 shipped the hot-tier (T0) only — k-mer vectors held in RAM.
//! v0.1 added the warm tier (T1): `.rvdna` v2 sections §2 (protein
//! embeddings), §3 (attention), and §4 (variant tensors) served as
//! `f32` slices straight out of an mmap'd file region.
//! **v0.2 ships T2** — cold lazy tier for raw DNA + epigenomic series
//! (sections §5–§8 per ADR-007 §6 and
//! `docs/research/rvdna/integration-with-rulake.md` §1.6). T2 differs
//! from T1 in storage path: it does **not** mmap the file. Raw DNA
//! payloads are gigabytes; T2 keeps a [`std::fs::File`] handle and
//! does lazy `seek+read` on each `pull_vectors` call, with a small
//! per-collection LRU window so repeated searches on the same range
//! avoid re-reading from disk.
//!
//! Trust boundary: the file's witness (SHAKE-256(32) over the bundle
//! pointer at byte 0x00B0) is the cache-key anchor. Two deployments
//! reading the same `.rvdna` file produce identical witnesses and share
//! cache via ruLake's content-addressed dedup (`src/cache.rs`). T1 and
//! T2 both reuse the T0 witness derivation [`witness::rvdna_bundle`]
//! verbatim — the trust contract is unchanged across all three tiers.
//! `tests/t2_smoke.rs::t2_witness_matches_t0_and_t1_for_same_inputs`
//! pins this byte-equality invariant.
//!
//! v0.2 is deliberately schema-light, same as v0.1: T1 and T2 take
//! section offsets directly from the operator at `register_file` time.
//! The full v2 header parser lands in a future point release without
//! changing the [`t1::RvdnaT1Backend`] or [`t2::RvdnaT2Backend`]
//! surfaces.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::sync::RwLock;

use rulake::backend::{BackendAdapter, CollectionId, PulledBatch};
use rulake::error::Result;

pub mod t1;
pub mod t2;
pub mod witness;

pub use t1::RvdnaT1Backend;
pub use t2::{RvdnaT2Backend, T2LruStats};

/// One in-memory hot-tier collection — the k-mer vectors from one gene
/// (or whatever the encoder considered a coherent block).
#[derive(Debug, Clone)]
pub struct RvdnaCollection {
    /// External ids from the `.rvdna` file's k-mer index. v0.0 uses the
    /// vector position; v0.0.2 will pull the real ids from section 1.
    pub ids: Vec<u64>,
    /// k-mer vectors. v0.0 holds them in RAM; v0.1 mmap's section 1.
    pub vectors: Vec<Vec<f32>>,
    /// Vector dimensionality.
    pub dim: usize,
    /// Coherence token. Bumps when the underlying `.rvdna` file rotates
    /// (re-encode against a newer model checkpoint, new variant calls).
    pub generation: u64,
}

/// Hot-tier (T0) BackendAdapter.
///
/// Holds k-mer vector collections in RAM. Backed by either:
///  - a synthetic in-memory fixture (v0.0; useful for tests and for the
///    Console's WASM-local demo);
///  - a `.rvdna` v2 file's section 1 read into memory (v0.0.2);
///  - a memory-mapped section 1 view (v0.1).
///
/// Witness derivation lives in [`witness`].
pub struct RvdnaT0Backend {
    id: String,
    collections: RwLock<BTreeMap<String, RvdnaCollection>>,
}

impl RvdnaT0Backend {
    /// Create an empty T0 backend with a stable id.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            collections: RwLock::new(BTreeMap::new()),
        }
    }

    /// Insert / replace a hot-tier collection by name.
    ///
    /// Returns the previous collection if one existed under this name.
    pub fn put_collection(&self, name: impl Into<String>, c: RvdnaCollection) -> Option<RvdnaCollection> {
        self.collections.write().expect("poisoned").insert(name.into(), c)
    }

    /// Bump the generation of a collection — call this after rotating
    /// the underlying `.rvdna` file so the cache invalidates.
    pub fn bump_generation(&self, name: &str) -> Result<u64> {
        let mut g = self.collections.write().expect("poisoned");
        match g.get_mut(name) {
            Some(c) => {
                c.generation = c.generation.checked_add(1).unwrap_or(c.generation);
                Ok(c.generation)
            }
            None => Err(rulake::error::RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: name.to_string(),
            }),
        }
    }
}

impl BackendAdapter for RvdnaT0Backend {
    fn id(&self) -> &str {
        &self.id
    }

    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        Ok(self.collections.read().expect("poisoned").keys().cloned().collect())
    }

    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        let g = self.collections.read().expect("poisoned");
        let c = g.get(collection).ok_or_else(|| {
            rulake::error::RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: collection.to_string(),
            }
        })?;
        Ok(PulledBatch {
            collection: collection.to_string(),
            ids: c.ids.clone(),
            vectors: c.vectors.clone(),
            dim: c.dim,
            generation: c.generation,
        })
    }

    fn generation(&self, collection: &str) -> Result<u64> {
        let g = self.collections.read().expect("poisoned");
        let c = g.get(collection).ok_or_else(|| {
            rulake::error::RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: collection.to_string(),
            }
        })?;
        Ok(c.generation)
    }
}
