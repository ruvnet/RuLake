"""Tests for the rulake-demo subprocess wrapper.

The parser tests run on canned stdout — fast and deterministic. The
end-to-end tests are gated behind ``RULAKE_RUN_END_TO_END=1`` so the
default suite doesn't depend on a built rabitq toolchain in CI.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from rulake import (
    BenchmarkReport,
    Rulake,
    RulakeError,
    _read_version_from_cargo,
    parse_benchmark,
)


CANNED_FAST = """\
=== ruLake benchmark harness ===
clustered Gaussian, D=128, 100 clusters, rerank×20, Fresh consistency unless noted
queries are warm (not timing first miss unless labeled 'prime')

── n = 5000 ──────────────────────────────────────────
  direct RaBitQ+ (Haar)    build=    21.9 ms   qps=   19867
  direct RaBitQ+ (Hadamard) build=     6.9 ms   qps=   21275   build_speedup=3.17×
  ruLake (Fresh)           prime=     4.5 ms   qps=   19298   tax=1.03×
  ruLake (Eventual 60s)    prime=     3.1 ms   qps=   19830   tax=1.00×
  ruLake federated (2 shards, Eventual)  prime=   3.4 ms   qps=   22867

Notes:
  - 'tax' is the direct-QPS / lake-QPS ratio.
"""

CANNED_FULL_HEADER = """\
── n = 50_000 ──────────────────────────────────────────
  direct RaBitQ+ (Haar)    build=   210.0 ms   qps=    9000
  ruLake (Fresh)           prime=    50.0 ms   qps=    8500   tax=1.06×
"""


# --------------------------------------------------------------------------
# Parser
# --------------------------------------------------------------------------

def test_parse_fast_canned() -> None:
    rows = parse_benchmark(CANNED_FAST)
    assert len(rows) == 5
    haar = rows[0]
    assert haar.n == 5000
    assert "Haar" in haar.label
    assert haar.build_ms == 21.9
    assert haar.qps == 19867
    assert haar.prime_ms is None
    assert haar.tax is None

    fresh = rows[2]
    assert "Fresh" in fresh.label
    assert fresh.prime_ms == 4.5
    assert fresh.qps == 19298
    assert fresh.tax == 1.03

    fed = rows[4]
    assert "federated" in fed.label
    assert fed.prime_ms == 3.4
    assert fed.qps == 22867


def test_parse_underscored_n() -> None:
    rows = parse_benchmark(CANNED_FULL_HEADER)
    assert all(r.n == 50_000 for r in rows)


def test_parse_empty_raises() -> None:
    with pytest.raises(RulakeError, match="no rows"):
        parse_benchmark("nothing useful here\n")


def test_report_helpers() -> None:
    report = BenchmarkReport(fast=True, rows=parse_benchmark(CANNED_FAST))
    assert len(report.for_n(5000)) == 5
    assert report.find(5000, "Hadamard") is not None
    assert report.find(5000, "Hadamard").qps == 21275  # type: ignore[union-attr]
    assert report.find(5000, "nope") is None


# --------------------------------------------------------------------------
# Locator / version
# --------------------------------------------------------------------------

def test_version_reads_cargo_toml(tmp_path: Path) -> None:
    cargo = tmp_path / "Cargo.toml"
    cargo.write_text(
        '[package]\n'
        'name = "ruvector-rulake"\n'
        'version = "9.9.9"\n'
        '\n[dependencies]\n'
        'foo = "1.0"\n',
        encoding="utf-8",
    )
    assert _read_version_from_cargo(tmp_path) == "9.9.9"


def test_version_skips_non_package_blocks(tmp_path: Path) -> None:
    cargo = tmp_path / "Cargo.toml"
    cargo.write_text(
        '[dependencies]\n'
        'foo = { version = "1.2.3" }\n'
        '\n[package]\n'
        'name = "ruvector-rulake"\n'
        'version = "2.2.0"\n',
        encoding="utf-8",
    )
    assert _read_version_from_cargo(tmp_path) == "2.2.0"


def test_constructor_resolves_real_cargo_root() -> None:
    """When run from inside the repo, the wrapper finds the parent crate."""
    w = Rulake()
    assert w.version() != ""  # Did locate Cargo.toml.


# --------------------------------------------------------------------------
# End-to-end (gated by env var so CI doesn't need cargo)
# --------------------------------------------------------------------------

@pytest.mark.skipif(
    os.environ.get("RULAKE_RUN_END_TO_END") != "1",
    reason="set RULAKE_RUN_END_TO_END=1 to run; needs the rulake-demo binary",
)
def test_end_to_end_fast() -> None:
    w = Rulake()
    report = w.benchmark(fast=True)
    assert report.fast is True
    assert report.rows
    assert all(r.qps > 0 for r in report.rows)
    # The Fresh row should always be present in --fast.
    fresh = report.find(5000, "Fresh")
    assert fresh is not None
    assert fresh.tax is not None and fresh.tax > 0


# --------------------------------------------------------------------------
# Spawn-error surfacing
# --------------------------------------------------------------------------

def test_missing_binary_and_no_cargo_raises(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """A wrapper with no resolvable binary and no cargo on PATH must error clearly."""
    monkeypatch.setenv("PATH", "/nonexistent")
    monkeypatch.delenv("RULAKE_DEMO_BINARY", raising=False)
    # Force a synthetic cargo_root with no target/ subdir.
    fake_root = tmp_path
    (fake_root / "Cargo.toml").write_text("[package]\nname='ruvector-rulake'\nversion='0.0.1'\n",
                                          encoding="utf-8")
    w = Rulake(cargo_root=fake_root, binary=None)
    with pytest.raises(RulakeError, match="cargo|not in PATH|not found"):
        w.benchmark(fast=True)
