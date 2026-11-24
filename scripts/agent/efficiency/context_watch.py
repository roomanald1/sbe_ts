import argparse
import json
from common import session_files, events

BUDGETS = {
    "coordinator": (80000, 131072),
    "architect": (60000, 98304),
    "quant": (60000, 98304),
    "financial": (60000, 98304),
    "rust": (40000, 65536),
    "integration": (40000, 65536),
    "react": (30000, 49152),
    "explorer": (30000, 49152),
    "docs": (30000, 49152),
}

p = argparse.ArgumentParser()
p.add_argument("--role", choices=BUDGETS,
               default="coordinator")
p.add_argument("--session")
p.add_argument("--limit", type=int, default=5)
a = p.parse_args()

warning, maximum = BUDGETS[a.role]
paths = session_files()

if a.session:
    paths = [
        f for f in paths
        if a.session in f.name
    ]

results = []

for path in paths[:max(1, a.limit)]:
    latest = None
    cwd = None

    for event in events(path):
        payload = event.get("payload") or {}

        if event.get("type") == "session_meta":
            cwd = payload.get("cwd")

        if payload.get("type") != "token_count":
            continue

        info = payload.get("info") or {}
        usage = info.get("last_token_usage") or {}

        if "input_tokens" in usage:
            latest = usage

    if latest is None:
        continue

    tokens = latest["input_tokens"]

    status = (
        "HANDOFF_RECOMMENDED"
        if tokens >= maximum else
        "COMPACT_RECOMMENDED"
        if tokens >= warning else
        "WITHIN_TARGET"
    )

    results.append({
        "session": path.name,
        "cwd": cwd,
        "assumed_role": a.role,
        "latest_request_input": tokens,
        "warning_threshold": warning,
        "target_maximum": maximum,
        "status": status
    })

print(json.dumps({
    "results": results,
    "limitations": [
        "Latest recorded request, not live context.",
        "Role supplied manually.",
        "Advisory only; no automatic compaction."
    ]
}, indent=2))
