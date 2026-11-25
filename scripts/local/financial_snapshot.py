import argparse
import time
from common import collect, compact, emit, save, now


def valid_financial_snapshot_body(body):
    if not isinstance(body, dict):
        return False
    coverage = body.get("ledger_coverage")
    if not isinstance(coverage, dict):
        return False
    count_fields = (
        "trade_rows", "known_fee_fills", "unknown_fee_fills",
        "fee_currency_unknown_rows", "fee_cashflow_currency_mismatch_rows",
        "parent_order_linked_rows", "rows_without_parent_order_id",
        "timestamp_order_inversion_rows",
    )
    if any(
        type(coverage.get(key)) is not int or coverage[key] < 0
        for key in count_fields
    ):
        return False
    return (
        bool(body.get("snapshot_id"))
        and body.get("snapshot_consistency") == "single_locked_ledger_clone"
        and body.get("ledger_snapshot_consistent") is True
        and body.get("execution_mode") == "paper"
        and all(isinstance(body.get(key), dict) for key in (
            "account", "positions", "realized_pnl", "fee_summary", "fee_rates"
        ))
        and coverage.get("independent_execution_ledger_reconciled") is False
        and coverage["known_fee_fills"] + coverage["unknown_fee_fills"]
            == coverage["trade_rows"]
        and coverage["parent_order_linked_rows"] + coverage["rows_without_parent_order_id"]
            == coverage["trade_rows"]
    )

def main():
    parser = argparse.ArgumentParser()
    parser.parse_args()

    started = time.monotonic()
    result = collect("positions", "financial-snapshot")
    body = result.get("body", {})
    fallback_sources = None
    if result["status"] != "COLLECTED":
        fallback_sources = []
        for endpoint in (
            "account", "positions", "realized-pnl", "fee-summary", "fee-rates"
        ):
            legacy = collect("positions", endpoint)
            fallback_sources.append(legacy)
            if legacy["status"] != "COLLECTED":
                break
        complete = (
            len(fallback_sources) == 5
            and all(item["status"] == "COLLECTED" for item in fallback_sources)
        )
    else:
        complete = valid_financial_snapshot_body(body)

    elapsed = time.monotonic() - started

    if fallback_sources is not None:
        sources = [compact(item) for item in fallback_sources]
        data = {
            "status": "COLLECTED" if complete else "BLOCKED",
            "timestamp": now(),
            "collection_seconds": round(elapsed, 3),
            "sources": sources,
            "near_synchronous": complete and elapsed <= 5,
            "atomic_snapshot": False,
            "ledger_snapshot_consistent": False,
            "snapshot_id": None,
            "execution_mode": None,
            "snapshot_consistency": "legacy_separate_endpoints",
            "independent_execution_ledger_reconciled": False,
            "historical_integrity_verified": False,
            "financial_acceptance": "NOT_EVALUATED",
        }
        path = save(data, "financial_snapshot")
        emit({"manifest": path, "result": data})
        return 0 if complete else 1

    data = {
        "status": "COLLECTED" if complete else "BLOCKED",
        "timestamp": now(),
        "collection_seconds": round(elapsed, 3),
        "sources": [compact(result)],
        "near_synchronous": complete,
        "atomic_snapshot": False,
        "ledger_snapshot_consistent": bool(complete),
        "snapshot_id": body.get("snapshot_id"),
        "execution_mode": body.get("execution_mode"),
        "snapshot_consistency": body.get("snapshot_consistency"),
        "market_marks_consistency": body.get("market_marks_consistency"),
        "fx_rates_updated_at_by_currency": body.get("fx_rates_updated_at_by_currency"),
        "ledger_coverage": body.get("ledger_coverage"),
        "independent_execution_ledger_reconciled": False,
        "historical_integrity_verified": False,
        "financial_acceptance": "NOT_EVALUATED"
    }

    path = save(data, "financial_snapshot")
    emit({"manifest": path, "result": data})

    return 0 if complete else 1

if __name__ == "__main__":
    raise SystemExit(main())
