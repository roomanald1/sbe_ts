import argparse
import json
import time
from common import api, collect, compact, emit, save, now


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

def collect_paged_source(service, endpoint, cursor_name, page_size=500, cap=200):
    """Collect a bounded cursor/offset history and retain every source page."""
    pages = []
    rows = []
    cursor = 0
    expected_total = None
    identity = None
    cumulative_row_bytes = 0
    for _ in range(cap):
        params = {"limit": page_size, cursor_name: cursor}
        response = api(
            service, endpoint, params=params,
            max_bytes=32_000_000 if endpoint == "portfolio-evidence-intervals" else 4_000_000,
        )
        page = response.get("body")
        if not isinstance(page, dict):
            raise ValueError(f"{service}.{endpoint} page is not an object")
        path = save(response, f"{service}-{endpoint}-page")
        pages.append({"evidence": path, "bytes": response.get("bytes"), "route": response.get("route")})
        if endpoint == "trades":
            page_rows = page.get("trades")
            total = page.get("total")
            if (not isinstance(page_rows, list) or type(total) is not int
                    or page.get("has_older") not in (False, None)
                    or (expected_total is not None and total != expected_total)):
                raise ValueError("PM trade page lacks stable complete-history coverage")
            expected_total = total
            cumulative_row_bytes += sum(len(json.dumps(row, separators=(",", ":"))) for row in page_rows)
            if cumulative_row_bytes > 64_000_000:
                raise ValueError("PM fill evidence exceeded the bounded collection size")
            rows.extend(page_rows)
            next_cursor = page.get("next_offset")
            if next_cursor is None:
                if len(rows) != total:
                    raise ValueError("PM trade pages do not cover the declared total")
                body = {"trades": rows, "total": total, "has_older": False,
                        "collector_complete": True, "page_evidence": pages}
                break
            if type(next_cursor) is not int or next_cursor <= cursor or next_cursor != len(rows):
                raise ValueError("PM trade paging cursor is inconsistent")
            cursor = next_cursor
        elif endpoint == "portfolio-evidence-intervals":
            page_rows = page.get("intervals")
            if not isinstance(page_rows, list):
                raise ValueError("PM portfolio interval page is malformed")
            current_identity = page.get("coverage")
            if identity is None:
                identity = page.get("schema_version")
            elif identity != page.get("schema_version"):
                raise ValueError("PM portfolio interval schema changed during collection")
            rows.extend(page_rows)
            cumulative_row_bytes += sum(len(json.dumps(row, separators=(",", ":"))) for row in page_rows)
            if cumulative_row_bytes > 64_000_000:
                raise ValueError("PM portfolio interval evidence exceeded the bounded collection size")
            next_cursor = page.get("next_cursor")
            if next_cursor is None:
                if page.get("truncated") is True:
                    raise ValueError("PM portfolio interval history ended while truncated")
                body = {"intervals": rows, "coverage": current_identity,
                        "schema_version": identity,
                        "truncated": False, "next_cursor": None,
                        "collector_complete": True, "page_evidence": pages}
                break
            if type(next_cursor) is not int or next_cursor <= cursor:
                raise ValueError("PM portfolio interval cursor is inconsistent")
            cursor = next_cursor
        else:
            current = (page.get("source_epoch"), page.get("max_cursor"))
            if identity is None:
                identity = current
            elif identity != current:
                raise ValueError("Paper Exchange epoch or maximum cursor changed during collection")
            page_rows = page.get("events")
            if not isinstance(page_rows, list) or page.get("after_cursor") != cursor:
                raise ValueError("Paper Exchange event page is malformed")
            rows.extend(page_rows)
            cumulative_row_bytes += sum(len(json.dumps(row, separators=(",", ":"))) for row in page_rows)
            if cumulative_row_bytes > 64_000_000:
                raise ValueError("Paper Exchange financial evidence exceeded the bounded collection size")
            next_cursor = page.get("next_cursor")
            if page.get("page_complete") is True:
                if cursor != 0 and page_rows and page_rows[0].get("source_cursor") != cursor + 1:
                    raise ValueError("Paper Exchange event pages have a cursor gap")
                if len(rows) != page.get("max_cursor"):
                    raise ValueError("Paper Exchange event history is incomplete")
                body = dict(page)
                body.update({"after_cursor": 0, "next_cursor": None,
                             "max_cursor": len(rows), "page_size": len(rows),
                             "events": rows, "page_evidence": pages})
                break
            if type(next_cursor) is not int or next_cursor <= cursor:
                raise ValueError("Paper Exchange event cursor is inconsistent")
            if page_rows and page_rows[0].get("source_cursor") != cursor + 1:
                raise ValueError("Paper Exchange event pages have a cursor gap")
            cursor = next_cursor
    else:
        raise ValueError(f"{service}.{endpoint} exceeded bounded page count")

    normalized_route = {
        ("positions", "trades"): "/positions/trades",
        ("positions", "portfolio-evidence-intervals"): "/positions/portfolio-evidence/intervals",
        ("paper_exchange", "financial-evidence"): "/internal/financial/evidence",
    }[(service, endpoint)]
    encoded = json.dumps(body, separators=(",", ":"), allow_nan=False).encode()
    aggregate = {"service": service, "route": normalized_route,
                 "application_status": 200, "bytes": len(encoded),
                 "body": body, "collector_bundle": True}
    return {"status": "COLLECTED", "service": service, "endpoint": endpoint,
            "timestamp": now(), "bytes": len(encoded), "records": len(rows),
            "evidence": save(aggregate, f"{service}-{endpoint}-complete"),
            "body": body, "collector_bundle": True, "page_evidence": pages}
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

    reconciliation_sources = []
    if complete and result["status"] == "COLLECTED":
        for service, endpoint, cursor_name, size in (
            ("positions", "trades", "offset", 500),
            ("positions", "portfolio-evidence-intervals", "after_cursor", 50),
            ("paper_exchange", "financial-evidence", "after_cursor", 500),
        ):
            try:
                reconciliation_sources.append(
                    collect_paged_source(service, endpoint, cursor_name, size)
                )
            except Exception as error:
                reconciliation_sources.append({
                    "status": "BLOCKED", "service": service,
                    "endpoint": endpoint, "error": str(error),
                })
                complete = False

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
        "execution_reconciliation_sources": [compact(item) for item in reconciliation_sources],
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
