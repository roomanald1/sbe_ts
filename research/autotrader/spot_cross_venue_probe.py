#!/usr/bin/env python3
"""Read-only $1k depth walk comparing public Binance and OKX spot books."""
import datetime
import json
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "spot_cross_venue_snapshot_2026-10-28.json"
SYMBOLS = ("BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT", "ADAUSDT", "DOGEUSDT")
NOTIONAL = 1000.0
USER_AGENT = "read-only-strategy-research/2026-10-28"


def get(url):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.load(response)


def walk(levels, quote_notional):
    left = quote_notional
    quantity = value = 0.0
    for row in levels:
        price, size = float(row[0]), float(row[1])
        take = min(left, size * price)
        quantity += take / price
        value += take
        left -= take
        if left <= 1e-6:
            return {"vwap": value / quantity, "quantity": quantity, "complete": True}
    return {"vwap": value / quantity if quantity else None, "quantity": quantity, "complete": False}


def main():
    result = {
        "sampled_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "notional_usdt_per_leg": NOTIONAL,
        "fee_scenarios_bps_per_side": [10, 20],
        "venues": ["Binance Spot", "OKX Spot"],
        "markets": {},
    }
    for symbol in SYMBOLS:
        pair = symbol[:-4] + "-USDT"
        try:
            binance = get(f"https://api.binance.com/api/v3/depth?symbol={symbol}&limit=100")
            okx_response = get(f"https://www.okx.com/api/v5/market/books?instId={pair}&sz=100")
            okx = okx_response["data"][0]
            directions = {}
            for name, buy_levels, sell_levels in (
                ("buy_binance_sell_okx", binance["asks"], okx["bids"]),
                ("buy_okx_sell_binance", okx["asks"], binance["bids"]),
            ):
                buy = walk(buy_levels, NOTIONAL)
                sell = walk(sell_levels, NOTIONAL)
                gross_bps = (sell["vwap"] / buy["vwap"] - 1) * 10000 if buy["complete"] and sell["complete"] else None
                directions[name] = {
                    "buy_vwap": buy["vwap"], "sell_vwap": sell["vwap"],
                    "gross_spread_bps": gross_bps,
                    "net_after_10bp_each_venue_bps": gross_bps - 20 if gross_bps is not None else None,
                    "net_after_20bp_each_venue_bps": gross_bps - 40 if gross_bps is not None else None,
                    "both_books_complete": buy["complete"] and sell["complete"],
                }
            result["markets"][symbol] = {
                "binance_last_update_id": binance.get("lastUpdateId"),
                "okx_timestamp_ms": okx.get("ts"),
                "directions": directions,
            }
        except Exception as error:
            result["markets"][symbol] = {"error": str(error)}
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    for symbol, item in result["markets"].items():
        if "directions" in item:
            best = max((x["gross_spread_bps"] for x in item["directions"].values() if x["gross_spread_bps"] is not None), default=None)
            print(symbol, "max_gross_bps", round(best, 3) if best is not None else None)


if __name__ == "__main__":
    main()
