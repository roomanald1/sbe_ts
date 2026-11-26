import argparse
import json
import hashlib
from pathlib import Path
import math
from collections import defaultdict
from decimal import Decimal
from datetime import datetime, timezone

from common import emit, save, now
from evidence_analysis import latest, body, age

ZERO = Decimal("0")
EXECUTION_RECONCILIATION_TOLERANCE = Decimal("0.00000001")
PORTFOLIO_INTERVAL_RECONCILIATION_TOLERANCE = Decimal("0.00000001")
PORTFOLIO_FX_MAX_AGE_MS = 120_000

def dec(value):
    if isinstance(value, bool) or not isinstance(
        value, (int, float, Decimal)
    ):
        raise ValueError("Expected numeric financial value")
    result = Decimal(str(value))
    if not result.is_finite():
        raise ValueError("Non-finite financial value")
    return result

def rounded(value, places=8):
    return float(round(value, places))

def pct(numerator, denominator):
    return rounded(numerator / denominator * 100, 4) \
        if denominator != 0 else None

def total(values):
    return sum((dec(v) for v in values), ZERO)


def reconcile_independent_execution_fills(
    position_trades, execution_ledger, position_source_coverage=None
):
    """Compare PM fills to a separately sourced Paper Exchange fill ledger.

    This reconciles execution quantity, notional, and signed fee cashflows by
    currency, not trading performance. PM parent order IDs must be propagated
    into the independent source as ``order_id``; timestamps or symbol/side are
    not safe join keys. Both sources must explicitly establish complete row
    coverage, and each row must carry known fee amount/currency metadata.
    """
    result = {
        "status": "BLOCKED",
        "independent_execution_ledger_reconciled": False,
        "position_trade_rows": 0,
        "independent_execution_rows": 0,
        "matched_order_count": 0,
        "unmatched_position_order_count": 0,
        "unmatched_execution_order_count": 0,
        "mismatch_count": 0,
        "unmatched_position_orders": [],
        "unmatched_execution_orders": [],
        "mismatches": [],
        "blockers": [],
        "position_source_complete": False,
        "independent_execution_source_complete": False,
        "fee_rows_reconciled": 0,
        "note": "Execution reconciliation does not estimate returns or profitability.",
    }
    if (
        not isinstance(position_source_coverage, dict)
        or position_source_coverage.get("has_older") is not False
        or isinstance(position_source_coverage.get("total_records"), bool)
        or not isinstance(position_source_coverage.get("total_records"), int)
        or isinstance(position_source_coverage.get("returned_records"), bool)
        or not isinstance(position_source_coverage.get("returned_records"), int)
        or position_source_coverage.get("total_records")
        != position_source_coverage.get("returned_records")
        or not isinstance(position_trades, list)
        or position_source_coverage.get("returned_records") != len(position_trades)
    ):
        result["blockers"].append(
            "Position fill source lacks complete, count-matched history coverage"
        )
        return result
    result["position_source_complete"] = True
    if not isinstance(execution_ledger, dict):
        result["blockers"].append("Independent Paper Exchange execution ledger unavailable")
        return result
    if (
        execution_ledger.get("schema_version") != 1
        or execution_ledger.get("source_system") != "paper_exchange"
        or execution_ledger.get("execution_mode") != "paper"
        or execution_ledger.get("independent_source") is not True
        or execution_ledger.get("complete") is not True
        or not isinstance(execution_ledger.get("snapshot_id"), str)
        or not execution_ledger.get("snapshot_id")
    ):
        result["blockers"].append("Independent execution ledger contract is incomplete or unverified")
        return result
    if not isinstance(position_trades, list) or not isinstance(execution_ledger.get("fills"), list):
        result["blockers"].append("Position fills or independent execution rows are unavailable")
        return result
    result["independent_execution_source_complete"] = True

    fee_schedules = execution_ledger.get("fee_schedules")
    if not isinstance(fee_schedules, list) or not fee_schedules:
        result["blockers"].append("Independent versioned fee schedule evidence is unavailable")
        return result
    fee_schedule_by_identity = {}
    for schedule in fee_schedules:
        if not isinstance(schedule, dict):
            result["blockers"].append("Independent fee schedule row is invalid")
            return result
        schedule_id, schedule_version = schedule.get("fee_schedule_id"), schedule.get("fee_schedule_version")
        symbol, source = schedule.get("symbol"), schedule.get("source_system")
        source_record_id = schedule.get("source_record_id")
        effective_from, effective_to = schedule.get("effective_from_utc_ms"), schedule.get("effective_to_utc_ms")
        if (
            not isinstance(schedule_id, str) or not schedule_id
            or not isinstance(schedule_version, str) or not schedule_version
            or not isinstance(symbol, str) or not symbol
            or not isinstance(source, str) or not source
            or not isinstance(source_record_id, str) or not source_record_id
            or isinstance(effective_from, bool) or not isinstance(effective_from, int)
            or (effective_to is not None and (
                isinstance(effective_to, bool) or not isinstance(effective_to, int)
                or effective_to <= effective_from
            ))
            or "maker_rate" not in schedule
            or "taker_rate" not in schedule
        ):
            result["blockers"].append("Independent fee schedule identity, source or effective range is incomplete")
            return result
        normalized_source = source.strip().lower().replace("-", "_").replace(" ", "_")
        if normalized_source in {
            "pm", "position_management", "position_manager",
            "position_manager_account_topup_ledger", "auto_trader",
        }:
            result["blockers"].append("Independent fee schedule was copied from Position Management or AutoTrader")
            return result
        try:
            maker_rate = dec(schedule["maker_rate"])
            taker_rate = dec(schedule["taker_rate"])
        except (ValueError, TypeError):
            result["blockers"].append("Independent fee schedule rates are missing or invalid")
            return result
        if maker_rate < 0 or taker_rate < 0 or maker_rate > Decimal("0.1") or taker_rate > Decimal("0.1"):
            result["blockers"].append("Independent fee schedule rates are outside simulator bounds")
            return result
        identity = (schedule_id, schedule_version, symbol)
        if identity in fee_schedule_by_identity:
            result["blockers"].append("Independent fee schedule identity is duplicated")
            return result
        fee_schedule_by_identity[identity] = schedule

    def aggregate(rows, source, id_field):
        by_order = defaultdict(lambda: {
            "symbol": None, "side": None, "quantity": ZERO, "notional": ZERO,
            "fees_by_currency": defaultdict(lambda: ZERO), "fee_rows": 0,
        })
        seen_ids = set()
        for row in rows:
            if not isinstance(row, dict):
                raise ValueError(f"{source} row is not an object")
            order_id = row.get(id_field)
            if not isinstance(order_id, str) or not order_id.strip():
                raise ValueError(f"{source} row has no stable order identity")
            execution_id = row.get("fill_id") if source == "Position Management" else row.get("execution_id")
            if not isinstance(execution_id, str) or not execution_id.strip() or execution_id in seen_ids:
                raise ValueError(f"{source} row has a missing or duplicate execution identity")
            seen_ids.add(execution_id)
            symbol, side = row.get("symbol"), row.get("side")
            if not isinstance(symbol, str) or not symbol.strip() or side not in ("buy", "sell"):
                raise ValueError(f"{source} row has invalid instrument or side")
            quantity, price = dec(row.get("quantity")), dec(row.get("price", row.get("execution_price")))
            if quantity <= 0 or price <= 0:
                raise ValueError(f"{source} row has non-positive quantity or price")
            if row.get("fee_known") is not True:
                raise ValueError(f"{source} row has unknown fee metadata")
            fee = dec(row.get("fee"))
            fee_currency = row.get("fee_currency")
            if not isinstance(fee_currency, str) or not fee_currency.strip():
                raise ValueError(f"{source} row has missing fee currency")
            if source == "Paper Exchange":
                schedule_id = row.get("fee_schedule_id")
                schedule_version = row.get("fee_schedule_version")
                timestamp = row.get("timestamp_utc_ms")
                if (
                    not isinstance(schedule_id, str) or not schedule_id
                    or not isinstance(schedule_version, str) or not schedule_version
                    or isinstance(timestamp, bool) or not isinstance(timestamp, int)
                ):
                    raise ValueError("Paper Exchange row lacks versioned fee schedule or source timestamp")
                fill_source = row.get("source_system")
                source_epoch = row.get("source_epoch")
                source_cursor = row.get("source_cursor")
                source_record_id = row.get("source_record_id")
                if (
                    not isinstance(fill_source, str) or not fill_source.strip()
                    or not isinstance(source_epoch, str) or not source_epoch
                    or isinstance(source_cursor, bool) or not isinstance(source_cursor, int)
                    or source_cursor <= 0
                    or not isinstance(source_record_id, str) or not source_record_id
                ):
                    raise ValueError("Paper Exchange execution lacks source epoch, cursor or record identity")
                normalized_fill_source = fill_source.strip().lower().replace("-", "_").replace(" ", "_")
                if normalized_fill_source in {
                    "pm", "position_management", "position_manager",
                    "position_manager_account_topup_ledger", "auto_trader",
                }:
                    raise ValueError("Paper Exchange execution row was copied from Position Management or AutoTrader")
                schedule = fee_schedule_by_identity.get((schedule_id, schedule_version, symbol))
                if schedule is None:
                    raise ValueError("Paper Exchange fill references a missing or mismatched fee schedule")
                if timestamp < schedule["effective_from_utc_ms"] or (
                    schedule["effective_to_utc_ms"] is not None
                    and timestamp >= schedule["effective_to_utc_ms"]
                ):
                    raise ValueError("Paper Exchange fee schedule was not effective at fill time")
                pair = symbol.rsplit("-", 1)
                if len(pair) != 2 or not pair[1] or fee_currency != pair[1]:
                    raise ValueError("Paper Exchange fee currency must match the instrument quote currency")
                origin = row.get("execution_origin")
                if origin not in {"market_match", "resting_limit", "protective_barrier", "vwap", "twap"}:
                    raise ValueError("Paper Exchange execution origin is unsupported for fee reconciliation")
                rate = dec(schedule["maker_rate"] if origin == "resting_limit" else schedule["taker_rate"])
                expected_fee = -(quantity * price * rate)
                if abs(fee - expected_fee) > EXECUTION_RECONCILIATION_TOLERANCE:
                    raise ValueError("Paper Exchange signed fee differs from the versioned simulator fee schedule")
            aggregate = by_order[order_id]
            if aggregate["symbol"] not in (None, symbol) or aggregate["side"] not in (None, side):
                raise ValueError(f"{source} order identity crosses symbol or side")
            aggregate["symbol"], aggregate["side"] = symbol, side
            aggregate["quantity"] += quantity
            aggregate["notional"] += quantity * price
            aggregate["fees_by_currency"][fee_currency] += fee
            aggregate["fee_rows"] += 1
        return by_order

    try:
        pm = aggregate(position_trades, "Position Management", "parent_order_id")
        independent = aggregate(execution_ledger["fills"], "Paper Exchange", "order_id")
    except (ValueError, TypeError) as error:
        result["blockers"].append(str(error))
        return result

    pm_orders, external_orders = set(pm), set(independent)
    unmatched_pm = sorted(pm_orders - external_orders)
    unmatched_external = sorted(external_orders - pm_orders)
    mismatches = []
    for order_id in sorted(pm_orders & external_orders):
        left, right = pm[order_id], independent[order_id]
        reasons = []
        if (left["symbol"], left["side"]) != (right["symbol"], right["side"]):
            reasons.append("instrument_or_side_mismatch")
        if abs(left["quantity"] - right["quantity"]) > EXECUTION_RECONCILIATION_TOLERANCE:
            reasons.append("executed_quantity_mismatch")
        if abs(left["notional"] - right["notional"]) > EXECUTION_RECONCILIATION_TOLERANCE:
            reasons.append("executed_notional_mismatch")
        left_fee_currencies = set(left["fees_by_currency"])
        right_fee_currencies = set(right["fees_by_currency"])
        if left_fee_currencies != right_fee_currencies:
            reasons.append("fee_currency_mismatch")
        elif any(
            abs(left["fees_by_currency"][currency] - right["fees_by_currency"][currency])
            > EXECUTION_RECONCILIATION_TOLERANCE
            for currency in left_fee_currencies
        ):
            reasons.append("fee_amount_mismatch")
        if reasons:
            mismatches.append({"order_id": order_id, "reasons": reasons})

    result.update({
        "status": "RECONCILED" if not unmatched_pm and not unmatched_external and not mismatches else "MISMATCH",
        "position_trade_rows": len(position_trades),
        "independent_execution_rows": len(execution_ledger["fills"]),
        "matched_order_count": len(pm_orders & external_orders) - len(mismatches),
        "unmatched_position_order_count": len(unmatched_pm),
        "unmatched_execution_order_count": len(unmatched_external),
        "mismatch_count": len(mismatches),
        "unmatched_position_orders": unmatched_pm[:100],
        "unmatched_execution_orders": unmatched_external[:100],
        "mismatches": mismatches[:100],
        "fee_rows_reconciled": sum(
            pm[order_id]["fee_rows"]
            for order_id in pm_orders & external_orders
            if order_id not in {mismatch["order_id"] for mismatch in mismatches}
        ),
        "independent_execution_ledger_reconciled": not unmatched_pm and not unmatched_external and not mismatches,
    })
    if unmatched_pm:
        result["blockers"].append("Position fills are absent from the independent execution ledger")
    if unmatched_external:
        result["blockers"].append("Independent execution fills are absent from Position Management")
    if mismatches:
        result["blockers"].append(
            "Independent execution identity, quantity, notional or fee evidence does not reconcile"
        )
    return result


