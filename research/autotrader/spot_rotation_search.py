#!/usr/bin/env python3
"""Test simple spot cross-sectional momentum rotations with daily Binance candles."""
import json
from datetime import datetime,timezone
from pathlib import Path
ROOT=Path(__file__).resolve().parent
DATA=ROOT/'data/binance_spot_1d_api_2019_2026'
SYMBOLS=['BTCUSDT','ETHUSDT','BNBUSDT','XRPUSDT','SOLUSDT','ADAUSDT','DOGEUSDT']
COST=.0015

def load():
    out={}
    for s in SYMBOLS:
        raw=json.loads((DATA/f'{s}-1d.json').read_text())
        out[s]={datetime.fromtimestamp(r[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'open':float(r[1]),'close':float(r[4])} for r in raw}
    dates=sorted(set.intersection(*(set(x) for x in out.values())))
    return dates,out

def stats(rs):
    if not rs:return {'return_pct':0,'sharpe':0,'max_dd_pct':0}
    w=peak=1.;dd=0.
    for r in rs:w*=1+r;peak=max(peak,w);dd=min(dd,w/peak-1)
    avg=sum(rs)/len(rs);var=sum((r-avg)**2 for r in rs)/max(1,len(rs)-1)
    return {'return_pct':100*(w-1),'sharpe':avg/(var**.5)*(365**.5) if var else 0,'max_dd_pct':100*dd}

def run(dates,data,lookback,filter_days,frequency):
    rets=[]; weights={s:0. for s in SYMBOLS}; last_day=None
    for i in range(1,len(dates)-1):
        day=dates[i]; prev=dates[i-1]
        rebalance=(frequency=='daily' or (frequency=='weekly' and datetime.strptime(day,'%Y-%m-%d').weekday()==0) or (frequency=='monthly' and day[8:10]=='01'))
        if rebalance and i>max(lookback,filter_days)+1:
            ranking=[]
            for s in SYMBOLS:
                close=data[s][prev]['close']; old=data[s][dates[i-1-lookback]]['close']
                trend=sum(data[s][dates[k]]['close'] for k in range(i-filter_days,i))/filter_days
                if close>trend: ranking.append((close/old-1,s))
            target={s:0. for s in SYMBOLS}
            if ranking:
                _,chosen=max(ranking)
                target[chosen]=1.
            turnover=sum(abs(target[s]-weights[s]) for s in SYMBOLS)
            weights=target
        else: turnover=0.
        gross=sum(weights[s]*(data[s][dates[i+1]]['open']/data[s][day]['open']-1) for s in SYMBOLS)
        net=gross-COST*turnover
        rets.append((day,net,turnover))
    splits={k:[] for k in ['train_2020_2022','validation_2023_2024','selection_2025','holdout_2026']}
    for d,r,_ in rets:
        key='train_2020_2022' if d<'2023-01-01' else 'validation_2023_2024' if d<'2025-01-01' else 'selection_2025' if d<'2026-01-01' else 'holdout_2026'
        splits[key].append(r)
    return {k:stats(v) for k,v in splits.items()},sum(x[2] for x in rets),len(rets)

def main():
    dates,data=load(); results=[]
    for lb in [30,90,180]:
        for filt in [50,100,200]:
            for freq in ['weekly','monthly']:
                sp,to,n=run(dates,data,lb,filt,freq)
                results.append({'strategy':f'momentum{lb}_above_sma{filt}_{freq}','lookback_days':lb,'trend_filter_days':filt,'rebalance':freq,'cost_bps_per_side':15,'splits':sp,'turnover_units':to,'observations':n})
    results.sort(key=lambda x:x['splits']['validation_2023_2024']['return_pct'],reverse=True)
    out=ROOT/'spot_rotation_search_results.json'
    out.write_text(json.dumps({'universe':SYMBOLS,'data':'Binance Spot daily klines; common calendar across all seven pairs','execution':'signal from prior daily close; rebalance at next open; open-to-next-open returns','cost':'15 bps per side on absolute weight changes; includes assumed 10bp fee + 5bp slippage','selection_rule':'rank by validation 2023-24 only; report 2025 and 2026 separately','results':results},indent=2))
    for r in results: print(r['strategy'],*[round(r['splits'][p]['return_pct'],1) for p in ['train_2020_2022','validation_2023_2024','selection_2025','holdout_2026']],round(r['splits']['holdout_2026']['sharpe'],2),round(r['splits']['holdout_2026']['max_dd_pct'],1))
    print('saved',out)
if __name__=='__main__':main()
