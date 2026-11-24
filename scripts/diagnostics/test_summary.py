#!/usr/bin/env python3
import argparse
import re
from collections import deque
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("log", type=Path)
p.add_argument("--limit", type=int, default=25)
a = p.parse_args()

pattern = re.compile(
    r"error(?:\[[A-Z0-9]+\])?:|FAILED|panicked at|"
    r"AssertionError|test result:|error TS\d+|"
    r"fatal error|failures:|Caused by:",
    re.IGNORECASE
)

matches = deque(maxlen=a.limit)
tail = deque(maxlen=10)
count = 0
lines = 0

with a.log.open(errors="replace") as f:
    for i, line in enumerate(f, 1):
        lines = i
        line = line.rstrip()
        tail.append(line[:500])
        if pattern.search(line):
            count += 1
            matches.append((i, line[:1000]))

print(f"Log: {a.log}")
print(f"Lines: {lines:,}")
print(f"Diagnostic matches: {count:,}")
for i, line in matches:
    print(f"{i}: {line}")

print("\nFINAL LOG LINES")
print("\n".join(tail))
print("\nLog summary only; use the original command exit code "
      "to determine test success.")
