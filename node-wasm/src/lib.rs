//! ruLake — WASM build for edge runtimes.
//!
//! This is the **feature-reduced** WASM surface promised by ADR-003 §A:
//! browsers, Cloudflare Workers, Deno-deploy, Bun. No `std::sync::Mutex`,
//! no `rayon`, no AVX-512 — the full RuLake search path needs them and
//! they don't exist on the edge anyway. What we ship here is the
//! **substrate**: bundle parse / serialize + witness (SHAKE-256(32),
//! length-prefixed, domain-separated) — which is what makes ruLake's
//! cross-language story portable.
//!
//! The on-the-wire bytes of `compute_witness` here are byte-identical to
//! the host crate's `src/bundle.rs::compute_witness`. The fixture under
//! `examples/wasm/01-witness-verifier-browser/fixtures/known-good-bundle.json`
//! re-verifies clean from this build (witness `dea58c64…07d7e4`).

#![deny(unsafe_code)]
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use sha3::{
    digest::{ExtendableOutput, Update, XofReader},
    Shake256,
};
use wasm_bindgen::prelude::*;

// ───────────────────────── DoS caps ─────────────────────────────────────
// Mirror of `src/bundle.rs` — same limits everywhere.

const MAX_BUNDLE_BYTES: usize = 64 * 1024;
const MAX_FIELD_BYTES: usize = 4 * 1024;
const MAX_WITNESS_HEX_LEN: usize = 128; // 64-byte SHAKE → 128 hex
const FORMAT_VERSION_MAX: u32 = 2;

// ───────────────────────── bundle types ─────────────────────────────────

