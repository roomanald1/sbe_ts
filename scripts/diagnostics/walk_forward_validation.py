#!/usr/bin/env python3
from common import parser, load_rows, require, number, emit
from datetime import datetime

p = parser("Inspect walk-forward evidence metadata")
a = p.parse_args()
rows = load_rows(a.input, a.max_rows)

require(rows, [
    "strategy", "symbol", "train_end", "validation_start",
    "validation_end", "effective_samples", "net_return"
])

def timestamp(value):
    t = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    if t.tzinfo is None:
        raise ValueError("Timezone required")
    return t

issues = []

for r in rows:
    train = timestamp(r["train_end"])
    start = timestamp(r["validation_start"])
    end = timestamp(r["validation_end"])
    reasons = []
    if train >= start:
        reasons.append("training_validation_overlap")
    if end <= start:
        reasons.append("invalid_validation_window")
    if number(r["effective_samples"]) <= 0:
        reasons.append("no_effective_samples")
    number(r["net_return"])
    if reasons:
        issues.append({**r, "issues": reasons})

emit(a, {
    "windows": len(rows),
    "flagged_windows": len(issues),
    "examples": issues[:a.limit],
    "note": "Metadata checks only; cannot establish absence "
            "of feature leakage or statistical independence."
}, issues)
