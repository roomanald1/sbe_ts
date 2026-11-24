import argparse
import subprocess
import tempfile
import os
import signal
import sys
from pathlib import Path
from common import CACHE, ROOT

p = argparse.ArgumentParser()
p.add_argument("--chars", type=int, default=12000)
p.add_argument("--timeout", type=int, default=1800)
p.add_argument("command", nargs=argparse.REMAINDER)
a = p.parse_args()

cmd = a.command
if cmd and cmd[0] == "--":
    cmd = cmd[1:]

if not cmd:
    p.error("Command required after --")

if a.chars < 100 or a.timeout < 1:
    p.error("Invalid limits")

with tempfile.NamedTemporaryFile(
    prefix="bounded-", suffix=".log",
    dir=CACHE, delete=False
) as log:

    path = Path(log.name)

    proc = subprocess.Popen(
        cmd, cwd=ROOT,
        stdout=log,
        stderr=subprocess.STDOUT,
        start_new_session=True
    )

    try:
        rc = proc.wait(timeout=a.timeout)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
        rc = 124

size = path.stat().st_size

with path.open("rb") as f:
    if size <= a.chars:
        print(f.read().decode(errors="replace"))
    else:
        half = a.chars // 2
        print(f.read(half).decode(errors="replace"))
        print("\n--- OUTPUT OMITTED ---\n")
        f.seek(max(0, size-half))
        print(f.read(half).decode(errors="replace"))

print(f"\nExit code: {rc}")
print(f"Output bytes: {size}")
print(f"Full output: {path}")

sys.exit(rc if rc >= 0 else 128-rc)