def compare_paper_exchange_match_evidence(
    position_trades, match_ledger, position_source_coverage=None
):
    """Compare observed PM fills to versioned PEx matching evidence only.

    This diagnostic validates identity, source quantity, actual PM clipping,
    notional, and page cursors. It never treats matcher rows as settled fills,
    independent fee evidence, complete history, or profitability evidence.
    """
    result = {
        "diagnostic": "paper_exchange_match_evidence_comparison",
        "status": "BLOCKED",
        "evidence_kind": "paper_exchange_match_evidence",
        "observed_rows_consistent": False,
        "financial_reconciliation_eligible": False,
        "completed_financial_reconciliation": False,
        "source_history_complete": False,
        "position_source_complete": False,
        "position_fill_rows": len(position_trades) if isinstance(position_trades, list) else 0,
        "source_match_rows": 0,
        "matched_fill_count": 0,
        "clipped_fill_count": 0,
        "mismatch_count": 0,
        "missing_source_link_count": 0,
        "unmatched_source_row_count": 0,
        "source_epoch": None,
        "after_cursor": None,
        "next_cursor": None,
        "max_cursor": None,
        "page_complete": False,
        "blockers": [],
        "mismatches": [],
        "note": "Matching evidence is not settled execution, fee, or return evidence.",
    }
    if not isinstance(match_ledger, dict):
        result["blockers"].append("Paper Exchange match-evidence page unavailable")
        return result

    rows = match_ledger.get("rows")
    epoch = match_ledger.get("source_epoch")
    page_fields_valid = (
        match_ledger.get("schema_version") == "paper_exchange_match_ledger:v1"
        and match_ledger.get("evidence_kind") == "paper_exchange_match_evidence"
        and match_ledger.get("reconciliation_eligible") is False
        and match_ledger.get("cursor_kind") == "match_fill"
        and isinstance(epoch, str)
        and len(epoch) == 32
        and all(character in "0123456789abcdef" for character in epoch)
        and match_ledger.get("capture_scope") == "matches_with_caller_supplied_order_id_only"
        and match_ledger.get("identity_complete") is True
        and isinstance(rows, list)
        and isinstance(match_ledger.get("page_complete"), bool)
        and isinstance(match_ledger.get("history_complete"), bool)
        and match_ledger.get("history_complete") is False
    )
    if not page_fields_valid:
        result["blockers"].append(
            "Paper Exchange match-evidence schema, source epoch, cursor kind, or completeness metadata is invalid"
        )
    result["source_epoch"] = epoch if isinstance(epoch, str) else None
    result["source_history_complete"] = match_ledger.get("history_complete") is True
    if match_ledger.get("history_complete") is not True:
        result["blockers"].append(
            "Paper Exchange source history is explicitly incomplete; comparison covers only returned rows"
        )

    after = match_ledger.get("after_cursor")
    next_cursor = match_ledger.get("next_cursor")
    max_cursor = match_ledger.get("max_cursor")
    page_size = match_ledger.get("page_size")
    page_complete = match_ledger.get("page_complete")
    cursor_number = lambda value: (
        isinstance(value, int) and not isinstance(value, bool) and value >= 0
    )
    cursor_fields_valid = (
        cursor_number(after)
        and cursor_number(max_cursor)
        and isinstance(page_size, int)
        and not isinstance(page_size, bool)
        and page_size == len(rows) if isinstance(rows, list) else False
    )
    if next_cursor is not None:
        cursor_fields_valid = cursor_fields_valid and cursor_number(next_cursor)
    if not cursor_fields_valid:
        result["blockers"].append("Match-evidence page cursor or row-count metadata is invalid")
    result.update({
        "after_cursor": after if cursor_number(after) else None,
        "next_cursor": next_cursor if cursor_number(next_cursor) else None,
        "max_cursor": max_cursor if cursor_number(max_cursor) else None,
        "page_complete": page_complete is True,
    })
    if isinstance(rows, list):
        result["source_match_rows"] = len(rows)
    else:
        result["blockers"].append("Match-evidence rows are unavailable")
        rows = []

    source_by_execution_id = {}
    source_ids = set()
    source_cursors = set()
    prior_cursor = after if cursor_number(after) else 0
    source_invalid = False
    for row in rows:
        if not isinstance(row, dict):
            source_invalid = True
            result["blockers"].append("Paper Exchange match-evidence row is not an object")
            continue
        execution_id = row.get("source_execution_id")
        order_id = row.get("order_id")
        cursor = row.get("source_cursor")
        fingerprint = row.get("request_fingerprint")
        symbol, side = row.get("symbol"), row.get("side")
        try:
            quantity, price = dec(row.get("quantity")), dec(row.get("price"))
        except (ValueError, TypeError):
            source_invalid = True
            result["blockers"].append("Paper Exchange row has invalid source quantity or price")
            continue
        row_valid = (
            isinstance(execution_id, str) and bool(execution_id.strip())
            and isinstance(order_id, str) and bool(order_id.strip())
            and cursor_number(cursor) and cursor > 0
            and isinstance(fingerprint, str) and bool(fingerprint.strip())
            and isinstance(symbol, str) and bool(symbol.strip())
            and side in ("buy", "sell")
            and quantity > 0 and price > 0
        )
        if not row_valid:
            source_invalid = True
            result["blockers"].append("Paper Exchange match-evidence row has incomplete identity or values")
            continue
        if execution_id in source_ids or cursor in source_cursors:
            source_invalid = True
            result["blockers"].append("Paper Exchange match-evidence contains duplicate fill ID or cursor")
            continue
        if (
            not cursor_number(after)
            or not cursor_number(max_cursor)
            or cursor <= after
            or cursor > max_cursor
            or cursor <= prior_cursor
        ):
            source_invalid = True
            result["blockers"].append("Paper Exchange match-evidence cursors are outside ordered page bounds")
            continue
        source_ids.add(execution_id)
        source_cursors.add(cursor)
        prior_cursor = cursor
        source_by_execution_id[execution_id] = {
            "order_id": order_id,
            "cursor": cursor,
            "request_fingerprint": fingerprint,
            "symbol": symbol,
            "side": side,
            "quantity": quantity,
            "price": price,
        }

    if isinstance(rows, list) and cursor_fields_valid:
        if rows and next_cursor != rows[-1].get("source_cursor"):
            source_invalid = True
            result["blockers"].append("Match-evidence next_cursor does not identify the final returned row")
        if page_complete is True and rows and rows[-1].get("source_cursor") != max_cursor:
            source_invalid = True
            result["blockers"].append("Complete page ends before its reported high-water cursor")
        if page_complete is False and (not rows or rows[-1].get("source_cursor") >= max_cursor):
            source_invalid = True
            result["blockers"].append("Incomplete page has no cursor position remaining before high-water")
        if page_complete is True and not rows and after < max_cursor:
            source_invalid = True
            result["blockers"].append("Empty complete page ends before its reported high-water cursor")

    if not isinstance(position_trades, list):
        result["blockers"].append("Position Management fill rows unavailable")
        return result
    result["position_source_complete"] = (
        isinstance(position_source_coverage, dict)
        and position_source_coverage.get("has_older") is False
        and isinstance(position_source_coverage.get("total_records"), int)
        and not isinstance(position_source_coverage.get("total_records"), bool)
        and position_source_coverage.get("total_records") == len(position_trades)
        and isinstance(position_source_coverage.get("returned_records"), int)
        and not isinstance(position_source_coverage.get("returned_records"), bool)
        and position_source_coverage.get("returned_records") == len(position_trades)
    )
    if not result["position_source_complete"]:
        result["blockers"].append(
            "Position fill comparison has partial or unverified source coverage"
        )

    seen_pm_fill_ids = set()
    seen_pm_source_ids = set()
    seen_pm_cursors = set()
    matched_source_ids = set()
    mismatches = []
    missing_source_links = 0
    clipped_count = 0
    consistent_count = 0
    for trade in position_trades:
        if not isinstance(trade, dict):
            missing_source_links += 1
            result["blockers"].append("Position Management fill row is not an object")
            continue
        pm_fill_id = trade.get("fill_id")
        execution_id = trade.get("paper_exchange_execution_id")
        order_id = trade.get("parent_order_id")
        cursor = trade.get("paper_exchange_source_cursor")
        reasons = []
        if not isinstance(pm_fill_id, str) or not pm_fill_id.strip():
            reasons.append("missing_pm_fill_id")
        elif pm_fill_id in seen_pm_fill_ids:
            reasons.append("duplicate_pm_fill_id")
        else:
            seen_pm_fill_ids.add(pm_fill_id)
        if trade.get("paper_exchange_source_complete") is not True:
            reasons.append("pm_source_link_incomplete")
        if trade.get("paper_exchange_schema_version") != "paper_exchange_match:v1":
            reasons.append("pm_source_schema_mismatch")
        if not isinstance(order_id, str) or not order_id.strip():
            reasons.append("missing_source_order_id")
        if not isinstance(execution_id, str) or not execution_id.strip():
            reasons.append("missing_source_fill_id")
        elif execution_id in seen_pm_source_ids:
            reasons.append("duplicate_pm_source_fill_id")
        else:
            seen_pm_source_ids.add(execution_id)
        if not cursor_number(cursor) or cursor <= 0:
            reasons.append("missing_or_invalid_pm_source_cursor")
        elif cursor in seen_pm_cursors:
            reasons.append("duplicate_pm_source_cursor")
        else:
            seen_pm_cursors.add(cursor)
        epoch_value = trade.get("paper_exchange_source_epoch")
        if not isinstance(epoch_value, str) or not epoch_value:
            reasons.append("missing_pm_source_epoch")
        elif isinstance(epoch, str) and epoch_value != epoch:
            reasons.append("source_epoch_mismatch")
        fingerprint_value = trade.get("paper_exchange_request_fingerprint")
        if not isinstance(fingerprint_value, str) or not fingerprint_value:
            reasons.append("missing_pm_request_fingerprint")
        link_incomplete = any(reason in {
            "missing_pm_fill_id",
            "duplicate_pm_fill_id",
            "pm_source_link_incomplete",
            "pm_source_schema_mismatch",
            "missing_source_order_id",
            "missing_or_invalid_pm_source_cursor",
            "duplicate_pm_source_cursor",
            "missing_pm_source_epoch",
            "source_epoch_mismatch",
            "missing_pm_request_fingerprint",
        } for reason in reasons)
        if "missing_source_fill_id" in reasons or "duplicate_pm_source_fill_id" in reasons:
            missing_source_links += 1
            mismatches.append({
                "pm_fill_id": pm_fill_id if isinstance(pm_fill_id, str) else None,
                "order_id": order_id if isinstance(order_id, str) else None,
                "source_execution_id": execution_id if isinstance(execution_id, str) else None,
                "reasons": reasons,
            })
            continue

        source_row = source_by_execution_id.get(execution_id)
        if source_row is None:
            missing_source_links += 1
            mismatches.append({
                "pm_fill_id": pm_fill_id,
                "order_id": order_id,
                "source_execution_id": execution_id,
                "reasons": ["source_fill_not_in_returned_page"],
            })
            continue
        matched_source_ids.add(execution_id)
        if link_incomplete:
            missing_source_links += 1
        if order_id != source_row["order_id"]:
            reasons.append("source_order_id_mismatch")
        if cursor != source_row["cursor"]:
            reasons.append("source_cursor_mismatch")
        if fingerprint_value != source_row["request_fingerprint"]:
            reasons.append("request_fingerprint_mismatch")
        if trade.get("symbol") != source_row["symbol"] or trade.get("side") != source_row["side"]:
            reasons.append("instrument_or_side_mismatch")
        try:
            pm_source_quantity = dec(trade.get("paper_exchange_source_quantity"))
            pm_quantity = dec(trade.get("quantity"))
            pm_price = dec(trade.get("execution_price"))
        except (ValueError, TypeError):
            reasons.append("invalid_pm_source_or_executed_quantity_or_price")
        else:
            if abs(pm_source_quantity - source_row["quantity"]) > EXECUTION_RECONCILIATION_TOLERANCE:
                reasons.append("source_quantity_mismatch")
            if pm_quantity <= 0 or pm_price <= 0:
                reasons.append("non_positive_pm_quantity_or_price")
            if pm_quantity > source_row["quantity"] + EXECUTION_RECONCILIATION_TOLERANCE:
                reasons.append("pm_quantity_exceeds_source_quantity")
            elif pm_quantity < source_row["quantity"] - EXECUTION_RECONCILIATION_TOLERANCE:
                clipped_count += 1
                reasons.append("position_manager_clipped_quantity")
            if abs(pm_price - source_row["price"]) > EXECUTION_RECONCILIATION_TOLERANCE:
                reasons.append("execution_price_mismatch")
            source_notional = source_row["quantity"] * source_row["price"]
            pm_source_notional = pm_source_quantity * pm_price
            if abs(pm_source_notional - source_notional) > EXECUTION_RECONCILIATION_TOLERANCE:
                reasons.append("source_notional_mismatch")
            pm_executed_notional = pm_quantity * pm_price
            expected_clipped_notional = pm_quantity * source_row["price"]
            if abs(pm_executed_notional - expected_clipped_notional) > EXECUTION_RECONCILIATION_TOLERANCE:
                reasons.append("clipped_execution_notional_mismatch")
        if reasons:
            mismatches.append({
                "pm_fill_id": pm_fill_id,
                "order_id": order_id,
                "source_execution_id": execution_id,
                "reasons": sorted(set(reasons)),
            })
        else:
            consistent_count += 1

    unmatched_sources = source_ids - matched_source_ids
    result["missing_source_link_count"] = missing_source_links
    result["unmatched_source_row_count"] = len(unmatched_sources)
    result["matched_fill_count"] = consistent_count
    result["clipped_fill_count"] = clipped_count
    result["mismatch_count"] = len(mismatches)
    result["mismatches"] = mismatches[:100]
    structurally_complete_page = (
        page_fields_valid and cursor_fields_valid and not source_invalid
    )
    result["observed_rows_consistent"] = (
        structurally_complete_page
        and not missing_source_links
        and not unmatched_sources
        and not mismatches
        and bool(consistent_count)
    )
    if result["observed_rows_consistent"]:
        result["status"] = "MATCHED_OBSERVED_EVIDENCE_PARTIAL_HISTORY"
    elif consistent_count or clipped_count or mismatches:
        result["status"] = "PARTIAL_OR_MISMATCHED_OBSERVED_EVIDENCE"
    if result["unmatched_source_row_count"]:
        result["blockers"].append("Returned Paper Exchange rows lack matching PM source links")
    if missing_source_links:
        result["blockers"].append("Some PM fills lack complete matching-evidence identity")
    if clipped_count:
        result["blockers"].append("PM clipped one or more source match quantities")
    if mismatches:
        result["blockers"].append("Observed PM and Paper Exchange match evidence differs")
    result["blockers"] = list(dict.fromkeys(result["blockers"]))
    return result


