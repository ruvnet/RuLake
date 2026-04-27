//! rvDNA v2 BackendAdapter for ruLake (ADR-007).
//!
//! v0.0 ships the hot-tier (T0) only — k-mer vectors loaded eagerly
//! from a `.rvdna` v2 file. T1 (warm mmap) and T2 (cold lazy) land in
//! v0.1 / v0.2 per ADR-007 §6 and `docs/research/rvdna/integration-with-rulake.md`.
//!
//! Trust boundary: the file's witness (SHAKE-256(32) over the bundle
//! pointer at byte 0x00B0) is the cache-key anchor. Two deployments
//! reading the same `.rvdna` file produce identical witnesses and share
//! cache via ruLake's content-addressed dedup (`src/cache.rs`).
//!
//! v0.0 is deliberately schema-light: a single test fixture exercises
//! the BackendAdapter contract end-to-end (id / list_collections /
//! pull_vectors / generation). The full v2 file format reader lands
//! in v0.0.2; v0.0.1 carries the trait + types + a deterministic
//! synthetic backend so the App Store status flips from PROPOSED →
//! SCAFFOLDED.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::sync::RwLock;

use rulake::backend::{BackendAdapter, CollectionId, PulledBatch};
use rulake::error::Result;

pub mod witness;

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
