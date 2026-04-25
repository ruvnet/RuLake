"""Tests for the RAG pipeline.

Coverage:

* round-trip ruvec1 reader / writer (matches the byte format that
  ``FsBackend`` writes in Rust);
* witness-verified ``load_corpus``;
* witness mismatch / bundle dim-vs-corpus dim mismatch are rejected;
* deterministic embedder and brute-force top-K return stable results;
* end-to-end pipeline returns hits whose ``provenance_id`` is the
  bundle's witness;
* the prompt carries the witness in both system block and per-hit
  provenance.
"""

from __future__ import annotations

import json
import struct
from pathlib import Path

import pytest

from pipeline import (
    Corpus,
    CorpusError,
    Hit,
    PipelineResult,
    brute_force_topk,
    build_prompt,
    embed_query,
    load_corpus,
    materialize_demo_snapshot,
    read_ruvec1,
    run_pipeline,
    write_ruvec1,
)
from rulake_witness import (
    SIDECAR_FILENAME,
    compute_witness,
    read_bundle_from_dir,
)


# --------------------------------------------------------------------------
# ruvec1 round-trip
# --------------------------------------------------------------------------

def test_ruvec1_round_trip(tmp_path: Path) -> None:
    p = tmp_path / "vectors.ruvec1"
    ids = [10, 20, 30]
    vectors = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]]
    write_ruvec1(p, 3, ids, vectors)
    rids, rvectors, rdim = read_ruvec1(p)
    assert rdim == 3
    assert rids == ids
    # f32 round-trip is exact for these values.
    assert rvectors == vectors


def test_ruvec1_header_layout(tmp_path: Path) -> None:
    """First 24 bytes must match the documented header verbatim."""
    p = tmp_path / "vectors.ruvec1"
    write_ruvec1(p, 4, [1], [[0.0, 0.0, 0.0, 0.0]])
    raw = p.read_bytes()
    assert raw[:8] == b"ruvec1\x00\x00"
    assert struct.unpack("<Q", raw[8:16])[0] == 1
    assert struct.unpack("<I", raw[16:20])[0] == 4
    assert struct.unpack("<I", raw[20:24])[0] == 0  # reserved zero


def test_ruvec1_rejects_bad_magic(tmp_path: Path) -> None:
    p = tmp_path / "v.ruvec1"
    p.write_bytes(b"NOTVECS\x00" + b"\x00" * 16)
    with pytest.raises(CorpusError, match="magic"):
        read_ruvec1(p)


def test_ruvec1_rejects_truncated(tmp_path: Path) -> None:
    p = tmp_path / "v.ruvec1"
    p.write_bytes(b"ruvec1\x00\x00")
    with pytest.raises(CorpusError, match="truncated"):
        read_ruvec1(p)


def test_ruvec1_rejects_oversize_dim(tmp_path: Path) -> None:
    p = tmp_path / "v.ruvec1"
    # Header claiming dim = u32::MAX — must be rejected before we try
    # to allocate.
    header = (b"ruvec1\x00\x00"
              + struct.pack("<Q", 0)
              + struct.pack("<I", 0xFFFF_FFFF)
              + struct.pack("<I", 0))
    p.write_bytes(header)
    with pytest.raises(CorpusError, match="dim"):
        read_ruvec1(p)


def test_ruvec1_rejects_payload_size_mismatch(tmp_path: Path) -> None:
    p = tmp_path / "v.ruvec1"
    # Claim 5 records but only write 1.
    header = (b"ruvec1\x00\x00"
              + struct.pack("<Q", 5)
              + struct.pack("<I", 2)
              + struct.pack("<I", 0)
              + struct.pack("<Q", 1)
              + struct.pack("<2f", 0.0, 0.0))
    p.write_bytes(header)
    with pytest.raises(CorpusError, match="payload size"):
        read_ruvec1(p)


# --------------------------------------------------------------------------
# Corpus loader: witness verification
# --------------------------------------------------------------------------

def test_load_corpus_verifies_witness(tmp_path: Path) -> None:
    materialize_demo_snapshot(tmp_path, dim=8, n=10)
    corpus = load_corpus(tmp_path)
    assert len(corpus.ids) == 10
    assert corpus.dim == 8
    assert corpus.bundle.verify_witness()
    # Witness changes if any bundle field changes.
    assert corpus.witness() == corpus.bundle.rvf_witness


