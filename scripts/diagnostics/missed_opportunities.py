#!/usr/bin/env python3
from common import (
    parser, load_rows, require, number, distribution, emit
)
from collections import defaultdict

p = parser("Analyse cost-adjusted rejected-signal outcomes")
a = p.parse_args()
rows = load_rows(a.input, a.max_rows)

require(rows, [
    "opportunity_id", "strategy", "rejection_reason",
    "horizon_seconds", "net_return", "mode"
])

groups = defaultdict(list)
seen = set()

for r in rows:
    identity = (
        r["mode"], r["opportunity_id"], r["horizon_seconds"]
    )
    if identity in seen:
        raise SystemExit("BLOCKED: duplicate opportunity/horizon")
    seen.add(identity)
    key = (
        r["strategy"], r["rejection_reason"],
        r["horizon_seconds"], r["mode"]
    )
    groups[key].append(number(r["net_return"]))

results = []

for key, values in groups.items():
    results.append({
        "strategy": key[0],
        "rejection_reason": key[1],
        "horizon_seconds": key[2],
        "mode": key[3],
        "positive": sum(v > 0 for v in values),
        "negative": sum(v < 0 for v in values),
        "neutral": sum(v == 0 for v in values),
        **distribution(values)
    })

results.sort(key=lambda r: -r["count"])

emit(a, {
    "observations": len(rows),
    "groups": len(results),
    "results": results[:a.limit],
    "note": "Counterfactual returns are not realised trades. "
            "Fill probability and overlapping evidence need validation."
}, results)
