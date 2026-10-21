#!/usr/bin/env python3
"""Review signal-record volume and daily summaries from SQLite databases."""
import argparse
import gzip
import json
import os
import sqlite3
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
KIND_NAMES = {
    0: "snapshot",
    1: "outcome",
    2: "evaluation",
    3: "book_snapshot",
    4: "research_features",
    5: "external_market",
}


def discover_databases(explicit):
    if explicit:
        return [Path(value).expanduser().resolve() for value in explicit]
    if os.environ.get("SIGNALS_DB_FILE"):
        return [Path(os.environ["SIGNALS_DB_FILE"]).expanduser().resolve()]
    if os.environ.get("SIGNALS_FILE"):
        return [Path(os.environ["SIGNALS_FILE"]).expanduser().with_suffix(".sqlite").resolve()]
    return [path for path in (ROOT / "signals.sqlite", ROOT / "signals.jsonl.sqlite") if path.exists()]


def kind_name(kind):
    return KIND_NAMES.get(kind, f"unknown_{kind}")


def timestamp_text(value):
    return datetime.fromtimestamp(value / 1000, timezone.utc).isoformat()


def summarize_execution_quality(records):
    orders = [record for record in records if record.get("stage") == "entry" or str(record.get("stage", "")).startswith("exit")]
    requested = 0.0
    filled_total = 0.0
    filled_orders = 0
    partial_orders = 0
    unfilled_orders = 0
    measurable = 0
    measured_notional = 0.0
    shortfall_total = 0.0
    fee_total = 0.0
    fee_notional = 0.0
    measured_fee_total = 0.0
    for record in orders:
        is_entry = record.get("stage") == "entry"
        quantity = max(0.0, record.get("order_quantity" if is_entry else "exit_order_quantity") or 0.0)
        filled = min(quantity, max(0.0, record.get("filled_quantity" if is_entry else "exit_filled_quantity") or 0.0))
        requested += quantity
        filled_total += filled
        if filled <= 0:
            unfilled_orders += 1
            continue
        filled_orders += 1
        if quantity > 0 and filled < quantity - 1e-12:
            partial_orders += 1
        price = record.get("average_price" if is_entry else "exit_average_price")
        mid = record.get("decision_mid_price")
        fee = abs(record.get("fees" if is_entry else "exit_fees") or 0.0)
        if not price or price <= 0:
            continue
        notional = filled * price
        fee_total += fee
        fee_notional += notional
        shortfall = record.get("implementation_shortfall_bps")
        if shortfall is None or not mid or mid <= 0:
            continue
        measurable += 1
        measured_notional += notional
        shortfall_total += shortfall * notional
        measured_fee_total += fee

    return {
        "order_samples": len(orders),
        "filled_order_samples": filled_orders,
        "partial_fill_samples": partial_orders,
        "unfilled_order_samples": unfilled_orders,
        "measured_fill_samples": measurable,
        "reference_price_coverage_percent": measurable / filled_orders * 100 if filled_orders else None,
        "average_fill_rate": filled_total / requested if requested else None,
        "average_implementation_shortfall_bps": shortfall_total / measured_notional if measured_notional else None,
        "average_fee_bps": fee_total / fee_notional * 10_000 if fee_notional else None,
        "average_all_in_cost_bps": (shortfall_total + measured_fee_total * 10_000) / measured_notional if measured_notional else None,
    }


