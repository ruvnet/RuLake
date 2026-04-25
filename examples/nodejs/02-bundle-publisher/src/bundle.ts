/**
 * TypeScript types and a hardened parser for the ruLake bundle sidecar
 * (`table.rulake.json`). Mirrors `src/bundle.rs` in the Rust crate.
 *
 * The DoS caps are copied verbatim from the Rust implementation so a
 * malicious bundle cannot force a Node reader to allocate an unbounded
 * amount of memory:
 *
 *   - whole bundle JSON  ≤ 64 KiB
 *   - each string field  ≤  4 KiB
 *   - rvf_witness        ≤  128 chars (real value is exactly 64 hex)
 *   - format_version     ≤  2
 */

/** Maximum bundle size in bytes — matches Rust `MAX_JSON_BYTES`. */
export const MAX_BUNDLE_BYTES = 64 * 1024;
/** Maximum per-field size in bytes — matches Rust `MAX_FIELD_BYTES`. */
export const MAX_FIELD_BYTES = 4 * 1024;
/** Highest `format_version` this Node reader understands. */
export const MAX_FORMAT_VERSION = 2;

/**
 * `Generation` — a backend's coherence token at bundle-emission time.
 *
 * Serializes as untagged JSON (matches Rust `#[serde(untagged)]`):
 *   - numeric  → a JSON number that fits in u64
 *   - opaque   → a JSON string
 */
export type Generation = { kind: "Num"; value: bigint } | { kind: "Opaque"; value: string };

/** Parsed shape of `table.rulake.json` after validation. */
export interface RuLakeBundle {
  format_version: number;
  data_ref: string;
  dim: number;
  rotation_seed: bigint;
  rerank_factor: number;
  generation: Generation;
  rvf_witness: string;
  pii_policy: string | null;
  lineage_id: string | null;
  /** Optional in the Rust struct — `skip_serializing_if = "Option::is_none"`. */
  memory_class?: string | null;
}

/** Coerce `unknown` to a finite non-negative integer or throw. */
function asNonNegativeInteger(v: unknown, field: string, max?: number): number {
  if (typeof v !== "number" || !Number.isFinite(v) || !Number.isInteger(v) || v < 0) {
    throw new Error(`bundle parse: ${field} must be a non-negative integer (got ${String(v)})`);
  }
  if (max !== undefined && v > max) {
    throw new Error(`bundle parse: ${field} ${v} exceeds ${max}`);
  }
  return v;
}

/** Coerce `unknown` to a u64 bigint or throw. */
function asU64(v: unknown, field: string): bigint {
  if (typeof v === "number") {
    if (!Number.isFinite(v) || !Number.isInteger(v) || v < 0) {
      throw new Error(`bundle parse: ${field} must be a non-negative integer (got ${v})`);
    }
    if (v > Number.MAX_SAFE_INTEGER) {
      throw new Error(
        `bundle parse: ${field}=${v} exceeds Number.MAX_SAFE_INTEGER; serialize as a JSON string instead`,
      );
    }
    return BigInt(v);
  }
  // Some publishers may emit u64 as a string to dodge JSON's 53-bit limit.
  if (typeof v === "string" && /^\d+$/.test(v)) {
    const n = BigInt(v);
    if (n < 0n || n > 0xffff_ffff_ffff_ffffn) {
      throw new Error(`bundle parse: ${field} ${v} out of u64 range`);
    }
    return n;
  }
  throw new Error(`bundle parse: ${field} must be a u64 (number or numeric string), got ${typeof v}`);
}

/** Coerce `unknown` to an optional string ≤ MAX_FIELD_BYTES. */
function asOptionalString(v: unknown, field: string): string | null {
  if (v === null || v === undefined) return null;
  if (typeof v !== "string") {
    throw new Error(`bundle parse: ${field} must be a string or null`);
  }
  if (Buffer.byteLength(v, "utf8") > MAX_FIELD_BYTES) {
    throw new Error(`bundle parse: ${field} exceeds ${MAX_FIELD_BYTES} bytes`);
  }
  return v;
}

