#!/usr/bin/env python3
from common import parser, load_rows, require, decimal, emit

p = parser("Inspect authoritative capital snapshots")
a = p.parse_args()
rows = load_rows(a.input, a.max_rows)

require(rows, [
    "scope", "currency", "mode", "available_capital",
    "reserved_capital", "deployed_capital"
])

results = []

for r in rows:
    available = decimal(r["available_capital"])
    reserved = decimal(r["reserved_capital"])
    deployed = decimal(r["deployed_capital"])
    if min(available, reserved, deployed) < 0:
        raise SystemExit("BLOCKED: negative capital value")
    results.append({
        **r,
        "reserved_ratio": str(reserved/available)
            if available else None,
        "deployed_ratio": str(deployed/available)
            if available else None,
    })

emit(a, {
    "snapshots": len(results),
    "results": results[:a.limit],
    "note": "Ratios only. Reservation/deployment overlap "
            "and capital conservation require contract validation."
}, results)
