#!/usr/bin/env python3
"""Monthly long-only risk-adjusted momentum rotation on seven liquid spot pairs."""
import json,math,statistics
from datetime import datetime,timezone
from pathlib import Path
ROOT=Path(__file__).resolve().parent;DATA=ROOT/'data/binance_spot_1d_api_2019_2026';SYMS=('BTCUSDT','ETHUSDT','BNBUSDT','XRPUSDT','SOLUSDT','ADAUSDT','DOGEUSDT');OUT=ROOT/'risk_adjusted_momentum_results.json'
def load():
 d={}
 for s in SYMS:
  raw=json.load(open(DATA/f'{s}-1d.json'));d[s]={datetime.fromtimestamp(r[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'o':float(r[1]),'c':float(r[4])} for r in raw}
 days=sorted(set.intersection(*(set(v) for v in d.values())));return days,d
def stats(rs):
 e=p=1.;dd=0.
 for r in rs:e*=1+r;p=max(p,e);dd=min(dd,e/p-1)
 m=statistics.mean(rs) if rs else 0;s=statistics.stdev(rs) if len(rs)>1 else 0
 return {'return_percent':100*(e-1),'max_drawdown_percent':100*dd,'annualized_sharpe':m/s*math.sqrt(365) if s else 0,'days':len(rs)}
def run(days,d,adj,cost,target_vol=None):
 w={s:0. for s in SYMS};rows=[]
 for i in range(202,len(days)-1):
  day,prev=days[i],days[i-1];turn=0.
  if day[8:10]=='01':
   btc_sma=sum(d['BTCUSDT'][days[k]]['c'] for k in range(i-200,i))/200
   btc_on=d['BTCUSDT'][prev]['c']>btc_sma
   rank=[]
   for s in SYMS:
    close=d[s][prev]['c'];mom=close/d[s][days[i-90]]['c']-1
    vol=statistics.stdev([d[s][days[k]]['c']/d[s][days[k-1]]['c']-1 for k in range(i-60,i)])
    score=mom/vol if vol else -1
    if btc_on and mom>0 and close>sum(d[s][days[k]]['c'] for k in range(i-100,i))/100:
     rank.append((score if adj else mom,s,vol))
   target={s:0. for s in SYMS}
   if rank:
    _,chosen,vol=max(rank)
    target[chosen]=min(1.0,target_vol/(vol*math.sqrt(365))) if target_vol is not None and vol>0 else 1.0
   turn=sum(abs(target[s]-w[s]) for s in SYMS);w=target
  gross=sum(w[s]*(d[s][days[i+1]]['o']/d[s][day]['o']-1) for s in SYMS)
  rows.append((day,gross-cost*turn))
 return rows
def main():
 days,d=load();ranges={'2021-2022':lambda x:'2021-01-01'<=x<'2023-01-01','2023-2024':lambda x:'2023-01-01'<=x<'2025-01-01','2025':lambda x:'2025-01-01'<=x<'2026-01-01','2026':lambda x:x>='2026-01-01'};out={'signal':'At monthly rebalance, restrict candidates to positive 90-day return, price above own SMA100, BTC above SMA200. Select max 90-day return divided by prior 60-day daily realized volatility (risk-adjusted momentum). One asset at a time, cash if no candidate. Signal at prior close; next-open rebalance.','benchmark':'same gates and rebalance, ranked by raw 90-day return','cost':'15 bps per side on absolute weight turnover; cash return zero','universe':SYMS,'data_range':[days[0],days[-1]],'usable_signal_range':[days[202],days[-2]],'periods':{}}
 for bp in (10,15,25):
  for adj,target_vol,label in ((False,None,'raw'),(True,None,'riskadj'),(True,.40,'riskadj_40vol')):
   rows=run(days,d,adj,bp/10000,target_vol);out['periods'][f'{label}_{bp}bp']={n:stats([r for day,r in rows if pred(day)]) for n,pred in ranges.items()}
 OUT.write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(out['periods'],indent=2))
if __name__=='__main__':main()
