#!/usr/bin/env python3
from common import parser, load_rows, require, decimal, emit
from collections import Counter

p = parser("Compare authoritative trade P&L values")
p.add_argument("--tolerance", required=True,
               help="Verified asset-specific rounding tolerance")
a = p.parse_args()
rows = load_rows(a.input, a.max_rows)

require(rows, [
    "trade_id", "ledger_net_pnl", "reported_net_pnl",
    "currency", "mode"
])

tolerance = decimal(a.tolerance)
if tolerance < 0:
    raise SystemExit("Tolerance must be non-negative")

scopes = {(r["currency"], r["mode"]) for r in rows}
if len(scopes) > 1:
    raise SystemExit("BLOCKED: mixed currency or execution mode")

ids = Counter(r["trade_id"] for r in rows)
duplicates = [k for k, v in ids.items() if v > 1]
errors = []
ledger = reported = decimal(0)

for r in rows:
    l = decimal(r["ledger_net_pnl"])
    v = decimal(r["reported_net_pnl"])
    ledger += l
    reported += v
    if abs(l-v) > tolerance:
        errors.append({
            "trade_id": r["trade_id"],
            "difference": str(l-v)
        })

emit(a, {
    "status": (
        "NO_DATA" if not rows else
        "FAIL" if errors or duplicates or
        abs(ledger-reported) > tolerance else "PASS"
    ),
    "trades": len(rows),
    "mismatches": len(errors),
    "duplicate_ids": len(duplicates),
    "ledger_net_pnl": str(ledger),
    "reported_net_pnl": str(reported),
    "difference": str(ledger-reported),
    "scope": list(scopes),
    "sample_errors": errors[:a.limit],
    "scope_note": "P&L comparison only; not full ledger conservation"
}, {"mismatches": errors, "duplicates": duplicates})
