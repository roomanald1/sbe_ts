import argparse
import fcntl
import json
import time
from pathlib import Path
from common import EVIDENCE, run, emit, save, now, collect, compact
from trading import report

def main(name):
    p = argparse.ArgumentParser(description=name)
    p.add_argument("--baseline")
    p.add_argument("--current")
    p.add_argument("--retention-days", type=int, default=14)
    p.add_argument("--prune", action="store_true")
    a = p.parse_args()

    with (EVIDENCE/"monitor.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            emit({"status":"SKIPPED", "reason":"Monitor already running"})
            return 1

        if a.prune:
            if not 1 <= a.retention_days <= 365:
                p.error("Invalid retention period")
            cutoff = time.time()-a.retention_days*86400
            removed = 0
            for file in EVIDENCE.iterdir():
                if (
                    file.is_file() and not file.is_symlink()
                    and file.suffix in (".json",".log")
                    and file.stat().st_mtime < cutoff
                ):
                    file.unlink()
                    removed += 1
            print("Expired evidence files removed:", removed)

        if name == "resource_monitor":
            data = {
                "memory": run(["free","-h"], timeout=5),
                "disk": run(["df","-h","."], timeout=5),
                "docker": run([
                    "docker","stats","--no-stream",
                    "--format",
                    "{{.Name}} {{.CPUPerc}} {{.MemUsage}}"
                ], timeout=15)
            }

        elif name in ("system_snapshot", "paper_soak_monitor"):
            data = report("trading_health")
            if name == "paper_soak_monitor":
                data["soak_status"] = "MODE_AND_BASELINE_NOT_VERIFIED"

        elif name == "learning_progress":
            data = report("learning_readiness")

        elif name == "feedback_monitor":
            data = report("feedback_health")

        elif name == "strategy_daily_report":
            data = report("strategy_scorecard")
            data["note"] = "Daily collection; source time scope unverified."

        elif name == "data_freshness":
            sources = [
                collect("learning","summary"),
                collect("autotrader","feedback-queues"),
                collect("positions","account")
            ]
            data = {
                "sources":[compact(s) for s in sources],
                "status":"REVIEW_REQUIRED",
                "note":"Collection timestamps are not source-data freshness."
            }

        elif name == "baseline_compare":
            if not a.baseline or not a.current:
                p.error("--baseline and --current required")
            old = json.loads(Path(a.baseline).read_text())
            new = json.loads(Path(a.current).read_text())
            data = {
                "status":"REVIEW_REQUIRED",
                "changed_fields":[
                    k for k in sorted(set(old)|set(new))
                    if old.get(k)!=new.get(k)
                ],
                "note":"Structural comparison; no financial promotion."
            }

        else:
            raise ValueError(name)

        result = {
            "monitor":name,
            "timestamp":now(),
            "data":data
        }
        path = save(result,name)
        emit({"evidence":path,"result":result})
        return 1 if (
            data.get("status") == "BLOCKED"
            or data.get("analysis_status","").startswith("BLOCKED")
        ) else 0
