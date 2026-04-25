/**
 * Load and verify a ruLake snapshot directory.
 *
 * A snapshot, written by the Rust crate's `RuLake::save_cache_to_dir`,
 * contains:
 *
 *   <dir>/table.rulake.json   # the bundle sidecar (ADR-155)
 *   <dir>/index.rbpx           # rabitq-persist payload (header + items)
 *
 * The `.rbpx` format is documented in
 * `vendor/ruvector/crates/ruvector-rabitq/src/persist.rs`:
 *
 *   | offset | size              | field                       |
 *   |--------|-------------------|-----------------------------|
 *   |   0    |  8                | magic = b"rbpx0001"         |
 *   |   8    |  4                | version (u32 LE, currently 1)|
 *   |  12    |  4                | dim (u32 LE)                |
 *   |  16    |  8                | seed (u64 LE)               |
 *   |  24    |  4                | rerank_factor (u32 LE)      |
 *   |  28    |  4                | n (u32 LE)                  |
 *   |  32    |  n × (4 + dim·4) | id (u32 LE) + dim·f32 LE    |
 *
 * Bounds (matched here so we behave identically to the Rust loader):
 *   dim        ≤ 8192
 *   n          ≤ 100_000_000
 *   rerank     ≤ 1024
 *
 * The Node side does NOT have the RaBitQ decompressor, so this loader
 * exposes the *original* (id, vector) pairs that the Rust loader uses
 * to deterministically rebuild the index. Brute-force L2 over those
 * pairs is exact and gives the agent a working search; for production
 * QPS, host the index in a Rust ruLake process and serve via
 * module 02 — this MCP tool is the agentic-substrate face.
 */

import { readFileSync, statSync } from "node:fs";
import { join } from "node:path";

import { MAX_BUNDLE_BYTES, parseBundle, type RuLakeBundle } from "./bundle.js";
import { verifyBundle } from "./witness.js";

const RBPX_MAGIC = Buffer.from("rbpx0001", "utf8");
const RBPX_VERSION_MAX = 1;
const MAX_DIM = 8192;
const MAX_N = 100_000_000;
const MAX_RERANK = 1024;
/**
 * Hard cap on the on-disk `.rbpx` file. The MCP tool is fed
 * `snapshot_dir` paths by an upstream agent / client — that input is
 * not trusted, so refuse to allocate unbounded memory for a hostile
 * symlink farm or an attacker-supplied path. The cap below is a few
 * GiB headroom over the largest realistic snapshot at MAX_N × MAX_DIM
 * × 4 bytes (~3.2 TiB), but capped to a manageable 4 GiB so a single
 * call cannot OOM the process.
 */
const MAX_RBPX_BYTES = 4 * 1024 * 1024 * 1024;

export interface RbpxItem {
  id: number;
  /** Float32Array of length `dim`. */
  vector: Float32Array;
}

export interface RbpxIndex {
  version: number;
  dim: number;
  seed: bigint;
  rerank_factor: number;
  n: number;
  items: RbpxItem[];
}

export interface SnapshotLoadResult {
  bundle: RuLakeBundle;
  /** True iff the bundle's witness verifies. Server should refuse to serve searches when false. */
  witnessOk: boolean;
  expectedWitness: string;
  index: RbpxIndex;
  /** Sizes for diagnostics. */
  sizes: { bundleBytes: number; rbpxBytes: number };
  /** Absolute paths read. */
  paths: { bundle: string; rbpx: string };
}

/**
 * Read + verify the snapshot directory.
 *
 * Throws on any structural problem (missing files, oversize fields,
 * truncated `.rbpx`). The witness check is reported as a boolean
 * (`witnessOk`) rather than thrown so callers can choose to surface
 * the diagnostic as a tool result.
 */
