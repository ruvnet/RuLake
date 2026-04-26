"""Witness-pinned RAG pipeline that reads a ruLake-style snapshot in Python.

Story
-----

A Rust service publishes a ruLake snapshot directory containing:

  - a vector data file
  - a ``table.rulake.json`` sidecar pointing at it (with the witness)

This Python pipeline:

  1. Verifies the bundle's witness (re-uses the verifier from
     ``01-verify-witness``).
  2. Parses the binary vector data file (the documented ``ruvec1``
     format — see ``src/fs_backend.rs``).
  3. Embeds an incoming query (optional — ``sentence-transformers``
     if installed, deterministic random fallback otherwise; the
     embedding model is not the point).
  4. Brute-force L2-searches for the top-K nearest vectors.
  5. Annotates every hit with the bundle's witness as
     ``provenance_id`` so a downstream LLM call can be audited back
     to the exact snapshot it was grounded on.
  6. Builds (and prints) a witness-pinned LLM prompt.

Why ``ruvec1`` instead of ``index.rbpx``
----------------------------------------

The ``warm_restart`` example writes a rabitq-compressed
``index.rbpx`` next to the bundle. That file is opaque without the
``ruvector-rabitq`` library, and we don't have it in pure Python. The
``ruvec1`` format that ``FsBackend`` writes IS plainly parseable from
Python — it's the same role (vector blob behind a witnessed bundle),
just before compression. The directional value (witness-pinned
provenance, deterministic snapshots, language-portable bundle)
travels intact.

Public surface
--------------

* :func:`load_corpus` — read a witness-verified ``ruvec1`` corpus.
* :func:`brute_force_topk` — top-K L2 over the loaded corpus.
* :func:`embed_query` — best-available embedder, deterministic fallback.
* :func:`build_prompt` — render the LLM prompt.
* :func:`run_pipeline` — wire all of the above together.
* :func:`write_ruvec1` — helper for the demo / tests to produce a corpus.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import struct
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, Optional, Sequence

from rulake_witness import (
    BundleError,
    RuLakeBundle,
    SIDECAR_FILENAME,
    compute_witness,
    read_bundle_from_dir,
)

# --------------------------------------------------------------------------
# ruvec1 binary format (from src/fs_backend.rs)
# --------------------------------------------------------------------------
#
#   bytes  field
#   0..8   magic         = b"ruvec1\0\0"
#   8..16  count : u64   little-endian
#  16..20  dim   : u32   little-endian
#  20..24  _reserved     (must be zero)
#  24..    records × count, each:
#            id : u64  little-endian
#            v  : f32 × dim  little-endian

RUVEC1_MAGIC = b"ruvec1\0\0"
RUVEC1_HEADER_BYTES = 24

# Mirror of rulake::backend::MAX_PULLED_VECTORS / MAX_PULLED_DIM,
# which gate the Rust loader. Anything beyond these is hostile or buggy.
MAX_VECTORS = 10_000_000
MAX_DIM = 65_536


class CorpusError(ValueError):
    """Raised on any ruvec1 / corpus parse failure."""


# --------------------------------------------------------------------------
# Hits + corpus dataclasses
# --------------------------------------------------------------------------

@dataclass(frozen=True)
class Hit:
    """One retrieval hit. ``provenance_id`` is the bundle witness."""

    id: int
    score: float
    provenance_id: str


@dataclass
class Corpus:
    """In-memory representation of a ruLake-published vector collection."""

    bundle: RuLakeBundle
    ids: list[int]
    vectors: list[list[float]]
    dim: int

    def witness(self) -> str:
        return self.bundle.rvf_witness


@dataclass
class PipelineResult:
    """Output of :func:`run_pipeline`."""

    query_text: str
    hits: list[Hit]
    prompt: str
    bundle: RuLakeBundle
    corpus_size: int = field(default=0)


# --------------------------------------------------------------------------
# ruvec1 read / write
# --------------------------------------------------------------------------

def write_ruvec1(path: Path, dim: int, ids: Sequence[int],
                 vectors: Sequence[Sequence[float]]) -> Path:
    """Write a ``ruvec1`` file. Mirror of ``FsBackend::write``.

    Used by the demo / tests to build a corpus the way a Rust
    publisher would. The on-disk bytes are identical to what the
    Rust writer produces.
    """
    if len(ids) != len(vectors):
        raise CorpusError(f"ids ({len(ids)}) != vectors ({len(vectors)})")
    if dim <= 0 or dim > MAX_DIM:
        raise CorpusError(f"dim {dim} outside (0, {MAX_DIM}]")
    for i, v in enumerate(vectors):
        if len(v) != dim:
            raise CorpusError(f"vectors[{i}] has len {len(v)}, expected {dim}")
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name("." + path.name + ".tmp")
    with tmp.open("wb") as f:
        f.write(RUVEC1_MAGIC)
        f.write(struct.pack("<Q", len(ids)))
        f.write(struct.pack("<I", dim))
        f.write(struct.pack("<I", 0))
        for idv, vec in zip(ids, vectors):
            f.write(struct.pack("<Q", idv))
            f.write(struct.pack(f"<{dim}f", *vec))
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)
    return path


def read_ruvec1(path: Path) -> tuple[list[int], list[list[float]], int]:
    """Read a ``ruvec1`` file and return ``(ids, vectors, dim)``.

    Mirror of ``FsBackend::pull_vectors``. Streams header-then-payload so
    a hostile / oversized file is rejected by the bounds checks BEFORE
    its bytes ever land in process memory. Without streaming, a
    `read_bytes()` of a 50 GB file would OOM the process before any
    guard fired.
    """
    with open(path, "rb") as f:
        header = f.read(RUVEC1_HEADER_BYTES)
        if len(header) < RUVEC1_HEADER_BYTES:
            raise CorpusError(f"{path}: truncated header ({len(header)} bytes)")
        if header[:8] != RUVEC1_MAGIC:
            raise CorpusError(f"{path}: bad magic {header[:8]!r}")
        count = struct.unpack("<Q", header[8:16])[0]
        dim = struct.unpack("<I", header[16:20])[0]
        # header[20:24] reserved — ignored.
        if count > MAX_VECTORS:
            raise CorpusError(f"{path}: count={count} exceeds MAX_VECTORS")
        if dim == 0 or dim > MAX_DIM:
            raise CorpusError(f"{path}: dim={dim} outside (0, {MAX_DIM}]")

        record_bytes = 8 + dim * 4
        payload_bytes = count * record_bytes
        # Cross-check on-disk size matches the declared header BEFORE we
        # try to read it. os.fstat is cheap and lets us refuse hostile
        # files (extra trailing bytes, truncated payload) up front.
        actual_size = os.fstat(f.fileno()).st_size
        expected_total = RUVEC1_HEADER_BYTES + payload_bytes
        if actual_size != expected_total:
            raise CorpusError(
                f"{path}: file size {actual_size} != expected {expected_total} "
                f"(count={count}, dim={dim})"
            )
        payload = f.read(payload_bytes)
        if len(payload) != payload_bytes:
            raise CorpusError(
                f"{path}: short read {len(payload)} of {payload_bytes} bytes"
            )

    ids: list[int] = []
    vectors: list[list[float]] = []
    off = 0
    for _ in range(count):
        idv = struct.unpack("<Q", payload[off:off + 8])[0]
        off += 8
        vec = list(struct.unpack(f"<{dim}f", payload[off:off + dim * 4]))
        off += dim * 4
        ids.append(idv)
        vectors.append(vec)
    return ids, vectors, dim


# --------------------------------------------------------------------------
# Loader: bundle + corpus, witness-verified
# --------------------------------------------------------------------------

def _resolve_data_file(bundle_dir: Path, bundle: RuLakeBundle,
                       data_filename: Optional[str]) -> Path:
    """Pick the on-disk vector file accompanying ``bundle``.

    Resolution order:

    1. ``data_filename`` argument, if given (relative to ``bundle_dir``).
    2. The ``data_ref`` of the bundle, if it parses as ``file://``.
    3. ``bundle_dir / "vectors.ruvec1"`` (this example's convention).

    All three branches enforce containment: the resolved data file MUST
    live inside ``bundle_dir`` (after symlink resolution). This refuses:

    - ``data_filename = "../../../etc/hostname"`` — relative escape.
    - ``data_ref = "file:///etc/passwd"`` — absolute escape via a hostile
      publisher (the witness covers ``data_ref``, so a quiet tamper of an
      already-published bundle is impossible — but a malicious publisher
      can still mint a fresh, witness-correct bundle that names an
      arbitrary local file).
    - A symlink inside ``bundle_dir`` that points outside it.

    The substrate trust boundary is the bundle dir; everything outside is
    untrusted regardless of who published the sidecar.
    """
    bundle_dir_resolved = bundle_dir.resolve()

    def _enforce_contained(candidate: Path, why: str) -> Path:
        resolved = candidate.resolve()
        try:
            resolved.relative_to(bundle_dir_resolved)
        except ValueError as e:
            raise CorpusError(
                f"refusing to load data file outside bundle dir "
                f"({resolved} is not under {bundle_dir_resolved}); "
                f"source: {why}"
            ) from e
        if not resolved.is_file():
            raise CorpusError(f"{resolved}: not a regular file (source: {why})")
        return resolved

    if data_filename is not None:
        candidate = bundle_dir / data_filename
        return _enforce_contained(candidate, why="data_filename argument")

    if bundle.data_ref.startswith("file://"):
        # ``file://`` URIs are absolute paths on POSIX. We resolve them
        # and require they land inside the bundle dir — the bundle is the
        # trust unit, not the URI.
        candidate = Path(bundle.data_ref[len("file://"):])
        return _enforce_contained(candidate, why=f"bundle.data_ref={bundle.data_ref!r}")

    default = bundle_dir / "vectors.ruvec1"
    if default.is_file():
        return _enforce_contained(default, why="convention vectors.ruvec1")

    raise CorpusError(
        f"could not locate the data file for bundle at {bundle_dir}; "
        f"data_ref={bundle.data_ref!r}"
    )


def load_corpus(bundle_dir: Path, *,
                data_filename: Optional[str] = None) -> Corpus:
    """Verify the witness, then load the accompanying ``ruvec1`` corpus.

    The bundle's ``dim`` is cross-checked against the ruvec1 header —
    a mismatch means the publisher rotated one without the other and
    the snapshot is incoherent.
    """
    bundle = read_bundle_from_dir(bundle_dir)
    if not bundle.verify_witness():
        raise CorpusError(
            f"bundle witness mismatch at {bundle_dir / SIDECAR_FILENAME}"
        )
    data_path = _resolve_data_file(bundle_dir, bundle, data_filename)
    ids, vectors, dim = read_ruvec1(data_path)
    if dim != bundle.dim:
        raise CorpusError(
            f"corpus dim {dim} != bundle dim {bundle.dim} "
            f"(snapshot is incoherent)"
        )
    return Corpus(bundle=bundle, ids=ids, vectors=vectors, dim=dim)


# --------------------------------------------------------------------------
# Brute-force L2 search
# --------------------------------------------------------------------------

def _l2_sq(a: Sequence[float], b: Sequence[float]) -> float:
    s = 0.0
    for x, y in zip(a, b):
        d = x - y
        s += d * d
    return s


def brute_force_topk(corpus: Corpus, query: Sequence[float], k: int) -> list[Hit]:
    """Top-K L2 over the corpus. Returns hits sorted by ascending distance."""
    if len(query) != corpus.dim:
        raise CorpusError(
            f"query dim {len(query)} != corpus dim {corpus.dim}"
        )
    if k <= 0:
        return []
    scored = [
        (idv, _l2_sq(query, vec))
        for idv, vec in zip(corpus.ids, corpus.vectors)
    ]
    scored.sort(key=lambda t: t[1])
    witness = corpus.witness()
    return [Hit(id=idv, score=score, provenance_id=witness)
            for idv, score in scored[:k]]


# --------------------------------------------------------------------------
# Embedding (best-available, deterministic fallback)
# --------------------------------------------------------------------------

def _try_sentence_transformers_embed(
    text: str, dim: int,
) -> Optional[list[float]]:
    """If sentence-transformers is installed, embed and (re-)dimension.

    The embedding model is NOT the point of this example; we just want
    a vector of the right shape. If the model dim doesn't match the
    corpus dim, we tile / truncate. A real deployment would pick a
    model whose dim matches the corpus.
    """
    try:
        from sentence_transformers import SentenceTransformer  # type: ignore
    except ImportError:
        return None
    model = SentenceTransformer("all-MiniLM-L6-v2")
    raw = model.encode([text])[0].tolist()
    return _conform_dim(raw, dim)


def _conform_dim(vec: Sequence[float], dim: int) -> list[float]:
    if len(vec) == dim:
        return list(vec)
    if len(vec) > dim:
        return list(vec)[:dim]
    out = list(vec)
    while len(out) < dim:
        out.extend(vec)
    return out[:dim]


def _deterministic_embed(text: str, dim: int) -> list[float]:
    """SHAKE-256-driven random projection — deterministic, no deps."""
    h = hashlib.shake_256()
    h.update(text.encode("utf-8"))
    raw = h.digest(dim * 4)
    out: list[float] = []
    for i in range(dim):
        # Map each 4-byte chunk to a float in [-1, 1).
        u = struct.unpack("<I", raw[i * 4:(i + 1) * 4])[0]
        out.append((u / (1 << 32)) * 2.0 - 1.0)
    return out


def embed_query(text: str, dim: int, *,
                use_sentence_transformers: bool = False) -> list[float]:
    """Embed ``text`` into a ``dim``-vector.

    Tries sentence-transformers if ``use_sentence_transformers=True``,
    falls back to a deterministic SHAKE-256-driven projection
    otherwise. The deterministic path is what the tests rely on so
    they don't drag in the model.
    """
    if use_sentence_transformers:
        st = _try_sentence_transformers_embed(text, dim)
        if st is not None:
            return st
    return _deterministic_embed(text, dim)


# --------------------------------------------------------------------------
# Prompt assembly
# --------------------------------------------------------------------------

def build_prompt(query: str, hits: Iterable[Hit], *,
                 corpus_label: Optional[str] = None) -> str:
    """Render a witness-pinned LLM prompt around the retrieved hits.

    The prompt is deliberately spartan — the goal is to show the
    structure, not to win prompt-engineering bake-offs. Note that the
    same witness appears on every hit AND in the system block, so any
    downstream auditor can verify the exact snapshot the model was
    grounded on.
    """
    hits = list(hits)
    if not hits:
        witness = "<no-hits>"
    else:
        witness = hits[0].provenance_id
    label = corpus_label or "ruLake-snapshot"

    lines: list[str] = []
    lines.append("[SYSTEM]")
    lines.append(f"Source: {label}")
    lines.append(f"Provenance witness: {witness}")
    lines.append("Use only the retrieved context below. Cite each fact by [id].")
    lines.append("")
    lines.append("[CONTEXT]")
    if not hits:
        lines.append("(no hits)")
    else:
        for h in hits:
            lines.append(
                f"  [id={h.id}]  score={h.score:.4f}  "
                f"provenance={h.provenance_id[:12]}…"
            )
    lines.append("")
    lines.append("[QUERY]")
    lines.append(query)
    lines.append("")
    lines.append("[RESPONSE]")
    return "\n".join(lines)


# --------------------------------------------------------------------------
# Top-level pipeline
# --------------------------------------------------------------------------

def run_pipeline(bundle_dir: Path, query: str, k: int = 5, *,
                 use_sentence_transformers: bool = False,
                 data_filename: Optional[str] = None) -> PipelineResult:
    """End-to-end: verify, load, embed, search, prompt."""
    corpus = load_corpus(bundle_dir, data_filename=data_filename)
    query_vec = embed_query(query, corpus.dim,
                            use_sentence_transformers=use_sentence_transformers)
    hits = brute_force_topk(corpus, query_vec, k)
    prompt = build_prompt(query, hits, corpus_label=corpus.bundle.data_ref)
    return PipelineResult(
        query_text=query,
        hits=hits,
        prompt=prompt,
        bundle=corpus.bundle,
        corpus_size=len(corpus.ids),
    )


# --------------------------------------------------------------------------
# Demo helper: build a witnessed snapshot from scratch
# --------------------------------------------------------------------------

def materialize_demo_snapshot(target_dir: Path, *,
                              dim: int = 16, n: int = 50,
                              rotation_seed: int = 42,
                              rerank_factor: int = 20,
                              generation: int = 1) -> Path:
    """Write a tiny ruvec1 corpus + matching witnessed bundle into ``target_dir``.

    Uses the SAME on-disk format that ``FsBackend`` writes in Rust, and
    the SAME bundle witness algorithm — so a Rust ruLake reader could
    consume the resulting directory unchanged.
    """
    target_dir.mkdir(parents=True, exist_ok=True)
    data_path = target_dir / "vectors.ruvec1"

    # Deterministic, vaguely-clustered f32s — the content is irrelevant.
    rng_state = hashlib.shake_256(b"rulake-demo-corpus").digest(n * dim * 4)
    vectors: list[list[float]] = []
    ids = list(range(1000, 1000 + n))
    for i in range(n):
        v: list[float] = []
        base = i % 5  # 5 "clusters"
        for j in range(dim):
            byte_off = (i * dim + j) * 4
            u = struct.unpack(
                "<I", rng_state[byte_off:byte_off + 4]
            )[0]
            v.append(base + (u / (1 << 32)) * 0.3 - 0.15)
        vectors.append(v)
    write_ruvec1(data_path, dim, ids, vectors)

    data_ref = f"file://{data_path}"
    witness = compute_witness(
        data_ref, dim, rotation_seed, rerank_factor, generation,
    )
    bundle = {
        "format_version": 2,
        "data_ref": data_ref,
        "dim": dim,
        "rotation_seed": rotation_seed,
        "rerank_factor": rerank_factor,
        "generation": generation,
        "rvf_witness": witness,
        "pii_policy": None,
        "lineage_id": "ol://rag-demo/python",
    }
    import json
    (target_dir / SIDECAR_FILENAME).write_text(
        json.dumps(bundle, indent=2), encoding="utf-8",
    )
    return target_dir


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------

def main(argv: Optional[Iterable[str]] = None) -> int:
    p = argparse.ArgumentParser(
        description="Witness-pinned RAG pipeline against a ruLake snapshot.",
    )
    p.add_argument("--snapshot", type=Path, default=None,
                   help="snapshot directory (created with --materialize-demo "
                        "if absent)")
    p.add_argument("--query", default="how do warm caches survive a restart?",
                   help="query text")
    p.add_argument("--k", type=int, default=5, help="top-K (default: 5)")
    p.add_argument("--data-filename", default=None,
                   help="explicit corpus filename inside the snapshot dir")
    p.add_argument("--materialize-demo", action="store_true",
                   help="write a tiny demo snapshot to --snapshot if it's empty")
    p.add_argument("--use-sentence-transformers", action="store_true",
                   help="try sentence-transformers if installed")
    args = p.parse_args(list(argv) if argv is not None else None)

    snapshot = args.snapshot or Path("./demo-snapshot")
    if args.materialize_demo:
        if not (snapshot / SIDECAR_FILENAME).exists():
            materialize_demo_snapshot(snapshot)
            print(f"materialized demo snapshot at {snapshot}")
    if not (snapshot / SIDECAR_FILENAME).exists():
        print(
            f"snapshot not found at {snapshot}; pass --materialize-demo "
            f"or point --snapshot at a ruLake-published directory.",
            file=sys.stderr,
        )
        return 2

    try:
        result = run_pipeline(
            snapshot, args.query, k=args.k,
            use_sentence_transformers=args.use_sentence_transformers,
            data_filename=args.data_filename,
        )
    except (CorpusError, BundleError) as e:
        print(f"pipeline: {e}", file=sys.stderr)
        return 1

    print(f"snapshot: {snapshot}")
    print(f"corpus size: {result.corpus_size}")
    print(f"witness: {result.bundle.rvf_witness}")
    print(f"data_ref: {result.bundle.data_ref}")
    print()
    print(f"top-{len(result.hits)} hits for: {result.query_text!r}")
    for h in result.hits:
        print(f"  id={h.id:>5d}  l2_sq={h.score:>10.4f}  "
              f"provenance={h.provenance_id[:12]}…")
    print()
    print(result.prompt)
    return 0


if __name__ == "__main__":
    sys.exit(main())