def test_load_corpus_rejects_dim_mismatch(tmp_path: Path) -> None:
    """If the bundle says dim=8 but the ruvec1 file has dim=16, refuse."""
    materialize_demo_snapshot(tmp_path, dim=8, n=4)
    sidecar = tmp_path / SIDECAR_FILENAME
    raw = json.loads(sidecar.read_text())
    raw["dim"] = 16  # lie about dim
    raw["rvf_witness"] = compute_witness(
        raw["data_ref"], 16, raw["rotation_seed"],
        raw["rerank_factor"], raw["generation"],
    )
    sidecar.write_text(json.dumps(raw))
    with pytest.raises(CorpusError, match="dim"):
        load_corpus(tmp_path)


def test_load_corpus_rejects_witness_mismatch(tmp_path: Path) -> None:
    materialize_demo_snapshot(tmp_path, dim=8, n=4)
    sidecar = tmp_path / SIDECAR_FILENAME
    raw = json.loads(sidecar.read_text())
    raw["rvf_witness"] = "0" * 64  # tamper
    sidecar.write_text(json.dumps(raw))
    with pytest.raises(CorpusError, match="witness"):
        load_corpus(tmp_path)


# --------------------------------------------------------------------------
# Embedder + search
# --------------------------------------------------------------------------

def test_embed_is_deterministic() -> None:
    a = embed_query("hello", 32)
    b = embed_query("hello", 32)
    assert a == b
    assert len(a) == 32
    assert all(-1.0 <= x < 1.0 for x in a)


def test_embed_changes_with_text() -> None:
    assert embed_query("a", 16) != embed_query("b", 16)


def test_brute_force_topk_orders_by_l2() -> None:
    bundle = read_bundle_from_dir(_make_snapshot()) if False else None
    # Synthetic corpus: identity vectors with one obviously-closest match.
    corpus = Corpus(
        bundle=_synthetic_bundle(),
        ids=[100, 200, 300],
        vectors=[[0.0, 0.0], [1.0, 0.0], [10.0, 10.0]],
        dim=2,
    )
    hits = brute_force_topk(corpus, [0.9, 0.0], k=2)
    assert [h.id for h in hits] == [200, 100]
    assert hits[0].score < hits[1].score
    # provenance_id is the bundle witness.
    assert all(h.provenance_id == corpus.witness() for h in hits)


def test_brute_force_topk_dim_mismatch() -> None:
    corpus = Corpus(
        bundle=_synthetic_bundle(),
        ids=[1], vectors=[[0.0, 0.0]], dim=2,
    )
    with pytest.raises(CorpusError, match="dim"):
        brute_force_topk(corpus, [0.0, 0.0, 0.0], k=1)


# --------------------------------------------------------------------------
# Prompt
# --------------------------------------------------------------------------

def test_build_prompt_carries_witness() -> None:
    witness = "deadbeef" * 8
    hits = [
        Hit(id=1, score=0.5, provenance_id=witness),
        Hit(id=2, score=0.7, provenance_id=witness),
    ]
    out = build_prompt("what is the witness?", hits, corpus_label="test")
    assert "Provenance witness:" in out
    assert witness in out
    assert "[id=1]" in out
    assert "[id=2]" in out
    assert "[QUERY]" in out
    assert "[RESPONSE]" in out


def test_build_prompt_handles_no_hits() -> None:
    out = build_prompt("nothing here", [])
    assert "(no hits)" in out
    assert "<no-hits>" in out


# --------------------------------------------------------------------------
# End-to-end
# --------------------------------------------------------------------------

def test_run_pipeline_end_to_end(tmp_path: Path) -> None:
    materialize_demo_snapshot(tmp_path, dim=16, n=20)
    result = run_pipeline(tmp_path, "find me cluster zero", k=3)

    assert isinstance(result, PipelineResult)
    assert result.corpus_size == 20
    assert len(result.hits) == 3
    # Witness propagates to every hit.
    witness = result.bundle.rvf_witness
    assert all(h.provenance_id == witness for h in result.hits)
    # Hits are L2-sorted.
    scores = [h.score for h in result.hits]
    assert scores == sorted(scores)
    # Prompt mentions the witness.
    assert witness in result.prompt


# --------------------------------------------------------------------------
# helpers
# --------------------------------------------------------------------------

def _synthetic_bundle():
    from rulake_witness import RuLakeBundle, compute_witness
    w = compute_witness("file:///tmp/x", 2, 0, 1, 1)
    return RuLakeBundle(
        format_version=2,
        data_ref="file:///tmp/x",
        dim=2,
        rotation_seed=0,
        rerank_factor=1,
        generation=1,
        rvf_witness=w,
        extra={},
    )


def _make_snapshot() -> Path:  # unused convenience for IDE navigation
    raise NotImplementedError
