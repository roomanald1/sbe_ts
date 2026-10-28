#!/usr/bin/env python3
"""Stress the risk-adjusted rotation with AutoTrader-style protective brackets."""
import json,math,statistics
from datetime import datetime,timezone
from pathlib import Path
ROOT=Path(__file__).resolve().parent;DATA=ROOT/'data/binance_spot_1d_api_2019_2026';SYMS=('BTCUSDT','ETHUSDT','BNBUSDT','XRPUSDT','SOLUSDT','ADAUSDT','DOGEUSDT');OUT=ROOT/'risk_adjusted_momentum_bracket_results.json'
def load():
 d={}
 for s in SYMS:
  raw=json.load(open(DATA/f'{s}-1d.json'));d[s]={datetime.fromtimestamp(r[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'o':float(r[1]),'h':float(r[2]),'l':float(r[3]),'c':float(r[4])} for r in raw}
 days=sorted(set.intersection(*(set(x) for x in d.values())));return days,d
def stats(rs):
 eq=peak=1.;dd=0.
 for r in rs:eq*=1+r;peak=max(peak,eq);dd=min(dd,eq/peak-1)
 mu=statistics.mean(rs) if rs else 0;sd=statistics.stdev(rs) if len(rs)>1 else 0
 return {'return_percent':100*(eq-1),'max_drawdown_percent':100*dd,'annualized_sharpe':mu/sd*math.sqrt(365) if sd else 0,'days':len(rs)}
def run(days,d,cost,stop=.005,target=.008):
 eq=1.;pos=None;rows=[];counts={'stops':0,'targets':0,'entries':0}
 for i in range(202,len(days)-1):
  day,prev,nxt=days[i],days[i-1],days[i+1];old_eq=eq
  if day[8:10]=='01':
   btc200=sum(d['BTCUSDT'][days[k]]['c'] for k in range(i-200,i))/200;rank=[]
   if d['BTCUSDT'][prev]['c']>btc200:
    for s in SYMS:
     c=d[s][prev]['c'];mom=c/d[s][days[i-90]]['c']-1
     sma100=sum(d[s][days[k]]['c'] for k in range(i-100,i))/100
     vol=statistics.stdev([d[s][days[k]]['c']/d[s][days[k-1]]['c']-1 for k in range(i-60,i)])
     if mom>0 and c>sma100:rank.append((mom/vol,s,vol))
   target_pos=None
   if rank:
    _,s,vol=max(rank);weight=min(1.,.4/(vol*math.sqrt(365)))
    target_pos={'symbol':s,'weight':weight,'entry':d[s][day]['o']}
   if pos is not None and target_pos is not None and pos['symbol']==target_pos['symbol']:
    delta=target_pos['weight']-pos['weight'];eq*=1-cost*abs(delta);pos['weight']=target_pos['weight']
   else:
    if pos is not None:eq*=1-cost*pos['weight'];pos=None
    if target_pos is not None:eq*=1-cost*target_pos['weight'];pos=target_pos;counts['entries']+=1
  if pos is not None:
   bar=d[pos['symbol']][day];entry=pos['entry'];sp=entry*(1-stop);tp=entry*(1+target)
   stop_hit=bar['l']<=sp;tp_hit=bar['h']>=tp
   if stop_hit or tp_hit:
    if stop_hit:
     px=min(bar['o'],sp);counts['stops']+=1
    else:
     px=max(bar['o'],tp);counts['targets']+=1
    eq*=1+pos['weight']*(px/bar['o']-1);eq*=1-cost*pos['weight'];pos=None
   else:
    eq*=1+pos['weight']*(d[pos['symbol']][nxt]['o']/bar['o']-1)
  rows.append((day,eq/old_eq-1))
 return rows,counts
def main():
 days,d=load();ranges={'2021-2022':lambda x:'2021-01-01'<=x<'2023-01-01','2023-2024':lambda x:'2023-01-01'<=x<'2025-01-01','2025':lambda x:'2025-01-01'<=x<'2026-01-01','2026':lambda x:x>='2026-01-01'};out={}
 for bp in (15,25):
  for stop,target,label in ((.005,.008,'default_bracket'),(.015,.03,'wide_bracket')):
   rows,ct=run(days,d,bp/10000,stop,target);out[f'{label}_{bp}bp']={'stop_percent':100*stop,'target_percent':100*target,'counts':ct,'splits':{n:stats([r for day,r in rows if pred(day)]) for n,pred in ranges.items()}}
 OUT.write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(out,indent=2))
if __name__=='__main__':main()
