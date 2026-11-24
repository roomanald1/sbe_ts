
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from common import ROOT, save, now
from freshness_check import fingerprint, compare

CACHE = Path.home() / ".cache/sbe-diagnostics"
LOCAL = ROOT / "scripts/local"

PLAN = ROOT / (
    "src/rust/auto_trader/TRADING_IMPROVEMENTS.MD"
)

ORIGINAL_BASELINE = (
    CACHE / "regression_baseline-xqvl04fb.json"
)

WP_MAP = {
    "strategy": [1, 3, 5, 10, 11, 14, 15],
    "learning": [3, 4, 6, 10, 11, 12, 13, 14, 15],
    "insights": [1, 3, 4, 6, 11, 13, 14],
    "auto_trader": list(range(1, 17)),
    "position_management": [2, 5, 6, 7, 9, 12, 16],
    "paper_exchange": [2, 4, 6, 7, 12, 16],
    "feed_handlers": [1, 4, 6, 7, 10],
    "binance_feed": [1, 4, 6, 7, 10],
    "typescript": [6, 13],
}

TOOL_MAP = {
    "strategy": [
        "strategy_performance",
        "missed_opportunities",
        "walk_forward_validation"
    ],
    "learning": [
        "learning_analysis",
        "learning_coverage",
        "profile_inventory",
        "walk_forward_validation"
    ],
    "insights": [
        "scout_health",
        "missed_opportunities",
        "learning_analysis"
    ],
    "auto_trader": [
        "feedback_analysis",
        "capital_allocation",
        "strategy_performance",
        "performance_gate",
        "latency_summary"
    ],
    "position_management": [
        "financial_health",
        "financial_reconciliation",
        "account_analysis",
        "fee_analysis",
        "position_analysis",
        "realized_pnl_analysis",
        "trade_reconciliation"
    ],
    "paper_exchange": [
        "trade_reconciliation",
        "financial_health",
        "paper_soak"
    ],
    "feed_handlers": [
        "latency_summary",
        "performance_gate"
    ],
    "binance_feed": [
        "latency_summary",
        "performance_gate"
    ],
    "typescript": [
        "contract_check",
        "integration_check"
    ]
}

def read_json(path):
    path = Path(path)
    if path.stat().st_size > 64_000_000:
        raise ValueError("JSON size limit: " + str(path))
    return json.loads(path.read_text())

