//! Warm-tier (T1) BackendAdapter — `.rvdna` v2 sections §2 (protein
//! embeddings), §3 (attention), and §4 (variant tensors) decoded from
//! an mmap'd file region into the ruLake cache (ADR-007 §1.4).
//!
//! T1 mirrors [`crate::RvdnaT0Backend`]'s shape: same `BackendAdapter`
//! contract, same witness derivation via [`crate::witness::rvdna_bundle`],
//! same `(ids, vectors, dim, generation)` `PulledBatch` quadruple. The
//! difference is the storage path — T0 holds `Vec<Vec<f32>>` in RAM, T1
//! reinterprets a slice of an mmap'd file as `&[f32]` (zero-copy via
//! [`bytemuck`]) and copies into the `PulledBatch` only at pull time.
//! Once the cache is primed the batch is in 1-bit codes anyway, so the
//! per-pull copy is a one-time cost.
//!
//! v0.1 does **not** parse the `.rvdna` v2 header. Operators register
//! files via [`RvdnaT1Backend::register_file`] passing the section
//! offsets they got from upstream tooling. A future point release adds
//! a header parser without changing this surface.
//!
//! Trust contract: T1 derives its witness from the same five inputs as
//! T0 — `(data_ref, dim, rotation_seed, rerank_factor, generation)` —
//! routed through [`crate::witness::rvdna_bundle`]. A T0 backend and a
//! T1 backend with identical inputs MUST produce byte-equal witnesses;
//! the `t1_witness_matches_t0_for_same_inputs` test in
//! `tests/t1_smoke.rs` enforces this.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use memmap2::Mmap;
use rulake::backend::{BackendAdapter, CollectionId, PulledBatch};
use rulake::error::{Result, RuLakeError};

/// One warm-tier mapping: an mmap'd `.rvdna` v2 file region plus the
/// metadata needed to decode a slice of it as a vector batch.
///
/// The mmap covers the whole file; `ids_offset` / `vectors_offset` /
/// `n_vectors` / `dim` describe a single contiguous tensor section
/// inside it. v0.1 carries one collection per registered file; if a
/// single `.rvdna` file holds multiple T1 sections the operator
/// registers them under distinct collection names.
struct T1Mapping {
    /// Source path. Kept for diagnostics / `Debug` only — the `mmap`
    /// already owns the page-cache reference.
    #[allow(dead_code)]
    path: PathBuf,
    /// Read-only memory map of the entire `.rvdna` v2 file.
    mmap: Mmap,
    /// Byte offset to a packed `[u64; n_vectors]` ids array (little-
    /// endian). v0.1 accepts an empty range — if `ids_offset == 0` and
    /// `n_vectors > 0` the backend synthesises positional ids.
    ids_offset: usize,
    /// Byte offset to a packed `[f32; n_vectors * dim]` vectors array
    /// (little-endian). MUST be 4-byte aligned.
    vectors_offset: usize,
    /// Number of vectors in this section.
    n_vectors: usize,
    /// Vector dimensionality.
    dim: usize,
    /// Coherence token. Bumps via [`RvdnaT1Backend::bump_generation`]
    /// when the underlying `.rvdna` file rotates.
    generation: u64,
    /// Whether `ids_offset` points at a real id table (true) or the
    /// backend should synthesise positional ids (false).
    has_ids: bool,
}

/// Warm-tier (T1) BackendAdapter. See module docs.
///
/// Holds a name → mmap'd-file-region map. `pull_vectors` decodes the
/// requested section by reinterpreting the mmap window as `&[f32]` via
/// [`bytemuck`] (no `unsafe`) and copying into a `PulledBatch`.
///
/// Thread-safe: the inner table is guarded by an `RwLock`, the same
/// pattern as [`crate::RvdnaT0Backend`].
pub struct RvdnaT1Backend {
    id: String,
    mappings: RwLock<BTreeMap<CollectionId, T1Mapping>>,
}

