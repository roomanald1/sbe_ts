#!/usr/bin/env python3
from common import (
    parser, load_rows, require, number, percentile, emit
)
from collections import defaultdict

p = parser("Calculate measured processing latency percentiles")
a = p.parse_args()
rows = load_rows(a.input, a.max_rows)

require(rows, ["stage", "latency_ms"])
groups = defaultdict(list)

for r in rows:
    latency = number(r["latency_ms"])
    if latency < 0:
        raise SystemExit("BLOCKED: negative latency")
    groups[r["stage"]].append(latency)

results = []

for stage, values in groups.items():
    results.append({
        "stage": stage,
        "samples": len(values),
        "p50_ms": percentile(values, 50),
        "p95_ms": percentile(values, 95),
        "p99_ms": percentile(values, 99),
        "p99_9_ms": percentile(values, 99.9),
        "max_ms": max(values),
        "p99_9_low_sample_warning": len(values) < 1000
    })

results.sort(key=lambda r: -r["p99_ms"])

emit(a, {
    "samples": len(rows),
    "stages": len(results),
    "results": results[:a.limit],
    "note": "Keep exchange/network latency separate from "
            "internal processing latency."
}, results)
