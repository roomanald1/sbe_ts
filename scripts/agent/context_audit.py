#!/usr/bin/env python3
import argparse
import json
from collections import Counter
from rollouts import rollout_files, events

p = argparse.ArgumentParser()
p.add_argument("--hours", type=int, default=48)
p.add_argument("--files", type=int, default=30)
a = p.parse_args()

counts = Counter()
sizes = Counter()
compactions = Counter()

for path in rollout_files(a.hours, a.files):
    for event in events(path):
        payload = event.get("payload") or {}
        if not isinstance(payload, dict):
            continue
        kind = str(event.get("type", "?"))
        subtype = str(payload.get("type", "?"))
        key = f"{kind}/{subtype}"
        if "compact" in key.lower():
            compactions[key] += 1
        raw = json.dumps(event, ensure_ascii=False)
        low = raw.lower()
        if any(s in low for s in (
            "additionalcontext", "additional_context",
            "hookoutput", "hook_output", "bd codex-hook",
            "beads workflow context", "persistent memories ("
        )):
            counts[key] += 1
            sizes[key] += len(raw.encode())

print("POTENTIAL CONTEXT INJECTION RECORDS")
for key, n in counts.most_common(20):
    print(f"{key:45} count={n:6} "
          f"bytes={sizes[key]:10,} avg={sizes[key]//n:,}")

print("\nCOMPACTION RECORDS")
for key, n in compactions.most_common():
    print(key, n)

print("\nCounts include matching log records, not necessarily "
      "distinct injections or exact model tokens.")
print("Time window selects recently modified rollout files.")