def project_paper_exchange_financial_evidence_page(page):
    """Project the durable Paper Exchange event page into existing fill reconciliation.

    This is a schema adapter only. It copies source identities and values; it
    does not calculate balances, PnL, fees, or returns. Incomplete page history
    stays incomplete and cannot pass ``reconcile_independent_execution_fills``.
    """
    if not isinstance(page, dict):
        return None
    events = page.get("events")
    page_epoch = page.get("source_epoch")
    if (
        page.get("schema_version") != "paper_exchange_financial_evidence:v1"
        or page.get("evidence_kind") != "paper_exchange_source_attributed_account_evidence"
        or page.get("cursor_kind") != "financial_event"
        or not isinstance(page_epoch, str) or not page_epoch
        or page.get("after_cursor") != 0
        or page.get("page_complete") is not True
        or not isinstance(page.get("history_complete"), bool)
        or not isinstance(events, list)
        or page.get("page_size") != len(events)
    ):
        return None
    local_cursors = []
    fills = []
    fee_schedules = []
    opening_capital_events = []
    contributions = []
    fx_marks = []
    for stored in events:
        if not isinstance(stored, dict):
            return None
        cursor = stored.get("source_cursor")
        event = stored.get("event")
        if (
            isinstance(cursor, bool) or not isinstance(cursor, int) or cursor <= 0
            or not isinstance(event, dict)
            or event.get("schema_version") != "paper_exchange_financial_evidence:v1"
            or not isinstance(event.get("event_id"), str) or not event["event_id"]
            or not isinstance(event.get("source_system"), str) or not event["source_system"]
            or not isinstance(event.get("source_epoch"), str) or not event["source_epoch"]
            or isinstance(event.get("source_cursor"), bool)
            or not isinstance(event.get("source_cursor"), int)
            or event["source_cursor"] <= 0
            or not isinstance(event.get("source_record_id"), str) or not event["source_record_id"]
            or isinstance(event.get("source_timestamp_utc_ms"), bool)
            or not isinstance(event.get("source_timestamp_utc_ms"), int)
            or event["source_timestamp_utc_ms"] <= 0
            or stored.get("source_epoch") != event.get("source_epoch")
            or stored.get("source_system") != event.get("source_system")
            or stored.get("source_record_id") != event.get("source_record_id")
            or stored.get("source_timestamp_utc_ms") != event.get("source_timestamp_utc_ms")
        ):
            return None
        local_cursors.append(cursor)
        kind = event.get("event_type")
        if kind == "fee_schedule":
            fee_schedules.append({
                "fee_schedule_id": event.get("fee_schedule_id"),
                "fee_schedule_version": event.get("fee_schedule_version"),
                "symbol": event.get("symbol"),
                "source_system": event["source_system"],
                "source_record_id": event["source_record_id"],
                "effective_from_utc_ms": event.get("effective_from_utc_ms"),
                "effective_to_utc_ms": event.get("effective_to_utc_ms"),
                "maker_rate": event.get("maker_rate"),
                "taker_rate": event.get("taker_rate"),
            })
        elif kind == "settled_execution":
            fills.append({
                "execution_id": event.get("execution_id"),
                "order_id": event.get("order_id"),
                "symbol": event.get("symbol"),
                "side": event.get("side"),
                "quantity": event.get("quantity"),
                "price": event.get("price"),
                "fee": event.get("fee"),
                "fee_known": True,
                "fee_currency": event.get("fee_currency"),
                "fee_schedule_id": event.get("fee_schedule_id"),
                "fee_schedule_version": event.get("fee_schedule_version"),
                "timestamp_utc_ms": event["source_timestamp_utc_ms"],
                "source_system": event["source_system"],
                "source_epoch": event["source_epoch"],
                "source_cursor": event["source_cursor"],
                "financial_evidence_cursor": cursor,
                "source_record_id": event["source_record_id"],
                "execution_origin": event.get("execution_origin"),
            })
        elif kind == "account_balance":
            if event.get("boundary") == "opening":
                opening_capital_events.append((stored, event))
        elif kind == "cash_movement":
            contributions.append((stored, event))
        elif kind == "fx_mark":
            fx_marks.append((stored, event))
    max_cursor = page.get("max_cursor")
    if (
        isinstance(max_cursor, bool) or not isinstance(max_cursor, int)
        or max_cursor < 0
        or local_cursors != list(range(1, max_cursor + 1))
    ):
        return None
    return {
        "schema_version": 1,
        "contract": "paper_exchange_execution_ledger_v1",
        "source_system": "paper_exchange",
        "execution_mode": "paper",
        "independent_source": True,
        "complete": page.get("history_complete") is True,
        "snapshot_id": f"paper-exchange-financial:v1:{page_epoch}:{max_cursor}",
        "source_epoch": page_epoch,
        "source_attribution_status": page.get("source_attribution_status"),
        "opening_capital_status": page.get("opening_capital_status"),
        "fee_schedule_status": page.get("fee_schedule_status"),
        "uncovered_execution_flow_types": page.get("uncovered_execution_flow_types"),
        "fills": fills,
        "fee_schedules": fee_schedules,
        "opening_capital_events": opening_capital_events,
        "contribution_events": contributions,
        "fx_marks": fx_marks,
    }


