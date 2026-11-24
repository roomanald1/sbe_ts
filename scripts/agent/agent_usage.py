#!/usr/bin/env python3
import argparse
from rollouts import rollout_files, usage, thread_metadata

p = argparse.ArgumentParser()
p.add_argument("--hours", type=int, default=24)
p.add_argument("--files", type=int, default=100)
p.add_argument("--budget", type=int, default=2000000)
a = p.parse_args()

metadata = thread_metadata()
rows = []

for path in rollout_files(a.hours, a.files):
    calls = usage(path, a.hours)
    if not calls:
        continue
    meta = metadata.get(str(path), {})
    name = (
        meta.get("agent_nickname")
        or meta.get("agent_role")
        or path.stem[-12:]
    )
    inp = sum(c["input"] for c in calls)
    cache = sum(c["cached"] for c in calls)
    out = sum(c["output"] for c in calls)
    reasoning = sum(c["reasoning"] for c in calls)
    rows.append((
        name, len(calls), inp, cache, out, reasoning,
        max(c["input"] for c in calls),
        meta.get("reasoning_effort") or "?"
    ))

print(f"Window: {a.hours}h; maximum {a.files} rollout files")
print(f"{'Agent':20} {'Calls':>6} {'Input M':>9} "
      f"{'Avg input':>10} {'Max input':>10} "
      f"{'Cache %':>8} {'Output K':>9} {'Effort':>8}")

for name, n, inp, cache, out, reason, peak, effort in sorted(
    rows, key=lambda r: r[2], reverse=True
):
    print(f"{name[:20]:20} {n:6} {inp/1e6:9.2f} "
          f"{inp/n:10,.0f} {peak:10,} "
          f"{100*cache/max(inp,1):8.1f} "
          f"{out/1000:9.1f} {effort:>8}")
    if inp > a.budget:
        print("  REVIEW: cumulative input review threshold exceeded")

inp = sum(r[2] for r in rows)
cache = sum(r[3] for r in rows)
print(f"\nTotal calls: {sum(r[1] for r in rows):,}")
print(f"Input:       {inp:,}")
print(f"Cached:      {cache:,}")
print(f"Uncached:    {inp-cache:,}")
print(f"Output:      {sum(r[4] for r in rows):,}")
print(f"Reasoning:   {sum(r[5] for r in rows):,}")
print("Reasoning may be included in output; do not add twice.")
print("Rollout-derived estimates, not subscription quota usage.")
