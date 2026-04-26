//! Local copy of the ruLake bundle witness algorithm.
//!
//! Byte-exactly compatible with `src/bundle.rs::compute_witness` from
//! the upstream `rulake` crate. Reproduced here (rather than
//! depending on the crate) so this example stays a standalone binary
//! that doesn't drag the full ruLake build into a CUDA-only example.
//!
//! Tested against the committed fixture
//! `examples/nodejs/01-verify-witness/fixtures/known-good-bundle.json`
//! — see `tests` at the bottom.

use serde::{Deserialize, Serialize};
use sha3::digest::{ExtendableOutput, Update, XofReader};
use sha3::Shake256;

pub const SIDECAR_FILENAME: &str = "table.rulake.json";

const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_FIELD_BYTES: usize = 4 * 1024;
const MAX_WITNESS_HEX: usize = 128;
const SUPPORTED_FORMAT_VERSION: u32 = 2;
const GEN_TAG_NUM: u8 = 0x00;
const GEN_TAG_OPAQUE: u8 = 0x01;

/// `Generation` token. Mirrors the upstream `serde(untagged)` enum so
/// the JSON shape is identical: a JSON number deserializes to `Num`,
/// a JSON string to `Opaque`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Generation {
    Num(u64),
    Opaque(String),
}

impl Generation {
    /// Domain-tagged byte-stream input for the witness digest. The
    /// 1-byte variant tag (audit fix from 2026-04-23) is mandatory:
    /// without it `Num(7)` and `Opaque("\x07\0…")` would collide.
    pub fn hash_bytes(&self) -> Vec<u8> {
        match self {
            Self::Num(n) => {
                let mut out = Vec::with_capacity(1 + 8);
                out.push(GEN_TAG_NUM);
                out.extend_from_slice(&n.to_le_bytes());
                out
            }
            Self::Opaque(s) => {
                let mut out = Vec::with_capacity(1 + s.len());
                out.push(GEN_TAG_OPAQUE);
                out.extend_from_slice(s.as_bytes());
                out
            }
        }
    }
}

/// On-disk bundle shape. `dim`/`rotation_seed`/`rerank_factor` are
/// `u64` here so we can deserialize either the canonical JSON
/// representation (numbers fit in u64) without coupling to a host-
/// specific `usize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuLakeBundle {
    pub format_version: u32,
    pub data_ref: String,
    pub dim: u64,
    pub rotation_seed: u64,
    pub rerank_factor: u64,
    pub generation: Generation,
    pub rvf_witness: String,
    #[serde(default)]
    pub pii_policy: Option<String>,
    #[serde(default)]
    pub lineage_id: Option<String>,
    #[serde(default)]
    pub memory_class: Option<String>,
}

/// SHAKE-256(32) over a length-prefixed, domain-separated byte stream.
/// Identical to `src/bundle.rs::compute_witness` upstream.
pub fn compute_witness(b: &RuLakeBundle) -> String {
    let mut h = Shake256::default();
    h.update(b"rulake-bundle-witness-v1|");
    h.update(&(b.data_ref.len() as u64).to_le_bytes());
    h.update(b.data_ref.as_bytes());
    h.update(b"|");
    h.update(&b.dim.to_le_bytes());
    h.update(&b.rotation_seed.to_le_bytes());
    h.update(&b.rerank_factor.to_le_bytes());
    h.update(b"|");
    let g = b.generation.hash_bytes();
    h.update(&(g.len() as u64).to_le_bytes());
    h.update(&g);
    let mut reader = h.finalize_xof();
    let mut out = [0u8; 32];
    reader.read(&mut out);
    hex::encode(out)
}

#[derive(Debug)]
pub struct WitnessError(pub String);

impl std::fmt::Display for WitnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for WitnessError {}

/// Read `dir/table.rulake.json`, parse with the upstream length caps,
/// and verify the witness. Errors on any caps violation, version
/// mismatch, or witness mismatch.
pub fn read_and_verify(dir: &std::path::Path) -> Result<RuLakeBundle, WitnessError> {
    let path = dir.join(SIDECAR_FILENAME);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| WitnessError(format!("bundle read: open {}: {e}", path.display())))?;
    if raw.len() > MAX_JSON_BYTES {
        return Err(WitnessError(format!(
            "bundle parse: exceeds {MAX_JSON_BYTES} bytes"
        )));
    }
    let b: RuLakeBundle = serde_json::from_str(&raw)
        .map_err(|e| WitnessError(format!("bundle parse: {e}")))?;
    if b.format_version > SUPPORTED_FORMAT_VERSION {
        return Err(WitnessError(format!(
            "bundle format_version={} newer than this binary supports ({})",
            b.format_version, SUPPORTED_FORMAT_VERSION
        )));
    }
    for (name, opt) in [
        ("pii_policy", b.pii_policy.as_deref()),
        ("lineage_id", b.lineage_id.as_deref()),
        ("memory_class", b.memory_class.as_deref()),
    ] {
        if let Some(v) = opt {
            if v.len() > MAX_FIELD_BYTES {
                return Err(WitnessError(format!(
                    "bundle parse: {name} exceeds {MAX_FIELD_BYTES} bytes"
                )));
            }
        }
    }
    if b.data_ref.len() > MAX_FIELD_BYTES {
        return Err(WitnessError(format!(
            "bundle parse: data_ref exceeds {MAX_FIELD_BYTES} bytes"
        )));
    }
    if b.rvf_witness.len() > MAX_WITNESS_HEX {
        return Err(WitnessError(
            "bundle parse: rvf_witness not a hex-encoded SHAKE-256(32)".to_string(),
        ));
    }
    let computed = compute_witness(&b);
    if computed != b.rvf_witness {
        return Err(WitnessError(format!(
            "bundle witness mismatch at {}: computed {} but found {}",
            path.display(),
            computed,
            b.rvf_witness
        )));
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed fixture used by every cross-language verifier.
    /// If this hash drifts the witness algorithm has been broken.
    const KNOWN_GOOD_WITNESS: &str =
        "dea58c64adb1eb4109438f0353a2b1749d4dc29ed7266e9236720ab6cf07d7e4";

    #[test]
    fn matches_known_good_fixture() {
        let b = RuLakeBundle {
            format_version: 2,
            data_ref: "local://publisher/memories".to_string(),
            dim: 8,
            rotation_seed: 42,
            rerank_factor: 20,
            generation: Generation::Num(1),
            rvf_witness: KNOWN_GOOD_WITNESS.to_string(),
            pii_policy: None,
            lineage_id: None,
            memory_class: None,
        };
        assert_eq!(compute_witness(&b), KNOWN_GOOD_WITNESS);
    }

    #[test]
    fn variant_tag_prevents_collision() {
        // Regression of the audit-driven fix: Num(7) vs the Opaque
        // string whose bytes look like u64::to_le_bytes(7).
        let a = RuLakeBundle {
            format_version: 2,
            data_ref: "x".into(),
            dim: 1,
            rotation_seed: 0,
            rerank_factor: 1,
            generation: Generation::Num(7),
            rvf_witness: String::new(),
            pii_policy: None,
            lineage_id: None,
            memory_class: None,
        };
        let mut b = a.clone();
        b.generation = Generation::Opaque("\x07\0\0\0\0\0\0\0".to_string());
        assert_ne!(compute_witness(&a), compute_witness(&b));
    }
}