def reconstruct_portfolio_intervals_from_paper_exchange(page, pm_intervals, pm_trades):
    """Reconstruct cash boundaries from Paper Exchange opening capital and durable events.

    Closing balances are derived from opening cash plus signed executions and
    sourced contributions. They are explicitly marked as reconstructed, never
    represented as a provider balance snapshot.
    """
    projected = project_paper_exchange_financial_evidence_page(page)
    if projected is None or projected.get("complete") is not True:
        return None
    if not isinstance(pm_intervals, list) or not pm_intervals or not isinstance(pm_trades, list):
        return None
    opening_events = projected["opening_capital_events"]
    if len(opening_events) != 1:
        return None
    opening_stored, opening_event = opening_events[0]
    opening_payload = opening_event.get("balances_by_currency")
    opening_portfolio_id = opening_event.get("portfolio_id")
    if (opening_event.get("boundary") != "opening"
            or not isinstance(opening_portfolio_id, str) or not opening_portfolio_id
            or not isinstance(opening_payload, dict) or not opening_payload):
        return None
    try:
        balances = {currency: dec(amount) for currency, amount in opening_payload.items()}
    except (ValueError, TypeError):
        return None

    all_fills = projected["fills"]
    all_contribution_events = projected["contribution_events"]
    raw_fx = projected["fx_marks"]
    page_epoch = page.get("source_epoch")
    if not isinstance(page_epoch, str) or not page_epoch:
        return None
    trade_by_execution = {}
    for trade in pm_trades:
        if not isinstance(trade, dict):
            return None
        execution_id = trade.get("paper_exchange_execution_id")
        if isinstance(execution_id, str) and execution_id:
            if execution_id in trade_by_execution:
                return None
            trade_by_execution[execution_id] = trade

    def fill_cursor(trade):
        fill_id = trade.get("fill_id")
        # Existing PM rows carry the cursor epoch in fill_id, not a redundant JSON field.
        if not isinstance(fill_id, str) or not fill_id.startswith("pm-fill:v1:"):
            return None
        try:
            return int(fill_id.rsplit(":", 1)[1])
        except (ValueError, IndexError):
            return None

    intervals = []
    ordered_pm = sorted(pm_intervals, key=lambda row: row.get("interval_end_utc_ms", -1))
    event_movements = []
    for event in all_fills:
        quote_pair = event.get("symbol", "").rsplit("-", 1)
        if len(quote_pair) != 2 or not quote_pair[1]:
            return None
        quote = quote_pair[1]
        quantity, price = dec(event.get("quantity")), dec(event.get("price"))
        side = event.get("side")
        if quantity <= 0 or price <= 0 or side not in ("buy", "sell"):
            return None
        amount = (-quantity * price if side == "buy" else quantity * price) + dec(event.get("fee"))
        event_movements.append((event["timestamp_utc_ms"], event["financial_evidence_cursor"], quote, amount))
    contribution_rows = []
    for stored, event in all_contribution_events:
        if (event.get("movement_kind") not in ("contribution", "deposit", "top_up", "topup")
                or event.get("portfolio_id") != opening_portfolio_id):
            return None
        contribution_rows.append((stored, event))
    event_movements.sort(key=lambda row: (row[0], row[1]))
    contribution_rows.sort(key=lambda row: (row[0]["source_timestamp_utc_ms"], row[0]["source_cursor"]))

    first_boundary = ordered_pm[0].get("interval_start_utc_ms")
    if not isinstance(first_boundary, int) or opening_stored["source_timestamp_utc_ms"] > first_boundary:
        return None
    # Event history before the first reported PM interval must be fully represented;
    # otherwise its opening cash cannot be reconstructed from this epoch.
    applied_fill_cursors = set()
    applied_contribution_ids = set()

    def apply_until(timestamp):
        for row in event_movements:
            event_time, cursor, currency, amount = row
            if event_time < timestamp and cursor not in applied_fill_cursors:
                balances[currency] = balances.get(currency, ZERO) + amount
                applied_fill_cursors.add(cursor)
        for stored, event in contribution_rows:
            if stored["source_timestamp_utc_ms"] < timestamp and event["event_id"] not in applied_contribution_ids:
                currency = event.get("currency")
                if not isinstance(currency, str) or not currency:
                    raise ValueError("Invalid contribution currency")
                balances[currency] = balances.get(currency, ZERO) + dec(event.get("amount"))
                applied_contribution_ids.add(event["event_id"])

    try:
        for pm in ordered_pm:
            start, end = pm.get("interval_start_utc_ms"), pm.get("interval_end_utc_ms")
            if (not isinstance(start, int) or not isinstance(end, int) or end <= start
                    or pm.get("portfolio_id") != opening_portfolio_id):
                return None
            apply_until(start)
            opening_balances = dict(balances)
            interval_fills = []
            cash_movements = defaultdict(lambda: ZERO)
            for event in all_fills:
                timestamp = event["timestamp_utc_ms"]
                if not start <= timestamp < end:
                    continue
                trade = trade_by_execution.get(event.get("execution_id"))
                sequence = fill_cursor(trade) if trade else None
                if sequence is None or sequence <= pm.get("fill_cursor_start", -1) or sequence > pm.get("fill_cursor_end", -1):
                    return None
                quote = event["symbol"].rsplit("-", 1)[1]
                signed_notional = (-dec(event["quantity"]) * dec(event["price"])
                                   if event["side"] == "buy" else dec(event["quantity"]) * dec(event["price"]))
                signed_notional += dec(event["fee"])
                balances[quote] = balances.get(quote, ZERO) + signed_notional
                cash_movements[quote] += signed_notional
                interval_fills.append({**event, "fill_cursor": sequence})
                applied_fill_cursors.add(event["financial_evidence_cursor"])
            independent_contributions = []
            for stored, event in contribution_rows:
                timestamp = stored["source_timestamp_utc_ms"]
                if not start <= timestamp < end:
                    continue
                pm_contribution = next((row for row in pm.get("external_contribution_events", [])
                                        if row.get("independent_source_record_id") == stored["source_record_id"]
                                        and row.get("independent_source_epoch") == page_epoch
                                        and row.get("independent_source_cursor") == stored["source_cursor"]), None)
                if pm_contribution is None:
                    return None
                amount = dec(event.get("amount"))
                currency = event.get("currency")
                balances[currency] = balances.get(currency, ZERO) + amount
                cash_movements[currency] += amount
                applied_contribution_ids.add(event["event_id"])
                independent_contributions.append({
                    "contribution_id": event.get("movement_id"),
                    "pm_contribution_id": pm_contribution.get("contribution_id"),
                    "source": stored.get("source_system"),
                    "source_record_id": stored.get("source_record_id"),
                    "provider_source_epoch": page_epoch,
                    "provider_source_cursor": stored.get("source_cursor"),
                    "source_cursor": pm_contribution.get("source_cursor"),
                    "timestamp_utc_ms": timestamp,
                    "currency": currency,
                    "amount": event.get("amount"),
                    "value_usd": pm_contribution.get("value_usd"),
                })
            closing_balances = dict(balances)
            fx_marks = []
            boundary_maps = {}
            for boundary, boundary_time, rates_key, times_key, balances_at_boundary in (
                ("opening", start, "opening_fx_rates_usd_by_currency", "opening_fx_rates_updated_at_by_currency", opening_balances),
                ("closing", end, "closing_fx_rates_usd_by_currency", "closing_fx_rates_updated_at_by_currency", closing_balances),
            ):
                pm_rates = pm.get(rates_key)
                pm_times = pm.get(times_key)
                if not isinstance(pm_rates, dict) or not isinstance(pm_times, dict):
                    return None
                marks_for_boundary = {}
                for currency in balances_at_boundary:
                    timestamp = pm_times.get(currency)
                    rate = pm_rates.get(currency)
                    if currency == "USD" and timestamp == 0 and rate == 1.0:
                        source_record_id = f"fixed-usd-parity:{boundary_time}"
                        source_cursor = 1
                        source_epoch = "usd-parity:v1"
                        source_system = "fixed_usd_parity"
                    else:
                        candidate = next((row for stored, row in raw_fx
                                          if row.get("currency") == currency
                                          and row.get("portfolio_id") == opening_portfolio_id
                                          and row.get("rate_usd") == rate
                                          and stored.get("source_timestamp_utc_ms") == timestamp), None)
                        if candidate is None:
                            return None
                        stored = next(stored for stored, row in raw_fx if row is candidate)
                        source_record_id = stored.get("source_record_id")
                        source_cursor = stored.get("source_cursor")
                        source_epoch = stored.get("source_epoch")
                        source_system = stored.get("source_system")
                    marks_for_boundary[currency] = (rate, timestamp)
                    fx_marks.append({"boundary": boundary, "currency": currency,
                                     "rate_usd": rate, "updated_at_utc_ms": timestamp,
                                     "schema_version": 1, "source_system": source_system,
                                     "source_record_id": source_record_id,
                                     "source_epoch": source_epoch, "source_cursor": source_cursor})
                boundary_maps[boundary] = marks_for_boundary
            opening_values = {currency: float(amount) * float(boundary_maps["opening"][currency][0])
                              for currency, amount in opening_balances.items()}
            closing_values = {currency: float(amount) * float(boundary_maps["closing"][currency][0])
                              for currency, amount in closing_balances.items()}
            interval_cash_movements = dict(cash_movements)
            contribution_totals = defaultdict(lambda: ZERO)
            for row in independent_contributions:
                contribution_totals[row["currency"]] += dec(row["amount"])
            contributions_usd = sum((dec(row["value_usd"]) for row in independent_contributions), ZERO)
            intervals.append({
                "interval_id": f"paper_exchange_reconstruction:{page_epoch}:{pm.get('evidence_id')}",
                "complete": True,
                **{key: pm.get(key) for key in ("portfolio_id", "execution_mode", "ledger_epoch",
                    "interval_start_utc_ms", "interval_end_utc_ms", "fill_cursor_start",
                    "fill_cursor_end", "trade_row_cursor_end", "topup_cursor_start", "topup_cursor_end")},
                "opening_cash_balances_by_currency": {k: float(v) for k, v in opening_balances.items()},
                "closing_cash_balances_by_currency": {k: float(v) for k, v in closing_balances.items()},
                "opening_balance_evidence": {"schema_version": 1, "source_system": "paper_exchange_event_reconstruction",
                    "source_record_id": f"opening:{opening_stored['source_record_id']}", "source_epoch": page_epoch,
                    "source_cursor": opening_stored["source_cursor"], "source_timestamp_utc_ms": start,
                    "balances_by_currency": {k: float(v) for k, v in opening_balances.items()}},
                "closing_balance_evidence": {"schema_version": 1, "source_system": "paper_exchange_event_reconstruction",
                    "source_record_id": f"derived-close:{page_epoch}:{pm.get('evidence_id')}", "source_epoch": page_epoch,
                    "source_cursor": page.get("max_cursor"), "source_timestamp_utc_ms": end,
                    "balances_by_currency": {k: float(v) for k, v in closing_balances.items()}},
                "opening_cash_value_usd_by_currency": opening_values,
                "closing_cash_value_usd_by_currency": closing_values,
                "opening_cash_total_usd": sum(opening_values.values()),
                "closing_cash_total_usd": sum(closing_values.values()),
                "cash_movements_by_currency": {k: float(v) for k, v in interval_cash_movements.items()},
                "external_contributions_by_currency": {k: float(v) for k, v in contribution_totals.items()},
                "external_contribution_events": independent_contributions,
                "known_fee_fills": len(interval_fills), "unknown_fee_fills": 0,
                "fee_schedules": projected["fee_schedules"], "fills": interval_fills,
                "fx_marks": fx_marks,
            })
    except (ValueError, TypeError, KeyError, StopIteration):
        return None

    return {
        "schema_version": 1,
        "contract": "paper_exchange_portfolio_interval_v1",
        "source_system": "paper_exchange",
        "execution_mode": "paper",
        "independent_source": True,
        "complete": True,
        "snapshot_id": f"paper-exchange-reconstructed:{page_epoch}:{page.get('max_cursor')}",
        "baseline_evidence_id": ordered_pm[0].get("baseline_evidence_id"),
        "fixture_only": False,
        "intervals": intervals,
        "source_attribution_status": page.get("source_attribution_status"),
    }


