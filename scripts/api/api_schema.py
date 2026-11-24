import time
import argparse
import sys
from common import query, shape, emit

p = argparse.ArgumentParser()
p.add_argument("service")
p.add_argument("endpoint")
p.add_argument("--max-bytes", type=int, default=4000000)
p.add_argument("--output")
p.add_argument(
    "--hours",
    type=int,
    choices=[1, 6, 24, 168, 720]
)
a = p.parse_args()

since = (
    (int(time.time()) // 300 * 300 - a.hours * 3600) * 1000
    if a.hours is not None else None
)

try:
    data = query(a.service, a.endpoint, a.max_bytes, since)

    emit({
        "service": a.service,
        "route": data["route"],
        "application_status": data["application_status"],
        "bytes": data["bytes"],
        "body": shape(data["body"])
    }, a.output)

except Exception as e:
    print("BLOCKED:", str(e), file=sys.stderr)
    sys.exit(1)
