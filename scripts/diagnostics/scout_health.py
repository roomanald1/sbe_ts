#!/usr/bin/env python3
from common import parser, load_rows, require, number, emit
from collections import Counter

p = parser("Summarise scout worker state")
p.add_argument("--stale-seconds", type=float, required=True)
a = p.parse_args()
rows = load_rows(a.input, a.max_rows)

require(rows, [
    "worker_id", "status", "seconds_since_progress",
    "pending_feedback"
])

states = Counter()
stale = []
pending = 0

for r in rows:
    states[r["status"]] += 1
    backlog = number(r["pending_feedback"])
    age = number(r["seconds_since_progress"])
    if backlog < 0 or age < 0:
        raise SystemExit("BLOCKED: invalid worker metrics")
    pending += backlog
    if r["status"] in ("active", "running") and \
       age > a.stale_seconds:
        stale.append(r)

emit(a, {
    "workers": len(rows),
    "states": dict(states),
    "pending_feedback": pending,
    "stale_workers": len(stale),
    "stale_examples": stale[:a.limit]
}, stale)
