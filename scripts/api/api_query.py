import time
import argparse
import json
import sys
from common import query, emit, shape, field

p = argparse.ArgumentParser()
p.add_argument("service")
p.add_argument("endpoint")
p.add_argument("--output")
p.add_argument("--max-bytes", type=int, default=4000000)
p.add_argument("--fields")
p.add_argument("--filter", action="append", default=[])
p.add_argument("--sort")
p.add_argument("--descending", action="store_true")
p.add_argument("--limit", type=int)
p.add_argument("--select")
p.add_argument("--summary", action="store_true")
p.add_argument(
    "--hours",
    type=int,
    choices=[1, 6, 24, 168, 720]
)
p.add_argument("--param", action="append", default=[], metavar="NAME=VALUE",
               help="Read-only route query parameter (currently positions.trades only)")
a = p.parse_args()

since = (
    (int(time.time()) // 300 * 300 - a.hours * 3600) * 1000
    if a.hours is not None else None
)

try:
    params = {}
    for item in a.param:
        name, sep, raw_value = item.partition("=")
        if not sep or not name or name in params:
            raise RuntimeError("Use unique --param NAME=VALUE entries")
        try:
            params[name] = json.loads(raw_value)
        except ValueError:
            params[name] = raw_value
    data = query(a.service, a.endpoint, a.max_bytes, since, params or None)
    body = data["body"]

    if a.select:
        body = field(body, a.select)

    original_count = len(body) if isinstance(body, list) else None

    if a.filter or a.sort or a.limit is not None:
        if not isinstance(body, list):
            raise RuntimeError(
                "Filtering, sorting and limits require an array; "
                "use --select for a nested array"
            )

        for expression in a.filter:
            key, sep, expected = expression.partition("=")

            if not sep:
                raise RuntimeError("Use --filter FIELD=VALUE")

            def matches(row):
                try:
                    value = field(row, key)
                except KeyError:
                    return False

                if isinstance(value, (bool, int, float)) or value is None:
                    return json.dumps(value) == expected

                return str(value) == expected

            body = [row for row in body if matches(row)]

        if a.sort:
            present = []
            missing = []

            for row in body:
                try:
                    value = field(row, a.sort)
                except KeyError:
                    value = None

                if value is None:
                    missing.append(row)
                else:
                    present.append((value, row))

            present.sort(
                key=lambda pair: pair[0],
                reverse=a.descending
            )

            body = [row for _, row in present] + missing

        if a.limit is not None:
            if not 0 <= a.limit <= 10000:
                raise RuntimeError("Limit must be 0-10000")
            body = body[:a.limit]

    if a.fields:
        keys = a.fields.split(",")

        def project(row):
            return {key: field(row, key) for key in keys}

        body = (
            [project(row) for row in body]
            if isinstance(body, list)
            else project(body)
        )

    returned_count = len(body) if isinstance(body, list) else None

    result = {
        "service": data["service"],
        "route": data["route"],
        "application_status": data["application_status"],
        "elapsed_ms": data["elapsed_ms"],
        "response_bytes": data["bytes"],
        "original_records": original_count,
        "returned_records": returned_count,
        "body": shape(body) if a.summary else body
    }

    emit(result, a.output)

except Exception as e:
    print("BLOCKED:", str(e), file=sys.stderr)
    sys.exit(1)
