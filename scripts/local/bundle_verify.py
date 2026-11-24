
import argparse
import hashlib
import json
import subprocess
import time
from pathlib import Path

from common import ROOT, save, now

CACHE = Path.home() / ".cache/sbe-diagnostics"
BUNDLES = CACHE / "work-packages"

PLAN = ROOT / (
    "src/rust/auto_trader/"
    "TRADING_IMPROVEMENTS.MD"
)

REQUIRED_FILES = {
    "requirements.md",
    "source-map.md",
    "symbol-map.json",
    "baseline.json",
    "test-plan.md",
    "acceptance.md",
    "agent-instructions.md"
}

def sha(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(
            lambda: f.read(1024 * 1024), b""
        ):
            h.update(chunk)
    return h.hexdigest()

def git(*args):
    return subprocess.check_output(
        ["git", "--no-pager", *args],
        cwd=ROOT
    ).decode().strip()

def latest_manifest():
    paths = list(BUNDLES.glob("*/manifest.json"))
    if not paths:
        raise ValueError("No work-package manifests")
    return max(paths, key=lambda p: p.stat().st_mtime)

def read_json(path):
    if path.stat().st_size > 16_000_000:
        raise ValueError("JSON exceeds size limit")
    return json.loads(path.read_text())

def verify(manifest, max_age_hours):
    manifest = manifest.resolve()

    if not manifest.is_relative_to(BUNDLES.resolve()):
        raise ValueError("Manifest outside bundle directory")

    directory = manifest.parent
    data = read_json(manifest)
    blockers = []
    warnings = []
    inventory = {}
    evidence = {}
    packages = []

    if data.get("status") != "PREPARED":
        blockers.append("Manifest not PREPARED")

    if not PLAN.is_file():
        raise ValueError("Master plan missing")

    plan_sha = sha(PLAN)
    head = git("rev-parse", "HEAD")

    if data.get("plan_sha256") != plan_sha:
        blockers.append("Master plan hash mismatch")

    if data.get("git_head") != head:
        blockers.append("Git HEAD mismatch")

    for name in ("master-plan.md", "shared-invariants.md"):
        path = directory / name

        if not path.is_file():
            blockers.append("Missing " + name)
        else:
            inventory[name] = sha(path)

    master = directory / "master-plan.md"

    if master.is_file() and sha(master) != plan_sha:
        blockers.append("Bundled master plan differs")

    package_dirs = sorted(
        p for p in directory.iterdir()
        if p.is_dir() and (p / "requirements.md").exists()
    )

    if len(package_dirs) != 16:
        blockers.append(
            "Expected 16 packages; found "
            + str(len(package_dirs))
        )

    def inspect_evidence(obj, package):
        if isinstance(obj, list):
            for item in obj:
                inspect_evidence(item, package)
            return

        if not isinstance(obj, dict):
            return

        path_value = obj.get("path")
        expected = obj.get("sha256")

        if isinstance(path_value, str) and expected:
            path = Path(path_value).expanduser().resolve()
            key = str(path)

            if key not in evidence:
                item = {
                    "path": key,
                    "packages": [],
                    "status": "VERIFIED"
                }

                if not path.is_relative_to(CACHE.resolve()):
                    item["status"] = "OUTSIDE_CACHE"
                elif not path.is_file():
                    item["status"] = "MISSING"
                elif path.stat().st_size > 64_000_000:
                    item["status"] = "SIZE_LIMIT"
                else:
                    item["sha256"] = sha(path)
                    age = (
                        time.time() - path.stat().st_mtime
                    ) / 3600

                    item["age_hours"] = round(age, 3)

                    if age < 0:
                        item["status"] = "FUTURE_TIMESTAMP"
                    elif age > max_age_hours:
                        item["status"] = "STALE"

                evidence[key] = item

            item = evidence[key]

            if (
                "sha256" in item
                and item["sha256"] != expected
            ):
                item["status"] = "HASH_MISMATCH"

            if package not in item["packages"]:
                item["packages"].append(package)

        for value in obj.values():
            if isinstance(value, (dict, list)):
                inspect_evidence(value, package)

    for package in package_dirs:
        missing = sorted(
            name for name in REQUIRED_FILES
            if not (package / name).is_file()
        )

        if missing:
            blockers.append(
                package.name + ": missing "
                + ", ".join(missing)
            )

        for name in sorted(REQUIRED_FILES):
            path = package / name

            if path.is_file():
                inventory[str(
                    path.relative_to(directory)
                )] = sha(path)

        baseline = package / "baseline.json"

        if baseline.is_file():
            inspect_evidence(
                read_json(baseline),
                package.name
            )

        packages.append({
            "package": package.name,
            "missing": missing,
            "status": (
                "INCOMPLETE" if missing
                else "FILES_PRESENT"
            )
        })

    for item in evidence.values():
        status = item["status"]

        if status == "STALE":
            warnings.append(
                "Evidence exceeds freshness window: "
                + item["path"]
            )
        elif status != "VERIFIED":
            blockers.append(
                status + ": " + item["path"]
            )

    if not evidence:
        warnings.append(
            "No recognised evidence references; "
            "baseline schema requires inspection"
        )

    warnings.append(
        "File inventory hashes establish current state, "
        "not original generation integrity"
    )

    return {
        "schema_version": 1,
        "timestamp": now(),
        "manifest": str(manifest),
        "manifest_sha256": sha(manifest),
        "git_head": head,
        "plan_sha256": plan_sha,
        "max_age_hours": max_age_hours,
        "status": (
            "BLOCKED" if blockers
            else "REVIEW_REQUIRED" if warnings
            else "VERIFIED"
        ),
        "packages": packages,
        "inventory": inventory,
        "evidence": list(evidence.values()),
        "blockers": blockers,
        "warnings": warnings,
        "acceptance": "NOT_EVALUATED"
    }

def compare_inventory(old, new):
    a = old["inventory"]
    b = new["inventory"]

    return {
        "added": sorted(b.keys() - a.keys()),
        "removed": sorted(a.keys() - b.keys()),
        "changed": sorted(
            key for key in a.keys() & b.keys()
            if a[key] != b[key]
        ),
        "manifest_changed": (
            old["manifest_sha256"]
            != new["manifest_sha256"]
        )
    }

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument(
        "--max-age-hours", type=float, default=24
    )
    args = parser.parse_args()

    try:
        if not 0 < args.max_age_hours <= 8760:
            raise ValueError("Invalid freshness window")

        result = verify(
            args.manifest or latest_manifest(),
            args.max_age_hours
        )

        if args.baseline:
            old = read_json(args.baseline)

            if old.get("schema_version") != 1:
                raise ValueError("Unsupported baseline")

            if old["manifest"] != result["manifest"]:
                raise ValueError(
                    "Cannot compare different bundle runs"
                )

            delta = compare_inventory(old, result)
            result["comparison"] = delta

            if any(delta.values()):
                result["status"] = "BLOCKED"
                result["blockers"].append(
                    "Bundle changed since baseline"
                )

        path = save(result, "bundle_verification")

        counts = {}

        for item in result["evidence"]:
            key = item["status"]
            counts[key] = counts.get(key, 0) + 1

        print(json.dumps({
            "status": result["status"],
            "manifest": result["manifest"],
            "packages": len(result["packages"]),
            "files_fingerprinted": len(result["inventory"]),
            "evidence_status": counts,
            "blockers": result["blockers"],
            "warnings": result["warnings"],
            "comparison": result.get("comparison"),
            "acceptance": result["acceptance"],
            "report": str(path)
        }, indent=2))

        return 1 if result["status"] == "BLOCKED" else 0

    except Exception as e:
        print(json.dumps({
            "status": "BLOCKED",
            "error": str(e)
        }, indent=2))
        return 1

if __name__ == "__main__":
    raise SystemExit(main())