def reconcile_independent_portfolio_intervals(
    position_intervals,
    independent_contract,
    interval_source_coverage=None,
    position_trades=None,
    position_source_coverage=None,
):
    """Reconcile PM flat-cash intervals to a separately versioned source.

    This contract compares boundaries, fill/top-up cursors, per-currency cash,
    identified contributions, known fee-bearing fills, and timestamped FX
    marks. It publishes interval net cash return fields only when every input
    interval reconciles. It never estimates open-position returns.
    """
    result = {
        "status": "BLOCKED",
        "independent_portfolio_interval_reconciled": False,
        "position_interval_count": 0,
        "independent_interval_count": 0,
        "matched_interval_count": 0,
        "mismatch_count": 0,
        "intervals": [],
        "reconciled_flat_cash_period_net_pnl_usd": None,
        "reconciled_flat_cash_period_return_percent": None,
        "source_is_synthetic_test_fixture": False,
        "blockers": [],
        "note": (
            "Return fields require complete independent interval, fill, contribution, "
            "balance and timestamp-aligned FX reconciliation; this is not a profitability conclusion."
        ),
    }

    def block(reason):
        if reason not in result["blockers"]:
            result["blockers"].append(reason)

    def is_integer(value):
        return isinstance(value, int) and not isinstance(value, bool)

    def number(value, label):
        try:
            return dec(value)
        except (TypeError, ValueError) as error:
            raise ValueError(f"{label} is missing or invalid") from error

    def money_map(value, label, allow_empty=False):
        if not isinstance(value, dict) or (not value and not allow_empty):
            raise ValueError(f"{label} is missing or empty")
        normalized = {}
        for currency, amount in value.items():
            if not isinstance(currency, str) or not currency.strip():
                raise ValueError(f"{label} has an invalid currency")
            normalized[currency] = number(amount, f"{label}.{currency}")
        return normalized

    def int_map(value, label):
        if not isinstance(value, dict) or not value:
            raise ValueError(f"{label} is missing or empty")
        normalized = {}
        for currency, timestamp in value.items():
            if not isinstance(currency, str) or not currency.strip() or not is_integer(timestamp):
                raise ValueError(f"{label} has an invalid timestamp")
            normalized[currency] = timestamp
        return normalized

    def equal_money_maps(left, right, label):
        if set(left) != set(right):
            raise ValueError(f"{label} currency coverage mismatch")
        for currency in left:
            if abs(left[currency] - right[currency]) > PORTFOLIO_INTERVAL_RECONCILIATION_TOLERANCE:
                raise ValueError(f"{label} amount mismatch for {currency}")

    def require_independent_source(source, source_record_id, label):
        if not isinstance(source, str) or not source.strip() or not isinstance(source_record_id, str) or not source_record_id.strip():
            raise ValueError(f"{label} lacks independent source and source-record identity")
        normalized_source = source.strip().lower().replace("-", "_").replace(" ", "_")
        if normalized_source in {
            "pm", "position_management", "position_manager",
            "position_manager_account_topup_ledger", "auto_trader",
        }:
            raise ValueError(f"{label} is copied from the Position Management or AutoTrader source")

    def interval_identity(interval):
        fields = (
            "portfolio_id",
            "execution_mode",
            "ledger_epoch",
            "interval_start_utc_ms",
            "interval_end_utc_ms",
            "fill_cursor_start",
            "fill_cursor_end",
            "trade_row_cursor_end",
            "topup_cursor_start",
            "topup_cursor_end",
        )
        if not isinstance(interval, dict):
            raise ValueError("Interval row is not an object")
        identity = tuple(interval.get(field) for field in fields)
        if (
            not isinstance(identity[0], str)
            or not identity[0]
            or identity[1] != "paper"
            or not isinstance(identity[2], str)
            or not identity[2]
            or any(not is_integer(value) for value in identity[3:])
            or identity[4] <= identity[3]
            or identity[6] < identity[5]
            or identity[7] < 0
            or identity[9] < identity[8]
        ):
            raise ValueError("Interval identity or cursor bounds are incomplete")
        return identity

    def sequence_for_fill_id(fill_id, epoch):
        prefix = f"pm-fill:v1:{epoch}:"
        if not isinstance(fill_id, str) or not fill_id.startswith(prefix):
            return None
        suffix = fill_id[len(prefix):]
        if not suffix.isdigit():
            return None
        sequence = int(suffix)
        return sequence if sequence > 0 else None

    def contribution_projection(rows, identity, label):
        if not isinstance(rows, list):
            raise ValueError(f"{label} contribution events are unavailable")
        _, _, _, interval_start, interval_end, _, _, _, topup_start, topup_end = identity
        projected = {}
        seen_ids = set()
        by_currency = defaultdict(lambda: ZERO)
        for row in rows:
            if not isinstance(row, dict):
                raise ValueError(f"{label} contribution row is not an object")
            contribution_id = row.get("contribution_id")
            source = row.get("source")
            source_cursor = row.get("source_cursor")
            timestamp = row.get("timestamp_utc_ms")
            currency = row.get("currency")
            if (
                not isinstance(contribution_id, str)
                or not contribution_id
                or not isinstance(source, str)
                or not source
                or not is_integer(source_cursor)
                or not topup_start <= source_cursor < topup_end
                or not is_integer(timestamp)
                or not interval_start <= timestamp < interval_end
                or not isinstance(currency, str)
                or not currency
            ):
                raise ValueError(f"{label} contribution identity is incomplete")
            if contribution_id in seen_ids:
                raise ValueError(f"{label} contribution ID is duplicated")
            seen_ids.add(contribution_id)
            amount = number(row.get("amount"), f"{label} contribution amount")
            value_usd = number(row.get("value_usd"), f"{label} contribution USD value")
            if label == "PM":
                projection_id = contribution_id
                if source != "position_manager_account_topup_ledger":
                    raise ValueError("PM contribution row has an unexpected source")
                independent_record_id = row.get("independent_source_record_id")
                independent_source_epoch = row.get("independent_source_epoch")
                independent_source_cursor = row.get("independent_source_cursor")
            else:
                require_independent_source(source, row.get("source_record_id"), "Independent contribution")
                projection_id = row.get("pm_contribution_id")
                provider_cursor = row.get("provider_source_cursor")
                provider_epoch = row.get("provider_source_epoch")
                if (
                    not isinstance(projection_id, str) or not projection_id
                    or not is_integer(provider_cursor) or provider_cursor <= 0
                    or not isinstance(provider_epoch, str) or not provider_epoch
                ):
                    raise ValueError("Independent contribution lacks provider epoch/cursor or PM linkage")
                independent_record_id = row.get("source_record_id")
                independent_source_epoch = provider_epoch
                independent_source_cursor = provider_cursor
            if projection_id in projected:
                raise ValueError(f"{label} contribution PM link is duplicated")
            projected[projection_id] = {
                "contribution_id": contribution_id,
                "source": source,
                "source_record_id": independent_record_id,
                "independent_source_epoch": independent_source_epoch,
                "independent_source_cursor": independent_source_cursor,
                "source_cursor": source_cursor,
                "timestamp_utc_ms": timestamp,
                "currency": currency,
                "amount": amount,
                "value_usd": value_usd,
            }
            by_currency[currency] += amount
        if len(projected) != topup_end - topup_start:
            raise ValueError(f"{label} contribution cursor coverage is incomplete")
        return projected, dict(by_currency)

    def validate_flat_interval(pm, external, identity):
        _, _, epoch, interval_start, interval_end, fill_start, fill_end, _, topup_start, topup_end = identity
        if pm.get("baseline_evidence_id") != interval_source_coverage.get("baseline_evidence_id"):
            raise ValueError("PM interval does not link to the verified opening baseline")
        if (
            pm.get("schema_version") != 1
            or pm.get("is_baseline") is not False
            or pm.get("complete") is not True
            or pm.get("truncated") is not False
            or pm.get("coverage_gap") is not False
            or pm.get("heartbeat_gap") is not False
            or pm.get("ledger_epoch_changed") is not False
            or pm.get("ledger_epoch") != epoch
            or pm.get("portfolio_return_complete") is not True
            or pm.get("portfolio_return_completeness_reasons") != []
            or pm.get("portfolio_return_scope") != "pm_internal_flat_cash_only"
            or pm.get("all_symbols_covered") is not True
            or pm.get("unknown_cashflow_fills") != 0
            or pm.get("unknown_fee_fills") != 0
            or pm.get("legacy_fill_rows_without_allocator_identity") != 0
        ):
            raise ValueError("PM interval is incomplete, non-flat, or outside flat-cash scope")
        if external.get("complete") is not True:
            raise ValueError("Independent interval is not marked complete")
        if (
            not is_integer(pm.get("known_fee_fills"))
            or not is_integer(pm.get("unknown_fee_fills"))
            or pm.get("known_fee_fills") < 0
            or pm.get("unknown_fee_fills") < 0
        ):
            raise ValueError("PM fee fill counts are missing or invalid")
        positions = pm.get("positions")
        if not isinstance(positions, list):
            raise ValueError("PM flat-cash position coverage is unavailable")
        for position in positions:
            if not isinstance(position, dict):
                raise ValueError("PM position row is invalid")
            if "quantity" in position:
                if abs(number(position["quantity"], "PM position quantity")) > PORTFOLIO_INTERVAL_RECONCILIATION_TOLERANCE:
                    raise ValueError("PM interval contains a non-flat position")
            elif not (
                isinstance(position.get("status"), str)
                and position["status"].startswith("closed")
                and is_integer(position.get("closed_at_ms"))
            ):
                raise ValueError("PM interval contains an unvalued dated-carry position")
        for interval in (pm, external):
            if interval_identity(interval) != identity:
                raise ValueError("Shared portfolio, epoch, interval, or cursor identity mismatch")

        opening = money_map(pm.get("opening_cash_balances_by_currency"), "PM opening cash")
        closing = money_map(pm.get("cash_balances_by_currency"), "PM closing cash")
        external_opening = money_map(external.get("opening_cash_balances_by_currency"), "Independent opening cash")
        external_closing = money_map(external.get("closing_cash_balances_by_currency"), "Independent closing cash")
        equal_money_maps(opening, external_opening, "Opening cash")
        equal_money_maps(closing, external_closing, "Closing cash")
        for boundary, balances, boundary_ms in (
            ("opening", external_opening, interval_start),
            ("closing", external_closing, interval_end),
        ):
            balance_source = external.get(f"{boundary}_balance_evidence")
            if not isinstance(balance_source, dict):
                raise ValueError(f"Independent {boundary} account balance source evidence is unavailable")
            require_independent_source(
                balance_source.get("source_system"),
                balance_source.get("source_record_id"),
                f"Independent {boundary} account balance",
            )
            if (
                balance_source.get("schema_version") != 1
                or not isinstance(balance_source.get("source_epoch"), str)
                or not balance_source.get("source_epoch")
                or not is_integer(balance_source.get("source_cursor"))
                or balance_source.get("source_cursor") <= 0
                or balance_source.get("source_timestamp_utc_ms") != boundary_ms
            ):
                raise ValueError(f"Independent {boundary} account balance provenance is incomplete or misaligned")
            equal_money_maps(
                balances,
                money_map(balance_source.get("balances_by_currency"), f"Independent {boundary} source balances"),
                f"Independent {boundary} source balances",
            )

        marks = external.get("fx_marks")
        if not isinstance(marks, list):
            raise ValueError("Independent timestamped FX marks are unavailable")
        mark_map = {}
        for mark in marks:
            if not isinstance(mark, dict):
                raise ValueError("Independent FX mark row is invalid")
            boundary, currency = mark.get("boundary"), mark.get("currency")
            if boundary not in ("opening", "closing") or not isinstance(currency, str) or not currency:
                raise ValueError("Independent FX mark identity is invalid")
            require_independent_source(
                mark.get("source_system"), mark.get("source_record_id"), "Independent FX mark"
            )
            if (
                mark.get("schema_version") != 1
                or not isinstance(mark.get("source_epoch"), str)
                or not mark.get("source_epoch")
                or not is_integer(mark.get("source_cursor"))
                or mark.get("source_cursor") <= 0
            ):
                raise ValueError("Independent FX mark provenance is incomplete")
            key = (boundary, currency)
            if key in mark_map:
                raise ValueError("Independent FX mark is duplicated")
            mark_map[key] = mark

        valuations = {}
        for boundary, balances, boundary_ms, pm_rates_key, pm_times_key, pm_values_key, independent_values_key, pm_total_key, independent_total_key in (
            ("opening", opening, interval_start, "opening_fx_rates_usd_by_currency", "opening_fx_rates_updated_at_by_currency", "opening_cash_value_usd_by_currency", "opening_cash_value_usd_by_currency", "opening_cash_total_usd", "opening_cash_total_usd"),
            ("closing", closing, interval_end, "closing_fx_rates_usd_by_currency", "closing_fx_rates_updated_at_by_currency", "closing_cash_value_usd_by_currency", "closing_cash_value_usd_by_currency", "closing_cash_total_usd", "closing_cash_total_usd"),
        ):
            pm_rates = money_map(pm.get(pm_rates_key), f"PM {boundary} FX rates")
            pm_times = int_map(pm.get(pm_times_key), f"PM {boundary} FX timestamps")
            external_values = money_map(external.get(independent_values_key), f"Independent {boundary} USD valuation")
            pm_values = money_map(pm.get(pm_values_key), f"PM {boundary} USD valuation")
            if set(pm_rates) != set(balances) or set(pm_times) != set(balances):
                raise ValueError(f"{boundary.title()} FX mark currency coverage is incomplete")
            if set(pm_values) != set(balances) or set(external_values) != set(balances):
                raise ValueError(f"{boundary.title()} USD valuation currency coverage is incomplete")
            boundary_values = {}
            for currency, balance in balances.items():
                mark = mark_map.get((boundary, currency))
                if mark is None:
                    raise ValueError(f"Missing independent {boundary} FX mark for {currency}")
                rate = number(mark.get("rate_usd"), f"Independent {boundary} FX rate {currency}")
                updated_at = mark.get("updated_at_utc_ms")
                if (
                    not is_integer(updated_at)
                    or updated_at != pm_times[currency]
                    or abs(rate - pm_rates[currency]) > PORTFOLIO_INTERVAL_RECONCILIATION_TOLERANCE
                ):
                    raise ValueError(f"Timestamp-aligned {boundary} FX mark mismatch for {currency}")
                if updated_at > boundary_ms or (currency != "USD" and boundary_ms - updated_at > PORTFOLIO_FX_MAX_AGE_MS):
                    raise ValueError(f"Independent {boundary} FX mark is future-dated or stale for {currency}")
                if currency == "USD" and rate != Decimal("1"):
                    raise ValueError("USD FX mark must equal one")
                valued = balance * rate
                if abs(valued - pm_values[currency]) > PORTFOLIO_INTERVAL_RECONCILIATION_TOLERANCE or abs(valued - external_values[currency]) > PORTFOLIO_INTERVAL_RECONCILIATION_TOLERANCE:
                    raise ValueError(f"{boundary.title()} cash valuation mismatch for {currency}")
                boundary_values[currency] = valued
            total_usd = sum(boundary_values.values(), ZERO)
            pm_total = number(pm.get(pm_total_key), f"PM {boundary} cash total")
            external_total = number(external.get(independent_total_key), f"Independent {boundary} cash total")
            if abs(total_usd - pm_total) > PORTFOLIO_INTERVAL_RECONCILIATION_TOLERANCE or abs(total_usd - external_total) > PORTFOLIO_INTERVAL_RECONCILIATION_TOLERANCE:
                raise ValueError(f"{boundary.title()} cash total mismatch")
            valuations[boundary] = boundary_values

        pm_contributions, pm_contributions_by_currency = contribution_projection(
            pm.get("external_contribution_events"), identity, "PM"
        )
        external_contributions, external_contributions_by_currency = contribution_projection(
            external.get("external_contribution_events"), identity, "Independent"
        )
        if set(pm_contributions) != set(external_contributions):
            raise ValueError("Independent contribution rows do not link to every PM contribution")
        for contribution_id in pm_contributions:
            pm_contribution = pm_contributions[contribution_id]
            independent_contribution = external_contributions[contribution_id]
            for field in (
                "source_record_id", "independent_source_epoch", "independent_source_cursor",
                "source_cursor", "timestamp_utc_ms", "currency", "amount", "value_usd",
            ):
                if pm_contribution[field] != independent_contribution[field]:
                    raise ValueError("Independently sourced contribution does not match PM-linked event")
        pm_contribution_map = money_map(pm.get("external_contributions_by_currency"), "PM contributions by currency") if pm.get("external_contributions_by_currency") else {}
        external_contribution_map = money_map(external.get("external_contributions_by_currency"), "Independent contributions by currency") if external.get("external_contributions_by_currency") else {}
        if pm_contributions_by_currency != pm_contribution_map or pm_contributions_by_currency != external_contributions_by_currency or pm_contributions_by_currency != external_contribution_map:
            raise ValueError("External contribution currency totals mismatch")
        for contribution_id, contribution in pm_contributions.items():
            if contribution["currency"] != "USD":
                raise ValueError(f"Contribution {contribution_id} lacks timestamp-aligned PM FX valuation")
            if contribution["value_usd"] != contribution["amount"]:
                raise ValueError(f"USD contribution {contribution_id} has mismatched value")

        sequence_start, sequence_end = fill_start, fill_end
        if pm.get("known_fee_fills") + pm.get("unknown_fee_fills") != sequence_end:
            raise ValueError("PM lifetime fee counts do not match its allocator cursor")
        pm_interval_fills = []
        pm_sequences = set()
        for trade in position_trades:
            if not isinstance(trade, dict):
                raise ValueError("Position fill row is invalid")
            sequence = sequence_for_fill_id(trade.get("fill_id"), epoch)
            if sequence is None:
                continue
            if sequence_start < sequence <= sequence_end:
                timestamp = trade.get("timestamp")
                if not is_integer(timestamp) or not interval_start <= timestamp < interval_end:
                    raise ValueError("PM fill timestamp is outside its interval bounds")
                if sequence in pm_sequences:
                    raise ValueError("PM fill cursor is duplicated")
                pm_sequences.add(sequence)
                pm_interval_fills.append(trade)
        if pm_sequences != set(range(sequence_start + 1, sequence_end + 1)):
            raise ValueError("PM fill rows do not cover the shared fill cursor range")
        if len(pm_interval_fills) != sequence_end - sequence_start:
            raise ValueError("PM interval fill count does not match its cursor range")
        if external.get("known_fee_fills") != len(pm_interval_fills) or external.get("unknown_fee_fills") != 0:
            raise ValueError("Independent fill fee coverage is incomplete or mismatched")
        external_fills = external.get("fills")
        if not isinstance(external_fills, list):
            raise ValueError("Independent interval fills are unavailable")
        external_sequences = [row.get("fill_cursor") if isinstance(row, dict) else None for row in external_fills]
        if (
            any(not is_integer(sequence) for sequence in external_sequences)
            or len(set(external_sequences)) != len(external_sequences)
            or set(external_sequences) != pm_sequences
        ):
            raise ValueError("Independent fill rows do not cover the shared fill cursor range")
        interval_position_coverage = {
            "has_older": False,
            "total_records": len(pm_interval_fills),
            "returned_records": len(pm_interval_fills),
        }
        fill_result = reconcile_independent_execution_fills(
            pm_interval_fills, {
                "schema_version": 1,
                "source_system": "paper_exchange",
                "execution_mode": "paper",
                "independent_source": True,
                "complete": True,
                "snapshot_id": independent_contract["snapshot_id"],
                "fee_schedules": external.get("fee_schedules"),
                "fills": external_fills,
            }, interval_position_coverage
        )
        if not fill_result["independent_execution_ledger_reconciled"]:
            raise ValueError(
                "Interval fill and known fee rows do not reconcile: "
                + "; ".join(fill_result["blockers"])
            )

        fill_cash_movements = defaultdict(lambda: ZERO)
        for fill in external_fills:
            symbol = fill.get("symbol")
            if not isinstance(symbol, str) or "-" not in symbol:
                raise ValueError("Independent fill has an invalid currency pair")
            _, quote = symbol.rsplit("-", 1)
            if not quote:
                raise ValueError("Independent fill has an invalid quote currency")
            quantity = number(fill.get("quantity"), "Independent fill quantity")
            price = number(fill.get("price", fill.get("execution_price")), "Independent fill price")
            sign = Decimal("-1") if fill.get("side") == "buy" else Decimal("1")
            fill_cash_movements[quote] += sign * quantity * price
            fee_currency = fill.get("fee_currency")
            if not isinstance(fee_currency, str) or not fee_currency:
                raise ValueError("Independent known fee currency is missing")
            fill_cash_movements[fee_currency] += number(fill.get("fee"), "Independent signed fee cashflow")
        for currency, amount in external_contributions_by_currency.items():
            fill_cash_movements[currency] += amount
        pm_movements = money_map(pm.get("cash_movements_by_currency"), "PM cash movements", allow_empty=True)
        external_movements = money_map(external.get("cash_movements_by_currency"), "Independent cash movements", allow_empty=True)
        equal_money_maps(dict(fill_cash_movements), pm_movements, "PM cash movements from fills/contributions")
        equal_money_maps(dict(fill_cash_movements), external_movements, "Independent cash movements from fills/contributions")
        for currency in set(opening) | set(closing) | set(fill_cash_movements):
            opening_amount = opening.get(currency, ZERO)
            closing_amount = closing.get(currency, ZERO)
            movement = fill_cash_movements.get(currency, ZERO)
            if abs(closing_amount - opening_amount - movement) > PORTFOLIO_INTERVAL_RECONCILIATION_TOLERANCE:
                raise ValueError(f"Opening balance plus fills/contributions does not reconcile for {currency}")

        opening_total = sum(valuations["opening"].values(), ZERO)
        closing_total = sum(valuations["closing"].values(), ZERO)
        contributions_usd = sum(
            (contribution["value_usd"] for contribution in pm_contributions.values()), ZERO
        )
        net_pnl = closing_total - opening_total - contributions_usd
        opening_capital = number(pm.get("opening_cash_total_usd"), "PM opening USD capital")
        if opening_capital <= 0:
            raise ValueError("PM opening USD capital is non-positive")
        period_return = net_pnl / opening_capital * Decimal("100")
        pm_net_pnl = number(pm.get("flat_cash_period_net_pnl_usd"), "PM internal flat-cash net PnL")
        pm_return = number(pm.get("flat_cash_period_return_pct"), "PM internal flat-cash return")
        if abs(net_pnl - pm_net_pnl) > PORTFOLIO_INTERVAL_RECONCILIATION_TOLERANCE:
            raise ValueError("Independently reconciled cash PnL differs from PM interval")
        if abs(period_return - pm_return) > PORTFOLIO_INTERVAL_RECONCILIATION_TOLERANCE:
            raise ValueError("Independently reconciled cash return differs from PM interval")
        return {
            "status": "RECONCILED",
            "evidence_id": pm.get("evidence_id"),
            "independent_interval_id": external.get("interval_id"),
            "reconciled_flat_cash_period_net_pnl_usd": rounded(net_pnl),
            "reconciled_flat_cash_period_return_percent": rounded(period_return, 8),
            "position_fill_rows_reconciled": len(pm_interval_fills),
            "known_fee_fill_rows_reconciled": len(pm_interval_fills),
            "contribution_events_reconciled": len(pm_contributions),
        }

    coverage_ok = (
        isinstance(interval_source_coverage, dict)
        and interval_source_coverage.get("schema_version") == 1
        and interval_source_coverage.get("coverage") == "pm_internal_complete"
        and interval_source_coverage.get("truncated") is False
        and interval_source_coverage.get("baseline_verified") is True
        and isinstance(interval_source_coverage.get("baseline_evidence_id"), str)
        and bool(interval_source_coverage.get("baseline_evidence_id"))
        and isinstance(position_intervals, list)
        and bool(position_intervals)
        and interval_source_coverage.get("returned_intervals") == len(position_intervals)
    )
    if not coverage_ok:
        block("PM portfolio interval source is missing, truncated, or incomplete")
        return result
    if (
        not isinstance(independent_contract, dict)
        or independent_contract.get("schema_version") != 1
        or independent_contract.get("contract") != "paper_exchange_portfolio_interval_v1"
        or independent_contract.get("source_system") != "paper_exchange"
        or independent_contract.get("execution_mode") != "paper"
        or independent_contract.get("independent_source") is not True
        or independent_contract.get("complete") is not True
        or independent_contract.get("baseline_evidence_id")
        != interval_source_coverage.get("baseline_evidence_id")
        or not isinstance(independent_contract.get("snapshot_id"), str)
        or not independent_contract.get("snapshot_id")
        or not isinstance(independent_contract.get("intervals"), list)
    ):
        block("Independent versioned Paper Exchange portfolio interval contract is unavailable or incomplete")
        return result
    result["source_is_synthetic_test_fixture"] = independent_contract.get("fixture_only") is True
    result["position_interval_count"] = len(position_intervals)
    result["independent_interval_count"] = len(independent_contract["intervals"])
    if len(position_intervals) != len(independent_contract["intervals"]):
        block("PM and independent interval counts do not match")
        return result
    if (
        not isinstance(position_source_coverage, dict)
        or position_source_coverage.get("has_older") is not False
        or position_source_coverage.get("total_records") != position_source_coverage.get("returned_records")
        or not isinstance(position_trades, list)
        or position_source_coverage.get("returned_records") != len(position_trades)
    ):
        block("Complete Position Management fill rows are required for interval reconciliation")
        return result

    def order_intervals(intervals, label):
        by_identity = {}
        for interval in intervals:
            try:
                identity = interval_identity(interval)
            except ValueError as error:
                raise ValueError(f"{label}: {error}") from error
            if identity in by_identity:
                raise ValueError(f"{label} interval identity is duplicated")
            by_identity[identity] = interval
        return by_identity

    try:
        pm_by_identity = order_intervals(position_intervals, "PM")
        source_by_identity = order_intervals(independent_contract["intervals"], "Independent")
    except ValueError as error:
        block(str(error))
        return result
    if set(pm_by_identity) != set(source_by_identity):
        block("Shared portfolio, epoch, interval, or cursor bounds do not match")
        return result

    ordered_identities = sorted(
        pm_by_identity, key=lambda item: (item[4], item[3], item[5], item[6])
    )
    for previous_identity, current_identity in zip(
        ordered_identities, ordered_identities[1:]
    ):
        if (
            previous_identity[0:3] != current_identity[0:3]
            or previous_identity[4] != current_identity[3]
            or previous_identity[6] != current_identity[5]
            or previous_identity[9] != current_identity[8]
        ):
            block("PM and independent interval series has a time, epoch, fill, or top-up cursor gap")
            return result

    reconciled_intervals = []
    mismatches = []
    for identity in ordered_identities:
        try:
            interval_result = validate_flat_interval(
                pm_by_identity[identity], source_by_identity[identity], identity
            )
        except (KeyError, TypeError, ValueError, ArithmeticError) as error:
            mismatches.append({
                "evidence_id": pm_by_identity[identity].get("evidence_id"),
                "independent_interval_id": source_by_identity[identity].get("interval_id"),
                "reason": str(error),
            })
            continue
        reconciled_intervals.append(interval_result)
    result["matched_interval_count"] = len(reconciled_intervals)
    result["mismatch_count"] = len(mismatches)
    result["intervals"] = reconciled_intervals + mismatches
    if mismatches or len(reconciled_intervals) != len(position_intervals):
        result["status"] = "MISMATCH"
        # Do not expose per-interval returns from a partial portfolio history.
        # A neighboring missing or mismatched interval invalidates the claimed
        # complete return series, even when one isolated pair matched.
        for interval in reconciled_intervals:
            interval["status"] = "WITHHELD"
            interval["reconciled_flat_cash_period_net_pnl_usd"] = None
            interval["reconciled_flat_cash_period_return_percent"] = None
            interval["return_fields_withheld_reason"] = "portfolio interval series is not fully reconciled"
        result["blockers"].extend(
            f"Portfolio interval mismatch {row.get('evidence_id')}: {row['reason']}"
            for row in mismatches[:100]
        )
        return result
    result.update({
        "status": "RECONCILED",
        "independent_portfolio_interval_reconciled": True,
        "reconciled_flat_cash_period_net_pnl_usd": [
            interval["reconciled_flat_cash_period_net_pnl_usd"]
            for interval in reconciled_intervals
        ],
        "reconciled_flat_cash_period_return_percent": [
            interval["reconciled_flat_cash_period_return_percent"]
            for interval in reconciled_intervals
        ],
    })
    return result


