import json
import subprocess
import sys
import time
import tempfile
import os
from pathlib import Path
from datetime import datetime, timezone

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
EVIDENCE = Path.home() / ".cache/sbe-diagnostics"
EVIDENCE.mkdir(parents=True, exist_ok=True, mode=0o700)

def now():
    return datetime.now(timezone.utc).isoformat()

def run(args, timeout=30, cap=12000):
    started = time.monotonic()
    try:
        with tempfile.NamedTemporaryFile(
            dir=EVIDENCE, prefix="command-",
            suffix=".log", delete=False
        ) as f:
            path = Path(f.name)
            result = subprocess.run(
                args, cwd=ROOT, stdout=f,
                stderr=subprocess.STDOUT,
                timeout=timeout
            )
        with path.open("rb") as f:
            f.seek(max(0, path.stat().st_size-cap))
            tail = f.read().decode(errors="replace")
        return {
            "status": "PASS" if result.returncode == 0 else "FAIL",
            "exit_code": result.returncode,
            "elapsed_ms": round((time.monotonic()-started)*1000),
            "log": str(path),
            "output_tail": tail
        }
    except Exception as e:
        return {"status": "BLOCKED", "error": str(e)}

def save(data, prefix="evidence"):
    with tempfile.NamedTemporaryFile(
        mode="w", dir=EVIDENCE, prefix=prefix+"-",
        suffix=".json", delete=False
    ) as f:
        json.dump(data, f, indent=2, allow_nan=False)
        return f.name

def emit(data):
    raw = json.dumps(data, indent=2, default=str)
    if len(raw.encode()) <= 10000:
        print(raw)
    else:
        path = save(data)
        print(json.dumps({
            "status": data.get("status", "COLLECTED")
                      if isinstance(data, dict) else "COLLECTED",
            "evidence": path,
            "bytes": len(raw.encode()),
            "preview": raw[:2000]
        }, indent=2))

def api(service, endpoint):
    allowed = {
        "learning": {"summary", "profiles"},
        "autotrader": {"feedback-queues", "open-position-counts"},
        "insights": {"dashboard", "best"},
        "positions": {
            "positions", "account", "realized-pnl",
            "fee-summary", "fee-rates", "usd-rates",
            "financial-snapshot", "barriers", "persistence-timing"
        }
    }
    if endpoint not in allowed.get(service, set()):
        raise RuntimeError("Endpoint excluded from routine diagnostics")

    # Import the existing API client in an isolated process.
    code = (
        "import sys,json;"
        "sys.path.insert(0,sys.argv[1]);"
        "from common import query;"
        "print(json.dumps(query(sys.argv[2],sys.argv[3])))"
    )
    result = subprocess.run(
        [sys.executable, "-c", code,
         str(ROOT/"scripts/api"), service, endpoint],
        cwd=ROOT, capture_output=True,
        text=True, timeout=20
    )
    if result.returncode:
        raise RuntimeError(result.stderr[:500])
    data = json.loads(result.stdout)
    if data.get("application_status") != 200:
        raise RuntimeError("Unverified application status")
    return data

def collect(service, endpoint):
    try:
        data = api(service, endpoint)
        body = data["body"]
        return {
            "status": "COLLECTED",
            "service": service,
            "endpoint": endpoint,
            "timestamp": now(),
            "elapsed_ms": data.get("elapsed_ms"),
            "bytes": data.get("bytes"),
            "records": len(body) if isinstance(body, list) else None,
            "evidence": save(data, service+"-"+endpoint),
            "body": body
        }
    except Exception as e:
        return {
            "status": "BLOCKED",
            "service": service,
            "endpoint": endpoint,
            "error": str(e)
        }

def compact(item):
    return {k:v for k,v in item.items() if k != "body"}

def git_files():
    p = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=ROOT, capture_output=True,
        check=True, timeout=15
    )
    return [
        ROOT/x for x in
        p.stdout.decode(errors="replace").split("\0")
        if x
    ]

def changed_files():
    paths = set()
    for args in [
        ["git", "diff", "--name-only", "-z", "HEAD"],
        ["git", "ls-files", "--others", "--exclude-standard", "-z"]
    ]:
        p = subprocess.run(
            args, cwd=ROOT, capture_output=True,
            check=True, timeout=15
        )
        paths.update(
            x for x in p.stdout.decode(
                errors="replace"
            ).split("\0") if x
        )
    return sorted(paths)

def read_text(path, cap=2000000):
    path = Path(path)
    if not path.is_file() or path.stat().st_size > cap:
        return ""
    return path.read_text(errors="replace")
