import argparse
import json
from datetime import datetime, timezone
from common import ROOT, session_files, events

p = argparse.ArgumentParser()
p.add_argument("--since", required=True,
               help="ISO timestamp or YYYY-MM-DD")
p.add_argument("--budget", type=float, default=45)
a = p.parse_args()

since = datetime.fromisoformat(
    a.since.replace("Z", "+00:00")
)

if since.tzinfo is None:
    since = since.replace(tzinfo=timezone.utc)

totals = {
    "input_tokens": 0,
    "cached_input_tokens": 0,
    "output_tokens": 0,
    "reasoning_output_tokens": 0
}

requests = []
seen_sessions = set()
matched_sessions = 0

for path in session_files():
    if path.stat().st_mtime < since.timestamp():
        continue

    session_id = None
    matched = False
    previous = None
    counted = False

    for event in events(path):
        payload = event.get("payload") or {}

        if event.get("type") == "session_meta":
            session_id = payload.get("id")
            cwd = payload.get("cwd")

            if cwd:
                from pathlib import Path
                matched = Path(cwd).is_relative_to(ROOT)

            if session_id in seen_sessions:
                break

            if matched and session_id:
                seen_sessions.add(session_id)

        if not matched:
            continue

        if payload.get("type") != "token_count":
            continue

        info = payload.get("info") or {}
        cumulative = info.get("total_token_usage")
        last = info.get("last_token_usage")

        if not cumulative or not last:
            continue

        marker = json.dumps(cumulative, sort_keys=True)

        if marker == previous:
            continue

        previous = marker

        try:
            ts = datetime.fromisoformat(
                event["timestamp"].replace("Z", "+00:00")
            )
        except (KeyError, ValueError):
            continue

        if ts < since:
            continue

        if not counted:
            matched_sessions += 1
            counted = True

        for key in totals:
            totals[key] += last.get(key, 0)

        requests.append(last.get("input_tokens", 0))

used = totals["input_tokens"]
budget = a.budget * 1000000

if budget <= 0:
    p.error("Budget must be positive")

print(json.dumps({
    "since": since.isoformat(),
    "sessions": matched_sessions,
    "requests": len(requests),
    "totals": totals,
    "input_budget": int(budget),
    "budget_used_percent": round(used/budget*100, 2),
    "mean_request_input": (
        round(sum(requests)/len(requests))
        if requests else 0
    ),
    "max_request_input": max(requests, default=0),
    "cache_percent": (
        round(totals["cached_input_tokens"]/used*100, 2)
        if used else 0
    ),
    "limitations": [
        "Active session directory only.",
        "Repository and date scoped, not work-package scoped.",
        "Historical token events may contain resets or forks.",
        "Not a measurement of Codex subscription allowance."
    ]
}, indent=2))
