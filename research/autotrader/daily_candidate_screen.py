#!/usr/bin/env python3
"""Reproducible daily screens for the 3-day breakout and diversified trend candidates.

Downloads Binance Spot daily candles on first run and keeps a local cache. Signals
use the prior UTC close; target weights are applied at the next UTC open. Returns
are marked open-to-next-open and charge a fee on absolute portfolio turnover.
This is a research screen, not an execution simulator.
"""
import json
import math
import statistics
import time
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DATA = ROOT / "data" / "binance_spot_candidates_16_daily"
OUT = ROOT / "daily_candidate_screen_results.json"
SYMBOLS = (
    "BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "ADAUSDT", "SOLUSDT",
    "DOGEUSDT", "LTCUSDT", "TRXUSDT", "LINKUSDT", "BCHUSDT", "XLMUSDT",
    "ETCUSDT", "FILUSDT", "AVAXUSDT", "DOTUSDT",
)
START_MS = int(datetime(2017, 1, 1, tzinfo=timezone.utc).timestamp() * 1000)
END_MS = int(datetime(2026, 10, 30, tzinfo=timezone.utc).timestamp() * 1000)


def download(symbol):
    path = DATA / f"{symbol}-1d.json"
    if path.exists():
        return json.loads(path.read_text())
    DATA.mkdir(parents=True, exist_ok=True)
    rows, start = [], START_MS
    while start < END_MS:
        query = urllib.parse.urlencode({"symbol": symbol, "interval": "1d", "startTime": start,
                                        "endTime": END_MS, "limit": 1000})
        req = urllib.request.Request("https://api.binance.com/api/v3/klines?" + query,
                                     headers={"User-Agent": "daily-candidate-research/1.0"})
        with urllib.request.urlopen(req, timeout=30) as response:
            page = json.loads(response.read())
        if isinstance(page, dict):
            raise RuntimeError(f"Binance API error for {symbol}: {page}")
        if not page:
            break
        rows.extend(page)
        start = int(page[-1][0]) + 86_400_000
        time.sleep(0.08)
    if not rows:
        raise RuntimeError(f"No Binance candles returned for {symbol}")
    path.write_text(json.dumps(rows, separators=(",", ":")) + "\n")
    return rows


def load():
    bars = {}
    for symbol in SYMBOLS:
        raw = download(symbol)
        bars[symbol] = {
            datetime.fromtimestamp(row[0] / 1000, timezone.utc).strftime("%Y-%m-%d"):
            {"o": float(row[1]), "h": float(row[2]), "c": float(row[4])}
            for row in raw if row[0] < END_MS
        }
    days = sorted(set.union(*(set(v) for v in bars.values())))
    return days, bars


def sma(values):
    return sum(values) / len(values)


def weights_for(day_index, days, data, policy, held, ages):
    day, prior = days[day_index], days[day_index - 1]
    btc_days = [d for d in days[:day_index] if d in data["BTCUSDT"]]
    btc_close = data["BTCUSDT"].get(prior, {}).get("c")
    btc_hist = [data["BTCUSDT"][d]["c"] for d in btc_days[-200:]]
    if len(btc_hist) < 200 or btc_close is None:
        return {s: 0.0 for s in SYMBOLS}
    btc_gate = btc_close > sma(btc_hist)
    target = {s: 0.0 for s in SYMBOLS}

    if policy in ("baseline", "A", "A2"):
        active = []
        for symbol in SYMBOLS:
            if prior not in data[symbol]:
                held[symbol] = False
                ages[symbol] = 0
                continue
            history = [d for d in days[:day_index] if d in data[symbol]]
            if len(history) < 22:
                continue
            close = data[symbol][prior]["c"]
            prior_highs = [data[symbol][d]["h"] for d in history[-21:-1]]
            signal = btc_gate and close > max(prior_highs)
            if policy == "A2" and held[symbol] and ages[symbol] < 3:
                signal = True
            if signal:
                active.append(symbol)
            held[symbol] = signal
        for symbol in active:
            cap = 1.0 / 7 if policy == "baseline" else 0.25
            target[symbol] = min(cap, 1.0 / len(active))
            ages[symbol] = ages[symbol] + 1 if target[symbol] else 0
        for symbol in SYMBOLS:
            if symbol not in active:
                ages[symbol] = 0
        return target

    # B: cross-asset positive trend score from 10/20/40/80/160 sessions;
    # inverse 30-day realized volatility; BTC SMA200 regime gate; 25% name cap.
    if not btc_gate:
        return target
    raw = {}
    for symbol in SYMBOLS:
        history = [d for d in days[:day_index] if d in data[symbol]]
        if len(history) < 161:
            continue
        closes = [data[symbol][d]["c"] for d in history]
        score = statistics.mean(closes[-1] / closes[-1 - n] - 1 for n in (10, 20, 40, 80, 160))
        daily_returns = [closes[i] / closes[i - 1] - 1 for i in range(len(closes) - 30, len(closes))]
        vol = statistics.stdev(daily_returns) if len(daily_returns) > 1 else 0
        if score > 0 and vol > 0:
            raw[symbol] = score / vol
    # Capped proportional allocation, iterated until all positive scores are used.
    remaining = 1.0
    pool = dict(raw)
    while pool and remaining > 1e-12:
        total = sum(pool.values())
        capped = [s for s, value in pool.items() if value / total * remaining >= 0.25]
        if not capped:
            for symbol, value in pool.items():
                target[symbol] = value / total * remaining
            break
        for symbol in capped:
            target[symbol] = 0.25
            remaining -= 0.25
            pool.pop(symbol)
        if remaining <= 0:
            break
    return target


