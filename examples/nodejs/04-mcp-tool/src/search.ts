/**
 * Brute-force exact L2 nearest-K over the items extracted from a ruLake
 * snapshot.
 *
 * Why brute-force, not RaBitQ? The Node side doesn't have the
 * `ruvector-rabitq` decompressor, so we cannot replay the compressed
 * search path here. The compressed path is what makes ruLake fast at
 * production scale, but for an agent demo over a few thousand
 * vectors, exact L2 is fine and matches the brute-force baseline the
 * Rust crate's tests use.
 *
 * The shape of the result mirrors `ruvector_rulake::SearchResult`
 * (`{ id, score }`) so an agent that reads this and an agent that
 * reads HTTP responses from a Rust ruLake see the same thing.
 *
 * For very large snapshots prefer module 02 (publisher) plus a Rust
 * ruLake search backend; this is the agentic substrate, not the
 * production serving path.
 */

import type { RbpxItem } from "./snapshot.js";

export interface Hit {
  id: number;
  /** Squared L2 distance, lower is closer. Matches RuLake's `SearchResult.score` (rerank distance). */
  score: number;
}

/**
 * Compute the K nearest items to `query` under squared L2.
 *
 * - `query.length` must equal each item's `dim`; mismatch throws.
 * - K is clamped into `[1, items.length]`.
 * - Stable on ties: returns items in insertion order on equal scores.
 */
export function nearestK(items: ReadonlyArray<RbpxItem>, query: Float32Array, k: number): Hit[] {
  if (items.length === 0) return [];
  const target = clampInt(k, 1, items.length);
  const dim = items[0]!.vector.length;
  if (query.length !== dim) {
    throw new Error(`nearestK: query.length=${query.length} != index dim=${dim}`);
  }

  // Maintain a small max-heap of the K best (lowest-score) hits.
  // For the snapshot sizes a Node MCP tool is realistically dealing
  // with (≤ a few hundred K vectors), a flat sort-then-slice is also
  // viable — but the heap keeps the hot loop allocation-free.
  const heap = new MaxHeap(target);

  for (let i = 0; i < items.length; i++) {
    const item = items[i]!;
    const score = squaredL2(query, item.vector);
    heap.offer({ id: item.id, score });
  }

  return heap.drainSorted();
}

function clampInt(v: number, lo: number, hi: number): number {
  if (!Number.isFinite(v)) return lo;
  return Math.max(lo, Math.min(hi, Math.floor(v)));
}

/** Branchless-ish squared L2 across two Float32Arrays of equal length. */
export function squaredL2(a: Float32Array, b: Float32Array): number {
  let s = 0;
  for (let i = 0; i < a.length; i++) {
    const d = a[i]! - b[i]!;
    s += d * d;
  }
  return s;
}

/** Bounded max-heap of `Hit`. Keeps only the K lowest-score entries. */
class MaxHeap {
  private readonly buf: Hit[] = [];
  constructor(private readonly capacity: number) {}

  offer(hit: Hit): void {
    if (this.buf.length < this.capacity) {
      this.buf.push(hit);
      this.bubbleUp(this.buf.length - 1);
    } else if (hit.score < this.buf[0]!.score) {
      this.buf[0] = hit;
      this.bubbleDown(0);
    }
  }

  /** Returns hits sorted ascending by score (best first). */
  drainSorted(): Hit[] {
    return [...this.buf].sort((a, b) => a.score - b.score);
  }

  private bubbleUp(i: number): void {
    while (i > 0) {
      const parent = (i - 1) >> 1;
      if (this.buf[i]!.score > this.buf[parent]!.score) {
        [this.buf[i], this.buf[parent]] = [this.buf[parent]!, this.buf[i]!];
        i = parent;
      } else break;
    }
  }

  private bubbleDown(i: number): void {
    const n = this.buf.length;
    for (;;) {
      const l = 2 * i + 1;
      const r = 2 * i + 2;
      let largest = i;
      if (l < n && this.buf[l]!.score > this.buf[largest]!.score) largest = l;
      if (r < n && this.buf[r]!.score > this.buf[largest]!.score) largest = r;
      if (largest === i) return;
      [this.buf[i], this.buf[largest]] = [this.buf[largest]!, this.buf[i]!];
      i = largest;
    }
  }
}
