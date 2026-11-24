import argparse
import hashlib
import json
import re
import subprocess
import tempfile
from datetime import datetime, timezone
from pathlib import Path

from common import ROOT
from orchestration import package

DEFAULT_PLAN = (
    "src/rust/auto_trader/TRADING_IMPROVEMENTS.MD"
)

CACHE = Path.home() / ".cache/sbe-diagnostics"

# Advisory navigation hints, not ownership assignments.
# Actual requirements and dependencies come from the plan.
MAP = {
    1: ("rust_worker", ["auto_trader", "insights", "learning"],
        ["entry", "candidate", "gate", "explor"],
        ["learning_readiness", "profile_inventory", "trading_health"]),
    2: ("financial", ["position_management", "auto_trader", "paper_exchange"],
        ["fee", "profit", "stop", "reconcil"],
        ["financial_snapshot", "financial_reconciliation", "fee_analysis"]),
    3: ("quantitative", ["position_management", "insights", "learning"],
        ["realized", "outcome", "performance", "return"],
        ["realized_pnl_analysis", "financial_reconciliation"]),
    4: ("architect", ["learning", "auto_trader", "insights"],
        ["feedback", "scout", "replay", "audit"],
        ["feedback_analysis", "queue_source_trace", "profile_inventory"]),
    5: ("architect", ["auto_trader", "strategy", "position_management"],
        ["allocation", "capital", "capacity", "timeframe"],
        ["position_analysis", "account_analysis", "trading_health"]),
    6: ("integration", ["insights", "auto_trader", "learning"],
        ["monitoring", "dashboard", "reporting", "metrics", "audit"],
        ["contract_check", "trading_health", "feedback_analysis"]),
    7: ("rust_worker", ["auto_trader", "position_management", "paper_exchange"],
        ["maker", "revalid", "resting", "execution"],
        ["fee_analysis", "financial_snapshot"]),
    8: ("rust_worker", ["auto_trader", "learning"],
        ["cooldown", "explor", "candidate", "feedback"],
        ["feedback_analysis", "learning_readiness"]),
    9: ("architect", ["auto_trader", "strategy"],
        ["correlation", "portfolio", "risk", "allocation"],
        ["position_analysis", "account_analysis"]),
    10: ("quantitative", ["learning", "strategy", "auto_trader"],
         ["edge", "return", "profit", "ranking"],
         ["learning_readiness", "profile_inventory", "fee_analysis"]),
    11: ("quantitative", ["learning", "strategy", "auto_trader"],
         ["threshold", "parameter", "filter", "optim"],
         ["learning_readiness", "profile_inventory"]),
    12: ("quantitative", ["auto_trader", "learning", "position_management"],
         ["exit", "holding", "duration", "outcome"],
         ["realized_pnl_analysis", "fee_analysis"]),
    13: ("architect", ["learning", "auto_trader", "insights"],
         ["profile", "promotion", "evidence", "sample", "scout"],
         ["learning_readiness", "profile_inventory", "feedback_analysis"]),
    14: ("quantitative", ["learning", "auto_trader"],
         ["explor", "scout", "opportunity"],
         ["profile_inventory", "feedback_analysis"]),
    15: ("quantitative", ["strategy", "auto_trader", "learning"],
         ["volatility", "stop", "barrier", "threshold"],
         ["learning_readiness", "fee_analysis"]),
    16: ("architect", ["auto_trader", "position_management", "paper_exchange"],
         ["stop", "maker", "barrier", "execution"],
         ["fee_analysis", "financial_reconciliation"]),
}

def git(*args):
    return subprocess.check_output(
        ["git", *args], cwd=ROOT, timeout=30
    )

def digest(data):
    return hashlib.sha256(data).hexdigest()

def write(path, content):
    path.write_text(content, encoding="utf-8")

def write_json(path, data):
    write(path, json.dumps(data, indent=2) + "\n")

def sections(text):
    matches = list(re.finditer(r"(?m)^## (.+)$", text))
    result = {}
    for i, match in enumerate(matches):
        end = matches[i+1].start() if i+1 < len(matches) else len(text)
        heading = match.group(1)
        if heading in result:
            raise ValueError("Duplicate heading: " + heading)
        result[heading] = text[match.start():end].strip()
    return result

