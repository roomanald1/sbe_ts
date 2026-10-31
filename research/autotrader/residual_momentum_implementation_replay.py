#!/usr/bin/env python3
"""Replay the implemented residual engine on retained daily OHLC (execution proxy).

No service is started. Quotes are modeled at the next open +/-5 bp, not historical
order-book observations. Costs: engine defaults, 15 bp/side and 10% short borrow APR.
"""
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
DATA = ROOT / "research/autotrader/data/binance_spot_candidates_16_daily"
SYMBOLS = ("BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT", "ADAUSDT", "DOGEUSDT", "LTCUSDT")
DAY = 86_400_000

def load():
    panel, hashes = {}, {}
    for symbol in SYMBOLS:
        path = DATA / f"{symbol}-1d.json"
        raw = path.read_bytes()
        hashes[path.name] = hashlib.sha256(raw).hexdigest()
        rows = json.loads(raw)
        panel[symbol] = {int(row[0]): row for row in rows}
    times = sorted(set.intersection(*(set(rows) for rows in panel.values())))
    return panel, times, hashes

def date(t):
    return datetime.fromtimestamp(t / 1000, timezone.utc).strftime("%Y-%m-%d")

def run_period(panel, times, start, end):
    selected = [(i, t) for i, t in enumerate(times) if i >= 45 and start <= date(t) <= end]
    if not selected:
        raise ValueError("No warm daily history in period")
    with tempfile.TemporaryDirectory(prefix="sbe-residual-replay-") as temporary:
        inputs = Path(temporary) / "observations.jsonl"
        output = Path(temporary) / "result.json"
        with inputs.open("w") as stream:
            for i, t in selected:
                assets = []
                for symbol in SYMBOLS:
                    rows = panel[symbol]
                    price = float(rows[t][1])
                    assets.append(dict(
                        symbol=symbol, short_available=True,
                        closes=[dict(timestamp_ms=prior + DAY - 1, close=float(rows[prior][4]))
                                for prior in times[i-45:i]],
                        quote=dict(bid=price * 0.9995, ask=price * 1.0005,
                                   bid_quantity=1e6, ask_quantity=1e6, timestamp_ms=t+1000),
                    ))
                observation = dict(id=f"ohlc-proxy:{t}", observed_at_ms=t+1000,
                    source="Historical Binance OHLC; +/-5bp modeled quotes and assumed depth",
                    strategy="residual-momentum", data=dict(assets=assets))
                stream.write(json.dumps(observation, separators=(",", ":")) + "\n")
        command = ["cargo", "run", "--quiet", "--manifest-path", "src/rust/auto_trader/Cargo.toml",
                   "--", "--replay-alternatives", str(inputs), str(output)]
        completed = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
        if completed.returncode:
            raise RuntimeError(completed.stderr + completed.stdout)
        sleeve = json.loads(output.read_text())["sleeves"]["residual-momentum"]
    first, last = selected[0][1], selected[-1][1]
    # Equal initial capital, 100% gross buy-and-hold: a higher-exposure comparator.
    buy_hold = sum(float(panel[s][last][1]) * 0.9995 /
                   (float(panel[s][first][1]) * 1.0005) * (1 - 0.0015) for s in SYMBOLS) / len(SYMBOLS)
    return dict(
        first_execution_date=date(first), last_mark_date=date(last),
        daily_observations=len(selected), initial_capital_usd=1000,
        ending_equity_usd=sleeve["equity_usd"],
        return_percent=(sleeve["equity_usd"]/1000-1)*100,
        realized_pnl_usd=sleeve["realized_pnl_usd"], fees_and_borrow_usd=sleeve["fees_usd"],
        maximum_drawdown_percent=sleeve["max_drawdown_percent"],
        open_positions_at_end=len(sleeve["positions"]),
        buy_hold_100_percent_gross_return_percent=(buy_hold-1)*100,
    )

def main():
    panel, times, hashes = load()
    results = {name: run_period(panel, times, lo, hi) for name, lo, hi in (
        ("2021-2022", "2021-01-01", "2022-12-31"),
        ("2023-2024", "2023-01-01", "2024-12-31"),
        ("2025-2026", "2025-01-01", date(times[-1])),
    )}
    document = dict(
        engine="Rust alternative_strategies/residual.rs via offline replay",
        rule="30-day lagged rolling OLS beta; 14-day residual formation; 7-day hold, top two/bottom two ranks",
        costs="15 bp per side, +/-5 bp modeled spread around next daily open, 10% annual short borrow",
        portfolio="1000 USD independent account; at most 100 USD per leg, two long/two short, weekly",
        limitations=[
            "Fixed present-day survivors; no delisted coins or point-in-time universe.",
            "Retrospective discovery screen; dates overlap earlier research and are not an untouched holdout.",
            "OHLC execution proxy with assumed depth, not executable historical quotes.",
            "No liquidation, funding, borrow recall, partial fill or intraday risk model.",
            "Final open positions marked, not liquidated; ending equity includes unrealized P&L.",
            "Benchmark is 100% gross; strategy initially uses about 40% gross. Risk budgets differ.",
        ], input_sha256=hashes, periods=results,
    )
    out = ROOT / "research/autotrader/residual_momentum_implementation_results.json"
    out.write_text(json.dumps(document, indent=2) + "\n")
    print(json.dumps(results, indent=2))

if __name__ == "__main__":
    main()
