#!/usr/bin/env python3
"""
Extract benchmark results from cargo bench output.

Supports three criterion output formats:
  1. criterion default: "group/bench   time:   [1.2340 ms 1.2567 ms 1.2890 ms]"
  2. criterion bencher: "bench_name\t12345\t678"  (name, mean_ns, stddev_ns, tab-separated)
  3. libtest bench:     "test bench_name ... bench:   1,234 ns/iter (+/- 99)"

Outputs JSON: {"version":1, "commit":"<sha>", "results":{name:{mean_ns:int, stddev_ns:int}}}
"""
import sys
import json
import re
from pathlib import Path


def _unit_to_ns(mult: float, unit: str) -> float:
    """Convert a value in the given unit to nanoseconds."""
    factors = {"ns": 1, "us": 1e3, "µs": 1e3, "ms": 1e6, "s": 1e9}
    return mult * factors.get(unit, 1)


def parse_criterion_default(text: str) -> dict:
    """Parse criterion's default output format (with time ranges)."""
    results = {}
    pattern = re.compile(
        r'^(\S+)\s+time:\s+\[[\d.]+\s+(\w+)\s+'
        r'(?P<mean>[\d.]+)\s+(?P<unit>\w+)\s+'
        r'[\d.]+\s+\w+\]',
        re.MULTILINE,
    )
    for m in pattern.finditer(text):
        name = m.group(1)
        mean_val = float(m.group("mean"))
        unit = m.group("unit")
        results[name] = {"mean_ns": int(_unit_to_ns(mean_val, unit)), "stddev_ns": 0}
    return results


def parse_criterion_bencher(text: str) -> dict:
    """Parse criterion --output-format bencher (tab-separated)."""
    results = {}
    for line in text.splitlines():
        parts = line.strip().split("\t")
        if len(parts) >= 2:
            try:
                name = parts[0]
                mean_ns = int(float(parts[1]))
                stddev_ns = int(float(parts[2])) if len(parts) >= 3 else 0
                results[name] = {"mean_ns": mean_ns, "stddev_ns": stddev_ns}
            except (ValueError, IndexError):
                continue
    return results


def parse_libtest_bench(text: str) -> dict:
    """Parse libtest benchmark output (rustc built-in)."""
    results = {}
    pattern = re.compile(
        r'test\s+(?P<name>\S+)\s+.*bench:\s+(?P<mean>[\d,]+)\s+ns/iter'
    )
    for m in pattern.finditer(text):
        name = m.group("name")
        mean_ns = int(m.group("mean").replace(",", ""))
        results[name] = {"mean_ns": mean_ns, "stddev_ns": 0}
    return results


def parse_benchmark_output(text: str) -> dict:
    """Try all parsers, return the one that found results."""
    for parser in (
        parse_criterion_default,
        parse_criterion_bencher,
        parse_libtest_bench,
    ):
        results = parser(text)
        if results:
            return results
    return {}


def main():
    text = sys.stdin.read()
    results = parse_benchmark_output(text)
    if not results:
        print(json.dumps({"error": "No benchmark results parsed"}), file=sys.stderr)
        sys.exit(1)

    commit = "unknown"
    head = Path(".git/refs/heads/main")
    if head.exists():
        commit = head.read_text().strip()[:12]
    elif Path(".git/HEAD").exists():
        commit = "detached"

    output = {
        "version": 1,
        "commit": commit,
        "results": results,
    }
    json.dump(output, sys.stdout, indent=2)
    print()


if __name__ == "__main__":
    main()
