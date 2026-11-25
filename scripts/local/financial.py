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
    if snapshot.get("independent_execution_ledger_reconciled") is not True:
        blockers.append("No independent execution ledger reconciliation")
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
