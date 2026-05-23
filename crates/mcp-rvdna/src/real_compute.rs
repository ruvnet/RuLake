//! Phase B: real compute for the rvdna_* tools (no more `stub: true`).
//!
//! These are pure-Rust implementations that operate on whatever the
//! registered backend pulls. We deliberately do NOT pull in the heavy
//! `vendor/ruvector/examples/dna` dep chain (`ruvector-{core,attention,
//! gnn,graph}`) because the v0.0 wire only needs:
//!
//!   - kNN over the registered vectors  (brute-force L2 — deterministic)
//!   - DNA → protein                    (standard codon table)
//!   - region → score                   (deterministic hash)
//!
//! All three are <50 lines of pure Rust each. When the operator
//! registers a richer backend with more vectors, the `find` brute force
//! becomes the bottleneck and the swap to a HNSW index is a follow-up.
//! For the v0.0 demo collection (64 vectors at D=8) brute force is
//! ~50 µs — well under the budget.

use std::sync::Arc;

use rulake::backend::BackendAdapter;

/// Real kNN: pull all vectors from the backend and brute-force L2.
/// Returns `(id, l2_distance)` pairs sorted ascending by distance,
/// truncated to top-k.
///
/// Determinism: ties broken by lower id (matches the kernel
/// conformance contract from ADR-157).
pub fn knn_l2(
    backend: &Arc<dyn BackendAdapter>,
    collection: &str,
    query: &[f32],
    k: usize,
) -> Result<Vec<(u64, f32)>, String> {
    let batch = backend
        .pull_vectors(collection)
        .map_err(|e| format!("pull_vectors: {e}"))?;
    let dim = query.len().min(batch.dim);
    let mut scored: Vec<(u64, f32)> = batch
        .vectors
        .iter()
        .zip(batch.ids.iter())
        .map(|(v, &id)| {
            let mut acc = 0.0f32;
            for j in 0..dim.min(v.len()) {
                let d = query[j] - v[j];
                acc += d * d;
            }
            (id, acc)
        })
        .collect();
    scored.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    scored.truncate(k);
    Ok(scored)
}

/// Standard genetic-code translation. Stops at first stop codon ('*')
/// per molecular-biology convention.
///
/// Input: DNA bases A/C/G/T (case-insensitive). Anything else maps to
/// 'X' (unknown amino acid). Length not divisible by 3: trailing
/// 1-2 bases are ignored.
pub fn translate_dna(dna: &str) -> String {
    let codon_table = |a: u8, b: u8, c: u8| -> char {
        let key = [a, b, c];
        match &key {
            // Phenylalanine
            b"TTT" | b"TTC" => 'F',
            // Leucine
            b"TTA" | b"TTG" | b"CTT" | b"CTC" | b"CTA" | b"CTG" => 'L',
            // Isoleucine
            b"ATT" | b"ATC" | b"ATA" => 'I',
            // Methionine (start)
            b"ATG" => 'M',
            // Valine
            b"GTT" | b"GTC" | b"GTA" | b"GTG" => 'V',
            // Serine
            b"TCT" | b"TCC" | b"TCA" | b"TCG" | b"AGT" | b"AGC" => 'S',
            // Proline
            b"CCT" | b"CCC" | b"CCA" | b"CCG" => 'P',
            // Threonine
            b"ACT" | b"ACC" | b"ACA" | b"ACG" => 'T',
            // Alanine
            b"GCT" | b"GCC" | b"GCA" | b"GCG" => 'A',
            // Tyrosine
            b"TAT" | b"TAC" => 'Y',
            // Histidine
            b"CAT" | b"CAC" => 'H',
            // Glutamine
            b"CAA" | b"CAG" => 'Q',
            // Asparagine
            b"AAT" | b"AAC" => 'N',
            // Lysine
            b"AAA" | b"AAG" => 'K',
            // Aspartate
            b"GAT" | b"GAC" => 'D',
            // Glutamate
            b"GAA" | b"GAG" => 'E',
            // Cysteine
            b"TGT" | b"TGC" => 'C',
            // Tryptophan
            b"TGG" => 'W',
            // Arginine
            b"CGT" | b"CGC" | b"CGA" | b"CGG" | b"AGA" | b"AGG" => 'R',
            // Glycine
            b"GGT" | b"GGC" | b"GGA" | b"GGG" => 'G',
            // Stop codons
            b"TAA" | b"TAG" | b"TGA" => '*',
            _ => 'X',
        }
    };

    let bytes: Vec<u8> = dna
        .bytes()
        .filter_map(|b| {
            match b.to_ascii_uppercase() {
                b'A' | b'C' | b'G' | b'T' => Some(b.to_ascii_uppercase()),
                _ => None, // skip whitespace, Ns, anything non-canonical
            }
        })
        .collect();

    let mut protein = String::with_capacity(bytes.len() / 3);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let aa = codon_table(bytes[i], bytes[i + 1], bytes[i + 2]);
        if aa == '*' {
            break;
        }
        protein.push(aa);
        i += 3;
    }
    protein
}

/// Deterministic scalar score for a (witness, region) pair. Returns a
/// value in `[0.0, 1.0]` from a simple FNV-1a hash mixed with the
/// region bytes. Production scoring would use the registered ML model;
/// for the v0.0 wire this gives a deterministic, region-sensitive
/// number that's reproducible across operators.
pub fn score_region(witness: &str, region: &str) -> f64 {
    // FNV-1a 64-bit
    let mut h: u64 = 0xcbf29ce484222325;
    for b in witness.bytes().chain(region.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    // Map u64 → [0, 1)
    (h >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_start_codon_only() {
        assert_eq!(translate_dna("ATG"), "M");
    }

    #[test]
    fn translate_bell_pair_stops_at_stop_codon() {
        // ATG (M) + TAA (stop) → "M"
        assert_eq!(translate_dna("ATGTAA"), "M");
    }

    #[test]
    fn translate_full_protein() {
        // ATG GCC TGC GAA TAA → M A C E *  (stops on the *)
        assert_eq!(translate_dna("ATGGCCTGCGAATAA"), "MACE");
    }

    #[test]
    fn translate_handles_lowercase_and_skips_whitespace() {
        assert_eq!(translate_dna("atg gcc tgc"), "MAC");
    }

    #[test]
    fn translate_skips_n_bases() {
        // ATG NNN GCC → ATG (skip Ns) GCC → MA
        assert_eq!(translate_dna("ATGNNNGCC"), "MA");
    }

    #[test]
    fn score_is_deterministic_and_in_unit_range() {
        let s1 = score_region("witness-abc", "chr1:0-100");
        let s2 = score_region("witness-abc", "chr1:0-100");
        let s3 = score_region("witness-abc", "chr1:100-200");
        assert_eq!(s1, s2, "same inputs → same score");
        assert!(s1 != s3, "different region → different score");
        assert!((0.0..1.0).contains(&s1));
        assert!((0.0..1.0).contains(&s3));
    }

    #[test]
    fn score_is_witness_sensitive() {
        let a = score_region("witness-A", "chr1:0-100");
        let b = score_region("witness-B", "chr1:0-100");
        assert!(a != b, "different witness → different score");
    }
}
