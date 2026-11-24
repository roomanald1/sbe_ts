#!/usr/bin/env python3
from common import parser, load_rows, require, number, emit
from pathlib import Path

p = parser("Compare baseline and candidate paper metrics")
p.add_argument("--baseline", type=Path, required=True)
a = p.parse_args()

baseline = load_rows(a.baseline, a.max_rows)
candidate = load_rows(a.input, a.max_rows)

fields = [
    "metric", "value", "unit", "mode", "scope",
    "measurement_definition"
]

def index(rows):
    require(rows, fields)
    result = {}
    for r in rows:
        if str(r["mode"]).lower() != "paper":
            raise SystemExit("BLOCKED: non-paper data")
        key = (
            r["metric"], r["unit"], r["scope"],
            r["measurement_definition"]
        )
        if key in result:
            raise SystemExit("BLOCKED: duplicate metric")
        result[key] = number(r["value"])
    return result

b = index(baseline)
c = index(candidate)
results = []

for key in sorted(b.keys() & c.keys()):
    results.append({
        "metric": key[0],
        "unit": key[1],
        "scope": key[2],
        "baseline": b[key],
        "candidate": c[key],
        "delta": c[key]-b[key],
        "relative_change": (c[key]-b[key])/abs(b[key])
            if b[key] else None
    })

emit(a, {
    "matched_metrics": len(results),
    "missing_candidate": len(b.keys()-c.keys()),
    "missing_baseline": len(c.keys()-b.keys()),
    "results": results[:a.limit],
    "status": "COMPARISON_ONLY",
    "note": "No automatic promotion. Comparable market regimes, "
            "sample support and acceptance gates require validation."
}, results)
