"""FastAPI app that serves ruLake bundle sidecars over HTTP.

In a deployed system, ruLake instances (Rust) atomically write
``table.rulake.json`` files into a publish directory. Other services
(any language) need to learn which witness is current so they can
decide whether their cache is still valid. This server is the
publication side of that loop.

Endpoints
---------

* ``GET /health`` — process liveness.
* ``GET /bundles`` — list of currently-known collection keys.
* ``GET /bundles/{key}/table.rulake.json`` — the full sidecar JSON
  payload.
* ``GET /bundles/{key}/witness`` — the bare ``rvf_witness`` string
  (cheap probe for "did the cache rotate?").

The server watches a directory tree::

    <root>/<key>/table.rulake.json

and (re-)indexes a key whenever its sidecar appears, changes, or
disappears. ``key`` may include a single ``/`` to model the typical
``backend-id/collection`` pair (e.g. ``prod-warehouse/memories``); the
server URL-decodes it.

Witness verification runs on every load, so a sidecar that fails its
own witness is **not** served — clients only ever observe trusted
bundles.
"""

from __future__ import annotations

import logging
import threading
import time
from contextlib import asynccontextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import AsyncIterator, Optional

from fastapi import FastAPI, HTTPException, Response
from fastapi.responses import JSONResponse, PlainTextResponse
from watchdog.events import FileSystemEvent, FileSystemEventHandler
from watchdog.observers import Observer

from rulake_witness import (
    BundleError,
    SIDECAR_FILENAME,
    RuLakeBundle,
    read_bundle,
)

logger = logging.getLogger("rulake.bundle_server")


# --------------------------------------------------------------------------
# In-memory bundle index
# --------------------------------------------------------------------------

@dataclass(frozen=True)
class CachedBundle:
    """A bundle plus the on-disk metadata we need for HTTP semantics."""

    key: str
    path: Path
    bundle: RuLakeBundle
    raw_json: str
    mtime_ns: int


