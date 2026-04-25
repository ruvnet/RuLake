"""CLI: verify the witness on a ruLake ``table.rulake.json`` sidecar.

Usage::

    python verify_witness.py <path>

``<path>`` may point at either:

* a directory containing ``table.rulake.json`` (the canonical layout
  produced by ``RuLake::write_to_dir`` / ``RuLake::save_cache_to_dir``);
* the sidecar file itself.

Exit codes:

* ``0`` — bundle parsed cleanly AND the recomputed witness matches.
* ``1`` — witness mismatch (the bundle was tampered or built against a
  different field set).
* ``2`` — bundle could not be opened or parsed (missing file, oversize,
  unknown format_version, malformed JSON, etc).
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from rulake_witness import (
    BundleError,
    SIDECAR_FILENAME,
    read_bundle,
)


def _resolve(path_str: str) -> Path:
    p = Path(path_str)
    if p.is_dir():
        return p / SIDECAR_FILENAME
    return p


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Verify a ruLake table.rulake.json bundle witness.",
    )
    parser.add_argument(
        "path",
        help="directory containing table.rulake.json, or the sidecar file itself",
    )
    parser.add_argument(
        "-q", "--quiet",
        action="store_true",
        help="print only on mismatch / error",
    )
    args = parser.parse_args(argv)

    sidecar = _resolve(args.path)
    if not sidecar.exists():
        print(f"verify-witness: not found: {sidecar}", file=sys.stderr)
        return 2

    try:
        bundle = read_bundle(sidecar)
    except BundleError as e:
        print(f"verify-witness: parse error: {e}", file=sys.stderr)
        return 2
    except OSError as e:
        print(f"verify-witness: I/O error: {e}", file=sys.stderr)
        return 2

    if not bundle.verify_witness():
        print(
            f"verify-witness: WITNESS MISMATCH at {sidecar}\n"
            f"  data_ref       = {bundle.data_ref!r}\n"
            f"  dim            = {bundle.dim}\n"
            f"  rotation_seed  = {bundle.rotation_seed}\n"
            f"  rerank_factor  = {bundle.rerank_factor}\n"
            f"  generation     = {bundle.generation!r}\n"
            f"  on-disk witness= {bundle.rvf_witness}",
            file=sys.stderr,
        )
        return 1

    if not args.quiet:
        print(f"verify-witness: OK  {sidecar}")
        print(f"  format_version = {bundle.format_version}")
        print(f"  data_ref       = {bundle.data_ref}")
        print(f"  dim            = {bundle.dim}")
        print(f"  rotation_seed  = {bundle.rotation_seed}")
        print(f"  rerank_factor  = {bundle.rerank_factor}")
        print(f"  generation     = {bundle.generation!r}")
        print(f"  rvf_witness    = {bundle.rvf_witness}")
        for k, v in bundle.extra.items():
            print(f"  {k:14s} = {v!r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
