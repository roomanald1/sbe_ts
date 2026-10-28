#!/usr/bin/env python3
"""Buy failed downside breakouts after a 20-day low reclaim; spot-only replay."""
import json,math,statistics
from datetime import datetime,timezone
from pathlib import Path
ROOT=Path(__file__).resolve().parent;DATA=ROOT/'data/binance_spot_1d_api_2019_2026'
SYMBOLS=('BTCUSDT','ETHUSDT','BNBUSDT','XRPUSDT','SOLUSDT','ADAUSDT','DOGEUSDT')
OUT=ROOT/'failed_breakdown_reversal_results.json';COST=.0015

def load():
 d={}
 for s in SYMBOLS:
  raw=json.loads((DATA/f'{s}-1d.json').read_text());d[s]={datetime.fromtimestamp(r[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'open':float(r[1]),'low':float(r[3]),'close':float(r[4])} for r in raw}
 days=sorted(set.intersection(*(set(x) for x in d.values())));return days,d
def met(rs):
 eq=peak=1.;dd=0.
 for r in rs:eq*=1+r;peak=max(peak,eq);dd=min(dd,eq/peak-1)
 m=statistics.mean(rs) if rs else 0;s=statistics.stdev(rs) if len(rs)>1 else 0
 return {'days':len(rs),'return_percent':100*(eq-1),'max_drawdown_percent':100*dd,'annualized_sharpe':m/s*math.sqrt(365) if s else 0}
def run(days,data,hold):
 sleeve={s:1. for s in SYMBOLS};pos={s:0 for s in SYMBOLS};rows=[]
 for i in range(22,len(days)-1):
  day,prev,nxt=days[i],days[i-1],days[i+1];daily=[]
  for s in SYMBOLS:
   if pos[s]==0:
    low=min(data[s][days[k]]['low'] for k in range(i-22,i-2))
    candle=data[s][prev]
    if candle['low']<low and candle['close']>low:
     sleeve[s]*=(1-COST);pos[s]=hold
   gross=data[s][nxt]['open']/data[s][day]['open']-1
   sleeve[s]*=1+gross
   pos[s]-=1 if pos[s]>0 else 0
   if pos[s]==0 and sleeve[s]>0:
    # Exit fee is debited at the next open for trades that expire here.
    # Since this loop's mark was next open, charge the exit cost now.
    # The subsequent flat return is zero.
    pass
   daily.append(sleeve[s])
  # Exit fee at next open for any sleeve whose final open-to-open interval just completed.
  for s in SYMBOLS:
   if pos[s]==0 and sleeve[s]>0:
    # Only charge when it was a held position at this interval; tracked separately below.
    pass
  rows.append((day,sum(daily)/len(SYMBOLS)))
 return rows

def run_correct(days,data,hold):
 sleeve={s:1. for s in SYMBOLS};remaining={s:0 for s in SYMBOLS};rows=[]
 for i in range(22,len(days)-1):
  day,prev,nxt=days[i],days[i-1],days[i+1]
  for s in SYMBOLS:
   active=remaining[s]>0
   if not active:
    level=min(data[s][days[k]]['low'] for k in range(i-22,i-2))
    c=data[s][prev]
    if c['low']<level and c['close']>level:
     sleeve[s]*=(1-COST);remaining[s]=hold;active=True
   if active:
    sleeve[s]*=data[s][nxt]['open']/data[s][day]['open']
    remaining[s]-=1
    if remaining[s]==0:sleeve[s]*=(1-COST)
  rows.append((day,sum(sleeve.values())/len(SYMBOLS)))
 # Convert aggregate equity levels to net daily account returns.
 returns=[];old=1.
 # At each day store aggregate sleeve wealth; each sleeve starts with  and account normalized by 7.
 for _,wealth in rows:returns.append(wealth/old-1);old=wealth
 return [(d,r) for (d,_),r in zip(rows,returns)]
def main():
 days,data=load();spans={'2020-2022':lambda d:'2020-01-01'<=d<'2023-01-01','2023-2024':lambda d:'2023-01-01'<=d<'2025-01-01','2025':lambda d:'2025-01-01'<=d<'2026-01-01','2026':lambda d:d>='2026-01-01'}
 out={'source':'Binance daily spot OHLCV, seven fixed liquid symbols','rule':'Prior closed candle made a lower low than the preceding 20 completed daily candles but closed back above that prior 20-day low; enter next open, long spot only. Hold 1, 3, 5, or 10 open-to-open intervals, then exit. One position per asset; no overlap.','cost':'15 bps per side on entry and exit; cash zero return.','timing':'Prior daily candle signal, current open entry; fees charged at entry/exit; seven equal initial capital sleeves compound independently.','periods':{}}
 for h in (1,3,5,10):
  r=run_correct(days,data,h);out['periods'][str(h)]={name:met([v for d,v in r if pred(d)]) for name,pred in spans.items()};out['periods'][str(h)]['entries']=sum(1 for i in range(22,len(days)-1) for s in SYMBOLS if data[s][days[i-1]]['low']<min(data[s][days[k]]['low'] for k in range(i-22,i-2)) and data[s][days[i-1]]['close']>min(data[s][days[k]]['low'] for k in range(i-22,i-2)))
 OUT.write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(out['periods'],indent=2))
if __name__=='__main__':main()
