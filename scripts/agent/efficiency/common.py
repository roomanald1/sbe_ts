import json
import os
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CACHE = Path.home() / ".cache/sbe-diagnostics"
CACHE.mkdir(parents=True, exist_ok=True)

def save(prefix, data):
    with tempfile.NamedTemporaryFile(
        mode="w", prefix=prefix+"-",
        suffix=".json", dir=CACHE,
        delete=False
    ) as f:
        json.dump(data, f, indent=2)
        return f.name

def git(*args):
    return subprocess.check_output(
        ["git", "-C", str(ROOT), *args],
        text=True
    ).strip()

def session_files():
    base = Path.home() / ".codex/sessions"
    return sorted(
        base.rglob("*.jsonl"),
        key=lambda p: p.stat().st_mtime,
        reverse=True
    )

def events(path):
    with path.open(errors="replace") as f:
        for line in f:
            try:
                yield json.loads(line)
            except (ValueError, TypeError):
                continue
