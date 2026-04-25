/**
 * Bundle ETag policy.
 *
 * The witness IS the cache anchor — two bundles with the same witness
 * are interchangeable for any query. So we use it directly as the
 * strong ETag. No payload hashing needed; the publisher already paid
 * the SHAKE-256 cost.
 *
 * `Cache-Control: no-cache, must-revalidate` forces clients to revalidate
 * with `If-None-Match` instead of trusting the cached body, which is the
 * correct behaviour for a coherence-anchored sidecar — bundles are tiny
 * (a few hundred bytes) but their freshness is load-bearing.
 */

import type { Response } from "express";

/** Format a witness as an HTTP strong ETag (RFC 7232: quoted opaque). */
export function witnessETag(witness: string): string {
  return `"${witness}"`;
}

/** Compare an `If-None-Match` header against a witness ETag. */
export function ifNoneMatchHits(header: string | undefined, witness: string): boolean {
  if (!header) return false;
  const target = witnessETag(witness);
  // Multiple ETags are comma-separated; `*` matches any current entity.
  return header.split(",").some((tok) => {
    const t = tok.trim();
    return t === "*" || t === target;
  });
}

/** Apply the standard ETag + Cache-Control headers for a bundle response. */
export function applyBundleHeaders(res: Response, witness: string): void {
  res.setHeader("ETag", witnessETag(witness));
  res.setHeader("Cache-Control", "no-cache, must-revalidate");
  res.setHeader("X-Rulake-Witness", witness);
}
