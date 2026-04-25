"""End-to-end demo: spawn rulake-demo, parse it, render an ASCII bar chart.

No third-party plotting deps — just stdlib. Designed to be the first
script someone runs against the wrapper to satisfy themselves that the
end-to-end Python -> Rust hop is working.
"""

from __future__ import annotations

import sys

from rulake import BenchmarkReport, Rulake, RulakeError, _format_report


def _qps_bar(qps: float, max_qps: float, width: int = 40) -> str:
    if max_qps <= 0:
        return ""
    filled = int(round(width * qps / max_qps))
    filled = max(0, min(width, filled))
    return "█" * filled + "·" * (width - filled)


def _print_chart(report: BenchmarkReport) -> None:
    print()
    print("  QPS comparison (per-row, scaled to peak)")
    print("  " + "-" * 78)
    max_qps = max((r.qps for r in report.rows), default=1.0)
    for r in report.rows:
        bar = _qps_bar(r.qps, max_qps)
        print(f"  n={r.n:>7,} {r.label[:30]:30s} {bar} {r.qps:>9.0f}")


def main() -> int:
    try:
        wrapper = Rulake()
    except RulakeError as e:
        print(f"demo: {e}", file=sys.stderr)
        return 1

    print(f"  ruLake crate version: {wrapper.version()}")
    print(f"  binary: {wrapper.binary_path() or '<falling back to cargo run>'}")
    print()
    print("  Running --fast benchmark...")
    try:
        report = wrapper.benchmark(fast=True)
    except RulakeError as e:
        print(f"demo: {e}", file=sys.stderr)
        return 1

    print(_format_report(report))
    _print_chart(report)
    return 0


if __name__ == "__main__":
    sys.exit(main())
