/* tslint:disable */
/* eslint-disable */

/**
 * Build version string. Useful for browser-console diagnostics.
 */
export function buildInfo(): string;

/**
 * Compute the SHAKE-256(32) witness for the load-bearing bundle fields.
 *
 * Useful for "I have these fields, what witness should the bundle
 * carry?" — building bundles client-side, validating publishers, etc.
 * Generation accepts either a number (encoded as `Num`) or a string
 * (encoded as `Opaque`).
 */
export function computeWitness(data_ref: string, dim: number, rotation_seed: bigint, rerank_factor: number, generation: any): string;

/**
 * `format_version` of bundles this build understands.
 */
export function formatVersion(): number;

/**
 * Exact top-K nearest-neighbor search by squared L2 distance.
 *
 * `vectors` is a flat `Float32Array` of length `n * dim`; `ids` (if
 * non-empty) gives external u64 ids in the same order. `k` is capped at
 * `n` and at 4096 to bound the worst case in a constrained edge runtime.
 * Returns hits sorted ascending by score.
 *
 * **Recall**: 1.0. This is exact, not approximate. Use when n is small
 * enough that the wall-time fits your budget — at D=128, ~50k vectors is
 * under 10 ms in browser WASM on a modern laptop.
 */
export function searchBruteForceL2(vectors: Float32Array, ids: Float64Array, dim: number, query: Float32Array, k: number): any;

/**
 * Verify a `table.rulake.json` bundle.
 *
 * Parses the JSON (with DoS caps applied), recomputes the SHAKE-256(32)
 * witness, returns a structured comparison. Returns `Err` (rejected as
 * a JS exception) on any of: oversize input, missing fields, future
 * `format_version`, malformed `generation`.
 */
export function verifyBundleJson(json: string): any;
