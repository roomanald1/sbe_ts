#!/usr/bin/env python3
"""Test literature-specified 12h BTC/ETH session reversal on Binance hourly spot candles."""
import json, math, statistics, time, urllib.parse, urllib.request
from datetime import datetime, timezone, timedelta
from pathlib import Path
ROOT=Path(__file__).resolve().parent
CACHE=ROOT/'data/session_reversal_hourly_2019_2026.json'
OUT=ROOT/'session_reversal_multiyear_results.json'
START=int(datetime(2019,1,1,tzinfo=timezone.utc).timestamp()*1000)
END=int(datetime.now(timezone.utc).replace(minute=0,second=0,microsecond=0).timestamp()*1000)
SYMBOLS=('BTCUSDT','ETHUSDT')

def fetch(symbol):
    allrows=[]; start=START
    while start<END:
        query=urllib.parse.urlencode({'symbol':symbol,'interval':'1h','startTime':start,'endTime':END,'limit':1000})
        req=urllib.request.Request('https://api.binance.com/api/v3/klines?'+query,headers={'User-Agent':'research-screen/1.0'})
        with urllib.request.urlopen(req,timeout=30) as response: rows=json.load(response)
        if not rows: break
        allrows.extend(rows); start=int(rows[-1][0])+3600000
        time.sleep(.08)
    return allrows

def session_records(rows, boundary):
    hours={int(r[0]):{'open':float(r[1]),'close':float(r[4])} for r in rows}
    night_boundary=(boundary+12)%24
    starts=sorted(t for t in hours if datetime.fromtimestamp(t/1000,timezone.utc).hour in (boundary,night_boundary))
    rec=[]
    for t in starts:
        closes=[t+j*3600000 for j in range(12)]
        if all(x in hours for x in closes) and t+12*3600000 in hours:
            dt=datetime.fromtimestamp(t/1000,timezone.utc)
            kind='day' if dt.hour==boundary else 'night'
            rec.append({'time':t,'date':dt.strftime('%Y-%m-%d'),'year':dt.year,'month':dt.strftime('%Y-%m'),'kind':kind,'open':hours[t]['open'],'close':hours[closes[-1]]['close'],'next_open':hours[t+12*3600000]['open']})
    return rec

def evaluate(records,symbol,cost):
    # Signal at the end of a session; execute at the next UTC session open and hold for 12h.
    result=[]; prior_by_kind={}; held=0
    for i,r in enumerate(records):
        prev=prior_by_kind.get(r['kind'])
        prior_by_kind[r['kind']]=r
        if prev is None or r['next_open'] is None: continue
        prior_ret=prev['close']/prev['open']-1
        if symbol=='BTCUSDT': target=int(prior_ret<0)
        else: target=1 if r['kind']=='night' else int(prior_ret<0)
        gross=r['next_open']/r['open']-1
        net=(1+target*gross)*(1-cost*abs(target-held))-1
        result.append({'date':r['date'],'year':r['year'],'month':r['month'],'kind':r['kind'],'return':net,'target':target,'gross':gross,'fee':cost*abs(target-held)})
        held=target
    return result

def stats(rows):
    eq=peak=1.;dd=0.;vals=[r['return'] for r in rows]
    for x in vals: eq*=1+x;peak=max(peak,eq);dd=min(dd,eq/peak-1)
    mu=statistics.mean(vals) if vals else 0;sd=statistics.stdev(vals) if len(vals)>1 else 0
    return {'sessions':len(vals),'return_percent':100*(eq-1),'max_drawdown_percent':100*dd,'annualized_sharpe':mu/sd*math.sqrt(365) if sd else 0,'exposure_fraction':sum(r['target'] for r in rows)/max(1,len(rows)),'entries':sum(r['target'] and (i==0 or not rows[i-1]['target']) for i,r in enumerate(rows))}

def main():
    if CACHE.exists(): all_data=json.loads(CACHE.read_text())
    else:
        CACHE.parent.mkdir(parents=True,exist_ok=True);all_data={s:fetch(s) for s in SYMBOLS};CACHE.write_text(json.dumps(all_data,separators=(',',':'))+'\n')
    result={'source':'Binance public spot hourly klines fetched via REST; UTC segment returns use 12 hourly candles, prior same-session information, next-session-open execution','specification':'BTC: 08:00-20:00 and 20:00-08:00 UTC, long following matching session after a negative previous same-session return, otherwise cash. ETH: 05:00-17:00 UTC daytime same reversal rule; 17:00-05:00 UTC overnight long (the literature-selected ETH specification). No leverage or shorting; cash earns zero.','cost':'10 bps per side on each position change (entry/exit). No slippage beyond this fee, so optimistic.','evidence_note':'Session cutoffs are from a published paper that tested 300 cutoff/rule combinations; it reports no SPA rejection of model superiority. This is an independent replication screen, not proof of a robust edge.','periods':{},'assets':{}}
    ranges={'2019-2020':lambda y:2019<=y<=2020,'2021-2022':lambda y:2021<=y<=2022,'2023-2024':lambda y:2023<=y<=2024,'2025-2026':lambda y:y>=2025}
    for bp in (10,20,30):
        result['assets'][str(bp)]={}
        for s in SYMBOLS:
            boundary=8 if s=='BTCUSDT' else 5
            rec=session_records(all_data[s],boundary); rows=evaluate(rec,s,bp/10000)
            result['assets'][str(bp)][s]={'periods':{name:stats([r for r in rows if cond(r['year'])]) for name,cond in ranges.items()},'all':stats(rows),'annual':{str(y):stats([r for r in rows if r['year']==y]) for y in sorted({r['year'] for r in rows})},'buy_hold_by_period':{name:stats([{'return':r['gross'],'target':1} for r in rows if cond(r['year'])]) for name,cond in ranges.items()}}
    result['data_range']={s:[datetime.fromtimestamp(r[0]/1000,timezone.utc).strftime('%Y-%m-%d %H:%M') for r in (all_data[s][0],all_data[s][-1])] for s in SYMBOLS}
    OUT.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result['assets']['10'],indent=2))
if __name__=='__main__':main()
