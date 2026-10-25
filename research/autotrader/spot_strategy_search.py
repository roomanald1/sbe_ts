#!/usr/bin/env python3
"""Fetch Binance daily klines and test simple long-only spot strategies."""
import json, time, urllib.parse, urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT=Path(__file__).resolve().parent
DATA=ROOT/'data'/'binance_spot_1d_api_2019_2026'
SYMBOLS=['BTCUSDT','ETHUSDT','BNBUSDT','XRPUSDT','SOLUSDT','ADAUSDT','DOGEUSDT']
START=int(datetime(2019,1,1,tzinfo=timezone.utc).timestamp()*1000)
END=int(datetime(2026,10,23,tzinfo=timezone.utc).timestamp()*1000)
API='https://data-api.binance.vision/api/v3/klines'
FEE_BPS=10.0
SLIPPAGE_BPS=5.0

def fetch(symbol):
    DATA.mkdir(parents=True,exist_ok=True); path=DATA/f'{symbol}-1d.json'
    if path.exists(): return json.loads(path.read_text())
    rows=[]; start=START
    while start<END:
        q=urllib.parse.urlencode({'symbol':symbol,'interval':'1d','startTime':start,'endTime':END,'limit':1000})
        req=urllib.request.Request(f'{API}?{q}',headers={'User-Agent':'historical-research/1.0'})
        for attempt in range(5):
            try:
                with urllib.request.urlopen(req,timeout=30) as res: batch=json.load(res)
                break
            except Exception:
                if attempt==4: raise
                time.sleep(1.5*(attempt+1))
        if not batch: break
        rows.extend(batch); start=int(batch[-1][0])+86400000
        if len(batch)<1000: break
        time.sleep(.15)
    path.write_text(json.dumps(rows,separators=(',',':'))); return rows

def dstr(ts): return datetime.fromtimestamp(ts/1000,timezone.utc).strftime('%Y-%m-%d')
def load():
    data={}
    for s in SYMBOLS:
        rs=fetch(s); data[s]={dstr(int(r[0])):{'open':float(r[1]),'close':float(r[4])} for r in rs}
        print(f'{s}: {len(rs)} daily bars',flush=True)
    return data

def signals(closes,kind,n):
    out=[0.0]*len(closes)
    for i in range(n+1,len(closes)):
        if kind=='sma': out[i]=float(closes[i-1]>sum(closes[i-n:i])/n)
        else: out[i]=float(closes[i-1]>closes[i-n-1])
    return out

def stat(a):
    if not a:return {'return_pct':0.0,'sharpe':0.0,'max_dd_pct':0.0}
    w=peak=1.; dd=0.
    for r in a: w*=1+r; peak=max(peak,w); dd=min(dd,w/peak-1)
    avg=sum(a)/len(a); var=sum((x-avg)**2 for x in a)/max(1,len(a)-1)
    return {'return_pct':(w-1)*100,'sharpe':avg/(var**.5)*(365**.5) if var else 0.,'max_dd_pct':dd*100}

def evaluate(series,sig,cost):
    rets=[]; prev=0.; turnover=0.; active=0
    for i in range(1,len(series)-1):
        exposure=sig[i]; change=abs(exposure-prev); turnover+=change
        r=(series[i+1][1]/series[i][1]-1)*exposure-change*cost
        rets.append((series[i][0],r)); active+=int(exposure>0); prev=exposure
    split={k:[] for k in ['train_2020_2022','validation_2023_2024','holdout_2025_2026']}
    for day,r in rets:
        key='train_2020_2022' if day<'2023-01-01' else 'validation_2023_2024' if day<'2025-01-01' else 'holdout_2025_2026'
        split[key].append(r)
    return {k:stat(v) for k,v in split.items()},turnover,active,len(rets)

def main():
    data=load(); days=sorted(set.intersection(*(set(x) for x in data.values())))
    aligned={s:[(d,data[s][d]['open'],data[s][d]['close']) for d in days] for s in SYMBOLS}
    cost=(FEE_BPS+SLIPPAGE_BPS)/10000; results=[]
    for s in SYMBOLS:
        c=[x[2] for x in aligned[s]]
        for kind,n in [('sma',50),('sma',100),('sma',200),('momentum',30),('momentum',90),('momentum',180)]:
            st,to,active,count=evaluate(aligned[s],signals(c,kind,n),cost)
            results.append({'strategy':f'{s}_{kind}{n}','cost_bps_per_side':FEE_BPS+SLIPPAGE_BPS,'splits':st,'turnover_units':to,'active_days':active,'observations':count})
    # Equal-weight daily portfolio of 90-day time-series momentum signals.
    sigs={s:signals([x[2] for x in aligned[s]],'momentum',90) for s in SYMBOLS}
    p_sig=[sum(sigs[s][i] for s in SYMBOLS)/len(SYMBOLS) for i in range(len(days))]
    st,to,active,count=evaluate(aligned[SYMBOLS[0]],p_sig,cost)
    results.append({'strategy':'equal_weight_mom90','cost_bps_per_side':FEE_BPS+SLIPPAGE_BPS,'splits':st,'turnover_units':to,'active_days':active,'observations':count})
    out=ROOT/'spot_strategy_search_results.json'
    out.write_text(json.dumps({'data_source':API,'symbols':SYMBOLS,'bar':'1d','execution':'prior-close signal, next open; open-to-next-open returns','cost_model':{'fee_bps_per_side':FEE_BPS,'slippage_bps_per_side':SLIPPAGE_BPS,'charged_on_exposure_change':True},'splits':'train through 2022; validation 2023-24; holdout 2025 onward','results':results},indent=2))
    for r in results: print(r['strategy'],json.dumps(r['splits'],separators=(',',':')))
    print('saved',out)
if __name__=='__main__':main()