SNAPSHOT_SOURCES = {}
SNAPSHOT_MANIFEST = None

REQUIRED_ENDPOINTS = {
    "account",
    "positions",
    "realized-pnl",
    "fee-summary",
    "fee-rates"
}

def load_snapshot_manifest(filename):
    global SNAPSHOT_SOURCES, SNAPSHOT_MANIFEST

    cache = (
        Path.home() / ".cache/sbe-diagnostics"
    ).resolve()

    manifest = Path(filename).expanduser().resolve()

    if manifest.parent != cache:
        raise ValueError("Manifest outside evidence directory")

    raw_manifest = manifest.read_bytes()

    if len(raw_manifest) > 1_000_000:
        raise ValueError("Manifest too large")

    data = json.loads(raw_manifest)

    if data.get("status") != "COLLECTED":
        raise ValueError("Incomplete snapshot")

    sources = data.get("sources", [])

    if len(sources) == 1 and sources[0].get("endpoint") == "financial-snapshot":
        item = sources[0]
        if item.get("status") != "COLLECTED" or item.get("service") != "positions":
            raise ValueError("Invalid atomic financial snapshot metadata")
        path = Path(item["evidence"]).resolve()
        if path.parent != cache or not path.name.startswith("positions-financial-snapshot-"):
            raise ValueError("Financial snapshot evidence identity mismatch")
        if path.stat().st_size > 16_000_000:
            raise ValueError("Evidence too large")
        raw = path.read_bytes()
        payload = json.loads(raw)
        if (
            payload.get("service") != "positions"
            or payload.get("route") != "/positions/financial-snapshot"
            or payload.get("application_status") != 200
            or "body" not in payload
            or payload.get("bytes") != item.get("bytes")
        ):
            raise ValueError("Financial snapshot evidence identity mismatch")
        snapshot = payload["body"]
        snapshot_id = data.get("snapshot_id")
        if (
            not isinstance(snapshot_id, str)
            or not snapshot_id
            or snapshot.get("snapshot_id") != snapshot_id
            or snapshot.get("snapshot_consistency") != "single_locked_ledger_clone"
            or snapshot.get("ledger_snapshot_consistent") is not True
            or snapshot.get("execution_mode") != "paper"
            or data.get("ledger_snapshot_consistent") is not True
            or data.get("atomic_snapshot") is not False
        ):
            raise ValueError("Atomic financial snapshot contract mismatch")
        collected = datetime.fromisoformat(item["timestamp"])
        if collected.tzinfo is None:
            raise ValueError("Missing timestamp timezone")
        evidence = {
            "endpoint": "financial-snapshot",
            "path": str(path),
            "age_seconds": age(path),
            "modified_at": datetime.fromtimestamp(
                path.stat().st_mtime, timezone.utc
            ).isoformat(),
            "collected_at": collected.isoformat(),
            "sha256": hashlib.sha256(raw).hexdigest(),
            "snapshot_id": snapshot_id,
        }
        SNAPSHOT_SOURCES = {
            endpoint: (snapshot[key], evidence)
            for endpoint, key in {
                "account": "account",
                "positions": "positions",
                "realized-pnl": "realized_pnl",
                "fee-summary": "fee_summary",
                "fee-rates": "fee_rates",
            }.items()
        }
        SNAPSHOT_MANIFEST = {
            "path": str(manifest),
            "sha256": hashlib.sha256(raw_manifest).hexdigest(),
            "timestamp": data["timestamp"],
            "collection_seconds": data["collection_seconds"],
            "near_synchronous": True,
            "atomic_snapshot": False,
            "ledger_snapshot_consistent": True,
            "snapshot_id": snapshot_id,
            "execution_mode": "paper",
            "fx_rates_updated_at_by_currency": snapshot.get("fx_rates_updated_at_by_currency"),
            "market_marks_consistency": snapshot.get("market_marks_consistency"),
            "ledger_coverage": snapshot.get("ledger_coverage"),
            "historical_integrity_verified": False,
            "independent_execution_ledger_reconciled": False,
            "acceptance": "NOT_EVALUATED",
        }
        SNAPSHOT_MANIFEST["execution_reconciliation_inputs"] = load_execution_reconciliation_inputs(
            data.get("execution_reconciliation_sources"), cache
        )
        return

    if len(sources) != len(REQUIRED_ENDPOINTS):
        raise ValueError("Unexpected source count")

    verified = {}

    for item in sources:
        endpoint = item["endpoint"]

        if endpoint not in REQUIRED_ENDPOINTS:
            raise ValueError("Unexpected endpoint")

        if endpoint in verified:
            raise ValueError("Duplicate endpoint")

        if (
            item.get("status") != "COLLECTED"
            or item.get("service") != "positions"
        ):
            raise ValueError("Invalid source metadata")

        path = Path(item["evidence"]).resolve()

        if path.parent != cache:
            raise ValueError("Evidence outside cache")

        if not path.name.startswith(
            "positions-" + endpoint + "-"
        ):
            raise ValueError("Evidence filename mismatch")

        if path.stat().st_size > 16_000_000:
            raise ValueError("Evidence too large")

        raw = path.read_bytes()
        payload = json.loads(raw)

        if (
            payload.get("service") != "positions"
            or payload.get("route") != (
                "/positions" if endpoint == "positions"
                else "/positions/" + endpoint
            )
            or payload.get("application_status") != 200
            or "body" not in payload
            or payload.get("bytes") != item.get("bytes")
        ):
            raise ValueError(
                "Evidence identity mismatch: " + endpoint
            )

        collected = datetime.fromisoformat(
            item["timestamp"]
        )

        if collected.tzinfo is None:
            raise ValueError("Missing timestamp timezone")

        verified[endpoint] = (
            payload["body"],
            {
                "endpoint": endpoint,
                "path": str(path),
                "age_seconds": age(path),
                "modified_at": datetime.fromtimestamp(
                    path.stat().st_mtime, timezone.utc
                ).isoformat(),
                "collected_at": collected.isoformat(),
                "sha256": hashlib.sha256(raw).hexdigest()
            }
        )

    if set(verified) != REQUIRED_ENDPOINTS:
        raise ValueError("Missing required endpoints")

    SNAPSHOT_SOURCES = verified

    SNAPSHOT_MANIFEST = {
        "path": str(manifest),
        "sha256": hashlib.sha256(
            raw_manifest
        ).hexdigest(),
        "timestamp": data["timestamp"],
        "collection_seconds": data["collection_seconds"],
        "near_synchronous": data.get("near_synchronous"),
        "atomic_snapshot": False,
        "ledger_snapshot_consistent": False,
        "snapshot_id": data.get("snapshot_id"),
        "execution_mode": data.get("execution_mode"),
        "historical_integrity_verified": False,
        "acceptance": "NOT_EVALUATED"
    }
    SNAPSHOT_MANIFEST["execution_reconciliation_inputs"] = load_execution_reconciliation_inputs(
        data.get("execution_reconciliation_sources"), cache
    )


