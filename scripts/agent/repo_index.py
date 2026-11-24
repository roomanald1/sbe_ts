#!/usr/bin/env python3
import argparse
import subprocess
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("query", nargs="?", default="")
p.add_argument("--limit", type=int, default=40)
p.add_argument("--symbols", action="store_true")
a = p.parse_args()

root = Path(subprocess.check_output(
    ["git", "rev-parse", "--show-toplevel"], text=True
).strip())

files = subprocess.check_output(
    ["git", "ls-files", "-z"], cwd=root
).decode(errors="replace").split("\0")

files = [
    f for f in files if f and not any(
        x in f.split("/") for x in
        ("target", "node_modules", "vendor", "dist")
    )
]

if not a.query:
    matches = [
        f for f in files
        if Path(f).name in ("Cargo.toml", "package.json", "AGENTS.md")
    ]
else:
    terms = a.query.lower().split()
    matches = [
        f for f in files
        if all(t in f.lower() for t in terms)
    ]
    matches.sort(key=lambda f: (
        a.query.lower() not in Path(f).name.lower(),
        len(f)
    ))

print(f"Matching tracked paths: {len(matches)}")
for f in matches[:a.limit]:
    print(f)

if a.symbols and a.query:
    print("\nSYMBOL/TEXT MATCHES")
    result = subprocess.run(
        ["git", "grep", "-n", "-I", "-F", "-e", a.query,
         "--", "*.rs", "*.ts", "*.tsx", "*.py"],
        cwd=root, capture_output=True, text=True, timeout=30
    )
    for line in result.stdout.splitlines()[:a.limit]:
        print(line[:500])
