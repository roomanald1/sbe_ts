from pathlib import Path
from common import parser, request, parameters, emit, main

def run():
    p = parser("Execute bounded read-only SQLite query")

    group = p.add_mutually_exclusive_group(required=True)
    group.add_argument("--sql")
    group.add_argument("--sql-file", type=Path)

    p.add_argument("--param", action="append", default=[])
    a = p.parse_args()

    sql = a.sql if a.sql else a.sql_file.read_text()

    data = request(
        a.database, "query", a.file,
        sql=sql,
        params=parameters(a.param),
        limit=a.limit,
        timeout=a.timeout
    )

    emit(data, a.output)

main(run)