def replay(days, data, policy, side_cost):
    held, ages = {s: False for s in SYMBOLS}, {s: 0 for s in SYMBOLS}
    weights = {s: 0.0 for s in SYMBOLS}
    rows = []
    for i, day in enumerate(days):
        if i < 201 or i + 1 == len(days) or day not in data["BTCUSDT"] or days[i + 1] not in data["BTCUSDT"]:
            continue
        target = weights_for(i, days, data, policy, held, ages)
        turnover = sum(abs(target[s] - weights[s]) for s in SYMBOLS)
        gross = 0.0
        for symbol in SYMBOLS:
            if day in data[symbol] and days[i + 1] in data[symbol]:
                gross += target[symbol] * (data[symbol][days[i + 1]]["o"] / data[symbol][day]["o"] - 1)
        rows.append({"date": day, "return": gross - side_cost * turnover,
                     "turnover": turnover, "exposure": sum(target.values())})
        weights = target
    return rows


def stats(rows):
    wealth = peak = 1.0
    drawdown = 0.0
    values = [r["return"] for r in rows]
    for value in values:
        wealth *= 1 + value
        peak = max(peak, wealth)
        drawdown = min(drawdown, wealth / peak - 1)
    avg = statistics.mean(values) if values else 0
    vol = statistics.stdev(values) if len(values) > 1 else 0
    return {"return_percent": 100 * (wealth - 1), "max_drawdown_percent": 100 * drawdown,
            "annualized_daily_sharpe": avg / vol * math.sqrt(365) if vol else 0,
            "mean_exposure_percent": 100 * statistics.mean(r["exposure"] for r in rows) if rows else 0,
            "turnover": sum(r["turnover"] for r in rows), "days": len(rows)}


def main():
    days, data = load()
    periods = {
        "2018-2019": lambda d: "2018-01-01" <= d < "2020-01-01",
        "2020-2022": lambda d: "2020-01-01" <= d < "2023-01-01",
        "2023-2024": lambda d: "2023-01-01" <= d < "2025-01-01",
        "2025-2026-10-29": lambda d: "2025-01-01" <= d <= "2026-10-29",
    }
    result = {
        "source": "Binance Spot public REST daily klines, cached locally on first run",
        "symbols": list(SYMBOLS), "data_range": [days[0], days[-1]],
        "timing": "Signals use prior UTC close, trades at current UTC open, returns current open to next open",
        "cost": "15 or 30 bps each side on absolute portfolio weight turnover; cash return zero",
        "baseline": "20-day high breakout, BTC SMA200 gate, one seventh max weight per coin, no minimum hold",
        "A": "20-day high breakout, BTC SMA200 gate, 25% max weight per coin, no minimum hold",
        "A2": "20-day high breakout, BTC SMA200 gate, min 3 open-to-open holding sessions, 25% max weight per coin, fully invested positions capped at 100% total",
        "B": "BTC SMA200 gate; positive average 10/20/40/80/160-day returns; weights proportional to score divided by 30-day realized volatility; 25% per-coin cap",
        "periods": {},
    }
    for cost_bp in (15, 30):
        for policy in ("baseline", "A", "A2", "B"):
            rows = replay(days, data, policy, cost_bp / 10000)
            result["periods"].setdefault(policy, {})[str(cost_bp)] = {}
            for name, predicate in periods.items():
                result["periods"][policy][str(cost_bp)][name] = stats([r for r in rows if predicate(r["date"])])
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["periods"], indent=2))


if __name__ == "__main__":
    main()
