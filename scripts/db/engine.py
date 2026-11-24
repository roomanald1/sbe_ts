import sqlite3
import json
import sys
import time
from pathlib import Path

def identifier(name):
    return '"' + name.replace('"', '""') + '"'

def execute(request):
    operation = request["operation"]
    path = Path(request["path"])

    if operation == "discover":
        files = []
        for p in sorted(path.glob("*")):
            if p.is_file() and p.suffix.lower() in (
                ".db", ".sqlite", ".sqlite3"
            ):
                files.append({
                    "path": str(p),
                    "bytes": p.stat().st_size
                })
        return {"files": files}

    if not path.is_file():
        raise RuntimeError("Database not found: " + str(path))

    timeout = min(30, max(1, request.get("timeout", 5)))
    limit = min(10000, max(1, request.get("limit", 20)))
    deadline = time.monotonic() + timeout

    conn = sqlite3.connect(
        path.resolve().as_uri() + "?mode=ro",
        uri=True,
        timeout=min(timeout, 2)
    )

    try:
        conn.row_factory = sqlite3.Row
        conn.execute("PRAGMA query_only=ON")
        conn.execute("PRAGMA trusted_schema=OFF")

        conn.set_progress_handler(
            lambda: int(time.monotonic() > deadline), 1000
        )

        if hasattr(conn, "setlimit"):
            conn.setlimit(sqlite3.SQLITE_LIMIT_LENGTH, 1000000)
            conn.setlimit(sqlite3.SQLITE_LIMIT_SQL_LENGTH, 100000)

        if operation == "health":
            version = conn.execute(
                "SELECT sqlite_version()"
            ).fetchone()[0]

            tables = conn.execute(
                "SELECT count(*) FROM sqlite_master "
                "WHERE type='table'"
            ).fetchone()[0]

            return {
                "path": str(path),
                "bytes": path.stat().st_size,
                "sqlite_version": version,
                "tables": tables,
                "status": "READABLE"
            }

        if operation == "schema":
            tables = conn.execute(
                "SELECT name, type FROM sqlite_master "
                "WHERE type IN ('table','view') "
                "ORDER BY name"
            ).fetchall()

            result = []

            for row in tables:
                name = row["name"]

                if request.get("table") and \
                   name != request["table"]:
                    continue

                columns = [
                    dict(r) for r in conn.execute(
                        "PRAGMA table_info(" +
                        identifier(name) + ")"
                    )
                ]

                indexes = [
                    dict(r) for r in conn.execute(
                        "PRAGMA index_list(" +
                        identifier(name) + ")"
                    )
                ]

                for index in indexes:
                    index["columns"] = [
                        dict(r) for r in conn.execute(
                            "PRAGMA index_info(" +
                            identifier(index["name"]) + ")"
                        )
                    ]

                result.append({
                    "name": name,
                    "type": row["type"],
                    "columns": columns,
                    "indexes": indexes
                })

            return {"tables": result}

        # Only read operations are authorised for user SQL.
        allowed = {
            sqlite3.SQLITE_SELECT,
            sqlite3.SQLITE_READ,
            sqlite3.SQLITE_FUNCTION,
        }

        if hasattr(sqlite3, "SQLITE_RECURSIVE"):
            allowed.add(sqlite3.SQLITE_RECURSIVE)

        blocked_functions = {
            "load_extension", "writefile", "readfile",
            "fts3_tokenizer", "eval"
        }

        def authorizer(action, arg1, arg2, db, source):
            if action not in allowed:
                return sqlite3.SQLITE_DENY

            if action == sqlite3.SQLITE_FUNCTION:
                function = (arg2 or arg1 or "").lower()
                if function in blocked_functions:
                    return sqlite3.SQLITE_DENY

            return sqlite3.SQLITE_OK

        conn.set_authorizer(authorizer)

        sql = request["sql"]
        params = request.get("params", {})

        if operation == "explain":
            sql = "EXPLAIN QUERY PLAN " + sql

        start = time.monotonic()
        cursor = conn.execute(sql, params)

        if cursor.description is None:
            raise RuntimeError("Query returned no result columns")

        columns = [d[0] for d in cursor.description]
        rows = []
        truncated = False
        cell_truncations = 0
        result_bytes = 0

        for i, row in enumerate(cursor):
            if i >= limit:
                truncated = True
                break

            values = []

            for value in row:
                if isinstance(value, bytes):
                    value = {
                        "blob_bytes": len(value),
                        "omitted": True
                    }
                    cell_truncations += 1

                elif isinstance(value, str) and len(value) > 1000:
                    value = value[:1000] + "...[truncated]"
                    cell_truncations += 1

                values.append(value)

            size = len(json.dumps(values).encode())

            if result_bytes + size > 4000000:
                truncated = True
                break

            result_bytes += size
            rows.append(values)

        return {
            "columns": columns,
            "rows": rows,
            "returned_rows": len(rows),
            "truncated": truncated,
            "cell_truncations": cell_truncations,
            "elapsed_ms": round(
                (time.monotonic()-start)*1000, 2
            )
        }

    finally:
        conn.close()

if __name__ == "__main__":
    try:
        request = json.load(sys.stdin)
        print(json.dumps(execute(request)))
    except Exception as e:
        print(json.dumps({
            "error": str(e),
            "status": "BLOCKED"
        }))
        sys.exit(1)
