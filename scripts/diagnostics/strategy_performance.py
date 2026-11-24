#!/usr/bin/env python3
from common import (
    parser, load_rows, require, number, pnl_stats, emit
)
from collections import defaultdict

p = parser("Summarise realised strategy outcomes")
a = p.parse_args()
rows = load_rows(a.input, a.max_rows)

require(rows, [
    "trade_id", "strategy", "net_pnl", "currency", "mode"
])

seen = set()
groups = defaultdict(list)

for r in rows:
    key = (r["mode"], r["trade_id"])
    if key in seen:
        raise SystemExit("BLOCKED: duplicate trade IDs")
    seen.add(key)
    groups[(r["strategy"], r["currency"], r["mode"])].append(
        number(r["net_pnl"])
    )

results = [
    {"strategy": k[0], "currency": k[1], "mode": k[2],
     **pnl_stats(v)}
    for k, v in groups.items()
]
results.sort(key=lambda r: (
    r["currency"], r["mode"], -r["count"]
))

emit(a, {
    "trades": len(rows),
    "groups": len(results),
    "results": results[:a.limit],
    "note": "Descriptive statistics; not independent evidence "
            "or predictive profitability"
}, results)
