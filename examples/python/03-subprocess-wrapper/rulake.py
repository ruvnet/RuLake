"""Python wrapper around the ``rulake-demo`` Rust binary.

There are no pyo3 / maturin bindings to ruLake — this module instead
spawns the existing ``rulake-demo`` benchmark binary as a child process,
parses its stdout, and exposes the numbers as typed Python objects.

This is the lightweight orchestration story for Python services that
want to invoke ruLake operations (benchmarks, smoke tests, capacity
checks) without taking on the Rust toolchain in their own repo.

Public surface
--------------

* :class:`Rulake` — the wrapper. Methods :meth:`Rulake.benchmark` and
  :meth:`Rulake.version`.
* :class:`BenchmarkRow` / :class:`BenchmarkReport` — typed parsed output.
* :class:`RulakeError` — raised on locate / spawn / parse failures.
* :func:`main` — a thin CLI that prints a formatted report.
"""

from __future__ import annotations

import os
import re
import shutil
import signal
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, Optional

DEFAULT_BIN_NAME = "rulake-demo"
DEFAULT_TIMEOUT_SEC = 600.0  # 10 min — the slow path covers n=100k with batches.


class RulakeError(RuntimeError):
    """Raised for any failure in locating, spawning, or parsing rulake-demo."""


# --------------------------------------------------------------------------
# Parse model
# --------------------------------------------------------------------------

@dataclass(frozen=True)
class BenchmarkRow:
    """One measurement row from the benchmark harness.

    Mirror of the lines printed by ``rulake-demo``. Not every row carries
    every metric — the ``rulake (Fresh)`` row has a tax, the direct
    rows do not, and so on. Missing metrics are :data:`None`.
    """

    n: int
    label: str
    qps: float
    build_ms: Optional[float] = None
    prime_ms: Optional[float] = None
    tax: Optional[float] = None
    speedup: Optional[float] = None


@dataclass(frozen=True)
class BenchmarkReport:
    """Parsed output of a full ``rulake-demo`` run."""

    fast: bool
    rows: list[BenchmarkRow] = field(default_factory=list)
    raw_stdout: str = ""

    def for_n(self, n: int) -> list[BenchmarkRow]:
        return [r for r in self.rows if r.n == n]

    def find(self, n: int, label_substr: str) -> Optional[BenchmarkRow]:
        for r in self.rows:
            if r.n == n and label_substr in r.label:
                return r
        return None


# --------------------------------------------------------------------------
# Stdout parser
# --------------------------------------------------------------------------

_RE_N_HEADER = re.compile(r"──\s*n\s*=\s*([\d_]+)\s*──")
_RE_NUM = r"([0-9]+(?:\.[0-9]+)?)"
_RE_BUILD = re.compile(rf"build=\s*{_RE_NUM}\s*ms")
_RE_PRIME = re.compile(rf"prime=\s*{_RE_NUM}\s*ms")
_RE_QPS = re.compile(rf"qps=\s*{_RE_NUM}")
_RE_TAX = re.compile(rf"tax=\s*{_RE_NUM}×")
_RE_SPEEDUP = re.compile(rf"build_speedup=\s*{_RE_NUM}×")
# A measurement line starts with two leading spaces and a non-space
# label character, then runs up to the first metric (build=/prime=/qps=).
# We can't require >=2 spaces before the metric — the harness sometimes
# emits exactly one (e.g. the Hadamard row).
_RE_METRIC_START = re.compile(r"\s+(?:build|prime|qps|build_speedup|tax)=")
_RE_LABEL_PREFIX = re.compile(r"^\s{2}([A-Za-z][^\n]*?)(?=\s+(?:build|prime|qps|build_speedup|tax)=)")


def _parse_int_with_underscores(s: str) -> int:
    return int(s.replace("_", ""))


