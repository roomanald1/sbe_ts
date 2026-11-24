import argparse
from pathlib import Path
from common import registry, discover, request, emit

p = argparse.ArgumentParser()
p.add_argument("keyword")
p.add_argument("--database")
p.add_argument("--include-local", action="store_true")
p.add_argument("--limit", type=int, default=30)
a = p.parse_args()

results = []
errors = []
seen = set()

for alias, entry in registry().items():
    if a.database and alias != a.database:
        continue

    if "path" in entry and not (
        a.include_local or a.database
    ):
        continue

    try:
        for f in discover(alias):
            identity = (
                entry.get("container", "local"), f["path"]
            )

            if identity in seen:
                continue

            seen.add(identity)

            data = request(
                alias, "schema",
                Path(f["path"]).name
            )

            for table in data["tables"]:
                cols = [
                    c["name"] for c in table["columns"]
                    if a.keyword.lower() in c["name"].lower()
                ]

                if cols or a.keyword.lower() in table["name"].lower():
                    results.append({
                        "database": alias,
                        "file": Path(f["path"]).name,
                        "table": table["name"],
                        "matching_columns": cols
                    })

    except Exception as e:
        errors.append({
            "database": alias,
            "error": str(e)
        })

emit({
    "matches": len(results),
    "results": results[:a.limit],
    "blocked_sources": errors
})
