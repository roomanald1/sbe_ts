#!/usr/bin/env python3
"""Predeclared BTC volatility-managed 60-day time-series momentum screen."""
import json, math, statistics
from pathlib import Path
from spot_trailing_breakout_screen import load
ROOT = Path(__file__).resolve().parent
OUT = ROOT / "vol_managed_momentum_results.json"

def summarize(values):
    wealth = peak = 1.0
    dd = 0.0
    for r in values:
        wealth *= 1 + r
        peak = max(peak, wealth)
        dd = min(dd, wealth / peak - 1)
    mean = statistics.mean(values) if values else 0
    vol = statistics.stdev(values) if len(values) > 1 else 0
    return {"return_percent": 100*(wealth-1), "max_drawdown_percent": 100*dd,
            "annualized_sharpe": mean/vol*math.sqrt(365) if vol else 0, "days":len(values)}

def run(dates, bars, cost):
    rows = {"unscaled":[], "vol_managed":[]}
    for i in range(202, len(dates)-1):
        prior, current, next_day = dates[i-1], dates[i], dates[i+1]
        mom = bars[prior]["close"] / bars[dates[i-61]]["close"] - 1
        signal = float(mom > 0)
        vol = statistics.stdev([bars[dates[k]]["close"]/bars[dates[k-1]]["close"]-1 for k in range(i-30,i)])*math.sqrt(365)
        weight = min(1.0, .50/vol) if vol else 1.0
        ret = bars[next_day]["open"]/bars[current]["open"]-1
        old_signal = float(bars[dates[i-2]]["close"]/bars[dates[i-62]]["close"]-1 > 0)
        oldvol = statistics.stdev([bars[dates[k]]["close"]/bars[dates[k-1]]["close"]-1 for k in range(i-31,i-1)])*math.sqrt(365)
        oldweight = min(1.0,.50/oldvol) if oldvol else 1.0
        for name, target, old in (("unscaled", signal, old_signal), ("vol_managed",signal*weight,old_signal*oldweight)):
            rows[name].append((1+target*ret)*(1-abs(target-old)*cost)-1)
    return rows

def main():
    dates, data = load()
    periods = {"train_2021_2022":lambda d:d<"2023-01-01", "validation_2023_2024":lambda d:"2023-01-01"<=d<"2025-01-01", "later_2025_2026":lambda d:d>="2025-01-01"}
    result={"source":"Binance BTCUSDT daily spot candles; prior-close signal, next-open execution", "rule":"long-only 60-day time-series momentum; risk-managed exposure targets 50% annualized volatility using the prior 30 daily close returns, capped at 100%; cash earns zero", "cost":"15/30/50 bps per dollar absolute exposure change; no borrow or leverage", "caveat":"research screen, not proof of future profits; 2025-26 was inspected in prior searches and is not a pristine holdout", "periods":{}}
    dates_used=dates[202:len(dates)-1]
    for bp in (15,30,50):
        rows=run(dates,data["BTCUSDT"],bp/10000)
        result["periods"][str(bp)]={name:{key:summarize([r for d,r in zip(dates_used,vals) if pred(d)]) for key,vals in rows.items()} for name,pred in periods.items()}
    OUT.write_text(json.dumps(result,indent=2)+"\n")
    print(json.dumps(result["periods"],indent=2))
if __name__=="__main__": main()