def parse_benchmark(stdout: str) -> list[BenchmarkRow]:
    """Walk the harness stdout and return one :class:`BenchmarkRow` per line.

    Robust to:

    - rows that have only ``qps`` (the ``per-query loop`` baseline);
    - rows that mix ``prime`` + ``qps`` + ``tax`` (the cache-hit rows);
    - rows that have only ``build`` + ``qps`` (the direct rows);
    - underscore-separated row counts in the section headers.
    """
    rows: list[BenchmarkRow] = []
    current_n: Optional[int] = None

    for line in stdout.splitlines():
        m = _RE_N_HEADER.search(line)
        if m:
            current_n = _parse_int_with_underscores(m.group(1))
            continue
        if current_n is None:
            continue
        # A measurement row always has at least one ``qps=``; skip
        # narrative lines.
        m_qps = _RE_QPS.search(line)
        if not m_qps:
            continue
        m_label = _RE_LABEL_PREFIX.match(line)
        if not m_label:
            continue
        label = m_label.group(1).strip()
        try:
            qps = float(m_qps.group(1))
            build = (float(_RE_BUILD.search(line).group(1))   # type: ignore[union-attr]
                     if _RE_BUILD.search(line) else None)
            prime = (float(_RE_PRIME.search(line).group(1))   # type: ignore[union-attr]
                     if _RE_PRIME.search(line) else None)
            tax = (float(_RE_TAX.search(line).group(1))       # type: ignore[union-attr]
                   if _RE_TAX.search(line) else None)
            speedup = (float(_RE_SPEEDUP.search(line).group(1))   # type: ignore[union-attr]
                       if _RE_SPEEDUP.search(line) else None)
        except (ValueError, AttributeError) as e:
            raise RulakeError(f"benchmark parse: malformed metrics in {line!r}: {e}")

        rows.append(BenchmarkRow(
            n=current_n,
            label=label,
            qps=qps,
            build_ms=build,
            prime_ms=prime,
            tax=tax,
            speedup=speedup,
        ))
    if not rows:
        raise RulakeError(
            "benchmark parse: no rows extracted from stdout; the binary may "
            "have failed silently or its output format changed."
        )
    return rows


# --------------------------------------------------------------------------
# Locator + version readers
# --------------------------------------------------------------------------

def _locate_cargo_root(start: Path) -> Optional[Path]:
    """Walk up from ``start`` looking for the parent crate's Cargo.toml."""
    cur = start.resolve()
    for _ in range(10):
        cargo = cur / "Cargo.toml"
        if cargo.is_file():
            text = cargo.read_text(encoding="utf-8", errors="replace")
            if "rulake" in text:
                return cur
        if cur.parent == cur:
            return None
        cur = cur.parent
    return None


def _find_prebuilt_binary(cargo_root: Optional[Path]) -> Optional[Path]:
    """Look for a pre-built ``rulake-demo`` in `target/release/`."""
    if cargo_root is None:
        return None
    for candidate in (
        cargo_root / "target" / "release" / DEFAULT_BIN_NAME,
        cargo_root / "target" / "debug" / DEFAULT_BIN_NAME,
    ):
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return candidate
    return None


def _read_version_from_cargo(cargo_root: Path) -> str:
    cargo = cargo_root / "Cargo.toml"
    text = cargo.read_text(encoding="utf-8", errors="replace")
    in_pkg = False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("[package]"):
            in_pkg = True
            continue
        if in_pkg and s.startswith("[") and not s.startswith("[package]"):
            in_pkg = False
            continue
        if in_pkg and s.startswith("version"):
            # version = "2.2.0"
            m = re.search(r'version\s*=\s*"([^"]+)"', s)
            if m:
                return m.group(1)
    raise RulakeError(f"could not read version from {cargo}")


# --------------------------------------------------------------------------
# Main wrapper
# --------------------------------------------------------------------------

