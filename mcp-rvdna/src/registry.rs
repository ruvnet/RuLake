//! Operator-pinned collection registry — the trust anchor for v0.0.1.
//!
//! At process start the operator hands `mcp-rvdna` one or more
//! `RvdnaT0Backend`s. For each `(backend_id, collection)` pair we
//! capture the backend's `current_bundle()` once — that snapshot is
//! the *pinned witness*. Every read-tool invocation re-derives the
//! current witness and compares; disagreement triggers a
//! `RVDNA_WITNESS_DRIFT` refusal (R-1 mitigation, see
//! `docs/research/security/v0.0-substrates.md`).
//!
//! The mutation surface here is process-internal only —
//! `RvdnaMcpServer::register_collection` is callable by the binary's
//! main(), not by any MCP tool. That is the v0.0.1 design: the wire
//! is read-only.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use rulake::backend::BackendAdapter;
use rulake::RuLakeBundle;

/// One pinned (backend, collection) entry.
#[derive(Clone)]
pub struct PinnedCollection {
    pub backend_id: String,
    pub collection: String,
    /// The backend the operator handed us. Kept as an `Arc<dyn>` so
    /// `current_bundle()` can be re-called on every tool invocation.
    pub backend: Arc<dyn BackendAdapter>,
    /// `RuLakeBundle::rvf_witness` captured at register time.
    pub pinned_witness: String,
    /// Full pinned bundle — included in `rvdna_lineage` responses so
    /// callers see the *original* declaration, not just the digest.
    pub pinned_bundle: RuLakeBundle,
    /// rotation_seed + rerank_factor used when deriving the bundle.
    /// Held so re-derivation uses the same parameters at audit time.
    pub rotation_seed: u64,
    pub rerank_factor: usize,
}

impl std::fmt::Debug for PinnedCollection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinnedCollection")
            .field("backend_id", &self.backend_id)
            .field("collection", &self.collection)
            .field("pinned_witness", &self.pinned_witness)
            .field("rotation_seed", &self.rotation_seed)
            .field("rerank_factor", &self.rerank_factor)
            .finish_non_exhaustive()
    }
}

/// Process-local registry — `(backend_id, collection)` → `PinnedCollection`.
#[derive(Default)]
pub struct RvdnaRegistry {
    inner: RwLock<BTreeMap<(String, String), PinnedCollection>>,
}

impl RvdnaRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a collection. Captures the current witness as the pin.
    /// `rotation_seed` and `rerank_factor` should be the operator's
    /// global ruLake settings so re-derivation is apples-to-apples.
    pub fn register(
        &self,
        backend: Arc<dyn BackendAdapter>,
        collection: impl Into<String>,
        rotation_seed: u64,
        rerank_factor: usize,
    ) -> anyhow::Result<PinnedCollection> {
        let backend_id = backend.id().to_string();
        let collection = collection.into();
        let bundle = backend
            .current_bundle(&collection, rotation_seed, rerank_factor)
            .map_err(|e| {
                anyhow::anyhow!(
                    "RVDNA_REGISTER_FAILED: current_bundle({backend_id}/{collection}): {e}"
                )
            })?;
        let entry = PinnedCollection {
            backend_id: backend_id.clone(),
            collection: collection.clone(),
            backend,
            pinned_witness: bundle.rvf_witness.clone(),
            pinned_bundle: bundle,
            rotation_seed,
            rerank_factor,
        };
        self.inner
            .write()
            .expect("poisoned")
            .insert((backend_id, collection), entry.clone());
        Ok(entry)
    }

    pub fn get(&self, backend_id: &str, collection: &str) -> Option<PinnedCollection> {
        self.inner
            .read()
            .expect("poisoned")
            .get(&(backend_id.to_string(), collection.to_string()))
            .cloned()
    }

    pub fn list(&self) -> Vec<(String, String)> {
        self.inner
            .read()
            .expect("poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// Re-derive the backend's current witness and compare to the pin.
    /// Returns `Ok(current_bundle)` on agreement, `Err(WitnessDrift)`
    /// otherwise. This is the R-1 gate every read tool funnels through.
    pub fn check_witness(
        &self,
        backend_id: &str,
        collection: &str,
    ) -> Result<(PinnedCollection, RuLakeBundle), WitnessCheckError> {
        let pin = self
            .get(backend_id, collection)
            .ok_or_else(|| WitnessCheckError::Unknown {
                backend_id: backend_id.to_string(),
                collection: collection.to_string(),
            })?;
        let live = pin
            .backend
            .current_bundle(collection, pin.rotation_seed, pin.rerank_factor)
            .map_err(|e| WitnessCheckError::Backend(e.to_string()))?;
        if live.rvf_witness != pin.pinned_witness {
            return Err(WitnessCheckError::Drift {
                pinned: pin.pinned_witness.clone(),
                live: live.rvf_witness.clone(),
            });
        }
        Ok((pin, live))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WitnessCheckError {
    #[error("RVDNA_UNKNOWN_COLLECTION: {backend_id}/{collection}")]
    Unknown {
        backend_id: String,
        collection: String,
    },
    #[error("RVDNA_BACKEND_ERROR: {0}")]
    Backend(String),
    #[error(
        "RVDNA_WITNESS_DRIFT: pinned={pinned} live={live} — backend mutated since register"
    )]
    Drift { pinned: String, live: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruvector_rulake_rvdna::{RvdnaCollection, RvdnaT0Backend};

    fn fixture_backend() -> Arc<RvdnaT0Backend> {
        let be = Arc::new(RvdnaT0Backend::new("rvdna-fixture"));
        be.put_collection(
            "chr1",
            RvdnaCollection {
                ids: (0..4).collect(),
                vectors: (0..4).map(|i| vec![i as f32; 8]).collect(),
                dim: 8,
                generation: 1,
            },
        );
        be
    }

    #[test]
    fn register_captures_current_witness() {
        let reg = RvdnaRegistry::new();
        let be = fixture_backend();
        let dyn_be: Arc<dyn BackendAdapter> = be.clone();
        let pin = reg.register(dyn_be, "chr1", 42, 20).unwrap();
        assert!(!pin.pinned_witness.is_empty(), "witness derived");
        let stored = reg.get("rvdna-fixture", "chr1").unwrap();
        assert_eq!(stored.pinned_witness, pin.pinned_witness);
    }

    #[test]
    fn check_witness_passes_when_backend_unchanged() {
        let reg = RvdnaRegistry::new();
        let be = fixture_backend();
        let dyn_be: Arc<dyn BackendAdapter> = be.clone();
        reg.register(dyn_be, "chr1", 42, 20).unwrap();
        let (pin, live) = reg.check_witness("rvdna-fixture", "chr1").unwrap();
        assert_eq!(pin.pinned_witness, live.rvf_witness);
    }

    #[test]
    fn check_witness_drifts_when_generation_bumps() {
        let reg = RvdnaRegistry::new();
        let be = fixture_backend();
        let dyn_be: Arc<dyn BackendAdapter> = be.clone();
        reg.register(dyn_be, "chr1", 42, 20).unwrap();
        // Mutate the backend out from under the pin.
        be.bump_generation("chr1").unwrap();
        let err = reg.check_witness("rvdna-fixture", "chr1").unwrap_err();
        assert!(
            matches!(err, WitnessCheckError::Drift { .. }),
            "drift: {err:?}"
        );
    }
}
