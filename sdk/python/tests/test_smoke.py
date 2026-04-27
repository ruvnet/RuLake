"""Smoke tests for the ruLake Python binding.

These mirror the acceptance block in ADR-002 §Verification: build a
LocalBackend with a small collection, register it, search, check stats,
round-trip a bundle through JSON + the filesystem.

Run after `maturin develop --release` from the python/ directory:

    pytest tests/ -v
"""
from __future__ import annotations

import json
import tempfile
from pathlib import Path

import numpy as np
import pytest

import rulake


# ─────────────────────────────────────────────────────────────────────
# Search smoke
# ─────────────────────────────────────────────────────────────────────


def _make_lake_with_data(n: int = 10_000, dim: int = 128, seed: int = 0):
    rng = np.random.default_rng(seed)
    ids = np.arange(n, dtype=np.uint64)
    vectors = rng.standard_normal((n, dim)).astype(np.float32)

    lake = rulake.RuLake(rerank_factor=20, rotation_seed=42)
    be = rulake.LocalBackend("local")
    be.put_collection("docs", ids, vectors)
    lake.register_backend(be)
    return lake, ids, vectors


def test_search_one_returns_k_hits():
    lake, ids, vectors = _make_lake_with_data(n=5_000, dim=64)
    q = vectors[42].copy()  # known-present vector
    hits = lake.search_one("local", "docs", q, k=10)

    assert len(hits) == 10
    # Vector 42 is the closest to itself; expect id=42 in top-1.
    assert hits[0].id == 42
    assert hits[0].score == pytest.approx(0.0, abs=1e-3)
    assert hits[0].backend == "local"
    assert hits[0].collection == "docs"


def test_search_one_arrays_returns_numpy():
    lake, _, vectors = _make_lake_with_data(n=2_000, dim=32)
    q = vectors[7].copy()
    out_ids, out_scores = lake.search_one_arrays("local", "docs", q, k=10)

    assert isinstance(out_ids, np.ndarray) and out_ids.dtype == np.uint64
    assert isinstance(out_scores, np.ndarray) and out_scores.dtype == np.float32
    assert out_ids.shape == (10,)
    assert out_scores.shape == (10,)
    assert int(out_ids[0]) == 7


def test_search_batch_preserves_order():
    lake, _, vectors = _make_lake_with_data(n=2_000, dim=32)
    queries = vectors[[1, 2, 3, 4]]  # shape (4, 32), float32
    batches = lake.search_batch("local", "docs", queries, k=5)

    assert len(batches) == 4
    for i, hits in enumerate(batches):
        assert len(hits) == 5
        # Each query is a known vector — expect it as top-1.
        expected_id = i + 1
        assert hits[0].id == expected_id


def test_search_federated_across_two_backends():
    rng = np.random.default_rng(1)
    ids = np.arange(1_000, dtype=np.uint64)
    vs1 = rng.standard_normal((1_000, 32)).astype(np.float32)
    vs2 = rng.standard_normal((1_000, 32)).astype(np.float32)

    lake = rulake.RuLake(rerank_factor=20, rotation_seed=42)
    be1 = rulake.LocalBackend("be1"); be1.put_collection("docs", ids, vs1)
    be2 = rulake.LocalBackend("be2"); be2.put_collection("docs", ids, vs2)
    lake.register_backend(be1); lake.register_backend(be2)

    q = vs1[100].copy()
    hits = lake.search_federated([("be1", "docs"), ("be2", "docs")], q, k=5)
    assert len(hits) == 5
    # Top-1 should be from be1 since q is exactly vs1[100].
    assert hits[0].backend == "be1"
    assert hits[0].id == 100


# ─────────────────────────────────────────────────────────────────────
# Cache stats
# ─────────────────────────────────────────────────────────────────────


def test_cache_stats_track_hits_and_misses():
    lake, _, vectors = _make_lake_with_data(n=1_000, dim=16)
    # Use Eventual so coherence checks are skipped within ttl, generating
    # cache hits without any backend round-trips.
    lake = lake.with_consistency(rulake.Consistency.eventual(ttl_ms=60_000))
    q = vectors[0].copy()
    for _ in range(5):
        lake.search_one("local", "docs", q, k=5)

    stats = lake.cache_stats()
    assert stats.hits + stats.misses >= 1
    rate = stats.hit_rate()
    assert rate is None or 0.0 <= rate <= 1.0


