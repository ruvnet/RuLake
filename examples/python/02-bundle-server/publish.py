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
    """Reject keys that would escape the publish root.

    Rejects empty / `.` / `..` components, backslashes, drive-letter colons,
    and any component containing newline / NUL / other control bytes
    (which would let a malicious component inject log lines or trip
    `mkdir` after passing this check).
    """
    if not key:
        raise SystemExit("publish: key must be non-empty")
    parts = key.split("/")
    for p in parts:
        if not p or p in (".", "..") or "\\" in p or ":" in p:
            raise SystemExit(f"publish: illegal key component: {p!r}")
        if any(c in p for c in ("\n", "\r", "\x00")) or any(
            ord(c) < 0x20 for c in p
        ):
            raise SystemExit(f"publish: control byte in key component: {p!r}")
    return tuple(parts)


def publish(root: Path, key: str, source: Path) -> Path:
    """Copy ``source`` to ``root/<key>/table.rulake.json`` atomically.

    Verifies the source bundle's witness *before* publishing so a broken
    bundle never reaches the publish dir. Also resolves the destination
    path against `root` and refuses to write outside it — defends against
    a symlink under the publish root being used to redirect the write.
    """
    parts = _validate_key(key)
    root_resolved = root.resolve()
    target_dir = root.joinpath(*parts)
    target_dir.mkdir(parents=True, exist_ok=True)
    target_dir_resolved = target_dir.resolve()
    try:
        target_dir_resolved.relative_to(root_resolved)
    except ValueError as e:
        raise SystemExit(
            f"publish: refusing to write outside publish root "
            f"({target_dir_resolved} is not under {root_resolved}); "
            f"a symlink under the root may be redirecting the write"
        ) from e

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
