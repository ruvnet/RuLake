"""Tests for the bundle-server FastAPI app and the publish CLI.

We disable the watchdog observer in tests (``watch=False``) and drive
:meth:`BundleIndex.load_one` / :meth:`BundleIndex.drop` directly. That
avoids the cross-platform timing slop of waiting for filesystem events
and lets the suite run in milliseconds.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from server import create_app
from publish import publish

from rulake_witness import compute_witness


# --------------------------------------------------------------------------
# Helpers
# --------------------------------------------------------------------------

def _make_bundle(data_ref: str = "gs://bucket/x", dim: int = 8,
                 rotation_seed: int = 42, rerank_factor: int = 20,
                 generation: int = 1) -> dict:
    return {
        "format_version": 2,
        "data_ref": data_ref,
        "dim": dim,
        "rotation_seed": rotation_seed,
        "rerank_factor": rerank_factor,
        "generation": generation,
        "rvf_witness": compute_witness(
            data_ref, dim, rotation_seed, rerank_factor, generation
        ),
    }


def _write(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")


# --------------------------------------------------------------------------
# Server tests
# --------------------------------------------------------------------------

def test_health(tmp_path: Path) -> None:
    app = create_app(tmp_path, watch=False)
    with TestClient(app) as c:
        r = c.get("/health")
        assert r.status_code == 200
        body = r.json()
        assert body["status"] == "ok"
        assert body["bundles"] == 0


def test_initial_scan_picks_up_existing(tmp_path: Path) -> None:
    bundle = _make_bundle()
    _write(tmp_path / "prod" / "memories" / "table.rulake.json", bundle)
    app = create_app(tmp_path, watch=False)
    with TestClient(app) as c:
        listed = c.get("/bundles").json()
        assert listed == {"keys": ["prod/memories"]}

        full = c.get("/bundles/prod/memories/table.rulake.json")
        assert full.status_code == 200
        assert full.headers["X-RuLake-Witness"] == bundle["rvf_witness"]
        assert json.loads(full.text)["rvf_witness"] == bundle["rvf_witness"]
        # ETag is the witness wrapped in quotes.
        assert full.headers["ETag"] == f'"{bundle["rvf_witness"]}"'

        wit = c.get("/bundles/prod/memories/witness")
        assert wit.status_code == 200
        assert wit.text == bundle["rvf_witness"]


def test_unknown_key_404(tmp_path: Path) -> None:
    app = create_app(tmp_path, watch=False)
    with TestClient(app) as c:
        assert c.get("/bundles/nope/witness").status_code == 404
        assert c.get("/bundles/nope/table.rulake.json").status_code == 404


def test_witness_mismatch_not_served(tmp_path: Path) -> None:
    """A sidecar whose witness doesn't match must NOT be exposed."""
    bundle = _make_bundle()
    bundle["rvf_witness"] = "0" * 64  # tamper
    _write(tmp_path / "bad" / "table.rulake.json", bundle)
    app = create_app(tmp_path, watch=False)
    with TestClient(app) as c:
        assert c.get("/bundles").json() == {"keys": []}
        assert c.get("/bundles/bad/table.rulake.json").status_code == 404


def test_load_one_then_drop(tmp_path: Path) -> None:
    app = create_app(tmp_path, watch=False)
    bundle = _make_bundle(data_ref="gs://bucket/y")
    sidecar = tmp_path / "team" / "coll" / "table.rulake.json"
    _write(sidecar, bundle)

    index = app.state.bundle_index
    index.load_one(sidecar)
    with TestClient(app) as c:
        assert c.get("/bundles").json() == {"keys": ["team/coll"]}

    sidecar.unlink()
    index.drop(sidecar)
    with TestClient(app) as c:
        assert c.get("/bundles").json() == {"keys": []}


def test_witness_rotates_when_bundle_changes(tmp_path: Path) -> None:
    """A re-published bundle with a new generation gets a new witness."""
    sidecar = tmp_path / "k" / "table.rulake.json"
    v1 = _make_bundle(generation=1)
    v2 = _make_bundle(generation=2)
    assert v1["rvf_witness"] != v2["rvf_witness"]

    _write(sidecar, v1)
    app = create_app(tmp_path, watch=False)
    index = app.state.bundle_index
    with TestClient(app) as c:
        assert c.get("/bundles/k/witness").text == v1["rvf_witness"]

        # Re-publish v2 atomically (mirror of the Rust write-then-rename).
        _write(sidecar, v2)
        index.load_one(sidecar)
        assert c.get("/bundles/k/witness").text == v2["rvf_witness"]


def test_root_index(tmp_path: Path) -> None:
    app = create_app(tmp_path, watch=False)
    with TestClient(app) as c:
        body = c.get("/").json()
        assert body["name"] == "rulake-bundle-server"
        assert "/health" in body["endpoints"]


# --------------------------------------------------------------------------
# Publish CLI tests
# --------------------------------------------------------------------------

def test_publish_writes_into_key_dir(tmp_path: Path) -> None:
    src = tmp_path / "src.json"
    _write(src, _make_bundle())
    root = tmp_path / "publish"
    target = publish(root, "prod-warehouse/memories", src)
    assert target == root / "prod-warehouse" / "memories" / "table.rulake.json"
    assert target.is_file()


def test_publish_refuses_bad_witness(tmp_path: Path) -> None:
    bundle = _make_bundle()
    bundle["rvf_witness"] = "0" * 64
    src = tmp_path / "bad.json"
    _write(src, bundle)
    with pytest.raises(SystemExit, match="witness mismatch"):
        publish(tmp_path / "publish", "k", src)


@pytest.mark.parametrize("bad", ["", ".", "..", "a/../b", "a\\b", "a:b", "/abs"])
def test_publish_refuses_traversal_keys(tmp_path: Path, bad: str) -> None:
    src = tmp_path / "ok.json"
    _write(src, _make_bundle())
    with pytest.raises(SystemExit):
        publish(tmp_path / "publish", bad, src)


def test_publish_round_trip_through_server(tmp_path: Path) -> None:
    """End-to-end: write source → publish → server serves matching bundle."""
    src = tmp_path / "src.json"
    bundle = _make_bundle(data_ref="iceberg://catalog/db/t", generation=99)
    _write(src, bundle)

    publish_root = tmp_path / "publish"
    publish(publish_root, "alpha/coll", src)

    app = create_app(publish_root, watch=False)
    with TestClient(app) as c:
        assert c.get("/bundles/alpha/coll/witness").text == bundle["rvf_witness"]
