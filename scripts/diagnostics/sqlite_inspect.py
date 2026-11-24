#!/usr/bin/env python3
import argparse
import sqlite3
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("database", type=Path)
p.add_argument("--table")
p.add_argument("--limit", type=int, default=50)
a = p.parse_args()

if not a.database.is_file():
    raise SystemExit("Database does not exist")

with sqlite3.connect(
    a.database.resolve().as_uri() + "?mode=ro",
    uri=True, timeout=5
) as conn:
    conn.execute("PRAGMA query_only=ON")
    tables = [
        r[0] for r in conn.execute(
            "SELECT name FROM sqlite_master "
            "WHERE type IN ('table','view') ORDER BY name"
        )
    ]
    if a.table:
        if a.table not in tables:
            raise SystemExit("Unknown table")
        name = '"' + a.table.replace('"', '""') + '"'
        print("COLUMNS")
        for row in conn.execute(f"PRAGMA table_info({name})"):
            print(row)
        print("\nINDEXES")
        for row in conn.execute(f"PRAGMA index_list({name})"):
            print(row)
    else:
        print(f"Tables/views: {len(tables)}")
        for table in tables[:a.limit]:
            print(table)
