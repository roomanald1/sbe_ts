import argparse
import time
from common import collect, compact, emit, save, now

def main():
    parser = argparse.ArgumentParser()
    parser.parse_args()

    started = time.monotonic()
    results = []

    for endpoint in [
        "account",
        "positions",
        "realized-pnl",
        "fee-summary",
        "fee-rates"
    ]:
        result = collect("positions", endpoint)
        results.append(compact(result))

        if result["status"] != "COLLECTED":
            break

    elapsed = time.monotonic() - started

    complete = (
        len(results) == 5
        and all(
            r["status"] == "COLLECTED"
            for r in results
        )
    )

    data = {
        "status": "COLLECTED" if complete else "BLOCKED",
        "timestamp": now(),
        "collection_seconds": round(elapsed, 3),
        "sources": results,
        "near_synchronous": complete and elapsed <= 5,
        "atomic_snapshot": False,
        "financial_acceptance": "NOT_EVALUATED"
    }

    path = save(data, "financial_snapshot")
    emit({"manifest": path, "result": data})

    return 0 if complete else 1

if __name__ == "__main__":
    raise SystemExit(main())
