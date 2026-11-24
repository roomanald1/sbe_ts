#!/usr/bin/env python3
import json
import subprocess
import argparse

p = argparse.ArgumentParser()
p.add_argument("--stats", action="store_true")
a = p.parse_args()

def run(command):
    try:
        r = subprocess.run(
            command, capture_output=True, text=True, timeout=30
        )
        if r.returncode:
            return {"error": r.stderr.strip()[:1000]}
        return r.stdout.strip()
    except (OSError, subprocess.TimeoutExpired) as e:
        return {"error": str(e)}

print("MEMORY")
print(run(["free", "-h"]))

print("\nDOCKER SERVICES")
raw = run(["docker", "ps", "-a", "--format", "{{json .}}"])
if isinstance(raw, dict):
    print(raw)
else:
    for line in raw.splitlines():
        row = json.loads(line)
        print(row.get("Names"), "|", row.get("Status"))

if a.stats:
    print("\nDOCKER RESOURCE USAGE")
    print(run([
        "docker", "stats", "--no-stream",
        "--format",
        "{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.PIDs}}"
    ]))

print("\nFILESYSTEM")
print(run(["df", "-h", "."]))

print("\nApplication worker and queue health requires "
      "verified service-specific adapters.")