class Rulake:
    """Spawn-and-parse wrapper around the ``rulake-demo`` Rust binary.

    Resolution order for the binary:

    1. ``binary`` argument to the constructor, if provided;
    2. ``RULAKE_DEMO_BINARY`` env var, if set;
    3. a pre-built ``target/release/rulake-demo`` in the parent crate;
    4. a pre-built ``target/debug/rulake-demo`` in the parent crate;
    5. ``cargo run --release --bin rulake-demo`` from the parent crate.

    Set ``cargo_root`` explicitly if your layout doesn't put this
    package inside the ruLake source tree.
    """

    def __init__(
        self,
        *,
        binary: Optional[Path] = None,
        cargo_root: Optional[Path] = None,
        cargo_executable: str = "cargo",
        timeout_sec: float = DEFAULT_TIMEOUT_SEC,
    ) -> None:
        self._cargo_root = cargo_root or _locate_cargo_root(Path(__file__).parent)
        env_bin = os.environ.get("RULAKE_DEMO_BINARY")
        candidate = binary or (Path(env_bin) if env_bin else None)
        if candidate is None:
            candidate = _find_prebuilt_binary(self._cargo_root)
        self._binary: Optional[Path] = candidate
        self._cargo = cargo_executable
        self._timeout_sec = timeout_sec

    # -- private ---------------------------------------------------------

    def _argv(self, fast: bool) -> tuple[list[str], dict[str, str], Optional[Path]]:
        args = ["--fast"] if fast else []
        if self._binary is not None:
            return [str(self._binary), *args], dict(os.environ), None
        if self._cargo_root is None:
            raise RulakeError(
                "rulake-demo binary not found and no Cargo.toml resolvable; "
                "set RULAKE_DEMO_BINARY or pass binary= explicitly."
            )
        if shutil.which(self._cargo) is None:
            raise RulakeError(
                f"`{self._cargo}` not in PATH and no pre-built rulake-demo "
                f"found at {self._cargo_root / 'target'}; build it once "
                f"with `cargo build --release --bin rulake-demo`."
            )
        return (
            [self._cargo, "run", "--release", "--bin", DEFAULT_BIN_NAME, "--quiet", "--", *args],
            dict(os.environ),
            self._cargo_root,
        )

    # -- public ----------------------------------------------------------

    def binary_path(self) -> Optional[Path]:
        """Resolved binary path, or :data:`None` if we'll fall back to cargo."""
        return self._binary

    def version(self) -> str:
        """Read the parent crate's ``Cargo.toml`` ``[package].version``."""
        if self._cargo_root is None:
            raise RulakeError(
                "version() requires the parent crate's Cargo.toml to be reachable"
            )
        return _read_version_from_cargo(self._cargo_root)

    def benchmark(self, *, fast: bool = True) -> BenchmarkReport:
        """Run ``rulake-demo`` and return the parsed report.

        ``fast=True`` runs the ``--fast`` smoke profile (a few seconds
        on a workstation). ``fast=False`` runs the full sweep; budget
        accordingly via ``timeout_sec`` on construction.
        """
        argv, env, cwd = self._argv(fast)
        try:
            proc = subprocess.run(
                argv,
                capture_output=True,
                text=True,
                check=False,
                env=env,
                cwd=str(cwd) if cwd else None,
                timeout=self._timeout_sec,
                # New process group so SIGINT / timeout-kill takes the
                # whole tree down with us, including a stuck `cargo run`.
                start_new_session=True,
            )
        except subprocess.TimeoutExpired as e:
            raise RulakeError(
                f"rulake-demo exceeded {self._timeout_sec:.0f}s; "
                f"argv={argv!r}; partial stdout={(e.stdout or '')[:200]!r}"
            ) from e
        except FileNotFoundError as e:
            raise RulakeError(f"could not spawn {argv[0]!r}: {e}") from e

        if proc.returncode != 0:
            tail = (proc.stderr or "")[-2000:]
            raise RulakeError(
                f"rulake-demo exited {proc.returncode}: {tail!r}"
            )

        rows = parse_benchmark(proc.stdout)
        return BenchmarkReport(fast=fast, rows=rows, raw_stdout=proc.stdout)


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------

def _format_report(report: BenchmarkReport) -> str:
    lines: list[str] = []
    lines.append("=" * 78)
    lines.append(f"  ruLake benchmark report  ({'--fast' if report.fast else 'full sweep'})")
    lines.append("=" * 78)
    by_n: dict[int, list[BenchmarkRow]] = {}
    for r in report.rows:
        by_n.setdefault(r.n, []).append(r)
    for n in sorted(by_n.keys()):
        lines.append(f"\n  n = {n:>10,}")
        lines.append("  " + "-" * 74)
        lines.append(f"  {'label':40s} {'qps':>10s} {'prime/build ms':>16s} {'tax':>6s}")
        for r in by_n[n]:
            timing = ""
            if r.prime_ms is not None:
                timing = f"{r.prime_ms:>10.2f} prime"
            elif r.build_ms is not None:
                timing = f"{r.build_ms:>10.2f} build"
            tax = f"{r.tax:.2f}×" if r.tax is not None else ""
            lines.append(f"  {r.label[:40]:40s} {r.qps:>10.0f} {timing:>16s} {tax:>6s}")
    lines.append("")
    return "\n".join(lines)


def main(argv: Optional[Iterable[str]] = None) -> int:
    import argparse

    p = argparse.ArgumentParser(description="Run rulake-demo and print a parsed report.")
    p.add_argument("--full", action="store_true", help="run the full sweep (slow)")
    p.add_argument("--binary", type=Path, default=None,
                   help="path to a pre-built rulake-demo binary")
    p.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT_SEC,
                   help="wall-clock timeout in seconds (default: 600)")
    p.add_argument("--raw", action="store_true",
                   help="print the binary's raw stdout instead of the parsed table")
    args = p.parse_args(list(argv) if argv is not None else None)

    try:
        wrapper = Rulake(binary=args.binary, timeout_sec=args.timeout)
        report = wrapper.benchmark(fast=not args.full)
    except RulakeError as e:
        print(f"rulake-wrapper: {e}", file=sys.stderr)
        return 1

    if args.raw:
        print(report.raw_stdout, end="")
    else:
        try:
            print(f"  ruLake crate version: {wrapper.version()}")
        except RulakeError:
            print("  ruLake crate version: <unknown>")
        print(_format_report(report))
    return 0


if __name__ == "__main__":
    # Make Ctrl-C take down the child too.
    signal.signal(signal.SIGINT, lambda *_: sys.exit(130))
    sys.exit(main())
