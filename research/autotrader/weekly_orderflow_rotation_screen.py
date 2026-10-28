#!/usr/bin/env python3
"""Weekly cross-sectional long-only rotation by lagged Binance taker-buy flow."""
import json, math, statistics
from datetime import datetime,timezone
from pathlib import Path
ROOT=Path(__file__).resolve().parent
DATA=ROOT/'data/binance_spot_1d_api_2019_2026'
SYMBOLS=('BTCUSDT','ETHUSDT','BNBUSDT','XRPUSDT','SOLUSDT','ADAUSDT','DOGEUSDT')
OUT=ROOT/'weekly_orderflow_rotation_results.json'

def load():
 d={}
 for s in SYMBOLS:
  raw=json.loads((DATA/f'{s}-1d.json').read_text());d[s]={datetime.fromtimestamp(r[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'open':float(r[1]),'close':float(r[4]),'quote':float(r[7]),'buy_quote':float(r[10])} for r in raw}
 dates=sorted(set.intersection(*(set(x) for x in d.values())))
 return dates,d

def stats(rs):
 eq=peak=1.;dd=0.
 for r in rs:eq*=1+r;peak=max(peak,eq);dd=min(dd,eq/peak-1)
 mu=statistics.mean(rs) if rs else 0;sd=statistics.stdev(rs) if len(rs)>1 else 0
 return {'days':len(rs),'return_percent':100*(eq-1),'max_drawdown_percent':100*dd,'annualized_sharpe':mu/sd*math.sqrt(365) if sd else 0}
def backtest(dates,data,cost,signal):
 weights={s:0. for s in SYMBOLS};rows=[]
 for i in range(8,len(dates)-1):
  day,prior=dates[i],dates[i-1];turnover=0
  if datetime.strptime(day,'%Y-%m-%d').weekday()==0:
   rank=[]
   for s in SYMBOLS:
    if signal in ('flow','flow_reversal'):
     q=sum(data[s][dates[k]]['quote'] for k in range(i-7,i));b=sum(data[s][dates[k]]['buy_quote'] for k in range(i-7,i))
     score=(2*b-q)/q if q else -1
    else:
     score=data[s][prior]['close']/data[s][dates[i-8]]['close']-1
    rank.append((score,s))
   selected={s:0. for s in SYMBOLS}
   top=(sorted(rank)[:2] if signal=='flow_reversal' else sorted(rank,reverse=True)[:2])
   if (signal=='flow_reversal' and top[-1][0]<0) or (signal!='flow_reversal' and (signal!='flow' or top[0][0]>0)):
    for _,s in top:selected[s]=.5
   turnover=sum(abs(selected[s]-weights[s]) for s in SYMBOLS);weights=selected
  gross=sum(weights[s]*(data[s][dates[i+1]]['open']/data[s][day]['open']-1) for s in SYMBOLS)
  rows.append((day,gross-cost*turnover))
 return rows

def main():
 dates,data=load();result={'universe':SYMBOLS,'signal':'Each Monday rank prior 7 complete days by aggregate Binance spot taker-buy imbalance (2*taker-buy quote volume minus total quote volume, divided by total quote volume); equally weight top two, cash if best flow score is not positive. Signal known at prior close, execute at next open.','benchmark':'same weekly rule ranking by prior 7-day return instead of flow','cost':'15 bps per side per dollar absolute portfolio-weight change; zero cash return','source':'Binance spot daily klines: quote volume and taker-buy quote volume fields; no external order-flow data.','periods':{},'limits':'Fixed seven-coin universe (not point-in-time); no delisted coins; screening only, not untouched holdout.'}
 spans={'2020-2022':lambda d:'2020-01-01'<=d<'2023-01-01','2023-2024':lambda d:'2023-01-01'<=d<'2025-01-01','2025':lambda d:'2025-01-01'<=d<'2026-01-01','2026':lambda d:d>='2026-01-01'}
 for strat in ('flow','flow_reversal','momentum'):
  for costbp in (10,15,25):
   rows=backtest(dates,data,costbp/10000,strat);result['periods'][f'{strat}_{costbp}bp']={name:stats([r for d,r in rows if pred(d)]) for name,pred in spans.items()}
 result['periods']['equal_weight_buyhold']={}
 bh=[]
 for i in range(1,len(dates)-1):bh.append((dates[i],sum(data[s][dates[i+1]]['open']/data[s][dates[i]]['open']-1 for s in SYMBOLS)/len(SYMBOLS)))
 result['periods']['equal_weight_buyhold']={name:stats([r for d,r in bh if pred(d)]) for name,pred in spans.items()}
 OUT.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result['periods'],indent=2))
if __name__=='__main__':main()