def sha(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(
            lambda: f.read(1024 * 1024), b""
        ):
            h.update(chunk)
    return h.hexdigest()

def latest(prefix):
    paths = list(CACHE.glob(prefix + "-*.json"))
    return max(
        paths, key=lambda p: p.stat().st_mtime
    ) if paths else None

def run_script(name, args=(), timeout=120):
    script = LOCAL / (name + ".py")

    if not script.is_file():
        return {
            "status": "BLOCKED",
            "reason": "Missing script",
            "script": str(script)
        }

    with tempfile.NamedTemporaryFile(
        dir=CACHE,
        prefix=name + "-",
        suffix=".log",
        delete=False
    ) as log:
        start = time.monotonic()

        process = subprocess.Popen(
            [sys.executable, str(script), *map(str, args)],
            cwd=ROOT,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            env={
                **os.environ,
                "GIT_PAGER": "cat",
                "PAGER": "cat",
                "PYTHONDONTWRITEBYTECODE": "1"
            }
        )

        try:
            rc = process.wait(timeout=timeout)
            status = "COMPLETED" if rc == 0 else "FAILED"
        except subprocess.TimeoutExpired:
            import signal
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            rc = None
            status = "TIMEOUT"

    path = Path(log.name)

    with path.open("rb") as f:
        f.seek(max(0, path.stat().st_size - 8000))
        tail = f.read().decode(errors="replace")

    return {
        "status": status,
        "returncode": rc,
        "seconds": round(time.monotonic() - start, 3),
        "log": str(path),
        "output_tail": tail,
        "acceptance": "NOT_EVALUATED"
    }

def impact(old, new):
    delta = compare(old, new)

    paths = sorted(set(
        delta["added"]
        + delta["removed"]
        + delta["changed"]
    ))

    packages = set()
    tools = set()
    contracts = []
    unmapped = []

    for name in paths:
        parts = set(Path(name).parts)
        mapped = False

        for component, numbers in WP_MAP.items():
            if component in parts:
                packages.update(numbers)
                tools.update(TOOL_MAP.get(component, []))
                mapped = True

        item = new["files"].get(name) or old["files"][name]
        category = item["category"]

        if category in {
            "api_contract_candidates",
            "database_contract_candidates",
            "configuration"
        }:
            contracts.append({
                "path": name,
                "category": category
            })
            tools.update([
                "contract_check",
                "integration_check"
            ])

        if category == "master_plan":
            packages.update(range(1, 17))
            mapped = True

        if category == "diagnostic_tooling":
            tools.add(Path(name).stem)
            mapped = True

        if not mapped:
            unmapped.append(name)

    if unmapped:
        packages.update(range(1, 17))

    return {
        "status": delta["status"],
        "changed_files": paths,
        "affected_packages": sorted(packages),
        "diagnostic_review": sorted(tools),
        "contract_candidates": contracts,
        "unmapped_files": unmapped,
        "semantic_compatibility": "NOT_EVALUATED",
        "mapping": "CONSERVATIVE_HEURISTIC"
    }

def regression_compare(old, new):
    def index(data):
        result = {}

        for item in data["results"]:
            command = item["command"]
            if isinstance(command, list):
                command = " ".join(map(str, command))

            if command in result:
                raise ValueError("Duplicate regression command")

            result[command] = item

        return result

    a, b = index(old), index(new)
    regressions = []
    resolved = []
    existing = []
    incomplete = []
    passed = []

    for command in sorted(a.keys() | b.keys()):
        if command not in a or command not in b:
            incomplete.append(command)
            continue

        before = a[command]["status"]
        after = b[command]["status"]

        if before == "PASS" and after == "PASS":
            passed.append(command)
        elif before == "PASS":
            regressions.append(command)
        elif after == "PASS":
            resolved.append(command)
        else:
            existing.append({
                "command": command,
                "before": before,
                "after": after,
                "same_failure_verified": False
            })

    return {
        "status": (
            "REGRESSIONS_DETECTED" if regressions
            else "REVIEW_REQUIRED"
            if existing or incomplete
            else "COMMANDS_PASS"
        ),
        "new_command_failures": regressions,
        "resolved_command_failures": resolved,
        "still_failing_commands": existing,
        "incomplete_comparison": incomplete,
        "passing_commands": passed,
        "acceptance": "NOT_EVALUATED"
    }

def evidence_status(hours):
    prefixes = [
        "financial_snapshot",
        "financial_reconciliation",
        "financial_health",
        "feedback_analysis",
        "learning_analysis",
        "profile_inventory",
        "regression_baseline",
        "performance_gate",
        "bundle_verification"
    ]

    records = []

    for prefix in prefixes:
        path = latest(prefix)

        if path is None:
            records.append({
                "type": prefix,
                "status": "MISSING"
            })
            continue

        age = (
            time.time() - path.stat().st_mtime
        ) / 3600

        try:
            data = read_json(path)
            status = (
                "FUTURE_TIMESTAMP" if age < 0
                else "HISTORICAL_BASELINE"
                if prefix == "regression_baseline"
                else "STALE" if age > hours
                else "RECENT_FILE"
            )

            records.append({
                "type": prefix,
                "status": status,
                "reported_status": data.get("status"),
                "age_hours": round(age, 2),
                "path": str(path),
                "sha256": sha(path),
                "acceptance": data.get(
                    "acceptance", "NOT_EVALUATED"
                )
            })

        except Exception as e:
            records.append({
                "type": prefix,
                "status": "BLOCKED",
                "error": str(e)
            })

    return records

def retention_report(days):
    # Report only. Never delete evidence automatically.
    referenced = set()
    unresolved = []

    def inspect(value):
        if isinstance(value, dict):
            for item in value.values():
                inspect(item)
        elif isinstance(value, list):
            for item in value:
                inspect(item)
        elif isinstance(value, str):
            if value.startswith(str(CACHE) + "/"):
                referenced.add(str(Path(value).resolve()))

    manifests = list(
        (CACHE / "work-packages").rglob("*.json")
    )

    manifests += list(CACHE.glob("*.json"))

    for path in manifests:
        try:
            inspect(read_json(path))
        except Exception:
            unresolved.append(str(path))

    candidates = []
    total = 0
    protected = 0

    for path in CACHE.glob("*"):
        if (
            not path.is_file()
            or path.is_symlink()
            or path.suffix not in {".json", ".log"}
        ):
            continue

        size = path.stat().st_size
        total += size

        if str(path.resolve()) in referenced:
            protected += size
            continue

        age = (
            time.time() - path.stat().st_mtime
        ) / 86400

        if age >= days:
            candidates.append({
                "path": str(path),
                "bytes": size,
                "age_days": round(age, 1)
            })

    return {
        "status": "REPORT_ONLY",
        "total_bytes": total,
        "referenced_bytes": protected,
        "candidate_bytes": sum(
            x["bytes"] for x in candidates
        ),
        "candidates": candidates,
        "unreadable_reference_sources": unresolved,
        "deletion_authorized": False,
        "external_references_verified": False
    }

def handoff(path):
    data = read_json(path)

    required = {
        "work_package",
        "owner",
        "status",
        "git_head",
        "plan_sha256",
        "changed_files",
        "tests",
        "evidence",
        "blockers",
        "next_action"
    }

    missing = sorted(required - data.keys())

    if missing:
        raise ValueError(
            "Missing handoff fields: " + ", ".join(missing)
        )

    if data["work_package"] not in range(1, 17):
        raise ValueError("Invalid work package")

    if data["status"] not in {
        "IN_PROGRESS", "BLOCKED", "READY_FOR_REVIEW"
    }:
        raise ValueError("Invalid handoff status")

    for key in ("changed_files", "tests", "evidence", "blockers"):
        if not isinstance(data[key], list):
            raise ValueError("Expected list: " + key)

    head = subprocess.check_output(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT, text=True
    ).strip()

    if data["git_head"] != head:
        raise ValueError("Handoff HEAD mismatch")

    if data["plan_sha256"] != sha(PLAN):
        raise ValueError("Handoff plan mismatch")

    for item in data["evidence"]:
        p = Path(item["path"]).expanduser().resolve()

        if not p.is_relative_to(CACHE.resolve()):
            raise ValueError("Evidence outside cache")

        if sha(p) != item["sha256"]:
            raise ValueError("Handoff evidence mismatch")

    data["validated_at"] = now()
    data["acceptance"] = "NOT_EVALUATED"
    data["validation_scope"] = "STRUCTURE_AND_EVIDENCE_HASHES"

    return data

def self_test():
    def file_item(category, value):
        return {
            "category": category,
            "sha256": value
        }

    old = {
        "git_head": "a",
        "files": {
            "src/rust/learning/src/test.rs":
                file_item("source", "old")
        }
    }

    new = {
        "git_head": "a",
        "files": {
            "src/rust/learning/src/test.rs":
                file_item("source", "new")
        }
    }

    result = impact(old, new)

    assert 4 in result["affected_packages"]
    assert "learning_analysis" in result["diagnostic_review"]
    print("PASS: diagnostic impact")

    assert not impact(old, old)["affected_packages"]
    print("PASS: unchanged impact")

    def regression(status):
        return {
            "results": [
                {"command": ["just", "test"], "status": status}
            ]
        }

    assert regression_compare(
        regression("PASS"), regression("FAIL")
    )["new_command_failures"]
    print("PASS: new regression")

    assert regression_compare(
        regression("FAIL"), regression("PASS")
    )["resolved_command_failures"]
    print("PASS: resolved regression")

    assert regression_compare(
        regression("FAIL"), regression("FAIL")
    )["still_failing_commands"]
    print("PASS: existing command failure")

    assert regression_compare(
        regression("PASS"), regression("PASS")
    )["status"] == "COMMANDS_PASS"
    print("PASS: unchanged passing tests")

    print("COORDINATOR SELF-TEST: 6/6 PASS")

def prepare(args):
    previous = (
        Path(args.baseline) if args.baseline
        else latest("repository_fingerprint")
    )

    current = fingerprint()
    fingerprint_path = save(
        current, "repository_fingerprint"
    )

    changes = (
        impact(read_json(previous), current)
        if previous else {
            "status": "INITIAL_BASELINE",
            "affected_packages": list(range(1, 17)),
            "diagnostic_review": [],
            "contract_candidates": [],
            "changed_files": []
        }
    )

    steps = {}

    if args.mode in {"full", "monitor"}:
        steps["financial_snapshot"] = run_script(
            "financial_snapshot"
        )

        if steps["financial_snapshot"]["status"] == "COMPLETED":
            # Bind analysis to the manifest emitted by this run,
            # not an independently selected latest snapshot.
            output = steps["financial_snapshot"]["output_tail"]
            matches = re.findall(
                r'(/[^\s"<>]+/financial_snapshot-[^\s"<>]+\.json)',
                output
            )

            if matches:
                manifest = Path(matches[-1])
                steps["financial_health"] = run_script(
                    "financial_health",
                    ["--manifest", str(manifest)]
                )
            else:
                steps["financial_health"] = {
                    "status": "BLOCKED",
                    "reason": "Snapshot manifest not identified"
                }

    if args.refresh_bundles:
        steps["generate_work_packages"] = run_script(
            "generate_work_packages", timeout=300
        )

    steps["bundle_verify"] = run_script(
        "bundle_verify",
        ["--max-age-hours", str(args.max_age_hours)]
    )

    if args.mode == "full":
        steps["integration_check"] = run_script(
            "integration_check", timeout=180
        )

    records = evidence_status(args.max_age_hours)

    baseline = None

    if ORIGINAL_BASELINE.is_file():
        original = read_json(ORIGINAL_BASELINE)

        baseline = {
            "path": str(ORIGINAL_BASELINE),
            "status": original.get("status"),
            "passing": sum(
                x["status"] == "PASS"
                for x in original["results"]
            ),
            "total": len(original["results"]),
            "failed_commands": [
                x["command"] for x in original["results"]
                if x["status"] != "PASS"
            ],
            "historical": True
        }

        if args.regression:
            baseline["comparison"] = regression_compare(
                original, read_json(args.regression)
            )

    end = fingerprint()
    during_run = compare(current, end)

    report = {
        "timestamp": now(),
        "mode": args.mode,
        "status": "REVIEW_REQUIRED",
        "repository_fingerprint": str(fingerprint_path),
        "repository_changed_during_run":
            during_run["status"] != "NO_TRACKED_CONTENT_DRIFT",
        "fingerprint_status": current["status"],
        "impact": changes,
        "steps": steps,
        "evidence": records,
        "historical_regression_baseline": baseline,
        "implementation_authorized": False,
        "acceptance": "NOT_EVALUATED"
    }

    path = save(report, "preparation_report")

    print(json.dumps({
        "status": report["status"],
        "mode": args.mode,
        "fingerprinted_files": current["file_count"],
        "repository_changed_during_run":
            report["repository_changed_during_run"],
        "changed_files": len(changes["changed_files"]),
        "affected_packages": changes["affected_packages"],
        "diagnostic_review": changes["diagnostic_review"],
        "contract_candidates": len(
            changes["contract_candidates"]
        ),
        "steps": {
            key: value["status"]
            for key, value in steps.items()
        },
        "evidence": {
            item["type"]: item["status"]
            for item in records
        },
        "historical_regression_baseline": baseline,
        "acceptance": "NOT_EVALUATED",
        "report": str(path)
    }, indent=2))

def main():
    parser = argparse.ArgumentParser()

    parser.add_argument(
        "--mode", choices=["quick", "full", "monitor"],
        default="quick"
    )
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--regression", type=Path)
    parser.add_argument("--refresh-bundles", action="store_true")
    parser.add_argument("--max-age-hours", type=float, default=24)
    parser.add_argument("--retention-report", type=int)
    parser.add_argument("--handoff", type=Path)
    parser.add_argument("--self-test", action="store_true")

    args = parser.parse_args()

    try:
        if not 0 < args.max_age_hours <= 8760:
            raise ValueError("Invalid freshness window")

        if args.self_test:
            self_test()
            return 0

        if args.retention_report is not None:
            if args.retention_report < 1:
                raise ValueError("Invalid retention period")

            result = retention_report(args.retention_report)
            path = save(result, "retention_report")

            print(json.dumps({
                "status": result["status"],
                "total_mb": round(result["total_bytes"]/1e6, 2),
                "referenced_mb": round(
                    result["referenced_bytes"]/1e6, 2
                ),
                "candidate_mb": round(
                    result["candidate_bytes"]/1e6, 2
                ),
                "candidates": len(result["candidates"]),
                "unreadable_sources": len(
                    result["unreadable_reference_sources"]
                ),
                "deletion_authorized": False,
                "report": str(path)
            }, indent=2))

            return 0

        if args.handoff:
            result = handoff(args.handoff)
            path = save(result, "validated_handoff")
            print(json.dumps({
                "status": "STRUCTURE_VALIDATED",
                "report": str(path),
                "acceptance": "NOT_EVALUATED"
            }, indent=2))
            return 0

        import fcntl

        with (CACHE / "prepare.lock").open("a") as lock:
            fcntl.flock(
                lock, fcntl.LOCK_EX | fcntl.LOCK_NB
            )
            prepare(args)

        return 0

    except Exception as e:
        print(json.dumps({
            "status": "BLOCKED",
            "error": str(e)
        }, indent=2))
        return 1

if __name__ == "__main__":
    raise SystemExit(main())