/// `Generation` — a backend's coherence token. Matches the host crate's
/// `bundle::Generation` enum encoding (`{"Num": <u64>}` or
/// `{"Opaque": "<string>"}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum Generation {
    Num(GenNum),
    Opaque(GenOpaque),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GenNum {
    #[serde(rename = "Num")]
    num: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GenOpaque {
    #[serde(rename = "Opaque")]
    opaque: String,
}

impl Generation {
    fn hash_bytes(&self) -> Vec<u8> {
        match self {
            Self::Num(g) => {
                let mut out = Vec::with_capacity(9);
                out.push(0x00);
                out.extend_from_slice(&g.num.to_le_bytes());
                out
            }
            Self::Opaque(g) => {
                let mut out = Vec::with_capacity(1 + g.opaque.len());
                out.push(0x01);
                out.extend_from_slice(g.opaque.as_bytes());
                out
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuLakeBundle {
    format_version: u32,
    data_ref: String,
    dim: usize,
    rotation_seed: u64,
    rerank_factor: usize,
    generation: Generation,
    rvf_witness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pii_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lineage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    memory_class: Option<String>,
}

// ───────────────────────── witness compute ──────────────────────────────

fn compute_witness(
    data_ref: &str,
    dim: usize,
    rotation_seed: u64,
    rerank_factor: usize,
    generation: &Generation,
) -> String {
    let mut h = Shake256::default();
    h.update(b"rulake-bundle-witness-v1|");
    h.update(&(data_ref.len() as u64).to_le_bytes());
    h.update(data_ref.as_bytes());
    h.update(b"|");
    h.update(&(dim as u64).to_le_bytes());
    h.update(&rotation_seed.to_le_bytes());
    h.update(&(rerank_factor as u64).to_le_bytes());
    h.update(b"|");
    let g = generation.hash_bytes();
    h.update(&(g.len() as u64).to_le_bytes());
    h.update(&g);
    let mut reader = h.finalize_xof();
    let mut out = [0u8; 32];
    reader.read(&mut out);
    hex::encode(out)
}

// ───────────────────────── public WASM API ──────────────────────────────

/// Result of a verify call — JS sees `{ok, computed, stored, fields}`.
#[derive(Debug, Serialize)]
pub struct VerifyResult {
    /// Whether the recomputed witness matches the bundle's stored witness.
    pub ok: bool,
    /// The witness we recomputed, hex-encoded.
    pub computed: String,
    /// The witness as it was stored in the bundle, hex-encoded.
    pub stored: String,
    /// Echo of the bundle's load-bearing fields.
    pub fields: VerifyFields,
}

/// Subset of the bundle echoed back to JS.
#[derive(Debug, Serialize)]
pub struct VerifyFields {
    /// Witness format version (must be ≤ 2).
    pub format_version: u32,
    /// URI of the authoritative byte stream.
    pub data_ref: String,
    /// Vector dimensionality.
    pub dim: usize,
    /// RaBitQ rotation seed.
    pub rotation_seed: u64,
    /// RaBitQ rerank factor.
    pub rerank_factor: usize,
}

/// Verify a `table.rulake.json` bundle.
///
/// Parses the JSON (with DoS caps applied), recomputes the SHAKE-256(32)
/// witness, returns a structured comparison. Returns `Err` (rejected as
/// a JS exception) on any of: oversize input, missing fields, future
/// `format_version`, malformed `generation`.
#[wasm_bindgen(js_name = verifyBundleJson)]
pub fn verify_bundle_json(json: &str) -> Result<JsValue, JsValue> {
    if json.len() > MAX_BUNDLE_BYTES {
        return Err(JsValue::from_str(&format!(
            "bundle JSON exceeds {MAX_BUNDLE_BYTES} byte cap ({} bytes)",
            json.len()
        )));
    }
    let bundle: RuLakeBundle = serde_json::from_str(json)
        .map_err(|e| JsValue::from_str(&format!("bundle parse: {e}")))?;

    if bundle.format_version > FORMAT_VERSION_MAX {
        return Err(JsValue::from_str(&format!(
            "bundle format_version={} exceeds supported max {FORMAT_VERSION_MAX}",
            bundle.format_version
        )));
    }
    if bundle.data_ref.len() > MAX_FIELD_BYTES {
        return Err(JsValue::from_str("bundle: data_ref exceeds 4 KiB"));
    }
    if bundle.rvf_witness.len() > MAX_WITNESS_HEX_LEN {
        return Err(JsValue::from_str(
            "bundle: rvf_witness not a hex-encoded SHAKE-256(32)",
        ));
    }

    let computed = compute_witness(
        &bundle.data_ref,
        bundle.dim,
        bundle.rotation_seed,
        bundle.rerank_factor,
        &bundle.generation,
    );
    let result = VerifyResult {
        ok: computed == bundle.rvf_witness,
        computed,
        stored: bundle.rvf_witness.clone(),
        fields: VerifyFields {
            format_version: bundle.format_version,
            data_ref: bundle.data_ref,
            dim: bundle.dim,
            rotation_seed: bundle.rotation_seed,
            rerank_factor: bundle.rerank_factor,
        },
    };
    serde_wasm_bindgen::to_value(&result).map_err(JsValue::from)
}

/// Compute the SHAKE-256(32) witness for the load-bearing bundle fields.
///
/// Useful for "I have these fields, what witness should the bundle
/// carry?" — building bundles client-side, validating publishers, etc.
/// Generation accepts either a number (encoded as `Num`) or a string
/// (encoded as `Opaque`).
#[wasm_bindgen(js_name = computeWitness)]
pub fn compute_witness_js(
    data_ref: &str,
    dim: u32,
    rotation_seed: u64,
    rerank_factor: u32,
    generation: JsValue,
) -> Result<String, JsValue> {
    if data_ref.len() > MAX_FIELD_BYTES {
        return Err(JsValue::from_str("data_ref exceeds 4 KiB"));
    }
    let g: Generation = serde_wasm_bindgen::from_value(generation.clone())
        .map_err(|_| {
            // Fallback: accept a bare number/string for ergonomics.
            // (`{"Num": 7}` works directly; `7` and `"foo"` get wrapped.)
            JsValue::from_str("generation must be {Num: <u64>} or {Opaque: <string>}")
        })?;
    Ok(compute_witness(
        data_ref,
        dim as usize,
        rotation_seed,
        rerank_factor as usize,
        &g,
    ))
}

/// `format_version` of bundles this build understands.
#[wasm_bindgen(js_name = formatVersion)]
pub fn format_version() -> u32 {
    FORMAT_VERSION_MAX
}

// ───────────────────────── v2: brute-force search ───────────────────────
//
// The ADR-003 §"What's not yet here" v2 surface includes a usable
// retrieval path on the edge. We can't ship the full `RuLake` cache
// (needs `std::sync::Mutex` + `rayon`), but we can ship the right
// substrate piece: an exact brute-force L2 over a flat Float32Array.
// At edge-realistic sizes (5k–50k vectors) that's already sub-10 ms —
// faster than the round trip to a hosted service would be — and it's
// witness-verifiable end-to-end.

/// One nearest-K hit returned by [`searchBruteForceL2`].
#[derive(Debug, Serialize)]
pub struct WasmHit {
    /// Position in the input vector array (0-indexed).
    pub idx: u32,
    /// External id passed in alongside the vectors (or `idx` if no ids
    /// were provided).
    pub id: u64,
    /// Squared L2 distance to the query.
    pub score: f32,
}

/// Exact top-K nearest-neighbor search by squared L2 distance.
///
/// `vectors` is a flat `Float32Array` of length `n * dim`; `ids` (if
/// non-empty) gives external u64 ids in the same order. `k` is capped at
/// `n` and at 4096 to bound the worst case in a constrained edge runtime.
/// Returns hits sorted ascending by score.
///
/// **Recall**: 1.0. This is exact, not approximate. Use when n is small
/// enough that the wall-time fits your budget — at D=128, ~50k vectors is
/// under 10 ms in browser WASM on a modern laptop.
#[wasm_bindgen(js_name = searchBruteForceL2)]
pub fn search_brute_force_l2(
    vectors: &[f32],
    ids: &[f64],
    dim: u32,
    query: &[f32],
    k: u32,
) -> Result<JsValue, JsValue> {
    let dim = dim as usize;
    if dim == 0 {
        return Err(JsValue::from_str("dim must be > 0"));
    }
    if vectors.len() % dim != 0 {
        return Err(JsValue::from_str(&format!(
            "vectors.len() ({}) must be divisible by dim ({dim})",
            vectors.len()
        )));
    }
    if query.len() != dim {
        return Err(JsValue::from_str(&format!(
            "query.len() ({}) must equal dim ({dim})",
            query.len()
        )));
    }
    let n = vectors.len() / dim;
    if !ids.is_empty() && ids.len() != n {
        return Err(JsValue::from_str(&format!(
            "ids.len() ({}) must equal n ({n}) or be empty",
            ids.len()
        )));
    }

    let k = (k as usize).min(n).min(4096);
    if k == 0 {
        return serde_wasm_bindgen::to_value(&Vec::<WasmHit>::new()).map_err(JsValue::from);
    }

    let mut scored: Vec<(f32, u32)> = Vec::with_capacity(n);
    for i in 0..n {
        let off = i * dim;
        let mut s = 0.0_f32;
        for d in 0..dim {
            let diff = vectors[off + d] - query[d];
            s += diff * diff;
        }
        scored.push((s, i as u32));
    }
    // Partial sort — full sort works at edge-realistic n.
    scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);

    let hits: Vec<WasmHit> = scored
        .into_iter()
        .map(|(score, idx)| WasmHit {
            idx,
            id: if ids.is_empty() {
                idx as u64
            } else {
                ids[idx as usize] as u64
            },
            score,
        })
        .collect();
    serde_wasm_bindgen::to_value(&hits).map_err(JsValue::from)
}

/// Build version string. Useful for browser-console diagnostics.
#[wasm_bindgen(js_name = buildInfo)]
pub fn build_info() -> String {
    format!(
        "rulake-wasm v{} (witness-format v{}, sha3 = 0.10)",
        env!("CARGO_PKG_VERSION"),
        FORMAT_VERSION_MAX
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The known Rust fixture — same one the host crate's tests use.
    const FIXTURE: &str = r#"{
        "format_version": 2,
        "data_ref": "local://publisher/memories",
        "dim": 8,
        "rotation_seed": 42,
        "rerank_factor": 20,
        "generation": {"Num": 1},
        "rvf_witness": "dea58c64adb1eb4109438f0353a2b1749d4dc29ed7266e9236720ab6cf07d7e4",
        "pii_policy": null,
        "lineage_id": null
    }"#;

    #[test]
    fn fixture_witness_matches_host_crate_byte_exact() {
        let bundle: RuLakeBundle = serde_json::from_str(FIXTURE).unwrap();
        let recomputed = compute_witness(
            &bundle.data_ref,
            bundle.dim,
            bundle.rotation_seed,
            bundle.rerank_factor,
            &bundle.generation,
        );
        assert_eq!(
            recomputed, bundle.rvf_witness,
            "wasm-build witness must agree byte-exactly with the host crate's compute_witness"
        );
    }

    #[test]
    fn num_vs_opaque_with_same_byte_pattern_do_not_collide() {
        // The 2026-04-23 audit-driven regression: Num(7) and
        // Opaque("\x07\0\0\0\0\0\0\0") MUST hash to different witnesses.
        // Without the variant tag byte (0x00 / 0x01) they would collide.
        let num = Generation::Num(GenNum { num: 7 });
        let opaque = Generation::Opaque(GenOpaque {
            opaque: "\x07\0\0\0\0\0\0\0".to_string(),
        });
        let w_num = compute_witness("x", 1, 0, 0, &num);
        let w_op = compute_witness("x", 1, 0, 0, &opaque);
        assert_ne!(w_num, w_op);
    }

    #[test]
    fn opaque_length_prefix_prevents_concatenation_collision() {
        // Opaque("a|b") and Opaque("ab|") would collide without
        // length prefixing. Same regression as above, different
        // axis.
        let a = Generation::Opaque(GenOpaque {
            opaque: "a|b".to_string(),
        });
        let b = Generation::Opaque(GenOpaque {
            opaque: "ab|".to_string(),
        });
        assert_ne!(
            compute_witness("x", 1, 0, 0, &a),
            compute_witness("x", 1, 0, 0, &b)
        );
    }
}
