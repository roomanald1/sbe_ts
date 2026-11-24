import sys
from common import query

probes = [
    ("learning", "summary"),
    ("autotrader", "feedback-queues"),
    ("insights", "dashboard"),
    ("positions", "fee-summary")
]

passed = 0

for service, endpoint in probes:
    try:
        data = query(service, endpoint)
        print(
            "[PASS]",
            service,
            endpoint,
            f"status={data['application_status']}",
            f"{data['elapsed_ms']}ms",
            f"{data['bytes']} bytes"
        )
        passed += 1

    except Exception as e:
        print("[BLOCKED]", service, str(e)[:500])

print(f"\nApplication responses: {passed}/{len(probes)}")
sys.exit(0 if passed == len(probes) else 1)
