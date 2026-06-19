#!/usr/bin/env python3
"""
Compare current benchmark results against a stored baseline.

Usage:
  compare_bench.py --baseline baseline.json --current current.json --threshold 0.15

Exits 0 if all benchmarks are within threshold of baseline.
Exits 1 with a formatted table if any benchmark regressed beyond threshold.
"""
import json
import sys
import argparse
from pathlib import Path

def load(path):
    p = Path(path)
    if not p.exists():
        return None
    return json.loads(p.read_text())

def main():
    parser = argparse.ArgumentParser(description="Compare benchmark results against baseline")
    parser.add_argument("--baseline", required=True, help="Path to baseline JSON")
    parser.add_argument("--current", required=True, help="Path to current JSON (or - for stdin)")
    parser.add_argument("--threshold", type=float, default=0.15, help="Fractional tolerance (default 0.15 = 15%%)")
    args = parser.parse_args()

    baseline_data = load(args.baseline)
    if baseline_data is None:
        print(f"ERROR: baseline file not found: {args.baseline}", file=sys.stderr)
        sys.exit(2)

    if args.current == "-":
        current_data = json.load(sys.stdin)
    else:
        current_data = load(args.current)

    if current_data is None:
        print(f"ERROR: current file not found: {args.current}", file=sys.stderr)
        sys.exit(2)

    baseline = baseline_data.get("results", {})
    current = current_data.get("results", {})

    regressions = []
    for name, cdata in current.items():
        bdata = baseline.get(name)
        if bdata is None:
            regressions.append({
                "name": name,
                "status": "NEW",
                "baseline_ns": None,
                "current_ns": cdata["mean_ns"],
                "change": None,
            })
            continue

        b_ns = bdata["mean_ns"]
        c_ns = cdata["mean_ns"]
        if c_ns == 0:
            pct = float("inf")
        else:
            pct = (c_ns - b_ns) / b_ns

        status = "OK"
        if abs(pct) > args.threshold:
            status = "REGRESSED"
        regressions.append({
            "name": name,
            "status": status,
            "baseline_ns": b_ns,
            "current_ns": c_ns,
            "change": pct,
        })

    # Sort: regressions first, then OK
    regressions.sort(key=lambda r: (0 if r["status"] == "REGRESSED" else 1, r["name"]))

    # Print table
    print()
    print(f"{'Benchmark':<30} {'Status':<12} {'Baseline':>12} {'Current':>12} {'Change':>9}")
    print("-" * 80)
    for r in regressions:
        if r["baseline_ns"] is not None:
            b_str = f"{r['baseline_ns']/1e6:.4f}ms"
            c_str = f"{r['current_ns']/1e6:.4f}ms"
            pct_str = f"{r['change']*100:+.2f}%"
        else:
            b_str = "N/A"
            c_str = f"{r['current_ns']/1e6:.4f}ms"
            pct_str = "NEW"
        status_marker = "✓" if r["status"] == "OK" else "✗"
        print(f"{r['name']:<30} {status_marker} {r['status']:<10} {b_str:>12} {c_str:>12} {pct_str:>9}")
    print()

    failed = [r for r in regressions if r["status"] in ("REGRESSED", "NEW")]
    if failed:
        print(f"RESULT: {len(failed)} benchmark(s) outside {args.threshold*100:.0f}% threshold")
        print("Detected regressions:")
        for r in failed:
            if r["baseline_ns"] is not None:
                print(f"  - {r['name']}: {r['baseline_ns']/1e6:.4f}ms -> {r['current_ns']/1e6:.4f}ms ({r['change']*100:+.2f}%)")
            else:
                print(f"  - {r['name']}: NEW (current: {r['current_ns']/1e6:.4f}ms)")
        sys.exit(1)
    else:
        print(f"RESULT: All {len(regressions)} benchmarks within {args.threshold*100:.0f}% threshold")
        sys.exit(0)

if __name__ == "__main__":
    main()