def load_execution_reconciliation_inputs(sources, cache):
    """Load optional PM fills and separate Paper Exchange evidence sources."""
    if not isinstance(sources, list) or len(sources) not in (2, 3, 4):
        return None
    contracts = {
        ("positions", "trades"): ("positions-trades-", "/positions/trades"),
        ("positions", "portfolio-evidence-intervals"):
            ("positions-portfolio-evidence-intervals-", "/positions/portfolio-evidence/intervals"),
        ("paper_exchange", "execution-ledger"):
            ("paper_exchange-execution-ledger-", "/internal/executions/ledger"),
        ("paper_exchange", "match-evidence-ledger"):
            ("paper_exchange-match-evidence-ledger-", "/internal/orders/match/ledger"),
        ("paper_exchange", "financial-evidence"):
            ("paper_exchange-financial-evidence-complete-", "/internal/financial/evidence"),
    }
    bodies = {}
    for item in sources:
        if not isinstance(item, dict):
            return None
        key = (item.get("service"), item.get("endpoint"))
        contract = contracts.get(key)
        if contract is None or key in bodies or item.get("status") != "COLLECTED":
            return None
        path = Path(item.get("evidence", "")).resolve()
        if (
            path.parent != cache
            or not path.name.startswith(contract[0])
            or path.stat().st_size > 32_000_000
        ):
            return None
        raw = path.read_bytes()
        payload = json.loads(raw)
        if (
            payload.get("service") != item["service"]
            or payload.get("route") != contract[1]
            or payload.get("application_status") != 200
            or payload.get("bytes") != item.get("bytes")
            or "body" not in payload
        ):
            return None
        bodies[key] = payload["body"]
    pm_body = bodies.get(("positions", "trades"))
    exchange_body = bodies.get(("paper_exchange", "execution-ledger"))
    match_evidence_body = bodies.get(("paper_exchange", "match-evidence-ledger"))
    financial_evidence_body = bodies.get(("paper_exchange", "financial-evidence"))
    interval_body = bodies.get(("positions", "portfolio-evidence-intervals"))
    if (
        not isinstance(pm_body, dict)
        or pm_body.get("has_older") is not False
        or not isinstance(pm_body.get("trades"), list)
        or isinstance(pm_body.get("total"), bool)
        or not isinstance(pm_body.get("total"), int)
        or pm_body.get("total") != len(pm_body["trades"])
        or not isinstance(exchange_body, dict) and not isinstance(match_evidence_body, dict)
            and not isinstance(financial_evidence_body, dict)
        or ("positions", "portfolio-evidence-intervals") in bodies
        and not isinstance(interval_body, dict)
    ):
        return None
    interval_coverage = None
    pm_portfolio_intervals = None
    if interval_body is not None:
        raw_intervals = interval_body.get("intervals")
        baseline_rows = [
            row for row in raw_intervals
            if isinstance(row, dict) and row.get("is_baseline") is True
        ] if isinstance(raw_intervals, list) else []
        all_period_rows = [
            row for row in raw_intervals
            if isinstance(row, dict) and row.get("is_baseline") is not True
        ] if isinstance(raw_intervals, list) else []
        latest_baseline = max(
            baseline_rows,
            key=lambda row: row.get("interval_end_utc_ms", -1)
            if isinstance(row.get("interval_end_utc_ms"), int) else -1,
            default=None,
        )
        baseline_end = (
            latest_baseline.get("interval_end_utc_ms")
            if isinstance(latest_baseline, dict) else None
        )
        # Keep earlier incomplete observations as history, but start this
        # reconciliation epoch at the explicit clean baseline boundary.
        period_rows = [
            row for row in all_period_rows
            if isinstance(baseline_end, int)
            and isinstance(row.get("interval_end_utc_ms"), int)
            and row["interval_end_utc_ms"] > baseline_end
        ]
        baseline_verified = False
        baseline_evidence_id = None
        if isinstance(latest_baseline, dict) and period_rows:
            baseline = latest_baseline
            baseline_evidence_id = baseline.get("evidence_id")
            baseline_verified = (
                isinstance(baseline_evidence_id, str)
                and bool(baseline_evidence_id)
                and baseline.get("complete") is True
                and isinstance(baseline.get("portfolio_id"), str)
                and bool(baseline.get("portfolio_id"))
                and baseline.get("execution_mode") == "paper"
                and isinstance(baseline.get("ledger_epoch"), str)
                and isinstance(baseline.get("interval_end_utc_ms"), int)
            )
            previous = baseline
            for row in sorted(
                period_rows,
                key=lambda value: value.get("interval_end_utc_ms", -1)
                if isinstance(value.get("interval_end_utc_ms"), int)
                else -1,
            ):
                baseline_verified = baseline_verified and (
                    row.get("complete") is True
                    and row.get("is_baseline") is False
                    and row.get("baseline_evidence_id") == baseline_evidence_id
                    and row.get("portfolio_id") == previous.get("portfolio_id")
                    and row.get("execution_mode") == previous.get("execution_mode")
                    and row.get("ledger_epoch") == previous.get("ledger_epoch")
                    and row.get("interval_start_utc_ms") == previous.get("interval_end_utc_ms")
                    and row.get("fill_cursor_start") == previous.get("fill_cursor_end")
                    and row.get("topup_cursor_start") == previous.get("topup_cursor_end")
                    and row.get("opening_cash_balances_by_currency")
                    == previous.get("cash_balances_by_currency")
                    and row.get("opening_cash_value_usd_by_currency")
                    == previous.get("closing_cash_value_usd_by_currency")
                    and row.get("opening_fx_rates_usd_by_currency")
                    == previous.get("closing_fx_rates_usd_by_currency")
                    and row.get("opening_fx_rates_updated_at_by_currency")
                    == previous.get("closing_fx_rates_updated_at_by_currency")
                )
                previous = row
        interval_coverage = {
            "schema_version": interval_body.get("schema_version"),
            "coverage": (
                "pm_internal_complete"
                if baseline_verified and period_rows
                and all(row.get("complete") is True for row in period_rows)
                and interval_body.get("truncated") is False
                else interval_body.get("coverage")
            ),
            "truncated": interval_body.get("truncated"),
            "returned_intervals": len(period_rows),
            "next_cursor": interval_body.get("next_cursor"),
            "baseline_verified": baseline_verified,
            "baseline_evidence_id": baseline_evidence_id,
        }
        if isinstance(raw_intervals, list):
            pm_portfolio_intervals = period_rows
    independent_interval_contract = (
        exchange_body.get("portfolio_interval_contract")
        if isinstance(exchange_body, dict) else None
    )
    if isinstance(financial_evidence_body, dict):
        projected = project_paper_exchange_financial_evidence_page(financial_evidence_body)
    else:
        projected = None
    if projected is not None:
        execution_ledger = projected
        if pm_portfolio_intervals is not None:
            independent_interval_contract = reconstruct_portfolio_intervals_from_paper_exchange(
                financial_evidence_body, pm_portfolio_intervals, pm_body["trades"]
            )
    else:
        execution_ledger = exchange_body
    if (
        not isinstance(independent_interval_contract, dict)
        or independent_interval_contract.get("fixture_only") is True
    ):
        independent_interval_contract = None
    return {
        "position_trades": pm_body["trades"],
        "position_source_coverage": {
            "has_older": pm_body["has_older"],
            "total_records": pm_body["total"],
            "returned_records": len(pm_body["trades"]),
        },
        "execution_ledger": execution_ledger,
        "paper_exchange_financial_evidence": financial_evidence_body,
        "match_evidence_ledger": match_evidence_body,
        "pm_portfolio_intervals": pm_portfolio_intervals,
        "portfolio_interval_source_coverage": interval_coverage,
        "independent_portfolio_interval_contract": independent_interval_contract,
    }


def source(endpoint):
    if SNAPSHOT_MANIFEST is not None:
        return SNAPSHOT_SOURCES[endpoint]
    path = latest("positions-" + endpoint)
    return body(path), {
        "endpoint": endpoint,
        "path": str(path),
        "age_seconds": age(path),
        "modified_at": datetime.fromtimestamp(
            path.stat().st_mtime, timezone.utc
        ).isoformat()
    }

