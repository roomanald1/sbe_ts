from common import parser, request, emit, main

def run():
    p = parser("Inspect SQLite schema and indexes")
    p.add_argument("--table")
    p.add_argument("--full", action="store_true")
    a = p.parse_args()

    data = request(
        a.database, "schema", a.file,
        timeout=a.timeout, table=a.table
    )

    if not a.full and not a.table:
        data["tables"] = [
            {
                "name": t["name"],
                "type": t["type"],
                "columns": [c["name"] for c in t["columns"]],
                "indexes": [i["name"] for i in t["indexes"]]
            }
            for t in data["tables"]
        ]

    emit(data, a.output)

main(run)
