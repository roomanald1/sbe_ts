from pathlib import Path
from common import parser, request, parameters, emit, main

def run():
    p = parser("Inspect SQLite query plan without executing query")

    group = p.add_mutually_exclusive_group(required=True)
    group.add_argument("--sql")
    group.add_argument("--sql-file", type=Path)

    p.add_argument("--param", action="append", default=[])
    a = p.parse_args()

    sql = a.sql if a.sql else a.sql_file.read_text()

    data = request(
        a.database, "explain", a.file,
        sql=sql,
        params=parameters(a.param),
        timeout=a.timeout,
        limit=a.limit
    )

    data["possible_scans"] = [
        row for row in data["rows"]
        if "SCAN" in str(row).upper()
    ]

    emit(data, a.output)

main(run)
