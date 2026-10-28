#!/usr/bin/env python3
"""Test a fixed UTC high-return window from published crypto intraday evidence."""
import json, math, statistics
from datetime import datetime,timezone
from pathlib import Path
from session_reversal_multiyear_screen import CACHE
RANGES={'2019-2020':lambda y:2019<=y<=2020,'2021-2022':lambda y:2021<=y<=2022,'2023-2024':lambda y:2023<=y<=2024,'2025-2026':lambda y:y>=2025}

def stats(rs):
 eq=peak=1.;dd=0.
 for r in rs:eq*=1+r;peak=max(peak,eq);dd=min(dd,eq/peak-1)
 mu=statistics.mean(rs) if rs else 0;sd=statistics.stdev(rs) if len(rs)>1 else 0
 return {'days':len(rs),'return_percent':100*(eq-1),'max_drawdown_percent':100*dd,'annualized_sharpe':mu/sd*math.sqrt(365) if sd else 0}
def main():
 raw=json.loads(CACHE.read_text());out={}
 for bp in (10,15,20):
  out[str(bp)]={}
  for s in ('BTCUSDT','ETHUSDT'):
   h={int(r[0]):float(r[1]) for r in raw[s]}; tests={}
   for start,end in ((15,16),(14,16),(15,17)):
    byyear={}
    for t,p in h.items():
     dt=datetime.fromtimestamp(t/1000,timezone.utc)
     if dt.hour==start:
      x=h.get(t+(end-start)*3600000)
      if x:byyear.setdefault(dt.strftime('%Y-%m-%d'),[]).append((dt.year,x/p-1))
    dayreturns={d:(1+rr[0][1])*(1-bp/10000)**2-1 for d,rr in byyear.items()}
    tests[f'{start}-{end}UTC']={k:stats([v for d,v in dayreturns.items() if pred(datetime.strptime(d,'%Y-%m-%d').year)]) for k,pred in RANGES.items()}
   out[str(bp)][s]=tests
 print(json.dumps(out,indent=2));Path('research/autotrader/utc_window_results.json').write_text(json.dumps(out,indent=2)+'\n')
if __name__=='__main__':main()