def evidence_refs(prefixes):
    result = []
    for prefix in prefixes:
        files = list(CACHE.glob(prefix + "-*.json"))
        if not files:
            result.append({
                "type": prefix,
                "status": "MISSING"
            })
            continue

        path = max(files, key=lambda p: p.stat().st_mtime)
        stat = path.stat()
        result.append({
            "type": prefix,
            "status": "AVAILABLE_NOT_ACCEPTANCE_EVIDENCE",
            "path": str(path),
            "bytes": stat.st_size,
            "sha256": digest(path.read_bytes()),
            "modified_at": datetime.fromtimestamp(
                stat.st_mtime, timezone.utc
            ).isoformat(),
            "age_seconds_at_generation": round(
                datetime.now(timezone.utc).timestamp() - stat.st_mtime
            )
        })
    return result

def source_map(files, crates, terms, number):
    candidates = []

    for name in files:
        if "/vendor/" in name or "/sbe_out/" in name:
            continue

        rust = any(
            name.startswith("src/rust/" + crate + "/")
            for crate in crates
        )

        ui = number in (6, 13) and (
            name.startswith(
                "src/typescript/workspace/apps/react-app/src/"
            )
            or name.startswith(
                "src/typescript/workspace/packages/ui-platform/src/"
            )
        )

        if not (rust or ui):
            continue

        if not name.endswith((".rs", ".ts", ".tsx")):
            continue

        score = sum(
            term in name.lower()
            for term in terms
        )

        if score:
            candidates.append((score, name))

    candidates.sort(key=lambda item: (-item[0], item[1]))
    return [name for _, name in candidates[:35]]