# ─────────────────────────────────────────────────────────────────────
# Bundle round-trip — JSON + filesystem + witness verification
# ─────────────────────────────────────────────────────────────────────


def test_bundle_witness_round_trip_through_json():
    b = rulake.Bundle("test://x", dim=128, rotation_seed=42, rerank_factor=20, generation=7)
    assert b.verify_witness()
    body = b.to_json()
    parsed = json.loads(body)
    assert parsed["data_ref"] == "test://x"
    assert parsed["dim"] == 128
    assert len(parsed["rvf_witness"]) == 64  # SHAKE-256(32) hex

    b2 = rulake.Bundle.from_json(body)
    assert b2.rvf_witness == b.rvf_witness
    assert b2.verify_witness()


def test_bundle_generation_collision_is_avoided():
    """Regression: Num(7) and Opaque('\\x07\\0\\0\\0\\0\\0\\0\\0') must
    produce DIFFERENT witnesses (the 2026-04-23 audit fix)."""
    num = rulake.Bundle("x", 128, 42, 20, 7)
    opq = rulake.Bundle("x", 128, 42, 20, "\x07\0\0\0\0\0\0\0")
    assert num.rvf_witness != opq.rvf_witness


def test_bundle_round_trip_via_filesystem():
    b = rulake.Bundle("test://y", dim=64, rotation_seed=42, rerank_factor=20, generation="abc")
    with tempfile.TemporaryDirectory() as d:
        path = b.write_to_dir(d)
        assert Path(path).name == "table.rulake.json"
        assert Path(path).exists()
        b2 = rulake.Bundle.read_from_dir(d)
        assert b2.rvf_witness == b.rvf_witness


# ─────────────────────────────────────────────────────────────────────
# Errors
# ─────────────────────────────────────────────────────────────────────


def test_unknown_backend_raises_typed_error():
    lake = rulake.RuLake(rerank_factor=20, rotation_seed=42)
    q = np.zeros(8, dtype=np.float32)
    with pytest.raises(rulake.BackendNotFoundError):
        lake.search_one("nope", "docs", q, k=5)


def test_unknown_backend_caught_as_rulake_error():
    """ADR-002 §6: typed subclasses of RuLakeError discriminate; the
    base class is the catch-all idiom."""
    lake = rulake.RuLake(rerank_factor=20, rotation_seed=42)
    q = np.zeros(8, dtype=np.float32)
    with pytest.raises(rulake.RuLakeError):
        lake.search_one("nope", "docs", q, k=5)


def test_dim_mismatch_raises_typed_error():
    lake, _, _ = _make_lake_with_data(n=100, dim=32)
    q = np.zeros(64, dtype=np.float32)  # wrong dim
    with pytest.raises(rulake.DimensionMismatchError):
        lake.search_one("local", "docs", q, k=5)


def test_non_contiguous_query_rejected_loudly():
    """ADR-002 §2: non-contiguous arrays error rather than silent-copy."""
    lake, _, vectors = _make_lake_with_data(n=100, dim=32)
    # Take a strided view: every other element of a 64-long vector.
    big = np.zeros(64, dtype=np.float32)
    big[:32] = vectors[0]
    strided = big[::2]   # non-contiguous, len 32, dtype float32
    assert not strided.flags["C_CONTIGUOUS"]
    with pytest.raises(ValueError, match="contiguous"):
        lake.search_one("local", "docs", strided, k=5)


# ─────────────────────────────────────────────────────────────────────
# Consistency knob
# ─────────────────────────────────────────────────────────────────────


def test_consistency_factories():
    a = rulake.Consistency.fresh()
    b = rulake.Consistency.eventual(ttl_ms=1000)
    c = rulake.Consistency.frozen()
    assert "fresh" in repr(a).lower()
    assert "eventual(1000)" in repr(b)
    assert "frozen" in repr(c).lower()


# ─────────────────────────────────────────────────────────────────────
# Version sanity
# ─────────────────────────────────────────────────────────────────────


def test_version_string():
    assert isinstance(rulake.__version__, str)
    assert len(rulake.__version__) > 0
