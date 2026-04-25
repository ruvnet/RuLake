//! `ruvec1` binary parser. Same shape as the module-1 copy — see the
//! upstream `src/fs_backend.rs` for the format and the upstream
//! `src/backend.rs::MAX_PULLED_*` for the caps.

use std::path::Path;

const MAGIC: [u8; 8] = *b"ruvec1\0\0";
const HEADER_BYTES: usize = 24;

pub const MAX_PULLED_VECTORS: usize = 100_000_000;
pub const MAX_PULLED_DIM: usize = 8192;
pub const MAX_PULLED_BYTES: usize = 16 * 1024 * 1024 * 1024;

#[derive(Debug)]
pub struct Ruvec1Error(pub String);

impl std::fmt::Display for Ruvec1Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Ruvec1Error {}

#[derive(Debug)]
pub struct Corpus {
    pub ids: Vec<u64>,
    pub vectors: Vec<f32>,
    pub dim: usize,
    pub count: usize,
}

pub fn read(path: &Path, max_vectors: usize) -> Result<Corpus, Ruvec1Error> {
    let raw = std::fs::read(path)
        .map_err(|e| Ruvec1Error(format!("ruvec1 read: open {}: {e}", path.display())))?;
    if raw.len() < HEADER_BYTES {
        return Err(Ruvec1Error(format!(
            "{}: truncated header ({} bytes)",
            path.display(),
            raw.len()
        )));
    }
    if raw[..8] != MAGIC {
        return Err(Ruvec1Error(format!(
            "{}: bad magic {:?}",
            path.display(),
            &raw[..8]
        )));
    }
    let count_u64 = u64::from_le_bytes(raw[8..16].try_into().unwrap());
    let dim_u32 = u32::from_le_bytes(raw[16..20].try_into().unwrap());
    let count = usize::try_from(count_u64).map_err(|_| {
        Ruvec1Error(format!(
            "{}: count={count_u64} exceeds usize",
            path.display()
        ))
    })?;
    let dim = dim_u32 as usize;
    if count > MAX_PULLED_VECTORS {
        return Err(Ruvec1Error(format!(
            "{}: count={count} exceeds MAX_PULLED_VECTORS",
            path.display()
        )));
    }
    if dim == 0 || dim > MAX_PULLED_DIM {
        return Err(Ruvec1Error(format!(
            "{}: dim={dim} outside (0, MAX_PULLED_DIM]",
            path.display()
        )));
    }
    let record_bytes = 8usize.checked_add(dim.checked_mul(4).ok_or_else(|| {
        Ruvec1Error(format!("{}: dim*4 overflow", path.display()))
    })?)
    .ok_or_else(|| Ruvec1Error(format!("{}: record bytes overflow", path.display())))?;
    let payload_bytes = count
        .checked_mul(record_bytes)
        .ok_or_else(|| Ruvec1Error(format!("{}: payload bytes overflow", path.display())))?;
    let total_bytes = HEADER_BYTES
        .checked_add(payload_bytes)
        .ok_or_else(|| Ruvec1Error(format!("{}: total bytes overflow", path.display())))?;
    if total_bytes > MAX_PULLED_BYTES {
        return Err(Ruvec1Error(format!(
            "{}: payload {total_bytes} bytes exceeds MAX_PULLED_BYTES",
            path.display()
        )));
    }
    if raw.len() != total_bytes {
        return Err(Ruvec1Error(format!(
            "{}: payload size {} != expected {} (count={}, dim={})",
            path.display(),
            raw.len(),
            total_bytes,
            count,
            dim
        )));
    }
    let kept = count.min(max_vectors);
    let mut ids = Vec::with_capacity(kept);
    let mut vectors = Vec::with_capacity(kept * dim);
    let mut off = HEADER_BYTES;
    for _ in 0..kept {
        let id = u64::from_le_bytes(raw[off..off + 8].try_into().unwrap());
        off += 8;
        ids.push(id);
        for k in 0..dim {
            let lo = off + k * 4;
            vectors.push(f32::from_le_bytes(raw[lo..lo + 4].try_into().unwrap()));
        }
        off += dim * 4;
    }
    Ok(Corpus {
        ids,
        vectors,
        dim,
        count: kept,
    })
}

pub fn write(
    path: &Path,
    dim: usize,
    ids: &[u64],
    vectors: &[Vec<f32>],
) -> Result<(), Ruvec1Error> {
    use std::io::Write;
    if ids.len() != vectors.len() {
        return Err(Ruvec1Error(format!(
            "ruvec1 write: ids.len={} != vectors.len={}",
            ids.len(),
            vectors.len()
        )));
    }
    if dim == 0 || dim > MAX_PULLED_DIM {
        return Err(Ruvec1Error(format!(
            "ruvec1 write: dim={dim} outside (0, MAX_PULLED_DIM]"
        )));
    }
    for (i, v) in vectors.iter().enumerate() {
        if v.len() != dim {
            return Err(Ruvec1Error(format!(
                "ruvec1 write: vectors[{i}] has len {} (expected {dim})",
                v.len()
            )));
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            Ruvec1Error(format!("ruvec1 write: mkdir {}: {e}", parent.display()))
        })?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.tmp",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("ruvec1")
    ));
    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| Ruvec1Error(format!("ruvec1 write: create {}: {e}", tmp.display())))?;
        f.write_all(&MAGIC)
            .and_then(|_| f.write_all(&(ids.len() as u64).to_le_bytes()))
            .and_then(|_| f.write_all(&(dim as u32).to_le_bytes()))
            .and_then(|_| f.write_all(&0u32.to_le_bytes()))
            .map_err(|e| Ruvec1Error(format!("ruvec1 write: header: {e}")))?;
        for (id, v) in ids.iter().zip(vectors.iter()) {
            f.write_all(&id.to_le_bytes())
                .map_err(|e| Ruvec1Error(format!("ruvec1 write: id: {e}")))?;
            for x in v {
                f.write_all(&x.to_le_bytes())
                    .map_err(|e| Ruvec1Error(format!("ruvec1 write: f32: {e}")))?;
            }
        }
        f.sync_all()
            .map_err(|e| Ruvec1Error(format!("ruvec1 write: fsync: {e}")))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        Ruvec1Error(format!(
            "ruvec1 write: rename {} → {}: {e}",
            tmp.display(),
            path.display()
        ))
    })?;
    Ok(())
}
