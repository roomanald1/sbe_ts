import argparse
from pathlib import Path
from common import registry, discover, request

p = argparse.ArgumentParser()
p.add_argument("database", nargs="?")
p.add_argument("--include-local", action="store_true")
a = p.parse_args()

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
                alias, "health", Path(f["path"]).name
            )

            print(
                "[READABLE]", alias,
                Path(f["path"]).name,
                f"{data['bytes']/1024/1024:,.1f} MiB",
                f"tables={data['tables']}"
            )

    except Exception as e:
        print("[BLOCKED]", alias, str(e))

print("Readability checks only; no full integrity scans.")
