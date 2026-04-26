// TypeScript declarations for @ruvector/rulake (ADR-003).
// In production builds, `napi build` regenerates this file from the
// #[napi] annotations on src/lib.rs. The hand-written version here
// is the authoritative shape until then; CI checks it against the
// generated output on every release.

/**
 * Single error class for every ruLake failure. Discriminated by
 * `error.code` (idiomatic Node — matches AWS SDK / SystemError style).
 *
 * Possible `code` values:
 * - `"RULAKE_BACKEND_NOT_FOUND"`
 * - `"RULAKE_COLLECTION_NOT_FOUND"`
 * - `"RULAKE_DIMENSION_MISMATCH"`
 * - `"RULAKE_INVALID_PARAMETER"`
 * - `"RULAKE_BACKEND"`
 * - `"RULAKE_RABITQ"`
 */
export class RuLakeError extends Error {
  readonly code: string;
  constructor(code: string, message: string);
}

/** Cache consistency knob. Three factory variants (ADR-155). */
export class Consistency {
  /** Per-query coherence check. Strongest staleness, lowest QPS on remote backends. */
  static fresh(): Consistency;
  /** Skip the coherence check for `ttlMs` after the last one. */
  static eventual(ttlMs: number): Consistency;
  /** Pin the cache forever — for witness-sealed audit-tier snapshots. */
  static frozen(): Consistency;
}

/** A single search hit. `id` is `bigint` because Rust ids are u64. */
export interface SearchResult {
  backend: string;
  collection: string;
  id: bigint;
  score: number;
}

export interface CacheStats {
  hits: bigint;
  misses: bigint;
  primes: bigint;
  invalidations: bigint;
  sharedHits: bigint;
  totalPrimeMs: number;
  lastPrimeMs: number;
  warmInstalls: bigint;
  /** `hits / (hits + misses)`, or `null` when no checks have run yet. */
  hitRate: number | null;
  /** Mean prime duration (ms), or `null` when no primes have run yet. */
  avgPrimeMs: number | null;
}

/** A `table.rulake.json` sidecar, witness-anchored. */
export class Bundle {
  /**
   * @param dataRef       Where the source bytes live (e.g. `"s3://bucket/path"`).
   * @param dim           Vector dimensionality.
   * @param rotationSeed  Haar rotation seed (deterministic compression).
   * @param rerankFactor  RaBitQ rerank factor (recall knob; 20 = recall@10 ≥ 0.90).
   * @param generation    Coherence token: `bigint` (Num variant) or `string` (Opaque).
   */
  constructor(
    dataRef: string,
    dim: number,
    rotationSeed: bigint,
    rerankFactor: number,
    generation: bigint | string,
  );

  readonly dataRef: string;
  readonly dim: number;
  readonly rotationSeed: bigint;
  readonly rerankFactor: number;
  readonly rvfWitness: string;          // SHAKE-256(32) hex (64 chars)
  readonly formatVersion: number;
  readonly piiPolicy: string | null;
  readonly lineageId: string | null;
  readonly memoryClass: string | null;

  generation(): bigint | string;
  verifyWitness(): boolean;
  toJson(): string;
  static fromJson(s: string): Bundle;

  /** Atomic write to `dir/table.rulake.json` (temp + rename). */
  writeToDir(dir: string): Promise<string>;
  static readFromDir(dir: string): Promise<Bundle>;

  withPiiPolicy(p: string): Bundle;
  withLineageId(id: string): Bundle;
  withMemoryClass(klass: string): Bundle;
}

/** In-memory backend. */
export class LocalBackend {
  constructor(id: string);
  readonly id: string;

  /**
   * Insert a collection.
   * @param ids       length N (BigInt64Array — values reinterpreted as u64).
   * @param vectors   length N*dim, row-major (Float32Array).
   */
  putCollection(
    name: string,
    ids: BigInt64Array,
    vectors: Float32Array,
    dim: number,
  ): Promise<void>;

  append(collection: string, id: bigint, vector: Float32Array): Promise<void>;
}

/** Filesystem-backed `ruvec1` files. */
export class FsBackend {
  constructor(id: string, root: string);
  register(collection: string, filename: string): void;
  write(
    collection: string,
    filename: string,
    ids: BigInt64Array,
    vectors: Float32Array,
    dim: number,
  ): Promise<string>;
}

/** Cache + router entry point. */
export class RuLake {
  /**
   * @param rerankFactor  RaBitQ rerank factor; 20 is the BENCHMARK.md gate (recall@10 ≥ 0.90).
   * @param rotationSeed  Haar rotation seed (deterministic; same seed → same witness across processes).
   */
  constructor(rerankFactor: number, rotationSeed: bigint);

  withConsistency(c: Consistency): RuLake;
  withMaxCacheEntries(n: number): RuLake;

  registerLocalBackend(b: LocalBackend): void;
  registerFsBackend(b: FsBackend): void;

  backendIds(): string[];
  cacheStats(): CacheStats;
  cacheEntryCount(): number;
  cacheWitnessOf(backend: string, collection: string): string | null;
  invalidateCache(backend: string, collection: string): void;

  searchOne(
    backend: string,
    collection: string,
    query: Float32Array,
    k: number,
  ): Promise<SearchResult[]>;

  /** `targets` is `[[backend, collection], …]`. Adaptive per-shard rerank. */
  searchFederated(
    targets: string[][],
    query: Float32Array,
    k: number,
  ): Promise<SearchResult[]>;

  /** `queries` is length `n*dim`, row-major. */
  searchBatch(
    backend: string,
    collection: string,
    queries: Float32Array,
    dim: number,
    k: number,
  ): Promise<SearchResult[][]>;

  publishBundle(backend: string, collection: string, dir: string): Promise<string>;

  /** Returns `"up_to_date"` | `"invalidated"` | `"bundle_missing"`. */
  refreshFromBundleDir(
    backend: string,
    collection: string,
    dir: string,
  ): Promise<string>;

  saveCacheToDir(backend: string, collection: string, dir: string): Promise<string>;
  warmFromDir(backend: string, collection: string, dir: string): Promise<number>;
}
