import argparse
import ast
import json
import re
from common import ROOT, save

p = argparse.ArgumentParser()
p.add_argument("query", nargs="*", default=[])
p.add_argument("--limit", type=int, default=12)
p.add_argument("--save", action="store_true")
a = p.parse_args()

entries = []

for directory in [
    "scripts/local", "scripts/diagnostics",
    "scripts/api", "scripts/db", "scripts/agent"
]:
    for path in (ROOT / directory).rglob("*.py"):
        if any(x in path.parts for x in
               ("__pycache__", "backups", "vendor")):
            continue

        try:
            if path.stat().st_size > 1000000:
                continue
            tree = ast.parse(path.read_text())
        except (OSError, SyntaxError, UnicodeError):
            continue

        options = set()
        functions = []

        for node in ast.walk(tree):
            if isinstance(node, ast.FunctionDef):
                functions.append(node.name)

            if (
                isinstance(node, ast.Call)
                and isinstance(node.func, ast.Attribute)
                and node.func.attr == "add_argument"
            ):
                for arg in node.args:
                    if (
                        isinstance(arg, ast.Constant)
                        and isinstance(arg.value, str)
                    ):
                        options.add(arg.value)

        entries.append({
            "path": str(path.relative_to(ROOT)),
            "description": (ast.get_docstring(tree)
                            or "")[:500],
            "options": sorted(options),
            "symbols": sorted(set(functions))[:30]
        })

words = re.findall(
    r"[a-z0-9]+", " ".join(a.query).lower()
)

ranked = []

for entry in entries:
    haystack = json.dumps(entry).lower()
    score = sum(word in haystack for word in words)

    if score or not words:
        ranked.append((score, entry))

ranked.sort(
    key=lambda x: (-x[0], x[1]["path"])
)

print(json.dumps({
    "indexed_tools": len(entries),
    "matches": [
        entry for _, entry in ranked[:a.limit]
    ],
    "limitations": [
        "Static inventory; not runtime validation.",
        "Inherited or dynamic arguments may be absent.",
        "Confirm actual interfaces using --help."
    ]
}, indent=2))

if a.save:
    print("CATALOGUE:", save("tool_catalog", entries))
