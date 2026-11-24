#!/usr/bin/env python3
import argparse
from collections import defaultdict
from rollouts import rollout_files, usage

p = argparse.ArgumentParser()
p.add_argument("--hours", type=int, default=168)
p.add_argument("--files", type=int, default=500)
a = p.parse_args()

days = defaultdict(list)

for path in rollout_files(a.hours, a.files):
    for call in usage(path, a.hours):
        days[call["timestamp"][:10]].append(call)

print(f"{'Date UTC':12} {'Calls':>6} {'Input M':>9} "
      f"{'Avg input':>10} {'Cache %':>8} "
      f"{'Output K':>9} {'Max input':>10}")

for day, calls in sorted(days.items()):
    inp = sum(c["input"] for c in calls)
    cache = sum(c["cached"] for c in calls)
    out = sum(c["output"] for c in calls)
    print(f"{day:12} {len(calls):6} {inp/1e6:9.2f} "
          f"{inp/len(calls):10,.0f} "
          f"{100*cache/max(inp,1):8.1f} "
          f"{out/1000:9.1f} "
          f"{max(c['input'] for c in calls):10,}")

print("Observed rollout events; file cap may exclude sessions.")