def summarize_decision_quality(records):
    history_truncated = len(records) > 50_000
    records = records[-50_000:]
    decisions = {record.get("decision_id") for record in records if record.get("stage") == "decision" and record.get("decision_id")}
    trades = {}
    for record in records:
        decision_id = record.get("decision_id")
        if not decision_id:
            continue
        trade = trades.setdefault(decision_id, {"entry_quantity": 0.0, "entry_price": None, "exit_quantity": 0.0, "realized_pnl": 0.0})
        stage = str(record.get("stage", ""))
        if stage == "entry":
            trade.update({
                "entry_quantity": max(0.0, record.get("filled_quantity") or 0.0),
                "entry_price": record.get("average_price"),
                "exploratory": record.get("exploratory"),
                "direction": record.get("direction"),
                "strength": record.get("strength"),
                "market_context": record.get("market_context"),
                "entry_timestamp": record.get("timestamp"),
                "signal_components": record.get("signal_components") or {},
            })
        elif stage.startswith("exit"):
            trade["exit_quantity"] += max(0.0, record.get("exit_filled_quantity") or 0.0)
            trade["realized_pnl"] += record.get("realized_pnl") or 0.0

    summary = {"decision_samples": len(decisions), "filled_entry_samples": 0, "unresolved_trade_samples": 0, "unpriced_closed_trade_samples": 0, "completed_trade_samples": 0, "history_truncated": history_truncated, "cohorts": []}
    groups = {}
    for trade in trades.values():
        quantity = trade["entry_quantity"]
        if quantity <= 0:
            continue
        summary["filled_entry_samples"] += 1
        if trade["exit_quantity"] + 1e-12 < quantity:
            summary["unresolved_trade_samples"] += 1
            continue
        price = trade.get("entry_price")
        if not price or price <= 0:
            summary["unpriced_closed_trade_samples"] += 1
            continue
        notional = quantity * price
        if notional <= 0:
            continue
        summary["completed_trade_samples"] += 1
        net_return = trade["realized_pnl"] / notional * 100
        strength = trade.get("strength") or 0.0
        strength_band = "<0.50" if strength < 0.50 else "0.50–0.59" if strength < 0.60 else "0.60–0.69" if strength < 0.70 else "0.70+"
        components = trade.get("signal_components") or {}

        def component_band(key, ranges, labels):
            value = components.get(key)
            if value is None:
                return "unavailable"
            for boundary, label in zip(ranges, labels):
                if value < boundary:
                    return label
            return labels[-1]

        context = trade.get("market_context") or ""
        market_regime = context.split(":", 1)[1] if ":" in context else "unavailable"
        timestamp = trade.get("entry_timestamp")
        if timestamp is None:
            entry_time_utc = "unavailable"
        else:
            hour = int(timestamp // 3_600_000) % 24
            start = (hour // 6) * 6
            entry_time_utc = f"{start:02}:00–{start + 5:02}:59 UTC"

        dimensions = [
            ("direction", trade.get("direction") or "unknown"),
            ("selection", "exploratory" if trade.get("exploratory") is True else "strict" if trade.get("exploratory") is False else "unknown"),
            ("market_context", trade.get("market_context") or "unknown"),
            ("market_regime", market_regime),
            ("entry_time_utc", entry_time_utc),
            ("strength", strength_band),
            ("10s_momentum", component_band("momentum_10s_percent", [-0.12, -0.04, 0.04, 0.12], ["<-0.12%", "-0.12 to -0.04%", "-0.04 to 0.04%", "0.04 to 0.12%", "≥0.12%"])),
            ("5m_trend", component_band("trend_5m_volatility_units", [-1.5, -0.5, 0.5, 1.5], ["<-1.5σ", "-1.5 to -0.5σ", "-0.5 to 0.5σ", "0.5 to 1.5σ", "≥1.5σ"])),
            ("1m_volatility", component_band("volatility_1m_percent", [0.10, 0.30, 1.0], ["<0.10%", "0.10 to 0.30%", "0.30 to 1.00%", "≥1.00%"])),
            ("VWAP_mispricing", component_band("mispricing_vs_vwap_bps", [-8, -3, 3, 8], ["<-8 bps", "-8 to -3 bps", "-3 to 3 bps", "3 to 8 bps", "≥8 bps"])),
            ("pressure_without_move", component_band("pressure_without_price_move", [-0.75, 0, 0.75], ["<-0.75", "-0.75 to 0", "0 to 0.75", "≥0.75"])),
            ("aggressor_cost", component_band("aggressor_cost_bps", [0, 2, 5], ["<0 bps", "0 to 2 bps", "2 to 5 bps", "≥5 bps"])),
        ]
        for dimension, value in dimensions:
            group = groups.setdefault((dimension, value), {"samples": 0, "wins": 0, "return_total": 0.0, "pnl_total": 0.0})
            group["samples"] += 1
            group["wins"] += int(net_return > 0)
            group["return_total"] += net_return
            group["pnl_total"] += trade["realized_pnl"]
    summary["cohorts"] = [
        {"dimension": dimension, "value": value, "completed_trade_samples": group["samples"],
         "average_net_return_percent": group["return_total"] / group["samples"],
         "win_rate": group["wins"] / group["samples"], "total_net_pnl": group["pnl_total"],
         "preliminary": group["samples"] < 10}
        for (dimension, value), group in sorted(groups.items())
    ]
    return summary


def summarize_database(path):
    if not path.exists():
        return {"error": "database file not found"}

    uri = path.as_uri() + "?mode=ro"
    with sqlite3.connect(uri, uri=True, timeout=10) as db:
        tables = {row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        result = {
            "database_bytes": path.stat().st_size,
            "wal_bytes": Path(str(path) + "-wal").stat().st_size if Path(str(path) + "-wal").exists() else 0,
            "tables": sorted(tables),
        }
        if "signal_records" not in tables:
            return result

        result["page_count"] = db.execute("PRAGMA page_count").fetchone()[0]
        result["page_size"] = db.execute("PRAGMA page_size").fetchone()[0]
        result["freelist_pages"] = db.execute("PRAGMA freelist_count").fetchone()[0]
        total = db.execute(
            "SELECT COUNT(*), COALESCE(SUM(length(payload)), 0), MIN(timestamp), MAX(timestamp) FROM signal_records"
        ).fetchone()
        result["records"] = {
            "count": total[0],
            "compressed_payload_bytes": total[1],
            "first_timestamp_utc": timestamp_text(total[2]) if total[2] is not None else None,
            "last_timestamp_utc": timestamp_text(total[3]) if total[3] is not None else None,
        }
        result["by_day_and_type"] = [
            {
                "day_utc": day,
                "record_type": kind_name(kind),
                "count": count,
                "compressed_payload_bytes": payload_bytes,
                "first_timestamp_utc": timestamp_text(first),
                "last_timestamp_utc": timestamp_text(last),
            }
            for day, kind, count, payload_bytes, first, last in db.execute(
                "SELECT date(timestamp / 1000, 'unixepoch'), kind, COUNT(*), "
                "COALESCE(SUM(length(payload)), 0), MIN(timestamp), MAX(timestamp) "
                "FROM signal_records GROUP BY 1, 2 ORDER BY 1, 2"
            )
        ]
        if "signal_daily_summary" in tables:
            summary_columns = {row[1] for row in db.execute("PRAGMA table_info(signal_daily_summary)")}
            archive_bytes_sql = "COALESCE(length(training_payload), 0)" if "training_payload" in summary_columns else "0"
            result["daily_summary_rows"] = [
                {
                    "day_utc": datetime.fromtimestamp(day_ms / 1000, timezone.utc).date().isoformat(),
                    "record_type": kind_name(kind),
                    "count": count,
                    "compressed_payload_bytes": payload_bytes,
                    "first_timestamp_utc": timestamp_text(first),
                    "last_timestamp_utc": timestamp_text(last),
                    "training_archive_bytes": archive_bytes,
                }
                for day_ms, kind, count, payload_bytes, first, last, archive_bytes in db.execute(
                    "SELECT day_start_ms, kind, record_count, compressed_payload_bytes, "
                    f"first_timestamp, last_timestamp, {archive_bytes_sql} "
                    "FROM signal_daily_summary "
                    "ORDER BY day_start_ms, kind"
                )
            ]
        if "signal_daily_summary_runs" in tables:
            result["summarized_days"] = [
                datetime.fromtimestamp(day_ms / 1000, timezone.utc).date().isoformat()
                for (day_ms,) in db.execute("SELECT day_start_ms FROM signal_daily_summary_runs ORDER BY day_start_ms")
            ]
        if "auto_trader_audit" in tables:
            all_audit_records = [
                json.loads(gzip.decompress(payload))
                for (payload,) in db.execute("SELECT payload FROM auto_trader_audit ORDER BY timestamp")
            ]
            stage_rows = db.execute(
                "SELECT stage, COUNT(*), MIN(timestamp), MAX(timestamp) FROM auto_trader_audit GROUP BY stage ORDER BY stage"
            ).fetchall()
            stage_counts = {}
            for stage, count, first, last in stage_rows:
                stage_name = "exit" if stage.startswith("exit") else stage
                entry = stage_counts.setdefault(stage_name, {"count": 0, "first": first, "last": last})
                entry["count"] += count
                entry["first"] = min(entry["first"], first)
                entry["last"] = max(entry["last"], last)
            result["auto_trader_audit"] = {
                "count": len(all_audit_records),
                "compressed_payload_bytes": db.execute("SELECT COALESCE(SUM(length(payload)), 0) FROM auto_trader_audit").fetchone()[0],
                "by_stage": [
                    {"stage": stage, "count": value["count"], "first_timestamp_utc": timestamp_text(value["first"]), "last_timestamp_utc": timestamp_text(value["last"])}
                    for stage, value in sorted(stage_counts.items())
                ],
                "execution_quality": summarize_execution_quality(all_audit_records),
                "decision_quality": summarize_decision_quality(all_audit_records),
                "latest": all_audit_records[-10:],
            }
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", action="append", help="SQLite database to inspect; may be repeated")
    args = parser.parse_args()
    databases = discover_databases(args.db)
    print(json.dumps({str(path): summarize_database(path) for path in databases}, indent=2))


if __name__ == "__main__":
    main()