def symbol_map(crates, terms, number):
    scopes = ["src/rust/" + c for c in crates]

    if number in (6, 13):
        scopes += [
            "src/typescript/workspace/apps/react-app/src",
            "src/typescript/workspace/packages/ui-platform/src"
        ]

    extra = {
        3: ["closed_trade", "realized_pnl", "trade_outcome",
            "win_rate", "profit_factor"],
        5: ["budget", "allocation", "capacity", "capital",
            "position_size", "reservation"],
        10: ["net_edge", "net_return_lcb", "expected_return",
             "expected_profit", "lower_bound"],
        14: ["exploration", "exploratory", "scout",
             "missed_opportunity", "risk_scale"]
    }

    keywords = sorted(set(terms + extra.get(number, [])))

    pattern = (
        r"\b(fn|function)[[:space:]]+[a-zA-Z0-9_]*("
        + "|".join(re.escape(t) for t in keywords)
        + r")[a-zA-Z0-9_]*"
    )

    result = subprocess.run(
        ["git", "grep", "-n", "-I", "-E", pattern,
         "--", *scopes],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=30
    )

    if result.returncode not in (0, 1):
        raise RuntimeError(result.stderr[:1000])

    matches = []

    for line in result.stdout.splitlines():
        if "/vendor/" in line or "/sbe_out/" in line:
            continue

        parts = line.split(":", 2)
        if len(parts) != 3:
            continue

        path, lineno, source = parts
        if not lineno.isdigit():
            continue

        if not path.endswith((".rs", ".ts", ".tsx")):
            continue

        match = re.search(
            r"\b(?:fn|function)\s+([A-Za-z0-9_]+)",
            source
        )
        if not match:
            continue

        symbol = match.group(1)
        score = sum(t in symbol.lower() for t in keywords)

        matches.append({
            "path": path,
            "line": int(lineno),
            "symbol": symbol,
            "score": score,
            "declaration": source.strip()[:240]
        })

    matches.sort(key=lambda m: (
        -m["score"], m["path"], m["line"]
    ))

    # Avoid one large source file dominating the map.
    counts = {}
    selected = []

    for item in matches:
        path = item["path"]
        if counts.get(path, 0) >= 6:
            continue

        counts[path] = counts.get(path, 0) + 1
        selected.append(item)

        if len(selected) >= 45:
            break

    return selected


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--plan", default=DEFAULT_PLAN)
    args = parser.parse_args()

    plan = Path(args.plan)
    if not plan.is_absolute():
        plan = ROOT / plan

    raw = plan.read_bytes()
    text = raw.decode("utf-8-sig")
    plan_hash = digest(raw)
    blocks = sections(text)

    numbered = {}
    for heading, block in blocks.items():
        match = re.match(r"(\d+)\.\s+", heading)
        if match:
            number = int(match.group(1))
            if number in numbered:
                raise ValueError("Duplicate package number")
            numbered[number] = (heading, block)

    if set(numbered) != set(range(1, 17)):
        raise ValueError("Expected exactly packages 1 through 16")

    required = [
        "Objective",
        "Step 0",
        "Operational ownership",
        "Mandatory cross-system",
        "Shared hot-path",
        "Phase 0",
        "Execution waves",
        "Delivery and acceptance"
    ]

    shared = []
    for prefix in required:
        matches = [
            block for heading, block in blocks.items()
            if heading.startswith(prefix)
        ]
        if len(matches) != 1:
            raise ValueError("Missing or ambiguous policy: " + prefix)
        shared.append(matches[0])

    shared_text = "\n\n".join(shared) + "\n"

    # Cross-check existing extractor, including WP16.
    for number, (_, block) in numbered.items():
        if package(plan, number).strip() != block:
            raise ValueError("Extractor mismatch: WP" + str(number))

    head = git("rev-parse", "HEAD").decode().strip()
    files = git("ls-files", "-z").decode().split("\0")
    status = git("status", "--porcelain=v1", "-z")

    parent = CACHE / "work-packages"
    parent.mkdir(parents=True, exist_ok=True)

    output = Path(tempfile.mkdtemp(
        prefix="run-" + plan_hash[:12] + "-",
        dir=parent
    ))

    write(output / "master-plan.md", text)
    write(output / "shared-invariants.md", shared_text)

    manifest = {
        "status": "PREPARATION_IN_PROGRESS",
        "plan": str(plan),
        "plan_title": text.splitlines()[0],
        "plan_sha256": plan_hash,
        "git_head": head,
        "git_status_sha256": digest(status),
        "dirty_worktree": bool(status),
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "packages": [],
        "implementation_authorized": False
    }

    for number in range(1, 17):
        heading, requirements = numbered[number]
        role, crates, terms, evidence = MAP[number]

        folder = output / f"WP{number:02d}"
        folder.mkdir()

        write(folder / "requirements.md", requirements + "\n")

        paths = source_map(files, crates, terms, number)

        write(
            folder / "source-map.md",
            "# Candidate source locations\n\n"
            "Filename-based navigation hints only. "
            "Verify symbols, ownership and current code before editing.\n\n"
            + "\n".join("- `" + p + "`" for p in paths)
            + "\n"
        )


        symbols = symbol_map(crates, terms, number)

        write_json(folder / "symbol-map.json", symbols)

        with (folder / "source-map.md").open(
            "a", encoding="utf-8"
        ) as f:
            f.write("\n## Observed function declarations\n\n")
            f.write(
                "These are static declaration matches, not "
                "verified call paths or behavioural contracts.\n\n"
            )
            for item in symbols:
                f.write(
                    f"- `{item['path']}:{item['line']}` "
                    f"— `{item['symbol']}`\n"
                )

        refs = evidence_refs(evidence)
        write_json(folder / "baseline.json", refs)

        tests = [
            "# Advisory test selection",
            "",
            "Verify supported just recipes before execution.",
            "Use one build owner. Run tests sequentially.",
            "Add targeted deterministic regression tests first.",
            ""
        ]

        for crate in crates:
            tests.append("- `just test " + crate + "`")

        if number in (6, 13):
            tests.append("- `just typecheck`")
            tests.append("- Verify rendered UI and API contracts.")

        tests.extend([
            "- `just diff-check`",
            "",
            "Apply all relevant mandatory integration and "
            "cumulative wave gates from shared-invariants.md.",
            "This list is not an exhaustive acceptance test suite."
        ])

        write(folder / "test-plan.md", "\n".join(tests) + "\n")

        instructions = f"""# Agent instructions — WP{number:02d}

## Task
{heading}

Suggested initial specialist: {role}.
Use the verified coordinator and delegation policy.
Role suggestion does not override ENG/QUANT ownership.

## Read first
- requirements.md
- ../shared-invariants.md
- source-map.md
- baseline.json
- test-plan.md
- Repository AGENTS.md

## Execution constraints
- Step 0 and applicable preceding wave gates are mandatory.
- Do not start implementation merely because this bundle exists.
- WP16 remains deferred unless explicitly authorised.
- Preserve existing Paper execution and off-exchange scouting.
- No live trading activation or new shadow/paper engine.
- Use RTK and existing SQL, RSocket and local diagnostic tools.
- Reuse confirmed Beads memories with bd remember for new discoveries.
- Inspect targeted source locations rather than broad repository dumps.
- Verify evidence age, mode, scope, currency and ledger identity.
- Do not treat baseline snapshots as independent reconciliation.
- Keep raw evidence on disk.
- Do not bypass mandatory tests to save tokens.
- Do not commit, deploy or alter trading settings without authorisation.

## Context policy
Target initial context: 5,000–11,000 tokens where feasible.
This is advisory, not a hard limit.
Never truncate mandatory requirements or acceptance criteria.
Use the plan's verified runtime context and delegation policy.

## Handoff
Report changed files, tests and results, evidence paths,
unresolved blockers, performance impact and acceptance status.
Do not claim acceptance from a dry run or static inspection.
"""

        write(folder / "agent-instructions.md", instructions)

        write(
            folder / "acceptance.md",
            "# Acceptance status: NOT_EVALUATED\n\n"
            "Apply the exact work-package requirements and "
            "applicable canonical gates in ../shared-invariants.md.\n\n"
            "- [ ] Prerequisite waves and ownership verified\n"
            "- [ ] Baseline scope and freshness verified\n"
            "- [ ] Requirements implemented and reviewed\n"
            "- [ ] Targeted regression tests passed\n"
            "- [ ] Applicable integration gates passed\n"
            "- [ ] Financial and performance invariants preserved\n"
            "- [ ] Coordinator acceptance recorded\n"
        )

        manifest["packages"].append({
            "number": number,
            "heading": heading,
            "directory": str(folder),
            "suggested_specialist": role,
            "requirements_sha256": digest(
                requirements.encode("utf-8")
            ),
            "source_candidates": len(paths),
            "symbol_candidates": len(symbols),
            "evidence_references": len(refs),
            "missing_evidence": sum(
                r["status"] == "MISSING" for r in refs
            ),
            "status": "DEFERRED" if number == 16 else "PREPARED",
            "acceptance": "NOT_EVALUATED"
        })

    # Detect changes during generation.
    if digest(plan.read_bytes()) != plan_hash:
        raise ValueError("Plan changed during generation")

    if git("rev-parse", "HEAD").decode().strip() != head:
        raise ValueError("Git HEAD changed during generation")

    manifest["status"] = "PREPARED"
    manifest["shared_policy_bytes"] = len(shared_text.encode())
    manifest["package_count"] = len(manifest["packages"])

    write_json(output / "manifest.json", manifest)

    print("BUNDLES:", output)
    print("PLAN:", manifest["plan_title"])
    print("PLAN SHA256:", plan_hash)
    print("GIT HEAD:", head)
    print("SHARED POLICY BYTES:", manifest["shared_policy_bytes"])
    print()

    for item in manifest["packages"]:
        print(
            f"WP{item['number']:02d}",
            item["status"],
            "|", item["suggested_specialist"],
            "| sources:", item["source_candidates"],
            "| symbols:", item["symbol_candidates"],
            "| evidence:", item["evidence_references"],
            "| missing:", item["missing_evidence"]
        )

    print()
    print("GENERATED:", len(manifest["packages"]), "packages")
    print("MANIFEST:", output / "manifest.json")
    print("ACCEPTANCE: NOT_EVALUATED")
    print("IMPLEMENTATION AUTHORIZED: NO")

if __name__ == "__main__":
    main()