export function loadSnapshot(dir: string): SnapshotLoadResult {
  const bundlePath = join(dir, "table.rulake.json");
  const rbpxPath = join(dir, "index.rbpx");

  const bundleStat = statSync(bundlePath);
  const rbpxStat = statSync(rbpxPath);
  // Refuse to read unreasonable inputs BEFORE allocating buffers. The
  // snapshot_dir comes from an MCP client and may be hostile; a 100 GB
  // index.rbpx must error out, not OOM the agent process.
  if (!bundleStat.isFile()) {
    throw new Error(`snapshot: ${bundlePath} is not a regular file`);
  }
  if (!rbpxStat.isFile()) {
    throw new Error(`snapshot: ${rbpxPath} is not a regular file`);
  }
  if (bundleStat.size > MAX_BUNDLE_BYTES) {
    throw new Error(
      `snapshot: bundle ${bundlePath} is ${bundleStat.size} bytes, exceeds cap ${MAX_BUNDLE_BYTES}`,
    );
  }
  if (rbpxStat.size > MAX_RBPX_BYTES) {
    throw new Error(
      `snapshot: index ${rbpxPath} is ${rbpxStat.size} bytes, exceeds cap ${MAX_RBPX_BYTES}`,
    );
  }
  const bundleRaw = readFileSync(bundlePath, "utf8");
  const bundle = parseBundle(bundleRaw);
  const { ok: witnessOk, expected } = verifyBundle(bundle);

  const rbpxBuf = readFileSync(rbpxPath);
  const index = parseRbpx(rbpxBuf, bundle);

  return {
    bundle,
    witnessOk,
    expectedWitness: expected,
    index,
    sizes: { bundleBytes: bundleStat.size, rbpxBytes: rbpxStat.size },
    paths: { bundle: bundlePath, rbpx: rbpxPath },
  };
}

function parseRbpx(buf: Buffer, bundle: RuLakeBundle): RbpxIndex {
  if (buf.length < 32) {
    throw new Error(`rbpx: file too small (${buf.length} bytes < 32-byte header)`);
  }
  if (!buf.subarray(0, 8).equals(RBPX_MAGIC)) {
    throw new Error(
      `rbpx: bad magic, expected ${RBPX_MAGIC.toString("hex")} got ${buf.subarray(0, 8).toString("hex")}`,
    );
  }
  const version = buf.readUInt32LE(8);
  if (version > RBPX_VERSION_MAX) {
    throw new Error(`rbpx: unsupported version ${version} (max ${RBPX_VERSION_MAX})`);
  }
  const dim = buf.readUInt32LE(12);
  if (dim === 0 || dim > MAX_DIM) {
    throw new Error(`rbpx: dim ${dim} out of range (1..=${MAX_DIM})`);
  }
  const seed = buf.readBigUInt64LE(16);
  const rerank_factor = buf.readUInt32LE(24);
  if (rerank_factor > MAX_RERANK) {
    throw new Error(`rbpx: rerank_factor ${rerank_factor} exceeds cap ${MAX_RERANK}`);
  }
  const n = buf.readUInt32LE(28);
  if (n > MAX_N) {
    throw new Error(`rbpx: n ${n} exceeds cap ${MAX_N}`);
  }

  const expectedBytes = 32 + n * (4 + dim * 4);
  if (buf.length < expectedBytes) {
    throw new Error(
      `rbpx: truncated payload (need ${expectedBytes} bytes, file is ${buf.length})`,
    );
  }

  // Cross-check vs the bundle. Only the dim is in both — but the seed
  // in the rbpx is the seed used to BUILD the index; the bundle's
  // rotation_seed is what the cache-key anchor uses. They should match.
  if (bundle.dim !== dim) {
    throw new Error(`rbpx/bundle mismatch: bundle.dim=${bundle.dim}, rbpx.dim=${dim}`);
  }
  if (bundle.rotation_seed !== seed) {
    throw new Error(
      `rbpx/bundle mismatch: bundle.rotation_seed=${bundle.rotation_seed}, rbpx.seed=${seed}`,
    );
  }

  const items: RbpxItem[] = new Array(n);
  let off = 32;
  for (let i = 0; i < n; i++) {
    const id = buf.readUInt32LE(off);
    off += 4;
    // Take a copy with `slice()` so the underlying Buffer can be GC'd
    // once we drop the reference; otherwise every Float32Array would
    // pin the entire `.rbpx` payload.
    const vec = new Float32Array(dim);
    for (let j = 0; j < dim; j++) {
      vec[j] = buf.readFloatLE(off);
      off += 4;
    }
    items[i] = { id, vector: vec };
  }

  return { version, dim, seed, rerank_factor, n, items };
}
