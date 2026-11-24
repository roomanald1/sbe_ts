import argparse
import json
import re
from pathlib import Path
from common import HERE, request, parameters, emit, main

CATALOG = HERE / "queries"

def run():
    p = argparse.ArgumentParser()
    p.add_argument("action", choices=["list", "show", "run"])
    p.add_argument("name", nargs="?")
    p.add_argument("--param", action="append", default=[])
    p.add_argument("--limit", type=int, default=20)
    p.add_argument("--timeout", type=int, default=5)
    p.add_argument("--output", type=Path)
    a = p.parse_args()

    if a.action == "list":
        for path in sorted(CATALOG.glob("*.json")):
            data = json.loads(path.read_text())
            print(
                path.stem,
                "|", data.get("database"),
                "|", data.get("description")
            )
        return

    if not a.name or not re.fullmatch(
        r"[A-Za-z0-9_-]+", a.name
    ):
        raise RuntimeError("Invalid query name")

    path = CATALOG / (a.name + ".json")
    data = json.loads(path.read_text())

    if a.action == "show":
        emit(data)
        return

    if not data.get("verified"):
        raise RuntimeError("Query has not been verified")

    if not data.get("verification"):
        raise RuntimeError("Missing verification evidence")

    params = parameters(a.param)

    required = set(data.get("required_parameters", []))

    if not required.issubset(params):
        raise RuntimeError(
            "Missing parameters: " +
            ", ".join(sorted(required-set(params)))
        )

    result = request(
        data["database"], "query", data.get("file"),
        sql=data["sql"],
        params=params,
        timeout=a.timeout,
        limit=a.limit
    )

    result["catalogue_query"] = a.name
    result["verification"] = data["verification"]

    emit(result, a.output)

main(run)
