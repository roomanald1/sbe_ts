from common import parser, request, emit, main
from engine import identifier
from collections import Counter

def run():
    p = parser("Profile a bounded sample, not an entire table")
    p.add_argument("--table", required=True)
    p.add_argument("--column", action="append")
    a = p.parse_args()

    schema = request(
        a.database, "schema", a.file,
        table=a.table, timeout=a.timeout
    )

    if not schema["tables"]:
        raise RuntimeError("Unknown table")

    available = [
        c["name"] for c in schema["tables"][0]["columns"]
    ]

    columns = a.column or available[:10]

    if any(c not in available for c in columns):
        raise RuntimeError("Unknown column")

    sql = (
        "SELECT " +
        ",".join(identifier(c) for c in columns) +
        " FROM " + identifier(a.table) +
        " LIMIT :n"
    )

    n = min(1000, max(1, a.limit))

    data = request(
        a.database, "query", a.file,
        sql=sql, params={"n": n},
        limit=n, timeout=a.timeout
    )

    profiles = []

    for i, name in enumerate(columns):
        values = [r[i] for r in data["rows"]]
        types = Counter(type(v).__name__ for v in values)

        numeric = [
            v for v in values
            if isinstance(v, (int, float))
        ]

        profiles.append({
            "column": name,
            "sample_nulls": sum(v is None for v in values),
            "types": dict(types),
            "sample_min": min(numeric) if numeric else None,
            "sample_max": max(numeric) if numeric else None,
            "examples": values[:3]
        })

    emit({
        "database_alias": a.database,
        "table": a.table,
        "sample_rows": len(data["rows"]),
        "profiles": profiles,
        "note": "Bounded non-random sample; not population statistics"
    }, a.output)

main(run)
