import argparse
import json
import re
import hashlib
from pathlib import Path
from common import ROOT, run, emit, save, now, collect, compact
from repository import impact, api_contracts

def failures(path):
    path = Path(path)
    with path.open("rb") as f:
        f.seek(max(0, path.stat().st_size-2000000))
        text = f.read().decode(errors="replace")
    lines = text.splitlines()
    matches = []
    for i,line in enumerate(lines):
        if re.search(
            r"error\[|error:|FAILED|panicked at|AssertionError|"
            r"FAIL\s|test result: FAILED",
            line
        ):
            matches.append({
                "line": i+1,
                "context": "\n".join(lines[max(0,i-2):i+5])[:1000]
            })
    return {
        "log": str(path),
        "matches": len(matches),
        "failures": matches[-30:],
        "scope": "Last 2 MB of log; line numbers relative to tail"
    }

def compare(old, new):
    if not isinstance(old, dict) or not isinstance(new, dict):
        raise ValueError("Expected JSON objects")
    changes = []
    for key in sorted(set(old)|set(new)):
        if old.get(key) != new.get(key):
            changes.append({
                "field": key,
                "before": old.get(key),
                "after": new.get(key)
            })
    return changes

def main(name):
    p = argparse.ArgumentParser(description=name)
    p.add_argument("--crate", action="append", default=[])
    p.add_argument("--affected", action="store_true")
    p.add_argument("--execute", action="store_true")
    p.add_argument("--log")
    p.add_argument("--baseline")
    p.add_argument("--current")
    p.add_argument("--service")
    p.add_argument("--endpoint")
    p.add_argument("--timeout", type=int, default=900)
    a = p.parse_args()

    if name == "test_runner":
        crates = a.crate or (
            impact()["rust_packages"] if a.affected else []
        )
        commands = [["just", "test", c] for c in crates]
        if any(not re.fullmatch(r"[A-Za-z0-9_-]+", c) for c in crates):
            p.error("Invalid crate name")

        if not a.execute:
            emit({
                "status": "DRY_RUN",
                "commands": commands,
                "note": "Use --execute to run sequentially."
            })
            return 0

        if not commands:
            p.error("Specify --crate or --affected")

        results = []
        for cmd in commands:
            result = run(cmd, timeout=min(max(a.timeout,1),1800))
            result["command"] = cmd
            if result.get("log") and result["status"] != "PASS":
                result["failure_summary"] = failures(result["log"])
            result.pop("output_tail", None)
            results.append(result)
            if result["status"] != "PASS":
                break

        data = {
            "timestamp": now(),
            "status": "PASS" if len(results) == len(commands)
                      and all(r["status"] == "PASS" for r in results)
                      else "FAIL",
            "planned": len(commands),
            "executed": len(results),
            "results": results
        }

    elif name == "test_failures":
        if not a.log:
            p.error("--log required")
        data = failures(a.log)

    elif name in ("regression_compare", "performance_compare"):
        if not a.baseline or not a.current:
            p.error("--baseline and --current required")
        old = json.loads(Path(a.baseline).read_text())
        new = json.loads(Path(a.current).read_text())
        data = {
            "status": "REVIEW_REQUIRED",
            "changes": compare(old, new),
            "note": "Structural comparison only; metric comparability "
                    "and acceptance thresholds require verification."
        }

    elif name == "contract_check":
        contracts = api_contracts()
        digest = hashlib.sha256(
            json.dumps(contracts, sort_keys=True).encode()
        ).hexdigest()
        data = {
            "status": "INVENTORY",
            "fingerprint": digest,
            "routes": len(contracts),
            "evidence": save(contracts, "contracts"),
            "note": "Route fingerprint is not schema compatibility."
        }

    elif name == "fixture_capture":
        if not a.service or not a.endpoint:
            p.error("--service and --endpoint required")
        result = collect(a.service, a.endpoint)
        data = compact(result)
        data["fixture_status"] = "PRIVATE_RAW_EVIDENCE_ONLY"
        data["note"] = (
            "Not sanitised. Do not commit or share until "
            "explicit fixture redaction rules are verified."
        )

    elif name == "integration_check":
        probes = [
            ("learning", "summary"),
            ("autotrader", "feedback-queues"),
            ("insights", "best"),
            ("positions", "fee-summary")
        ]
        results = [compact(collect(*x)) for x in probes]
        data = {
            "status": "PASS" if all(
                r["status"] == "COLLECTED" for r in results
            ) else "BLOCKED",
            "checks": results,
            "scope": "Read-only API connectivity smoke test, "
                     "not full trading integration acceptance."
        }

    else:
        raise ValueError(name)

    evidence = save(data, name)
    emit({"evidence": evidence, "result": data})
    return 1 if data.get("status") in ("FAIL","BLOCKED") else 0
