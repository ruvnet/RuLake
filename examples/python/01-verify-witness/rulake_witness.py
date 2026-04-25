"""Pure-Python implementation of the ruLake bundle witness.

Re-derives the SHAKE-256(32) ``rvf_witness`` of a ``table.rulake.json``
bundle from its other fields. The output of :func:`compute_witness` is
guaranteed to byte-exactly match what the Rust crate's
``ruvector_rulake::bundle::compute_witness`` produces (see
``src/bundle.rs`` in the parent repo).

Algorithm (verbatim, witness format v1, current as of bundle
``format_version=2``)::

    SHAKE-256(32) of the byte stream:
        b"rulake-bundle-witness-v1|"
        || u64_le(len(data_ref))
        || data_ref bytes
        || b"|"
        || u64_le(dim)
        || u64_le(rotation_seed)
        || u64_le(rerank_factor)
        || b"|"
        || u64_le(len(generation_bytes))
        || generation_bytes

Where ``generation_bytes`` is a tagged byte stream:

* For ``Num(n)``  — byte ``0x00`` followed by ``u64_le(n)``  (9 bytes).
* For ``Opaque(s)`` — byte ``0x01`` followed by UTF-8 bytes of ``s``.

The leading tag byte is the audit-driven fix (2026-04-23) preventing
the ``Num(7)`` vs ``Opaque("\\x07\\0\\0\\0\\0\\0\\0\\0")`` collision.
"""

from __future__ import annotations

import hashlib
import json
import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Union

#: Canonical filename of the on-disk bundle sidecar.
SIDECAR_FILENAME = "table.rulake.json"

#: Newest ``format_version`` this implementation understands.
SUPPORTED_FORMAT_VERSION = 2

#: Hard cap on bundle JSON size (mirror of the Rust DoS guard).
MAX_JSON_BYTES = 64 * 1024

#: Hard cap on individual string fields (mirror of the Rust DoS guard).
MAX_FIELD_BYTES = 4 * 1024

#: Witness is a hex-encoded SHAKE-256(32), so exactly 64 hex chars.
WITNESS_HEX_LEN = 64

#: A ruLake ``generation`` token is either a ``u64`` (numeric) or an
#: arbitrary string (opaque). Match the Rust untagged enum.
Generation = Union[int, str]


class BundleError(ValueError):
    """Raised for any bundle parse / shape / size failure."""


@dataclass(frozen=True)
class RuLakeBundle:
    """In-memory mirror of the on-disk ``table.rulake.json``.

    Only the witness-relevant fields are first-class. ``pii_policy``,
    ``lineage_id``, and ``memory_class`` are passed through verbatim
    via :attr:`extra` for round-tripping but never enter the digest.
    """

    format_version: int
    data_ref: str
    dim: int
    rotation_seed: int
    rerank_factor: int
    generation: Generation
    rvf_witness: str
    extra: dict[str, Any]

    def verify_witness(self) -> bool:
        """Recompute the witness and compare to the on-disk value."""
        recomputed = compute_witness(
            self.data_ref,
            self.dim,
            self.rotation_seed,
            self.rerank_factor,
            self.generation,
        )
        return recomputed == self.rvf_witness


def _generation_hash_bytes(gen: Generation) -> bytes:
    """Return the tagged byte stream that goes into the witness digest.

    Mirrors ``Generation::hash_bytes`` in ``src/bundle.rs``.
    """
    if isinstance(gen, bool):
        # In Python, ``bool`` is a subclass of ``int``. A bare ``True``
        # would silently digest as ``Num(1)`` — almost certainly a
        # bug at the call site. Refuse rather than let it pass.
        raise TypeError("generation must be int or str, not bool")
    if isinstance(gen, int):
        if gen < 0 or gen > 0xFFFF_FFFF_FFFF_FFFF:
            raise ValueError(f"generation int out of u64 range: {gen}")
        return b"\x00" + struct.pack("<Q", gen)
    if isinstance(gen, str):
        return b"\x01" + gen.encode("utf-8")
    raise TypeError(
        f"generation must be int (Num) or str (Opaque), got {type(gen).__name__}"
    )


def compute_witness(
    data_ref: str,
    dim: int,
    rotation_seed: int,
    rerank_factor: int,
    generation: Generation,
) -> str:
    """Compute the SHAKE-256(32) witness for the given bundle fields.

    Returns 64 lowercase hex characters, byte-equal to what
    ``ruvector_rulake::bundle::compute_witness`` returns in Rust.
    """
    if not isinstance(data_ref, str):
        raise TypeError("data_ref must be str")
    for name, val in (("dim", dim), ("rotation_seed", rotation_seed),
                      ("rerank_factor", rerank_factor)):
        if not isinstance(val, int) or isinstance(val, bool):
            raise TypeError(f"{name} must be int")
        if val < 0 or val > 0xFFFF_FFFF_FFFF_FFFF:
            raise ValueError(f"{name} out of u64 range: {val}")

    h = hashlib.shake_256()
    h.update(b"rulake-bundle-witness-v1|")
    data_ref_bytes = data_ref.encode("utf-8")
    h.update(struct.pack("<Q", len(data_ref_bytes)))
    h.update(data_ref_bytes)
    h.update(b"|")
    h.update(struct.pack("<Q", dim))
    h.update(struct.pack("<Q", rotation_seed))
    h.update(struct.pack("<Q", rerank_factor))
    h.update(b"|")
    g = _generation_hash_bytes(generation)
    h.update(struct.pack("<Q", len(g)))
    h.update(g)
    return h.hexdigest(32)