class BundleIndex:
    """Thread-safe in-memory cache of `<key>` -> :class:`CachedBundle`."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self._lock = threading.RLock()
        self._by_key: dict[str, CachedBundle] = {}

    @staticmethod
    def _key_from_path(root: Path, sidecar: Path) -> Optional[str]:
        try:
            rel = sidecar.relative_to(root)
        except ValueError:
            return None
        if rel.name != SIDECAR_FILENAME:
            return None
        parts = rel.parts[:-1]
        if not parts:
            return None
        # Reject obviously hostile components — watchdog wouldn't normally
        # surface them but the type system can't prove that.
        for p in parts:
            if p in (".", "..") or "/" in p or "\\" in p:
                return None
        return "/".join(parts)

    def load_one(self, sidecar: Path) -> None:
        """(Re-)load a single sidecar file into the index.

        Witness mismatches are logged and the entry is *not* cached, so
        an attacker who can write into the publish dir cannot get the
        server to vouch for an inconsistent bundle.
        """
        key = self._key_from_path(self.root, sidecar)
        if key is None:
            return
        try:
            stat = sidecar.stat()
            raw = sidecar.read_text(encoding="utf-8")
            bundle = read_bundle(sidecar)
        except (FileNotFoundError, BundleError, OSError) as e:
            logger.warning("bundle %s rejected at load: %s", sidecar, e)
            with self._lock:
                self._by_key.pop(key, None)
            return
        if not bundle.verify_witness():
            logger.warning("bundle %s witness MISMATCH; not serving", sidecar)
            with self._lock:
                self._by_key.pop(key, None)
            return
        cached = CachedBundle(
            key=key,
            path=sidecar,
            bundle=bundle,
            raw_json=raw,
            mtime_ns=stat.st_mtime_ns,
        )
        with self._lock:
            self._by_key[key] = cached
        logger.info("bundle loaded: key=%s witness=%s", key, bundle.rvf_witness[:16])

    def drop(self, sidecar: Path) -> None:
        key = self._key_from_path(self.root, sidecar)
        if key is None:
            return
        with self._lock:
            if self._by_key.pop(key, None) is not None:
                logger.info("bundle dropped: key=%s", key)

    def get(self, key: str) -> Optional[CachedBundle]:
        with self._lock:
            return self._by_key.get(key)

    def keys(self) -> list[str]:
        with self._lock:
            return sorted(self._by_key.keys())

    def initial_scan(self) -> None:
        """Walk the root once on startup so existing bundles get indexed."""
        if not self.root.exists():
            return
        for sidecar in self.root.rglob(SIDECAR_FILENAME):
            self.load_one(sidecar)


# --------------------------------------------------------------------------
# Watchdog wiring
# --------------------------------------------------------------------------

class _SidecarHandler(FileSystemEventHandler):
    def __init__(self, index: BundleIndex) -> None:
        self.index = index

    def _is_sidecar(self, path_str: str) -> bool:
        return Path(path_str).name == SIDECAR_FILENAME

    def on_created(self, event: FileSystemEvent) -> None:
        if event.is_directory:
            return
        if self._is_sidecar(event.src_path):
            self.index.load_one(Path(event.src_path))

    def on_modified(self, event: FileSystemEvent) -> None:
        if event.is_directory:
            return
        if self._is_sidecar(event.src_path):
            self.index.load_one(Path(event.src_path))

    def on_moved(self, event: FileSystemEvent) -> None:
        if event.is_directory:
            return
        # Atomic publish is rename(tmp -> table.rulake.json), so the
        # destination is what we care about.
        dest = getattr(event, "dest_path", None)
        if dest and self._is_sidecar(dest):
            self.index.load_one(Path(dest))

    def on_deleted(self, event: FileSystemEvent) -> None:
        if event.is_directory:
            return
        if self._is_sidecar(event.src_path):
            self.index.drop(Path(event.src_path))


# --------------------------------------------------------------------------
# FastAPI factory
# --------------------------------------------------------------------------

def create_app(root: Path, *, watch: bool = True) -> FastAPI:
    """Build a FastAPI app rooted at ``root``.

    Pass ``watch=False`` for tests that drive the index manually
    (avoids the cross-platform slop of waiting for filesystem events).
    """
    root = root.resolve()
    root.mkdir(parents=True, exist_ok=True)

    index = BundleIndex(root)
    index.initial_scan()

    observer: Observer | None = None
    if watch:
        observer = Observer()
        observer.schedule(_SidecarHandler(index), str(root), recursive=True)
        observer.start()

    @asynccontextmanager
    async def _lifespan(_app: FastAPI) -> AsyncIterator[None]:
        try:
            yield
        finally:
            if observer is not None:
                observer.stop()
                observer.join(timeout=2.0)

    app = FastAPI(
        title="ruLake bundle server",
        description=(
            "Serves ruLake table.rulake.json sidecars over HTTP, "
            "verifying each one's witness before exposing it."
        ),
        version="0.1.0",
        lifespan=_lifespan,
    )
    app.state.bundle_index = index
    app.state.bundle_root = root
    app.state.observer = observer

    @app.get("/health")
    def health() -> dict[str, object]:
        return {
            "status": "ok",
            "root": str(root),
            "bundles": len(index.keys()),
            "uptime_unix": int(time.time()),
        }

    @app.get("/bundles")
    def list_bundles() -> dict[str, object]:
        return {"keys": index.keys()}

    @app.get("/bundles/{key:path}/table.rulake.json")
    def get_bundle(key: str) -> Response:
        cached = index.get(key)
        if cached is None:
            raise HTTPException(status_code=404, detail=f"unknown bundle: {key!r}")
        # ETag is the witness — same witness ⇒ same bundle ⇒ 304-able.
        etag = f'"{cached.bundle.rvf_witness}"'
        return Response(
            content=cached.raw_json,
            media_type="application/json",
            headers={
                "ETag": etag,
                "Cache-Control": "no-cache",
                "X-RuLake-Witness": cached.bundle.rvf_witness,
            },
        )

    @app.get("/bundles/{key:path}/witness")
    def get_witness(key: str) -> PlainTextResponse:
        cached = index.get(key)
        if cached is None:
            raise HTTPException(status_code=404, detail=f"unknown bundle: {key!r}")
        return PlainTextResponse(
            cached.bundle.rvf_witness,
            headers={"Cache-Control": "no-cache"},
        )

    @app.get("/")
    def root_index() -> JSONResponse:
        return JSONResponse({
            "name": "rulake-bundle-server",
            "endpoints": [
                "/health",
                "/bundles",
                "/bundles/{key}/table.rulake.json",
                "/bundles/{key}/witness",
            ],
        })

    return app


def main() -> None:
    """`python server.py /path/to/publish-root`."""
    import argparse

    import uvicorn

    p = argparse.ArgumentParser(description="ruLake bundle server")
    p.add_argument("root", help="directory to watch for table.rulake.json files")
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=8088)
    args = p.parse_args()

    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )
    app = create_app(Path(args.root))
    uvicorn.run(app, host=args.host, port=args.port)


if __name__ == "__main__":
    main()
