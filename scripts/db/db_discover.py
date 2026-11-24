import argparse
from common import registry, discover

p = argparse.ArgumentParser()
p.add_argument("database", nargs="?")
a = p.parse_args()

for alias, entry in registry().items():
    if a.database and alias != a.database:
        continue

    print("\n" + alias)
    print("  Source:", entry.get("container", "local"))

    try:
        files = discover(alias)
        for f in files:
            size = f["bytes"]
            label = (
                f"{size/1024/1024:,.1f} MiB"
                if size is not None else "MISSING"
            )
            print(" ", f["path"], "|", label)

        if not files:
            print("  No database files discovered")

    except Exception as e:
        print("  BLOCKED:", str(e))