def account():
    data, evidence = source("account")

    initial = dec(data["initial_cash"])
    cash = dec(data["cash"])
    equity = dec(data["equity"])
    realized = dec(data["realized_pnl"])
    unrealized = dec(data["unrealized_pnl"])
    fees = dec(data["fees_paid"])
    exit_fees = dec(data["estimated_exit_fees"])

    no_topups = data.get("topups") == []
    equity_available = data.get("equity_available") is True

    result = {
        "status": "OBSERVED",
        "currency": data["currency"],
        "initial_cash": rounded(initial),
        "cash": rounded(cash),
        "equity": rounded(equity) if equity_available else None,
        "realized_pnl": rounded(realized),
        "unrealized_pnl": rounded(unrealized),
        "fees_paid": rounded(fees),
        "estimated_exit_fees": rounded(exit_fees),
        "max_leverage": data.get("max_leverage"),
        "equity_change": rounded(equity-initial)
            if equity_available and no_topups else None,
        "equity_return_percent": pct(equity-initial,initial)
            if equity_available and no_topups else None,
        "equity_minus_cash": rounded(equity-cash)
            if equity_available else None,
        "fees_relative_to_absolute_realized_pnl_percent":
            pct(fees,abs(realized)),
        "topups_present": not no_topups,
        "evidence": evidence
    }

    if no_topups and equity_available:
        result["accounting_hypotheses"] = {
            "residual_if_realized_and_unrealized_are_net":
                rounded(equity-initial-realized-unrealized),
            "residual_if_both_are_gross_and_fees_separate":
                rounded(equity-initial-realized-unrealized+fees),
            "residual_if_pnl_net_but_exit_fees_separate":
                rounded(equity-initial-realized-unrealized+exit_fees),
            "residual_if_equity_excludes_estimated_exit_fees":
                rounded(equity-initial-realized-unrealized-exit_fees)
        }

    result["note"] = (
        "Accounting hypotheses only. Fee inclusion, "
        "inventory valuation and ledger scope require source verification."
    )
    return result

def fees():
    data, evidence = source("fee-summary")
    rates, rate_evidence = source("fee-rates")

    groups = defaultdict(lambda: {
        "fills": 0,
        "fees": ZERO,
        "notional": ZERO
    })
    liquidity = defaultdict(int)
    rows = []

    for row in data["rows"]:
        currency = row["fee_currency"]
        kind = row["liquidity"]
        origin = row["source"]
        count = row["fills"]

        if type(count) is not int or count < 0:
            raise ValueError("Invalid fill count")

        paid = dec(row["fees_paid"])
        notional = dec(row["traded_notional"])

        key = (currency, kind, origin)
        group = groups[key]
        group["fills"] += count
        group["fees"] += paid
        group["notional"] += notional
        liquidity[kind] += count

    for (currency, kind, origin), group in sorted(groups.items()):
        paid = group["fees"]
        notional = group["notional"]
        configured = rates.get(kind)

        rows.append({
            "currency": currency,
            "liquidity": kind,
            "source": origin,
            "fills": group["fills"],
            "fees_paid": rounded(paid),
            "traded_notional": rounded(notional),
            "effective_bps": rounded(paid/notional*10000,4)
                if notional > 0 else None,
            "current_configured_bps":
                rounded(dec(configured)*10000,4)
                if configured is not None else None
        })

    observed = sum(liquidity.values())
    reported = data["fills"]

    return {
        "status": "OBSERVED",
        "reported_fills": reported,
        "summed_fills": observed,
        "fill_count_matches": observed == reported,
        "fill_share_percent": {
            key: round(value/observed*100,4)
            for key,value in liquidity.items()
        } if observed else {},
        "first_fill_ms": data.get("first_fill_ms"),
        "last_fill_ms": data.get("last_fill_ms"),
        "rows": rows,
        "evidence": [evidence, rate_evidence],
        "note": (
            "Currencies kept separate. Current configured rates "
            "are not necessarily historical rates."
        )
    }

def positions():
    data, evidence = source("positions")
    groups = defaultdict(lambda: {
        "long_positions": 0,
        "short_positions": 0,
        "flat_positions": 0,
        "long_entry_notional": ZERO,
        "short_entry_notional": ZERO,
        "realized_pnl": ZERO,
        "fees_paid": ZERO
    })
    opened = []
    symbols = set()

    for row in data["positions"]:
        symbol = row["symbol"]

        if symbol in symbols:
            raise ValueError("Repeated position symbol: "+symbol)
        symbols.add(symbol)

        if "-" not in symbol:
            raise ValueError("Unknown symbol format: "+symbol)

        quote = symbol.rsplit("-",1)[1]
        quantity = dec(row["quantity"])
        price = dec(row["average_entry_price"])
        realized = dec(row["realized_pnl"])
        paid = dec(row["fees_paid"])

        group = groups[quote]
        group["realized_pnl"] += realized
        group["fees_paid"] += paid

        if quantity == 0:
            group["flat_positions"] += 1
            continue

        if price <= 0:
            raise ValueError("Non-positive open entry price")

        side = "long" if quantity > 0 else "short"
        notional = abs(quantity*price)

        group[side+"_positions"] += 1
        group[side+"_entry_notional"] += notional

        opened.append({
            "symbol": symbol,
            "quote": quote,
            "side": side,
            "quantity": float(quantity),
            "entry_price": float(price),
            "entry_notional": rounded(notional),
            "realized_pnl_raw": rounded(realized),
            "entry_fees_raw": rounded(dec(row["entry_fees"]))
        })

    result = {}
    for quote, group in sorted(groups.items()):
        group["gross_entry_notional"] = (
            group["long_entry_notional"] +
            group["short_entry_notional"]
        )
        group["signed_entry_notional"] = (
            group["long_entry_notional"] -
            group["short_entry_notional"]
        )
        result[quote] = {
            key: rounded(value) if isinstance(value,Decimal)
                 else value
            for key,value in group.items()
        }

    return {
        "status": "PROVISIONAL",
        "position_records": len(symbols),
        "nonzero_position_records": len(opened),
        "reported_trade_count": data.get("trade_count"),
        "by_inferred_quote": result,
        "open_positions": sorted(
            opened,key=lambda x:x["entry_notional"],reverse=True
        )[:20],
        "evidence": evidence,
        "note": (
            "Entry-price exposure proxy only. Not marked exposure, "
            "reserved capital or margin. Assumes linear quantity-price "
            "contracts. Quote units and P&L denomination require verification."
        )
    }

def realized_pnl():
    data, evidence = source("realized-pnl")
    orders = data["realized_pnl_all_time_by_parent_order"]

    if not isinstance(orders,dict):
        raise ValueError("Unexpected parent order map")

    values = [dec(v) for v in orders.values()]
    positive = [v for v in values if v > 0]
    negative = [v for v in values if v < 0]
    zero = len(values)-len(positive)-len(negative)

    wins = sum(positive,ZERO)
    losses = abs(sum(negative,ZERO))

    return {
        "status": "PROVISIONAL",
        "reported_all_time_usd":
            rounded(dec(data["realized_pnl_all_time_usd"])),
        "reported_window_usd":
            rounded(dec(data["realized_pnl_usd"])),
        "reported_all_time_by_quote":
            data["realized_pnl_all_time_by_quote"],
        "reported_window_by_quote":
            data["realized_pnl_by_quote"],
        "parent_order_records": len(values),
        "positive_parent_orders": len(positive),
        "negative_parent_orders": len(negative),
        "zero_parent_orders": zero,
        "positive_record_percent":
            round(len(positive)/len(values)*100,4)
            if values else None,
        "parent_order_raw_distribution": {
            "minimum": rounded(min(values)) if values else None,
            "maximum": rounded(max(values)) if values else None
        },
        "evidence": evidence,
        "note": (
            "Parent-order records are not necessarily independent "
            "closed trades. Their denomination and fee treatment "
            "are unverified; no cross-currency sum or profit factor."
        )
    }

def reconciliation():
    a, ae = source("account")
    p, pe = source("positions")
    r, re = source("realized-pnl")
    f, fe = source("fee-summary")

    reported = dec(r["realized_pnl_all_time_usd"])
    account_pnl = dec(a["realized_pnl"])

    evidence = [ae,pe,re,fe]
    timestamps = [
        datetime.fromisoformat(
            x.get("collected_at", x["modified_at"])
        ).timestamp()
        for x in evidence
    ]

    snapshot = SNAPSHOT_MANIFEST or {}
    coverage = snapshot.get("ledger_coverage") or {}
    reconciliation_inputs = snapshot.get("execution_reconciliation_inputs")
    if reconciliation_inputs is None:
        execution_reconciliation = reconcile_independent_execution_fills(None, None)
        match_evidence_comparison = compare_paper_exchange_match_evidence(None, None)
        portfolio_interval_reconciliation = reconcile_independent_portfolio_intervals(
            None, None
        )
    else:
        execution_reconciliation = reconcile_independent_execution_fills(
            reconciliation_inputs["position_trades"],
            reconciliation_inputs["execution_ledger"],
            reconciliation_inputs.get("position_source_coverage"),
        )
        match_evidence_comparison = compare_paper_exchange_match_evidence(
            reconciliation_inputs.get("position_trades"),
            reconciliation_inputs.get("match_evidence_ledger"),
            reconciliation_inputs.get("position_source_coverage"),
        )
        portfolio_interval_reconciliation = reconcile_independent_portfolio_intervals(
            reconciliation_inputs.get("pm_portfolio_intervals"),
            reconciliation_inputs.get("independent_portfolio_interval_contract"),
            reconciliation_inputs.get("portfolio_interval_source_coverage"),
            reconciliation_inputs.get("position_trades"),
            reconciliation_inputs.get("position_source_coverage"),
        )
    blockers = []
    if snapshot.get("ledger_snapshot_consistent") is not True:
        blockers.append("Ledger snapshot consistency not guaranteed")
    if snapshot.get("execution_mode") != "paper" or not snapshot.get("snapshot_id"):
        blockers.append("Execution mode or snapshot identity not exposed")
    else:
        blockers.append("Durable ledger epoch across restarts is not exposed")
    if (
        a.get("fees_complete") is not True
        or coverage.get("unknown_fee_fills", 0) > 0
        or coverage.get("fee_currency_unknown_rows", 0) > 0
        or coverage.get("fee_cashflow_currency_mismatch_rows", 0) > 0
    ):
        blockers.append("Fee metadata is incomplete or its currency differs from the trade quote")
    if coverage.get("rows_without_parent_order_id", 0) > 0:
        blockers.append("Some PositionTrade rows lack parent-order attribution")
    if not execution_reconciliation["independent_execution_ledger_reconciled"]:
        blockers.extend(execution_reconciliation["blockers"])
    if not portfolio_interval_reconciliation["independent_portfolio_interval_reconciled"]:
        blockers.extend(portfolio_interval_reconciliation["blockers"])
    if coverage.get("timestamp_order_inversion_rows", 0) > 0:
        blockers.append("Ledger append order includes fill timestamp inversions")

    return {
        "status": "PROVISIONAL",
        "account_currency": a["currency"],
        "account_realized_pnl": rounded(account_pnl),
        "endpoint_all_time_realized_pnl_usd": rounded(reported),
        "account_minus_endpoint_usd": rounded(account_pnl-reported)
            if a["currency"] == "USD" else None,
        "reported_fee_fills": f["fills"],
        "position_trade_count": p.get("trade_count"),
        "snapshot_skew_seconds":
            round(max(timestamps)-min(timestamps),3),
        "snapshot_id": snapshot.get("snapshot_id"),
        "execution_mode": snapshot.get("execution_mode"),
        "atomic_snapshot": snapshot.get("atomic_snapshot") is True,
        "ledger_snapshot_consistent": snapshot.get("ledger_snapshot_consistent") is True,
        "fx_rates_updated_at_by_currency": snapshot.get("fx_rates_updated_at_by_currency"),
        "market_marks_consistency": snapshot.get("market_marks_consistency"),
        "ledger_coverage": coverage,
        "independent_execution_reconciliation": execution_reconciliation,
        "paper_exchange_match_evidence_comparison": match_evidence_comparison,
        "independent_portfolio_interval_reconciliation": portfolio_interval_reconciliation,
        "evidence": evidence,
        "acceptance": "NOT_EVALUATED",
        "blockers": blockers
    }

def main(name):
    parser = argparse.ArgumentParser(description=name)
    parser.add_argument("--manifest", type=Path)
    args = parser.parse_args()

    functions = {
        "account_analysis": account,
        "fee_analysis": fees,
        "position_analysis": positions,
        "realized_pnl_analysis": realized_pnl,
        "financial_reconciliation": reconciliation
    }

    try:
        if args.manifest:
            load_snapshot_manifest(args.manifest)
        if name == "financial_health":
            data = {}
            for key, fn in functions.items():
                try:
                    data[key] = fn()
                except Exception as e:
                    data[key] = {
                        "status": "BLOCKED",
                        "error": str(e)
                    }
            data["status"] = "BLOCKED" if any(
                isinstance(v,dict) and v.get("status")=="BLOCKED"
                for v in data.values()
            ) else "PROVISIONAL"
        else:
            data = functions[name]()

        data["generated_at"] = now()
        data["snapshot_manifest"] = SNAPSHOT_MANIFEST
        data["source_selection"] = (
            "MANIFEST_BOUND" if SNAPSHOT_MANIFEST
            else "INDEPENDENT_LATEST"
        )
        path = save(data,name)
        emit({"evidence":path,"result":data})
        return 1 if data.get("status")=="BLOCKED" else 0

    except Exception as e:
        emit({"status":"BLOCKED","error":str(e)})
        return 1
