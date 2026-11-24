import argparse
import json
import re
from pathlib import Path
from common import ROOT, run, emit, save, now, changed_files
from repository import impact

def package(plan, number):
    text = Path(plan).read_text()
    headings = list(re.finditer(
        r"(?m)^#{1,4}\s+(?:Section\s+)?(\d+)[.)]\s+.*$",
        text
    ))
    for i,m in enumerate(headings):
        if int(m[1]) == number:
            following = re.search(r"(?m)^## ", text[m.end():])
            end = m.end() + following.start() if following else len(text)
            return text[m.start():end]
    raise ValueError("Work package heading not found")

def main(name):
    p = argparse.ArgumentParser(description=name)
    p.add_argument("--plan")
    p.add_argument("--package", type=int)
    p.add_argument("--input")
    p.add_argument("--budget", type=int)
    p.add_argument("--used", type=int)
    p.add_argument("--term")
    p.add_argument("--max-chars", type=int, default=20000)
    a = p.parse_args()

    if name in ("work_package", "task_context"):
        if not a.plan or not a.package:
            p.error("--plan and --package required")
        section = package(a.plan, a.package)
        data = {
            "package": a.package,
            "plan": a.plan,
            "requirements": section,
            "note": "Section extraction does not replace shared "
                    "invariants, dependencies or acceptance gates."
        }
        if name == "task_context":
            data["change_impact"] = impact()
            data["agent_guidance"] = (
                "Use RTK, verified Beads memories, bounded reads, "
                "existing diagnostic scripts and mandatory tests."
            )
            if a.term:
                memory = run(["bd", "memories", a.term], timeout=10)
                data["memory_search"] = memory.get("output_tail","")

    elif name in ("agent_budget", "context_guard"):
        if a.budget is None or a.used is None:
            p.error("--budget and --used required")
        if a.budget <= 0 or a.used < 0:
            p.error("Invalid budget")
        ratio = a.used/a.budget
        data = {
            "status": "EXCEEDED" if ratio >= 1 else
                      "WARN" if ratio >= .8 else "OK",
            "budget": a.budget,
            "used": a.used,
            "remaining": max(0,a.budget-a.used),
            "utilisation_percent": round(ratio*100,2),
            "note": "Advisory; supplied usage is not independently verified."
        }

    elif name == "checkpoint":
        data = {
            "timestamp": now(),
            "git_head": run(["git","rev-parse","HEAD"]),
            "git_status": run(["git","status","--short"]),
            "changed_files": changed_files()
        }

    elif name == "handoff":
        if not a.input:
            p.error("--input required")
        source = json.loads(Path(a.input).read_text())
        data = {
            "timestamp": now(),
            "handoff": source,
            "changed_files": changed_files(),
            "instruction": "Verify unresolved blockers before continuing."
        }

    elif name == "wave_progress":
        if not a.input:
            p.error("--input required: JSON mapping of package to status")
        states = json.loads(Path(a.input).read_text())
        data = {
            "states": {
                str(i):states.get(str(i),"NOT_RECORDED")
                for i in range(1,17)
            },
            "note": "Reported progress only; not acceptance evidence."
        }

    elif name == "acceptance_check":
        if not a.input:
            p.error("--input required: JSON acceptance evidence")
        evidence = json.loads(Path(a.input).read_text())
        data = {
            "status": "REVIEW_REQUIRED",
            "evidence": evidence,
            "note": "Automatic promotion is prohibited. "
                    "Canonical acceptance criteria require verification."
        }

    else:
        raise ValueError(name)

    path = save(data, name)
    raw = json.dumps(data, indent=2)
    emit({
        "evidence": path,
        "preview": raw[:min(max(a.max_chars,1000),20000)],
        "truncated": len(raw) > min(max(a.max_chars,1000),20000)
    })
    return 1 if data.get("status") == "EXCEEDED" else 0
