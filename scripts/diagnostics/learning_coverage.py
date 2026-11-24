#!/usr/bin/env python3
from common import parser, load_rows, require, number, emit

p = parser("Report learning evidence gaps")
a = p.parse_args()
rows = load_rows(a.input, a.max_rows)

require(rows, [
    "strategy", "symbol", "effective_samples",
    "required_effective_samples", "status"
])

gaps = []

for r in rows:
    actual = number(r["effective_samples"])
    required = number(r["required_effective_samples"])
    if actual < 0 or required <= 0:
        raise SystemExit("BLOCKED: invalid sample counts")
    if actual < required or r["status"] != "ready":
        gaps.append({
            **r,
            "sample_deficit": max(0, required-actual),
            "coverage": min(1, actual/required)
        })

gaps.sort(key=lambda r: r["coverage"])

emit(a, {
    "profiles": len(rows),
    "profiles_with_gaps": len(gaps),
    "ready": len(rows)-len(gaps),
    "gaps": gaps[:a.limit]
}, gaps)
