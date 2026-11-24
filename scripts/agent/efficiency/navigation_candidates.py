import ast
import json
import re
from pathlib import Path
from common import ROOT, CACHE, save

reports = sorted(
    (CACHE / "session-review").glob("review-*.json"),
    key=lambda p: p.stat().st_mtime,
    reverse=True
)

if not reports:
    raise SystemExit("No session review reports found")

report = None
source = None

for path in reports:
    try:
        candidate = json.loads(path.read_text())
        if candidate.get("schema_version", 0) >= 4:
            report, source = candidate, path
            break
    except (OSError, ValueError):
        continue

if report is None:
    raise SystemExit("No schema v4 report found")

def find_navigation(obj):
    if isinstance(obj, dict):
        for key, value in obj.items():
            if (
                key.lower() == "navigation"
                and isinstance(value, list)
            ):
                return value

        for value in obj.values():
            found = find_navigation(value)
            if found:
                return found

    return []

navigation = find_navigation(report)

if not navigation:
    raise SystemExit(
        "Navigation data not found in report schema"
    )

groups = {}

for entry in navigation:
    relative = entry.get("file")
    if not relative:
        continue

    path = (ROOT / relative).resolve()

    if not path.is_relative_to(ROOT):
        continue

    if not path.is_file():
        continue

    if path.stat().st_size > 2000000:
        continue

    if "scout" in relative.lower():
        group = "Scout lifecycle"
    elif "auto_trader/entry/" in relative:
        group = "Entry decisions"
    elif "auto_trader/api" in relative:
        group = "AutoTrader API"
    elif "/learning/" in relative:
        group = "Learning"
    elif "/strategy/" in relative:
        group = "Strategy execution"
    elif "LearningWidget" in relative:
        group = "React learning"
    elif "/react-app/" in relative:
        group = "React outcomes"
    elif "compose" in relative:
        group = "Runtime infrastructure"
    else:
        group = "Other navigation"

    text = path.read_text(errors="replace")

    symbols = re.findall(
        r"\b(?:fn|struct|enum|trait|"
        r"function|class|interface|type)"
        r"\s+([A-Za-z_][A-Za-z0-9_]*)",
        text
    )

    groups.setdefault(group, []).append({
        "file": relative,
        "historical_reads": entry.get("reads"),
        "historical_sessions": entry.get("sessions"),
        "candidate_symbols": list(
            dict.fromkeys(symbols)
        )[:25]
    })

result = {
    "source_report": str(source),
    "status": "REVIEW_REQUIRED",
    "groups": groups,
    "limitations": [
        "File existence checked against current source.",
        "Symbols are lexical candidates.",
        "Responsibilities and relationships unverified.",
        "No automatic Beads memory writes."
    ]
}

print(json.dumps(result, indent=2))
print("REPORT:", save("navigation_candidates", result))
