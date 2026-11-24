
import argparse
import fcntl
import hashlib
import json
import os
import re
import signal
import subprocess
import tempfile
import time
from pathlib import Path

from common import ROOT, now, save
from testing import failures

CACHE = Path.home() / ".cache/sbe-diagnostics"
BUNDLES = CACHE / "work-packages"

ALLOWED_CRATES = {
    "auto_trader",
    "learning",
    "strategy",
    "insights",
    "position_management",
    "paper_exchange",
    "feed_handlers",
    "binance_feed"
}

def git(*args):
    return subprocess.check_output(
        ["git", *args],
        cwd=ROOT,
        timeout=30
    )

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def latest_manifest():
    files = list(BUNDLES.glob("run-*/manifest.json"))
    if not files:
        raise ValueError("No generated work-package bundles")
    return max(files, key=lambda p: p.stat().st_mtime)

def load_package(manifest_path, number):
    manifest = json.loads(manifest_path.read_text())

    if manifest.get("status") != "PREPARED":
        raise ValueError("Bundle generation incomplete")

    if manifest.get("package_count") != 16:
        raise ValueError("Incomplete package inventory")

    if sha(Path(manifest["plan"])) != manifest["plan_sha256"]:
        raise ValueError("Plan changed; regenerate bundles")

    if git("rev-parse", "HEAD").decode().strip() != manifest["git_head"]:
        raise ValueError("Git HEAD changed; regenerate bundles")

    matches = [
        item for item in manifest["packages"]
        if item["number"] == number
    ]

    if len(matches) != 1:
        raise ValueError("Missing or ambiguous work package")

    item = matches[0]

    if item["status"] == "DEFERRED":
        raise ValueError("Work package is deferred")

    folder = manifest_path.parent / f"WP{number:02d}"

    requirements = (
        folder / "requirements.md"
    ).read_text().strip()

    actual = hashlib.sha256(
        requirements.encode("utf-8")
    ).hexdigest()

    if actual != item["requirements_sha256"]:
        raise ValueError("Requirements hash mismatch")

    return manifest, folder

def commands_for(folder, extra_crates):
    text = (folder / "test-plan.md").read_text()

    crates = re.findall(
        r"`just test ([A-Za-z0-9_-]+)`",
        text
    )

    crates = list(dict.fromkeys(crates + extra_crates))

    if not crates:
        raise ValueError("No Rust test selection")

    if any(c not in ALLOWED_CRATES for c in crates):
        raise ValueError("Unsupported crate in test plan")

    commands = [["just", "diff-check"]]
    commands += [["just", "test", c] for c in crates]

    if "`just typecheck`" in text:
        commands.append(["just", "typecheck"])

    return commands

def terminate_group(proc):
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass

    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass

    # Also clean up surviving descendants.
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass

    proc.wait()

def execute(command, timeout):
    started = time.monotonic()

    with tempfile.NamedTemporaryFile(
        prefix="regression-",
        suffix=".log",
        dir=CACHE,
        delete=False
    ) as log:
        path = Path(log.name)

        proc = subprocess.Popen(
            command,
            cwd=ROOT,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True
        )

        try:
            rc = proc.wait(timeout=timeout)
            status = "PASS" if rc == 0 else "FAIL"

        except subprocess.TimeoutExpired:
            terminate_group(proc)
            rc = proc.returncode
            status = "TIMEOUT"

        except BaseException:
            terminate_group(proc)
            raise

    result = {
        "command": command,
        "status": status,
        "returncode": rc,
        "elapsed_seconds": round(
            time.monotonic() - started, 3
        ),
        "log": str(path),
        "log_bytes": path.stat().st_size
    }

    if status != "PASS":
        summary = failures(path)
        result["failure_matches"] = summary["matches"]
        result["failure_summary"] = summary["failures"][-5:]

    return result

def main():
    parser = argparse.ArgumentParser()

    parser.add_argument(
        "--package", type=int,
        choices=range(1,17), required=True
    )

    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--crate", action="append", default=[])
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--timeout", type=int, default=1800)

    args = parser.parse_args()

    if not 1 <= args.timeout <= 3600:
        parser.error("Timeout must be 1–3600 seconds")

    manifest_path = (
        args.manifest or latest_manifest()
    ).resolve()

    manifest, folder = load_package(
        manifest_path, args.package
    )

    commands = commands_for(folder, args.crate)

    report = {
        "package": args.package,
        "manifest": str(manifest_path),
        "plan_sha256": manifest["plan_sha256"],
        "git_head": manifest["git_head"],
        "started_at": now(),
        "planned_commands": commands,
        "selection": "ADVISORY_WORK_PACKAGE_TESTS",
        "acceptance": "NOT_EVALUATED",
        "results": [],
        "unverified_gates": [
            "Independent financial reconciliation",
            "API schema compatibility",
            "Trading integration invariants",
            "Hot-path performance thresholds",
            "Quantitative evidence and promotion gates",
            "Coordinator acceptance"
        ]
    }

    if not args.execute:
        report["status"] = "DRY_RUN"
        print(json.dumps(report, indent=2))
        return 0

    CACHE.mkdir(parents=True, exist_ok=True)

    with (CACHE / "build.lock").open("a") as lock:
        try:
            fcntl.flock(
                lock.fileno(),
                fcntl.LOCK_EX | fcntl.LOCK_NB
            )
        except BlockingIOError:
            raise ValueError(
                "Another regression runner owns the build lock"
            )

        for command in commands:
            result = execute(command, args.timeout)
            report["results"].append(result)

            print(
                result["status"],
                " ".join(command),
                result["elapsed_seconds"],
                "seconds",
                flush=True
            )

            if result["status"] != "PASS":
                break

    complete = len(report["results"]) == len(commands)

    passed = complete and all(
        r["status"] == "PASS"
        for r in report["results"]
    )

    unchanged_head = (
        git("rev-parse", "HEAD").decode().strip()
        == manifest["git_head"]
    )

    unchanged_plan = (
        sha(Path(manifest["plan"]))
        == manifest["plan_sha256"]
    )

    report["status"] = (
        "BLOCKED" if not unchanged_head or not unchanged_plan
        else "SELECTED_TESTS_PASS" if passed
        else "FAIL"
    )

    report["finished_at"] = now()
    report["executed_commands"] = len(report["results"])
    report["skipped_commands"] = (
        len(commands) - len(report["results"])
    )

    path = save(report, "regression_run")

    print(json.dumps({
        "status": report["status"],
        "package": args.package,
        "planned": len(commands),
        "executed": len(report["results"]),
        "skipped": report["skipped_commands"],
        "evidence": str(path),
        "acceptance": "NOT_EVALUATED"
    }, indent=2))

    return 0 if report["status"] == "SELECTED_TESTS_PASS" else 1

if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as e:
        print(json.dumps({
            "status": "BLOCKED",
            "error": str(e)[:1000],
            "acceptance": "NOT_EVALUATED"
        }, indent=2))
        raise SystemExit(1)
