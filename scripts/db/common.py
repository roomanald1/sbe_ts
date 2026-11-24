import argparse
import json
import subprocess
import tempfile
import sys
from pathlib import Path
from engine import execute

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent

def registry():
    return json.loads(
        (HERE / "databases.json").read_text()
    )

def remote(entry, request):
    engine = (HERE / "engine.py").read_text()

    try:
        result = subprocess.run(
            [
                "docker", "exec", "-i",
                entry["container"],
                "python3", "-c", engine
            ],
            input=json.dumps(request),
            capture_output=True,
            text=True,
            timeout=request.get("timeout", 5) + 10
        )
    except (OSError, subprocess.TimeoutExpired) as e:
        raise RuntimeError(str(e))

    try:
        data = json.loads(result.stdout)
    except ValueError:
        raise RuntimeError(
            "Container SQLite access unavailable: " +
            result.stderr.strip()[:500]
        )

    if result.returncode or "error" in data:
        raise RuntimeError(data.get("error", "Query failed"))

    return data

def discover(alias):
    entry = registry()[alias]

    if "path" in entry:
        path = ROOT / entry["path"]
        return [{
            "path": str(path),
            "bytes": path.stat().st_size if path.exists() else None
        }]

    return remote(entry, {
        "operation": "discover",
        "path": entry["directory"]
    })["files"]

def request(alias, operation, filename=None, **kwargs):
    entries = registry()

    if alias not in entries:
        raise RuntimeError("Unknown database alias: " + alias)

    entry = entries[alias]

    if "path" in entry:
        path = ROOT / entry["path"]
        data = execute({
            "operation": operation,
            "path": str(path),
            **kwargs
        })

    else:
        files = discover(alias)

        if filename:
            files = [
                f for f in files
                if Path(f["path"]).name == filename
            ]

        if len(files) != 1:
            raise RuntimeError(
                "Specify --file. Available files: " +
                ", ".join(Path(f["path"]).name for f in files)
            )

        data = remote(entry, {
            "operation": operation,
            "path": files[0]["path"],
            **kwargs
        })

    data["database_alias"] = alias
    data["source"] = (
        "local" if "path" in entry else "docker"
    )

    return data

def parser(description):
    p = argparse.ArgumentParser(description=description)
    p.add_argument("database", choices=registry().keys())
    p.add_argument("--file")
    p.add_argument("--timeout", type=int, default=5)
    p.add_argument("--limit", type=int, default=20)
    p.add_argument("--output", type=Path)
    return p

def parameters(values):
    result = {}

    for item in values:
        key, sep, value = item.partition("=")
        if not sep or not key:
            raise RuntimeError("Parameters must be NAME=VALUE")
        try:
            result[key] = json.loads(value)
        except ValueError:
            result[key] = value

    return result

def emit(data, output=None):
    raw = json.dumps(data, indent=2, default=str)

    if output:
        with output.open("x") as f:
            f.write(raw)

        print("Evidence:", output)

    elif len(raw.encode()) > 8000:
        with tempfile.NamedTemporaryFile(
            mode="w", prefix="sbe-sql-",
            suffix=".json", delete=False
        ) as f:
            f.write(raw)
            path = f.name

        print(json.dumps({
            "evidence": path,
            "bytes": len(raw.encode()),
            "returned_rows": data.get("returned_rows"),
            "truncated": data.get("truncated"),
            "database_alias": data.get("database_alias")
        }, indent=2))

    else:
        print(raw)

def main(fn):
    try:
        fn()
    except Exception as e:
        print("BLOCKED:", str(e), file=sys.stderr)
        sys.exit(1)
