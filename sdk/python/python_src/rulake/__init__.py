"""ruLake — vector cache + federation intermediary (Python bindings).

Re-exports the native classes from ``rulake._rulake``. The typed
exception hierarchy is rooted at ``RuLakeError`` and the typed children
``BackendNotFoundError``, ``CollectionNotFoundError``,
``DimensionMismatchError``, ``InvalidParameterError``, ``BackendError``
all inherit from it (ADR-002 §6). Catch ``RuLakeError`` as the
broad catch-all; catch the typed subclass when you need to discriminate.

Usage::

    import numpy as np
    import rulake

    lake = rulake.RuLake(rerank_factor=20, rotation_seed=42)
    lake = lake.with_consistency(rulake.Consistency.eventual(ttl_ms=5000))

    be = rulake.LocalBackend("local")
    ids = np.arange(10_000, dtype=np.uint64)
    vs  = np.random.randn(10_000, 768).astype(np.float32)
    be.put_collection("docs", ids, vs)
    lake.register_backend(be)

    q = np.random.randn(768).astype(np.float32)
    for hit in lake.search_one("local", "docs", q, k=10):
        print(hit.backend, hit.collection, hit.id, hit.score)
"""

from __future__ import annotations

from . import _rulake as _native

# Native classes — published as the "public" classes.
RuLake          = _native.RuLake
LocalBackend    = _native.LocalBackend
FsBackend       = _native.FsBackend
Bundle          = _native.Bundle
SearchResult    = _native.SearchResult
CacheStats      = _native.CacheStats
PerBackendStats = _native.PerBackendStats
Consistency     = _native.Consistency

__version__ = _native.__version__


# ─────────────────────────────────────────────────────────────────────
# Exceptions
#
# The native module raises these classes directly — we re-export them
# under the public ``rulake.*`` names. Multi-inheritance from stdlib
# bases (``LookupError`` etc.) was considered (ADR-002 §6) but does not
# survive PyO3's class-caching cleanly: the Rust side raises the
# native class, so monkey-patching from Python cannot change what
# ``isinstance`` sees. We accept ``except RuLakeError:`` as the
# catch-all and document the typed subclass for discrimination.
# Multi-inheritance is reopened as a v1.5 question.
# ─────────────────────────────────────────────────────────────────────

RuLakeError              = _native.RuLakeError
BackendNotFoundError     = _native.BackendNotFoundError
CollectionNotFoundError  = _native.CollectionNotFoundError
DimensionMismatchError   = _native.DimensionMismatchError
InvalidParameterError    = _native.InvalidParameterError
BackendError             = _native.BackendError


__all__ = [
    "RuLake",
    "LocalBackend",
    "FsBackend",
    "Bundle",
    "SearchResult",
    "CacheStats",
    "PerBackendStats",
    "Consistency",
    "RuLakeError",
    "BackendNotFoundError",
    "CollectionNotFoundError",
    "DimensionMismatchError",
    "InvalidParameterError",
    "BackendError",
    "__version__",
]
