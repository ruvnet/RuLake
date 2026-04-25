/**
 * Tests for the Node implementation of the ruLake bundle witness.
 *
 * The most important test is the cross-language fixture round-trip: we
 * load a `table.rulake.json` produced by the Rust crate and assert
 * that our recomputed witness matches the one Rust wrote. If this
 * fails, the byte stream layout has drifted and the Node code is
 * BROKEN — do not ship it.
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { parseBundle, MAX_BUNDLE_BYTES, MAX_FIELD_BYTES } from "../src/bundle.js";
import { computeWitness, generationBytes, u64LE, verifyBundle } from "../src/witness.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const FIXTURE = resolve(__dirname, "../fixtures/known-good-bundle.json");

describe("u64LE", () => {
  it("encodes 0 as 8 zero bytes", () => {
    expect([...u64LE(0n)]).toEqual([0, 0, 0, 0, 0, 0, 0, 0]);
  });

  it("encodes 7 as little-endian", () => {
    // 7 → [0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    expect([...u64LE(7n)]).toEqual([7, 0, 0, 0, 0, 0, 0, 0]);
  });

  it("rejects out-of-range values", () => {
    expect(() => u64LE(-1n)).toThrow(/out of u64 range/);
    expect(() => u64LE(0x1_0000_0000_0000_0000n)).toThrow(/out of u64 range/);
  });

  it("handles full u64", () => {
    expect([...u64LE(0xffff_ffff_ffff_ffffn)]).toEqual([255, 255, 255, 255, 255, 255, 255, 255]);
  });
});

describe("generationBytes", () => {
  it("tags Num with 0x00", () => {
    const b = generationBytes({ kind: "Num", value: 7n });
    expect(b.length).toBe(9);
    expect(b[0]).toBe(0x00);
    expect([...b.subarray(1)]).toEqual([7, 0, 0, 0, 0, 0, 0, 0]);
  });

  it("tags Opaque with 0x01", () => {
    const b = generationBytes({ kind: "Opaque", value: "abc" });
    expect(b.length).toBe(4);
    expect(b[0]).toBe(0x01);
    expect(b.toString("utf8", 1)).toBe("abc");
  });
});

describe("computeWitness", () => {
  it("is deterministic for identical inputs", () => {
    const a = computeWitness({
      dataRef: "gs://bucket/x.parquet",
      dim: 128,
      rotationSeed: 42n,
      rerankFactor: 20,
      generation: { kind: "Num", value: 7n },
    });
    const b = computeWitness({
      dataRef: "gs://bucket/x.parquet",
      dim: 128,
      rotationSeed: 42n,
      rerankFactor: 20,
      generation: { kind: "Num", value: 7n },
    });
    expect(a).toBe(b);
    expect(a).toHaveLength(64);
    expect(a).toMatch(/^[0-9a-f]{64}$/);
  });

  it("changes when any input field changes", () => {
    const base = {
      dataRef: "gs://bucket/x.parquet",
      dim: 128,
      rotationSeed: 42n,
      rerankFactor: 20,
      generation: { kind: "Num" as const, value: 7n },
    };
    const w0 = computeWitness(base);
    expect(w0).not.toBe(computeWitness({ ...base, dataRef: "gs://bucket/y.parquet" }));
    expect(w0).not.toBe(computeWitness({ ...base, dim: 256 }));
    expect(w0).not.toBe(computeWitness({ ...base, rotationSeed: 43n }));
    expect(w0).not.toBe(computeWitness({ ...base, rerankFactor: 5 }));
    expect(w0).not.toBe(
      computeWitness({ ...base, generation: { kind: "Num", value: 8n } }),
    );
  });

  // Regression: the 2026-04-23 audit found that without the variant tag
  // byte, Num(7) and Opaque("\x07\0\0\0\0\0\0\0") shared hash-input
  // bytes. The Rust fix added a leading 0x00/0x01 tag; this Node port
  // MUST honor it or it will miscompute witnesses for any v2 bundle
  // whose backend reports an Opaque token.
  it("Num(7) and Opaque('\\x07\\0\\0\\0\\0\\0\\0\\0') do not collide (audit regression)", () => {
    const wNum = computeWitness({
      dataRef: "x",
      dim: 1,
      rotationSeed: 0n,
      rerankFactor: 1,
      generation: { kind: "Num", value: 7n },
    });
    const wOpaque = computeWitness({
      dataRef: "x",
      dim: 1,
      rotationSeed: 0n,
      rerankFactor: 1,
      generation: { kind: "Opaque", value: "\x07\x00\x00\x00\x00\x00\x00\x00" },
    });
    expect(wNum).not.toBe(wOpaque);
  });

  it("length-prefixing prevents 'a|b' vs 'ab|' collisions", () => {
    const wA = computeWitness({
      dataRef: "x",
      dim: 1,
      rotationSeed: 0n,
      rerankFactor: 1,
      generation: { kind: "Opaque", value: "a|b" },
    });
    const wB = computeWitness({
      dataRef: "x",
      dim: 1,
      rotationSeed: 0n,
      rerankFactor: 1,
      generation: { kind: "Opaque", value: "ab|" },
    });
    expect(wA).not.toBe(wB);
  });
});

describe("parseBundle", () => {
  it("rejects oversize bundles", () => {
    const big = "x".repeat(MAX_BUNDLE_BYTES + 1);
    expect(() => parseBundle(big)).toThrow(/exceeds .* bytes/);
  });

  it("rejects oversize string fields", () => {
    const long = "p".repeat(MAX_FIELD_BYTES + 1);
    const body = JSON.stringify({
      format_version: 2,
      data_ref: "x",
      dim: 1,
      rotation_seed: 0,
      rerank_factor: 1,
      generation: 1,
      rvf_witness: "0".repeat(64),
      pii_policy: long,
      lineage_id: null,
    });
    expect(() => parseBundle(body)).toThrow(/pii_policy/);
  });

  it("rejects format_version > 2", () => {
    const body = JSON.stringify({
      format_version: 99,
      data_ref: "x",
      dim: 1,
      rotation_seed: 0,
      rerank_factor: 1,
      generation: 1,
      rvf_witness: "0".repeat(64),
      pii_policy: null,
      lineage_id: null,
    });
    expect(() => parseBundle(body)).toThrow(/format_version=99/);
  });

  it("accepts a numeric Generation as Num", () => {
    const body = JSON.stringify({
      format_version: 2,
      data_ref: "x",
      dim: 1,
      rotation_seed: 0,
      rerank_factor: 1,
      generation: 5,
      rvf_witness: "0".repeat(64),
      pii_policy: null,
      lineage_id: null,
    });
    const b = parseBundle(body);
    expect(b.generation).toEqual({ kind: "Num", value: 5n });
  });

  it("accepts a string Generation as Opaque", () => {
    const body = JSON.stringify({
      format_version: 2,
      data_ref: "x",
      dim: 1,
      rotation_seed: 0,
      rerank_factor: 1,
      generation: "abc-def",
      rvf_witness: "0".repeat(64),
      pii_policy: null,
      lineage_id: null,
    });
    const b = parseBundle(body);
    expect(b.generation).toEqual({ kind: "Opaque", value: "abc-def" });
  });
});

describe("Rust fixture round-trip (cross-language witness check)", () => {
  it("verifies a real Rust-produced table.rulake.json", () => {
    const raw = readFileSync(FIXTURE, "utf8");
    const bundle = parseBundle(raw);
    const result = verifyBundle(bundle);
    if (!result.ok) {
      // Print the diagnostic so a future drift is obvious in CI logs.
      // eslint-disable-next-line no-console
      console.error(
        `WITNESS MISMATCH against fixture:\n  expected (Node-recomputed): ${result.expected}\n  actual   (Rust-written):    ${result.actual}`,
      );
    }
    expect(result.ok).toBe(true);
    expect(result.actual).toBe(result.expected);
  });
});
