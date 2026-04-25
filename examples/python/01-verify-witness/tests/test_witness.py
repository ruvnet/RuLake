"""Tests for the Python witness implementation.

The smoking-gun assertion is that our :func:`compute_witness` matches
witnesses produced by the Rust crate byte-for-byte. We pin three
fixtures into ``fixtures/``:

* ``known-good-bundle.json`` — a hand-rolled deterministic bundle whose
  witness this Python module computes.
* ``rust-sidecar-daemon.json`` — captured from
  ``cargo run --release --example sidecar_daemon``.
* ``rust-warm-restart.json`` — captured from
  ``cargo run --release --example warm_restart``.

If either of the ``rust-*.json`` files disagrees with what Python
recomputes, the witness implementation is broken and EVERY downstream
ruLake-consuming Python service is broken with it.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from rulake_witness import (
    BundleError,
    MAX_FIELD_BYTES,
    MAX_JSON_BYTES,
    SUPPORTED_FORMAT_VERSION,
    compute_witness,
    parse_bundle_json,
    read_bundle,
)

FIXTURES = Path(__file__).resolve().parent.parent / "fixtures"


# --------------------------------------------------------------------------
# Cross-language ground-truth fixtures
# --------------------------------------------------------------------------

@pytest.mark.parametrize("filename", [
    "known-good-bundle.json",
    "rust-sidecar-daemon.json",
    "rust-warm-restart.json",
])
def test_fixture_witness_matches_python_recompute(filename: str) -> None:
    bundle = read_bundle(FIXTURES / filename)
    assert bundle.verify_witness(), (
        f"{filename}: on-disk witness {bundle.rvf_witness!r} did not "
        f"match Python recompute. The witness implementation has drifted "
        f"from the Rust source."
    )


def test_rust_witness_exact_value() -> None:
    """Belt-and-braces: pin the exact 64-hex value from the live Rust run."""
    sidecar = read_bundle(FIXTURES / "rust-sidecar-daemon.json")
    assert sidecar.data_ref == "local://publisher/memories"
    assert sidecar.dim == 8
    assert sidecar.rotation_seed == 42
    assert sidecar.rerank_factor == 20
    assert sidecar.generation == 2
    assert sidecar.rvf_witness == (
        "31c0865f078d9d646edaf0fe339d5d8c20a04bac9e95571fae91035306c2b584"
    )
    # And independently of parse_bundle_json:
    assert compute_witness(
        "local://publisher/memories", 8, 42, 20, 2
    ) == sidecar.rvf_witness


# --------------------------------------------------------------------------
# Audit-driven regressions
# --------------------------------------------------------------------------

def test_num_and_opaque_cannot_collide() -> None:
    """Regression: Generation::Num(7) vs Opaque("\\x07\\0\\0\\0\\0\\0\\0\\0").

    Before the 2026-04-23 audit fix added a leading variant tag byte to
    ``hash_bytes()``, these two produced byte-identical digest input
    streams and therefore identical witnesses. They MUST differ now.
    """
    w_num = compute_witness("x", 1, 0, 1, 7)
    w_opaque = compute_witness("x", 1, 0, 1, "\x07\0\0\0\0\0\0\0")
    assert w_num != w_opaque, (
        "Num(7) and Opaque('\\x07\\0\\0\\0\\0\\0\\0\\0') witnesses must "
        "be distinguishable; the variant tag byte regressed."
    )


def test_witness_is_length_prefixed() -> None:
    """Regression: 'a|b' vs 'ab|' should not collide on a concat-only scheme."""
    a = compute_witness("x", 1, 0, 1, "a|b")
    b = compute_witness("x", 1, 0, 1, "ab|")
    assert a != b


def test_witness_is_deterministic() -> None:
    a = compute_witness("gs://bucket/x.parquet", 128, 42, 20, 7)
    b = compute_witness("gs://bucket/x.parquet", 128, 42, 20, 7)
    assert a == b
    assert len(a) == 64
    assert all(c in "0123456789abcdef" for c in a)


@pytest.mark.parametrize("mutation", [
    {"data_ref": "gs://bucket/y.parquet"},
    {"dim": 256},
    {"rotation_seed": 43},
    {"rerank_factor": 5},
    {"generation": 8},
])
def test_witness_changes_on_any_field(mutation: dict) -> None:
    base = dict(data_ref="gs://bucket/x.parquet",
                dim=128, rotation_seed=42, rerank_factor=20, generation=7)
    a = compute_witness(**base)
    b = compute_witness(**{**base, **mutation})
    assert a != b


def test_memory_class_does_not_affect_witness() -> None:
    """``memory_class`` is opaque metadata and not part of the digest."""
    plain = json.dumps({
        "format_version": 2,
        "data_ref": "x",
        "dim": 8,
        "rotation_seed": 1,
        "rerank_factor": 5,
        "generation": 1,
        "rvf_witness": compute_witness("x", 8, 1, 5, 1),
    })
    tagged = json.loads(plain)
    tagged["memory_class"] = "episodic"
    tagged_text = json.dumps(tagged)

    p = parse_bundle_json(plain)
    t = parse_bundle_json(tagged_text)
    assert p.rvf_witness == t.rvf_witness
    assert t.extra["memory_class"] == "episodic"


# --------------------------------------------------------------------------
# DoS / parser hardening
# --------------------------------------------------------------------------

def test_oversize_json_rejected() -> None:
    payload = json.dumps({
        "format_version": 2,
        "data_ref": "x",
        "dim": 1, "rotation_seed": 0, "rerank_factor": 1,
        "generation": 1,
        "rvf_witness": "0" * 64,
        "_pad": "x" * (MAX_JSON_BYTES + 16),
    })
    with pytest.raises(BundleError, match="exceeds"):
        parse_bundle_json(payload)


def test_oversize_field_rejected() -> None:
    payload = json.dumps({
        "format_version": 2,
        "data_ref": "x" * (MAX_FIELD_BYTES + 8),
        "dim": 1, "rotation_seed": 0, "rerank_factor": 1,
        "generation": 1,
        "rvf_witness": "0" * 64,
    })
    with pytest.raises(BundleError, match="data_ref"):
        parse_bundle_json(payload)


def test_future_format_version_rejected() -> None:
    payload = json.dumps({
        "format_version": SUPPORTED_FORMAT_VERSION + 1,
        "data_ref": "x",
        "dim": 1, "rotation_seed": 0, "rerank_factor": 1,
        "generation": 1,
        "rvf_witness": "0" * 64,
    })
    with pytest.raises(BundleError, match="newer than supported"):
        parse_bundle_json(payload)


def test_bool_generation_rejected() -> None:
    """JSON bools must not silently digest as Num(0) / Num(1)."""
    payload = json.dumps({
        "format_version": 2,
        "data_ref": "x",
        "dim": 1, "rotation_seed": 0, "rerank_factor": 1,
        "generation": True,
        "rvf_witness": "0" * 64,
    })
    with pytest.raises(BundleError, match="generation"):
        parse_bundle_json(payload)


def test_missing_field_rejected() -> None:
    payload = json.dumps({
        "format_version": 2,
        "dim": 1, "rotation_seed": 0, "rerank_factor": 1,
        "generation": 1,
        "rvf_witness": "0" * 64,
    })
    with pytest.raises(BundleError, match="data_ref"):
        parse_bundle_json(payload)


def test_witness_field_too_long_rejected() -> None:
    payload = json.dumps({
        "format_version": 2,
        "data_ref": "x",
        "dim": 1, "rotation_seed": 0, "rerank_factor": 1,
        "generation": 1,
        "rvf_witness": "0" * 256,
    })
    with pytest.raises(BundleError, match="rvf_witness"):
        parse_bundle_json(payload)


def test_tamper_detected() -> None:
    """Mutate a field on disk without recomputing — verify must reject."""
    bundle = read_bundle(FIXTURES / "known-good-bundle.json")
    raw = json.loads((FIXTURES / "known-good-bundle.json").read_text())
    raw["dim"] = bundle.dim + 1  # tamper
    tampered = parse_bundle_json(json.dumps(raw))
    assert not tampered.verify_witness(), (
        "verify_witness must catch a tampered dim field"
    )


# --------------------------------------------------------------------------
# Rust-side test-vector parity
# --------------------------------------------------------------------------

def test_opaque_generation_witness_parity() -> None:
    """Mirror of ``opaque_generation_serialization_roundtrip`` in
    ``src/bundle.rs``."""
    w = compute_witness(
        "iceberg://catalog/db/table",
        384,
        0xdead_beef,
        5,
        "01JCX7NK6G5R9G1YZ7QH",
    )
    # The exact value is not asserted here (no Rust capture of this one),
    # but it must round-trip through parse_bundle_json.
    body = json.dumps({
        "format_version": 2,
        "data_ref": "iceberg://catalog/db/table",
        "dim": 384,
        "rotation_seed": 0xdead_beef,
        "rerank_factor": 5,
        "generation": "01JCX7NK6G5R9G1YZ7QH",
        "rvf_witness": w,
    })
    parsed = parse_bundle_json(body)
    assert parsed.verify_witness()
    assert parsed.generation == "01JCX7NK6G5R9G1YZ7QH"
