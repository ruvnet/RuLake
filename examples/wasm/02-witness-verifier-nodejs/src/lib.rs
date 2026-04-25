//! Node.js-target ruLake bundle witness verifier.
//!
//! Built with `wasm-pack build --target nodejs`. Produces a CommonJS
//! module that Node.js can `require()` directly — no bundler, no glue
//! beyond what wasm-bindgen emits.
//!
//! The verifier source is intentionally a near-clone of the browser
//! example so behavior stays identical across targets. The host
//! `src/bundle.rs::compute_witness` remains the source of truth.

#![deny(unsafe_code)]

use serde::{Deserialize, Serialize};
use sha3::{
    digest::{ExtendableOutput, Update, XofReader},
    Shake256,
};
use wasm_bindgen::prelude::*;

const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_FIELD_BYTES: usize = 4 * 1024;
const MAX_WITNESS_HEX: usize = 128;
const SUPPORTED_FORMAT_VERSION: u32 = 2;
const GEN_TAG_NUM: u8 = 0x00;
const GEN_TAG_OPAQUE: u8 = 0x01;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
enum Generation {
    Num(u64),
    Opaque(String),
}

impl Generation {
    fn hash_bytes(&self) -> Vec<u8> {
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

    fn pretty(&self) -> String {
        match self {
            Self::Num(n) => format!("Num({n})"),
            Self::Opaque(s) => format!("Opaque(\"{s}\")"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuLakeBundle {
    format_version: u32,
    data_ref: String,
    dim: u64,
    rotation_seed: u64,
    rerank_factor: u64,
    generation: Generation,
    rvf_witness: String,
    #[serde(default)]
    pii_policy: Option<String>,
    #[serde(default)]
    lineage_id: Option<String>,
    #[serde(default)]
    memory_class: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyResult {
    pub ok: bool,
    pub computed: String,
    pub stored: String,
    pub fields: BundleFields,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BundleFields {
    pub format_version: u32,
    pub data_ref: String,
    pub dim: u64,
    pub rotation_seed: u64,
    pub rerank_factor: u64,
    pub generation: String,
    pub pii_policy: Option<String>,
    pub lineage_id: Option<String>,
    pub memory_class: Option<String>,
}

fn compute_witness(
    data_ref: &str,
    dim: u64,
    rotation_seed: u64,
    rerank_factor: u64,
    generation: &Generation,
) -> String {
    let mut h = Shake256::default();
    h.update(b"rulake-bundle-witness-v1|");
    h.update(&(data_ref.len() as u64).to_le_bytes());
    h.update(data_ref.as_bytes());
    h.update(b"|");
    h.update(&dim.to_le_bytes());
    h.update(&rotation_seed.to_le_bytes());
    h.update(&rerank_factor.to_le_bytes());
    h.update(b"|");
    let g = generation.hash_bytes();
    h.update(&(g.len() as u64).to_le_bytes());
    h.update(&g);
    let mut reader = h.finalize_xof();
    let mut out = [0u8; 32];
    reader.read(&mut out);
    hex::encode(out)
}

fn validate_caps(b: &RuLakeBundle, raw_len: usize) -> Result<(), String> {
    if raw_len > MAX_JSON_BYTES {
        return Err(format!("bundle parse: exceeds {MAX_JSON_BYTES} bytes"));
    }
    if b.format_version > SUPPORTED_FORMAT_VERSION {
        return Err(format!(
            "bundle format_version={} newer than this verifier supports ({SUPPORTED_FORMAT_VERSION})",
            b.format_version
        ));
    }
    if b.data_ref.len() > MAX_FIELD_BYTES {
        return Err(format!("bundle parse: data_ref exceeds {MAX_FIELD_BYTES} bytes"));
    }
    for (name, opt) in [
        ("pii_policy", b.pii_policy.as_deref()),
        ("lineage_id", b.lineage_id.as_deref()),
        ("memory_class", b.memory_class.as_deref()),
    ] {
        if let Some(v) = opt {
            if v.len() > MAX_FIELD_BYTES {
                return Err(format!("bundle parse: {name} exceeds {MAX_FIELD_BYTES} bytes"));
            }
        }
    }
    if b.rvf_witness.len() > MAX_WITNESS_HEX {
        return Err("bundle parse: rvf_witness not a hex-encoded SHAKE-256(32)".to_string());
    }
    Ok(())
}

/// Verify a bundle JSON string. Returns a plain JS object on success.
/// Throws on malformed/oversize input.
#[wasm_bindgen]
pub fn verify_bundle_json(json: &str) -> Result<JsValue, JsValue> {
    if json.len() > MAX_JSON_BYTES {
        return Err(JsValue::from_str(&format!(
            "bundle parse: exceeds {MAX_JSON_BYTES} bytes"
        )));
    }
    let bundle: RuLakeBundle = serde_json::from_str(json)
        .map_err(|e| JsValue::from_str(&format!("bundle parse: {e}")))?;
    validate_caps(&bundle, json.len()).map_err(|e| JsValue::from_str(&e))?;

    let computed = compute_witness(
        &bundle.data_ref,
        bundle.dim,
        bundle.rotation_seed,
        bundle.rerank_factor,
        &bundle.generation,
    );
    let ok = computed == bundle.rvf_witness;

    let result = VerifyResult {
        ok,
        computed: computed.clone(),
        stored: bundle.rvf_witness.clone(),
        fields: BundleFields {
            format_version: bundle.format_version,
            data_ref: bundle.data_ref.clone(),
            dim: bundle.dim,
            rotation_seed: bundle.rotation_seed,
            rerank_factor: bundle.rerank_factor,
            generation: bundle.generation.pretty(),
            pii_policy: bundle.pii_policy.clone(),
            lineage_id: bundle.lineage_id.clone(),
            memory_class: bundle.memory_class.clone(),
        },
        error: if ok {
            None
        } else {
            Some("witness mismatch — bundle has been tampered with or was produced by a different witness algorithm".to_string())
        },
    };

    serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[wasm_bindgen]
pub fn compute_witness_for_fields(
    data_ref: &str,
    dim: u64,
    rotation_seed: u64,
    rerank_factor: u64,
    generation_json: &str,
) -> Result<String, JsValue> {
    let g: Generation = serde_json::from_str(generation_json)
        .map_err(|e| JsValue::from_str(&format!("generation parse: {e}")))?;
    Ok(compute_witness(data_ref, dim, rotation_seed, rerank_factor, &g))
}
