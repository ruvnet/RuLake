/**
 * Compute a ruLake bundle's `rvf_witness` from Node, byte-for-byte
 * compatible with `src/bundle.rs::compute_witness` in the Rust crate.
 *
 * The witness is a SHAKE-256(32 bytes) hex digest over a stable,
 * length-prefixed concatenation of the witnessed bundle fields, with a
 * domain-separating prefix.
 *
 *   SHAKE-256(32) of:
 *       "rulake-bundle-witness-v1|"
 *       || u64_le(len(data_ref))
 *       || data_ref bytes
 *       || "|"
 *       || u64_le(dim)
 *       || u64_le(rotation_seed)
 *       || u64_le(rerank_factor)
 *       || "|"
 *       || u64_le(len(generation_bytes))
 *       || generation_bytes
 *
 *   generation_bytes:
 *       Num(n)     → 0x00 || u64_le(n)             (9 bytes)
 *       Opaque(s)  → 0x01 || utf8(s)
 *
 * The leading variant tag byte (0x00/0x01) was added on 2026-04-23 to
 * close the `Num(7)` vs `Opaque("\x07\0…")` collision; the witness
 * format version is therefore 2.
 */

import { shake256 } from "@noble/hashes/sha3";
import { bytesToHex } from "@noble/hashes/utils";

import type { Generation, RuLakeBundle } from "./bundle.js";

/** Encode a u64 (BigInt or small number) as 8 little-endian bytes. */
export function u64LE(n: bigint | number): Buffer {
  const v = typeof n === "bigint" ? n : BigInt(n);
  if (v < 0n || v > 0xffff_ffff_ffff_ffffn) {
    throw new Error(`u64LE: ${v} out of u64 range`);
  }
  const out = Buffer.alloc(8);
  out.writeBigUInt64LE(v);
  return out;
}

/**
 * Serialize a `Generation` to its witness-input byte form.
 *
 * Mirrors `Generation::hash_bytes()` in `src/bundle.rs`:
 *   - `Num(n)`    → 0x00 || u64_le(n)        (9 bytes)
 *   - `Opaque(s)` → 0x01 || UTF-8(s)
 */
export function generationBytes(g: Generation): Buffer {
  if (g.kind === "Num") {
    return Buffer.concat([Buffer.from([0x00]), u64LE(g.value)]);
  }
  return Buffer.concat([Buffer.from([0x01]), Buffer.from(g.value, "utf8")]);
}

/**
 * The lower-level witness function. Takes the raw witnessed inputs and
 * returns the lowercase hex SHAKE-256(32) digest. Useful when you have
 * the fields in hand without a full bundle (e.g. a builder).
 */
export function computeWitness(args: {
  dataRef: string;
  dim: number | bigint;
  rotationSeed: bigint | number;
  rerankFactor: number | bigint;
  generation: Generation;
}): string {
  const dataRefBytes = Buffer.from(args.dataRef, "utf8");
  const genBytes = generationBytes(args.generation);

  const buf = Buffer.concat([
    Buffer.from("rulake-bundle-witness-v1|", "utf8"),
    u64LE(BigInt(dataRefBytes.length)),
    dataRefBytes,
    Buffer.from("|", "utf8"),
    u64LE(args.dim),
    u64LE(args.rotationSeed),
    u64LE(args.rerankFactor),
    Buffer.from("|", "utf8"),
    u64LE(BigInt(genBytes.length)),
    genBytes,
  ]);

  // dkLen=32 → 32-byte XOF output, hex-encoded → 64 chars lowercase.
  const digest = shake256(buf, { dkLen: 32 });
  return bytesToHex(digest);
}

/**
 * Recompute a parsed bundle's witness from its other fields and report
 * whether it matches the on-disk `rvf_witness`.
 */
export function verifyBundle(bundle: RuLakeBundle): {
  ok: boolean;
  expected: string;
  actual: string;
} {
  const expected = computeWitness({
    dataRef: bundle.data_ref,
    dim: bundle.dim,
    rotationSeed: bundle.rotation_seed,
    rerankFactor: bundle.rerank_factor,
    generation: bundle.generation,
  });
  return { ok: expected === bundle.rvf_witness, expected, actual: bundle.rvf_witness };
}
