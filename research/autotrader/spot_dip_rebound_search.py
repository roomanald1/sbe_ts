#!/usr/bin/env python3
"""Test daily long-only dip-buy/exit rules with causal next-open execution."""
import json
from datetime import datetime,timezone
from pathlib import Path
ROOT=Path(__file__).resolve().parent; DATA=ROOT/'data/binance_spot_1d_api_2019_2026'
SYMBOLS=['BTCUSDT','ETHUSDT','BNBUSDT','SOLUSDT']; COST=.0015

def stats(a):
 if not a:return {'return_pct':0,'sharpe':0,'max_dd_pct':0,'trades':0}
 w=peak=1.;dd=0.;rr=[]
 for r in a:w*=1+r;peak=max(w,peak);dd=min(dd,w/peak-1);rr.append(r)
 avg=sum(rr)/len(rr);var=sum((x-avg)**2 for x in rr)/max(1,len(rr)-1)
 return {'return_pct':100*(w-1),'sharpe':avg/(var**.5)*365**.5 if var else 0,'max_dd_pct':100*dd}

def evaluate(dates,prices,entry_bps,target_bps,max_days):
 exposure=0;entry_open=0.;entry_i=0;turns=0;daily=[]
 for i in range(2,len(dates)-1):
  d=dates[i]
  # PnL from the exposure established at the previous open.
  net=exposure*(prices[dates[i]]['o']/prices[dates[i-1]]['o']-1)
  prev=dates[i-1]
  if exposure and (prices[prev]['c']/entry_open-1>=target_bps/10000 or i-entry_i>=max_days):
   exposure=0;turns+=1;net-=COST
  elif not exposure and prices[prev]['c']/prices[dates[i-2]]['c']-1<=-entry_bps/10000:
   exposure=1;entry_open=prices[d]['o'];entry_i=i;turns+=1;net-=COST
  daily.append((d,net))
 splits={k:[] for k in ['train_2020_2022','validation_2023_2024','selection_2025','holdout_2026']}
 for d,r in daily:
  key='train_2020_2022' if d<'2023-01-01' else 'validation_2023_2024' if d<'2025-01-01' else 'selection_2025' if d<'2026-01-01' else 'holdout_2026'
  splits[key].append(r)
 return {k:stats(v) for k,v in splits.items()},turns

def main():
 data={}
 for s in SYMBOLS:
  data[s]={datetime.fromtimestamp(x[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'o':float(x[1]),'c':float(x[4])} for x in json.loads((DATA/f'{s}-1d.json').read_text())}
 dates=sorted(set.intersection(*(set(data[s]) for s in SYMBOLS)));results=[]
 for s in SYMBOLS:
  for entry in [100,200,300,500]:
   for target in [100,200,300]:
    for hold in [3,7,14]:
     sp,turns=evaluate(dates,data[s],entry,target,hold)
     results.append({'strategy':f'{s}_dip{entry}_tp{target}_time{hold}','entry_dip_bps':entry,'take_profit_bps':target,'max_hold_days':hold,'cost_bps_per_side':15,'splits':sp,'turnovers':turns})
 results.sort(key=lambda x:x['splits']['validation_2023_2024']['return_pct'],reverse=True)
 path=ROOT/'spot_dip_rebound_results.json';path.write_text(json.dumps({'universe':SYMBOLS,'timing':'close-based trigger, next-open execution; open-to-open returns','cost':'15 bps each entry and exit','selection':'rank by validation 2023-24; show 2025 and 2026 separately','results':results},indent=2))
 for r in results[:15]:
  s=r['splits'];print(r['strategy'],*[round(s[k]['return_pct'],1) for k in ['train_2020_2022','validation_2023_2024','selection_2025','holdout_2026']],round(s['holdout_2026']['max_dd_pct'],1))
 print('saved',path)
if __name__=='__main__':main()
