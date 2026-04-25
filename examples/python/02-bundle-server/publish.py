"""CLI to atomically publish a `table.rulake.json` into a publish root.

Mirrors the rename-into-place pattern that ``RuLakeBundle::write_to_dir``
uses on the Rust side: the publish-server only ever observes a
fully-written file. The temp file lives next to the destination so the
rename stays on the same filesystem.

Usage::

    python publish.py <root> <key> <source.json>

Example::

    python publish.py ./publish-root prod-warehouse/memories ./bundle.json

That produces ``./publish-root/prod-warehouse/memories/table.rulake.json``
in one atomic rename. The bundle server's watcher will pick it up.
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
import tempfile
from pathlib import Path

from rulake_witness import BundleError, read_bundle


def _validate_key(key: str) -> tuple[str, ...]:
    """Reject keys that would escape the publish root."""
    if not key:
        raise SystemExit("publish: key must be non-empty")
    parts = key.split("/")
    for p in parts:
        if not p or p in (".", "..") or "\\" in p or ":" in p:
            raise SystemExit(f"publish: illegal key component: {p!r}")
    return tuple(parts)


def publish(root: Path, key: str, source: Path) -> Path:
    """Copy ``source`` to ``root/<key>/table.rulake.json`` atomically.

    Verifies the source bundle's witness *before* publishing so a broken
    bundle never reaches the publish dir.
    """
    parts = _validate_key(key)
    target_dir = root.joinpath(*parts)
    target_dir.mkdir(parents=True, exist_ok=True)

    # Verify before publish — refuse to publish a broken bundle.
    try:
        bundle = read_bundle(source)
    except BundleError as e:
        raise SystemExit(f"publish: source rejected: {e}") from e
    if not bundle.verify_witness():
        raise SystemExit(f"publish: source witness mismatch at {source}")

    target = target_dir / "table.rulake.json"
    fd, tmp_path = tempfile.mkstemp(
        prefix=".table.rulake.json.tmp.",
        dir=target_dir,
    )
    os.close(fd)
    try:
        shutil.copyfile(source, tmp_path)
        # fsync so a crash mid-publish leaves either old or new on disk.
        with open(tmp_path, "rb") as f:
            os.fsync(f.fileno())
        os.replace(tmp_path, target)
    except Exception:
        # Best-effort cleanup of leftover tmp.
        try:
            os.unlink(tmp_path)
        except OSError:
            pass
        raise
    return target


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="atomically publish a ruLake bundle")
    p.add_argument("root", help="publish root directory")
    p.add_argument("key", help="collection key, e.g. 'backend-id/collection'")
    p.add_argument("source", help="source table.rulake.json to publish")
    args = p.parse_args(argv)

    target = publish(Path(args.root), args.key, Path(args.source))
    print(f"published: {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
