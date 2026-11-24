import argparse
import json
import sys
import time
from pathlib import Path
from common import catalogue, query, shape

p = argparse.ArgumentParser()
p.add_argument("--service")
p.add_argument("--include-large", action="store_true")
p.add_argument("--max-bytes", type=int, default=4000000)
p.add_argument("--output")
a = p.parse_args()

large = {
    ("learning", "decision-snapshot"),
    ("autotrader", "monitoring-metrics")
}

results = []

for service, endpoints in catalogue().items():
    if a.service and a.service != service:
        continue

    for endpoint, route in endpoints.items():
        item = {
            "service": service,
            "endpoint": endpoint,
            "route": route
        }

        if (service, endpoint) in large and not a.include_large:
            item["status"] = "SKIPPED_LARGE"

        else:
            try:
                data = query(service, endpoint, a.max_bytes)

                item.update({
                    "status": "PASS",
                    "application_status": data["application_status"],
                    "elapsed_ms": data["elapsed_ms"],
                    "bytes": data["bytes"],
                    "response_shape": shape(data["body"])
                })

            except Exception as e:
                item.update({
                    "status": "BLOCKED",
                    "error": str(e)[:500]
                })

        print(
            f"{item['status']:15} {service:12} {endpoint:25}",
            item.get("bytes", ""),
            item.get("error", "")
        )

        results.append(item)
        time.sleep(0.25)

passed = sum(r["status"] == "PASS" for r in results)
blocked = sum(r["status"] == "BLOCKED" for r in results)
skipped = sum(r["status"] == "SKIPPED_LARGE" for r in results)

print("\n=== VALIDATION SUMMARY ===")
print("Passed:", passed)
print("Blocked:", blocked)
print("Skipped:", skipped)

if a.output:
    with Path(a.output).open("x") as f:
        json.dump(results, f, indent=2)
    print("Evidence:", a.output)

sys.exit(1 if blocked or not results else 0)
