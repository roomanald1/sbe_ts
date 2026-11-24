
import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from common import ROOT, save, now

CACHE = Path.home() / ".cache/sbe-diagnostics"

PLAN = (
    "src/rust/auto_trader/"
    "TRADING_IMPROVEMENTS.MD"
)

MAX_FILE_BYTES = 32 * 1024 * 1024

SOURCE_SUFFIXES = {
    ".rs", ".ts", ".tsx", ".js", ".jsx",
    ".cjs", ".mjs", ".py", ".sql", ".toml",
    ".yaml", ".yml", ".json", ".md",
    ".sh", ".lock", ".proto"
}

EXCLUDED_PARTS = {
    "node_modules", "target", "vendor",
    "__pycache__", ".git", "sbe_out",
    "dist", "backups"
}

def git(*args):
    return subprocess.check_output(
        ["git", "--no-pager", *args],
        cwd=ROOT
    )

def digest(raw):
    return hashlib.sha256(raw).hexdigest()

def file_hash(path):
    h = hashlib.sha256()

    with path.open("rb") as f:
        for chunk in iter(
            lambda: f.read(1024 * 1024), b""
        ):
            h.update(chunk)

    return h.hexdigest()

def relevant(name):
    p = Path(name)

    if EXCLUDED_PARTS.intersection(p.parts):
        return False

    if p.name.lower() in {
        "justfile", "dockerfile", "makefile"
    }:
        return True

    return p.suffix.lower() in SOURCE_SUFFIXES

def category(name):
    lower = name.lower()

    if name == PLAN:
        return "master_plan"

    if name.startswith("scripts/"):
        return "diagnostic_tooling"

    if (
        name.endswith(".sql")
        or "migration" in lower
        or "schema" in lower
        or "persistence" in lower
        or "ledger" in lower
    ):
        return "database_contract_candidates"

    if any(x in lower for x in (
        "/api/", "route", "rsocket",
        "dto", "contract"
    )):
        return "api_contract_candidates"

    if Path(name).suffix.lower() in {
        ".toml", ".yaml", ".yml", ".json"
    }:
        return "configuration"

    return "source"

def fingerprint():
    head_before = git(
        "rev-parse", "HEAD"
    ).decode().strip()

    names_before = git(
        "ls-files", "-z", "--cached",
        "--others", "--exclude-standard"
    )

    names = sorted(set(
        names_before.decode(
            errors="surrogateescape"
        ).split("\0")
    ))

    files = {}
    skipped = []

    for name in names:
        if not name or not relevant(name):
            continue

        p = ROOT / name

        if p.is_symlink():
            skipped.append({
                "path": name,
                "reason": "symlink"
            })
            continue

        if not p.is_file():
            skipped.append({
                "path": name,
                "reason": "missing"
            })
            continue

        before = p.stat()

        if before.st_size > MAX_FILE_BYTES:
            skipped.append({
                "path": name,
                "reason": "size_limit"
            })
            continue

        sha = file_hash(p)
        after = p.stat()

        if (
            before.st_size != after.st_size
            or before.st_mtime_ns != after.st_mtime_ns
            or before.st_ino != after.st_ino
        ):
            raise RuntimeError(
                "File changed during scan: " + name
            )

        files[name] = {
            "sha256": sha,
            "bytes": after.st_size,
            "category": category(name)
        }

    head_after = git(
        "rev-parse", "HEAD"
    ).decode().strip()

    names_after = git(
        "ls-files", "-z", "--cached",
        "--others", "--exclude-standard"
    )

    if (
        head_before != head_after
        or names_before != names_after
    ):
        raise RuntimeError(
            "Repository changed during scan"
        )

    groups = {}

    for name, item in files.items():
        groups.setdefault(
            item["category"], {}
        )[name] = item["sha256"]

    hashes = {
        key: digest(json.dumps(
            value, sort_keys=True
        ).encode())
        for key, value in groups.items()
    }

    return {
        "timestamp": now(),
        "schema_version": 1,
        "git_head": head_after,
        "status": (
            "PARTIAL_FINGERPRINT" if skipped
            else "FINGERPRINTED"
        ),
        "file_count": len(files),
        "category_hashes": hashes,
        "files": files,
        "skipped": skipped,
        "acceptance": "NOT_EVALUATED"
    }

def compare(old, new):
    a = old["files"]
    b = new["files"]

    added = sorted(b.keys() - a.keys())
    removed = sorted(a.keys() - b.keys())

    changed = sorted(
        name for name in a.keys() & b.keys()
        if a[name]["sha256"] != b[name]["sha256"]
    )

    categories = {}

    for name in added + removed + changed:
        item = b.get(name) or a[name]
        key = item["category"]
        categories[key] = categories.get(key, 0) + 1

    return {
        "added": added,
        "removed": removed,
        "changed": changed,
        "affected_categories": categories,
        "head_changed": (
            old["git_head"] != new["git_head"]
        ),
        "status": (
            "REFRESH_REQUIRED"
            if added or removed or changed
            or old["git_head"] != new["git_head"]
            else "NO_TRACKED_CONTENT_DRIFT"
        )
    }

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", type=Path)
    args = parser.parse_args()

    try:
        result = fingerprint()

        path = save(
            result, "repository_fingerprint"
        )

        summary = {
            "status": result["status"],
            "git_head": result["git_head"],
            "files": result["file_count"],
            "categories": list(
                result["category_hashes"]
            ),
            "skipped": len(result["skipped"]),
            "evidence": str(path)
        }

        if args.baseline:
            old = json.loads(
                args.baseline.read_text()
            )

            if old.get("schema_version") != 1:
                raise ValueError(
                    "Unsupported fingerprint version"
                )

            delta = compare(old, result)
            delta["baseline"] = str(args.baseline)
            delta["current"] = str(path)

            delta_path = save(
                delta, "repository_drift"
            )

            summary["comparison"] = {
                "status": delta["status"],
                "added": len(delta["added"]),
                "removed": len(delta["removed"]),
                "changed": len(delta["changed"]),
                "affected_categories":
                    delta["affected_categories"],
                "head_changed":
                    delta["head_changed"],
                "evidence": str(delta_path)
            }

        print(json.dumps(summary, indent=2))
        return 0

    except Exception as e:
        print(json.dumps({
            "status": "BLOCKED",
            "error": str(e)
        }, indent=2))
        return 1

if __name__ == "__main__":
    raise SystemExit(main())