def _parse_generation(raw: Any) -> Generation:
    """Lift the on-disk JSON shape of ``generation`` into a Python value.

    Rust serializes ``Generation`` with ``#[serde(untagged)]``, so on
    disk it is *either* a JSON number (Num) *or* a JSON string (Opaque).
    Booleans, lists, objects, and null are all rejected.
    """
    if isinstance(raw, bool):
        raise BundleError("generation must be number or string, not bool")
    if isinstance(raw, int):
        if raw < 0 or raw > 0xFFFF_FFFF_FFFF_FFFF:
            raise BundleError(f"generation int out of u64 range: {raw}")
        return raw
    if isinstance(raw, str):
        if len(raw.encode("utf-8")) > MAX_FIELD_BYTES:
            raise BundleError(
                f"generation Opaque exceeds {MAX_FIELD_BYTES} bytes"
            )
        return raw
    raise BundleError(
        f"generation must be number or string, got {type(raw).__name__}"
    )


def parse_bundle_json(text: str) -> RuLakeBundle:
    """Parse a ``table.rulake.json`` payload with full DoS guards.

    Mirrors the size/shape checks in
    ``RuLakeBundle::from_json`` (``src/bundle.rs``):

    * total payload at most :data:`MAX_JSON_BYTES`;
    * ``format_version`` not newer than :data:`SUPPORTED_FORMAT_VERSION`;
    * ``data_ref``, ``pii_policy``, ``lineage_id``, ``memory_class`` each
      at most :data:`MAX_FIELD_BYTES`;
    * ``rvf_witness`` length ≤ 128 (and exactly 64 hex chars in practice).

    Returns a populated :class:`RuLakeBundle`. The witness is *not*
    verified here — call :meth:`RuLakeBundle.verify_witness` for that.
    """
    if not isinstance(text, str):
        raise BundleError("bundle text must be str")
    if len(text.encode("utf-8")) > MAX_JSON_BYTES:
        raise BundleError(f"bundle exceeds {MAX_JSON_BYTES} bytes")

    try:
        raw = json.loads(text)
    except json.JSONDecodeError as e:
        raise BundleError(f"bundle parse: invalid JSON: {e}") from e
    if not isinstance(raw, dict):
        raise BundleError("bundle root must be a JSON object")

    required = ("format_version", "data_ref", "dim", "rotation_seed",
                "rerank_factor", "generation", "rvf_witness")
    missing = [k for k in required if k not in raw]
    if missing:
        raise BundleError(f"bundle missing fields: {missing}")

    format_version = raw["format_version"]
    if not isinstance(format_version, int) or isinstance(format_version, bool):
        raise BundleError("format_version must be int")
    if format_version > SUPPORTED_FORMAT_VERSION:
        raise BundleError(
            f"bundle format_version={format_version} newer than supported "
            f"({SUPPORTED_FORMAT_VERSION})"
        )

    data_ref = raw["data_ref"]
    if not isinstance(data_ref, str):
        raise BundleError("data_ref must be str")
    if len(data_ref.encode("utf-8")) > MAX_FIELD_BYTES:
        raise BundleError(f"data_ref exceeds {MAX_FIELD_BYTES} bytes")

    for name, val in (("dim", raw["dim"]),
                      ("rotation_seed", raw["rotation_seed"]),
                      ("rerank_factor", raw["rerank_factor"])):
        if not isinstance(val, int) or isinstance(val, bool):
            raise BundleError(f"{name} must be int")
        if val < 0 or val > 0xFFFF_FFFF_FFFF_FFFF:
            raise BundleError(f"{name} out of u64 range")

    generation = _parse_generation(raw["generation"])

    rvf_witness = raw["rvf_witness"]
    if not isinstance(rvf_witness, str):
        raise BundleError("rvf_witness must be str")
    if len(rvf_witness) > 128:
        raise BundleError(
            "rvf_witness not a hex-encoded SHAKE-256(32) (length > 128)"
        )

    extra: dict[str, Any] = {}
    for opt in ("pii_policy", "lineage_id", "memory_class"):
        if opt in raw and raw[opt] is not None:
            v = raw[opt]
            if not isinstance(v, str):
                raise BundleError(f"{opt} must be str or null")
            if len(v.encode("utf-8")) > MAX_FIELD_BYTES:
                raise BundleError(f"{opt} exceeds {MAX_FIELD_BYTES} bytes")
            extra[opt] = v
        elif opt in raw:
            extra[opt] = None

    return RuLakeBundle(
        format_version=format_version,
        data_ref=data_ref,
        dim=raw["dim"],
        rotation_seed=raw["rotation_seed"],
        rerank_factor=raw["rerank_factor"],
        generation=generation,
        rvf_witness=rvf_witness,
        extra=extra,
    )


def read_bundle(path: Union[str, Path]) -> RuLakeBundle:
    """Read and parse a ``table.rulake.json`` file from disk.

    Refuses to even ``read()`` files larger than :data:`MAX_JSON_BYTES`.
    """
    p = Path(path)
    size = p.stat().st_size
    if size > MAX_JSON_BYTES:
        raise BundleError(f"{p} is {size} bytes; cap is {MAX_JSON_BYTES}")
    return parse_bundle_json(p.read_text(encoding="utf-8"))


def read_bundle_from_dir(directory: Union[str, Path]) -> RuLakeBundle:
    """Read ``directory / table.rulake.json``."""
    return read_bundle(Path(directory) / SIDECAR_FILENAME)