function parseGeneration(v: unknown): Generation {
  // Untagged: a JSON number is `Num`, a JSON string is `Opaque`.
  if (typeof v === "number") {
    return { kind: "Num", value: asU64(v, "generation") };
  }
  if (typeof v === "string") {
    if (Buffer.byteLength(v, "utf8") > MAX_FIELD_BYTES) {
      throw new Error(`bundle parse: generation (Opaque) exceeds ${MAX_FIELD_BYTES} bytes`);
    }
    return { kind: "Opaque", value: v };
  }
  // Some Rust enum encodings might also emit a tagged form. Be tolerant.
  if (v && typeof v === "object") {
    const obj = v as Record<string, unknown>;
    if ("Num" in obj) return { kind: "Num", value: asU64(obj.Num, "generation.Num") };
    if ("Opaque" in obj) {
      if (typeof obj.Opaque !== "string") {
        throw new Error("bundle parse: generation.Opaque must be a string");
      }
      return parseGeneration(obj.Opaque);
    }
  }
  throw new Error(`bundle parse: generation must be a number or string, got ${typeof v}`);
}

/**
 * Parse a `table.rulake.json` payload into a validated `RuLakeBundle`.
 *
 * Throws `Error` (with a `bundle parse:` prefix) on any cap violation,
 * malformed JSON, or unknown future format version.
 */
export function parseBundle(raw: string): RuLakeBundle {
  if (Buffer.byteLength(raw, "utf8") > MAX_BUNDLE_BYTES) {
    throw new Error(`bundle parse: exceeds ${MAX_BUNDLE_BYTES} bytes`);
  }

  let json: unknown;
  try {
    json = JSON.parse(raw);
  } catch (err) {
    throw new Error(`bundle parse: invalid JSON: ${(err as Error).message}`);
  }

  if (!json || typeof json !== "object" || Array.isArray(json)) {
    throw new Error("bundle parse: top-level value must be a JSON object");
  }
  const o = json as Record<string, unknown>;

  const format_version = asNonNegativeInteger(o.format_version, "format_version");
  if (format_version > MAX_FORMAT_VERSION) {
    throw new Error(
      `bundle parse: format_version=${format_version} newer than this reader supports (${MAX_FORMAT_VERSION})`,
    );
  }

  const data_ref = (() => {
    const v = o.data_ref;
    if (typeof v !== "string") throw new Error("bundle parse: data_ref must be a string");
    if (Buffer.byteLength(v, "utf8") > MAX_FIELD_BYTES) {
      throw new Error(`bundle parse: data_ref exceeds ${MAX_FIELD_BYTES} bytes`);
    }
    return v;
  })();

  const dim = asNonNegativeInteger(o.dim, "dim");
  const rotation_seed = asU64(o.rotation_seed, "rotation_seed");
  const rerank_factor = asNonNegativeInteger(o.rerank_factor, "rerank_factor");
  const generation = parseGeneration(o.generation);

  const rvf_witness = (() => {
    const v = o.rvf_witness;
    if (typeof v !== "string") throw new Error("bundle parse: rvf_witness must be a string");
    if (v.length > 128) {
      throw new Error("bundle parse: rvf_witness not a hex-encoded SHAKE-256(32)");
    }
    return v;
  })();

  const pii_policy = asOptionalString(o.pii_policy, "pii_policy");
  const lineage_id = asOptionalString(o.lineage_id, "lineage_id");
  const memory_class =
    "memory_class" in o ? asOptionalString(o.memory_class, "memory_class") : undefined;

  return {
    format_version,
    data_ref,
    dim,
    rotation_seed,
    rerank_factor,
    generation,
    rvf_witness,
    pii_policy,
    lineage_id,
    ...(memory_class !== undefined ? { memory_class } : {}),
  };
}