impl RvdnaT1Backend {
    /// Create an empty T1 backend with a stable id. Operators then
    /// call [`Self::register_file`] for each `.rvdna` v2 section they
    /// want to expose.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            mappings: RwLock::new(BTreeMap::new()),
        }
    }

    /// Register one warm-tier collection backed by an mmap'd file
    /// region.
    ///
    /// Arguments:
    /// - `name`: collection id under this backend (e.g.
    ///   `"protein_brca1"`, `"variants_chr17"`).
    /// - `path`: filesystem path to the `.rvdna` v2 file (or any file
    ///   whose layout matches — v0.1 doesn't parse the header).
    /// - `dim`: vector dimensionality.
    /// - `generation`: starting coherence token.
    /// - `ids_offset`: byte offset to the `[u64]` ids table. Pass `0`
    ///   together with `has_ids=false` to have the backend synthesise
    ///   positional ids `0..n_vectors`.
    /// - `vectors_offset`: byte offset to the `[f32]` vectors table.
    ///   MUST be 4-byte aligned.
    /// - `n_vectors`: number of vectors in this section.
    /// - `has_ids`: see `ids_offset`.
    ///
    /// Returns the previously-registered mapping under this name, if
    /// any (lets operators atomically rotate one section).
    ///
    /// Errors if the file can't be opened / mapped, or if the declared
    /// region is out of bounds.
    #[allow(clippy::too_many_arguments)]
    pub fn register_file(
        &self,
        name: impl Into<String>,
        path: impl AsRef<Path>,
        dim: usize,
        generation: u64,
        ids_offset: usize,
        vectors_offset: usize,
        n_vectors: usize,
        has_ids: bool,
    ) -> Result<()> {
        if dim == 0 {
            return Err(RuLakeError::InvalidParameter(
                "rvdna-t1: dim must be > 0".into(),
            ));
        }
        if vectors_offset % std::mem::align_of::<f32>() != 0 {
            return Err(RuLakeError::InvalidParameter(format!(
                "rvdna-t1: vectors_offset={vectors_offset} not f32-aligned"
            )));
        }
        let path_buf = path.as_ref().to_path_buf();
        let file = std::fs::File::open(&path_buf).map_err(|e| {
            RuLakeError::InvalidParameter(format!(
                "rvdna-t1: open {}: {e}",
                path_buf.display()
            ))
        })?;
        // SAFETY: `memmap2::Mmap::map` is `unsafe` because the OS could
        // mutate the mapped pages out from under us if another process
        // writes to the file. Operators are expected to treat `.rvdna`
        // files as immutable — they're content-addressed by the bundle
        // pointer at byte 0x00B0. The returned `Mmap` is a safe `&[u8]`
        // view from this module's perspective, so the crate-level
        // `#![deny(unsafe_code)]` invariant holds for everything outside
        // this single line.
        #[allow(unsafe_code)]
        let mmap = unsafe { Mmap::map(&file) }.map_err(|e| {
            RuLakeError::InvalidParameter(format!(
                "rvdna-t1: mmap {}: {e}",
                path_buf.display()
            ))
        })?;

        // Bounds-check the declared region against the mapped length.
        let vectors_bytes = n_vectors
            .checked_mul(dim)
            .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| {
                RuLakeError::InvalidParameter(format!(
                    "rvdna-t1: vectors-region size overflow (n={n_vectors}, dim={dim})"
                ))
            })?;
        let vectors_end = vectors_offset.checked_add(vectors_bytes).ok_or_else(|| {
            RuLakeError::InvalidParameter(
                "rvdna-t1: vectors-region offset+size overflow".into(),
            )
        })?;
        if vectors_end > mmap.len() {
            return Err(RuLakeError::InvalidParameter(format!(
                "rvdna-t1: vectors region [{vectors_offset}, {vectors_end}) exceeds file len {}",
                mmap.len()
            )));
        }
        if has_ids {
            let ids_bytes = n_vectors
                .checked_mul(std::mem::size_of::<u64>())
                .ok_or_else(|| {
                    RuLakeError::InvalidParameter(
                        "rvdna-t1: ids-region size overflow".into(),
                    )
                })?;
            let ids_end = ids_offset.checked_add(ids_bytes).ok_or_else(|| {
                RuLakeError::InvalidParameter(
                    "rvdna-t1: ids-region offset+size overflow".into(),
                )
            })?;
            if ids_end > mmap.len() {
                return Err(RuLakeError::InvalidParameter(format!(
                    "rvdna-t1: ids region [{ids_offset}, {ids_end}) exceeds file len {}",
                    mmap.len()
                )));
            }
            if ids_offset % std::mem::align_of::<u64>() != 0 {
                return Err(RuLakeError::InvalidParameter(format!(
                    "rvdna-t1: ids_offset={ids_offset} not u64-aligned"
                )));
            }
        }

        let mapping = T1Mapping {
            path: path_buf,
            mmap,
            ids_offset,
            vectors_offset,
            n_vectors,
            dim,
            generation,
            has_ids,
        };
        self.mappings
            .write()
            .expect("poisoned")
            .insert(name.into(), mapping);
        Ok(())
    }

    /// Bump the generation of a collection — call this after rotating
    /// the underlying `.rvdna` file so the cache invalidates. Mirrors
    /// [`crate::RvdnaT0Backend::bump_generation`] verbatim.
    pub fn bump_generation(&self, name: &str) -> Result<u64> {
        let mut g = self.mappings.write().expect("poisoned");
        match g.get_mut(name) {
            Some(m) => {
                m.generation = m.generation.checked_add(1).unwrap_or(m.generation);
                Ok(m.generation)
            }
            None => Err(RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: name.to_string(),
            }),
        }
    }
}

impl BackendAdapter for RvdnaT1Backend {
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

        // Decode the f32 region. `bytemuck::try_cast_slice` returns an
        // error if the slice isn't aligned / sized correctly — we
        // already checked at registration so this is belt-and-braces.
        let vectors_bytes_len = m.n_vectors * m.dim * std::mem::size_of::<f32>();
        let raw = &m.mmap[m.vectors_offset..m.vectors_offset + vectors_bytes_len];
        let flat: &[f32] = bytemuck::try_cast_slice(raw).map_err(|e| {
            RuLakeError::InvalidParameter(format!(
                "rvdna-t1: vectors region not castable to &[f32]: {e}"
            ))
        })?;

        let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(m.n_vectors);
        for i in 0..m.n_vectors {
            let start = i * m.dim;
            let end = start + m.dim;
            vectors.push(flat[start..end].to_vec());
        }

        let ids: Vec<u64> = if m.has_ids {
            let ids_bytes_len = m.n_vectors * std::mem::size_of::<u64>();
            let raw_ids = &m.mmap[m.ids_offset..m.ids_offset + ids_bytes_len];
            let flat_ids: &[u64] = bytemuck::try_cast_slice(raw_ids).map_err(|e| {
                RuLakeError::InvalidParameter(format!(
                    "rvdna-t1: ids region not castable to &[u64]: {e}"
                ))
            })?;
            flat_ids.to_vec()
        } else {
            (0..m.n_vectors as u64).collect()
        };

        Ok(PulledBatch {
            collection: collection.to_string(),
            ids,
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
