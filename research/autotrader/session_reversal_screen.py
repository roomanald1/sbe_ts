#!/usr/bin/env python3
"""Long-or-cash session reversal probe on retained Binance one-minute spot candles."""
import csv, glob, json, math, statistics, zipfile
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
ROOT=Path(__file__).resolve().parent
DATA=ROOT/'data/binance_spot_1m_2026q3'
OUT=ROOT/'session_reversal_results.json'
SYMBOLS=('BTCUSDT','ETHUSDT')

def load_segments(symbol):
    hourly={}
    for path in sorted(glob.glob(str(DATA/f'{symbol}-1m-*.zip'))):
        with zipfile.ZipFile(path) as z, z.open(z.namelist()[0]) as f:
            for row in csv.reader(line.decode() for line in f):
                ts=int(row[0])//1_000_000
                dt=datetime.fromtimestamp(ts,timezone.utc)
                hour=dt.replace(minute=0,second=0,microsecond=0)
                v=hourly.setdefault(hour,{'open':float(row[1]),'close':float(row[4]),'last':ts})
                if ts>=v['last']:
                    v['close']=float(row[4]);v['last']=ts
    segs=[]
    for hour in sorted(hourly):
        # UTC 08:00-20:00 daytime, 20:00-08:00 overnight, both 12h bins.
        start=hour.replace(hour=8) if hour.hour>=8 else hour.replace(hour=20)-__import__('datetime').timedelta(days=1)
        if hour.hour>=20: start=hour.replace(hour=20)
        if hour.hour<8: start=hour.replace(hour=20)-__import__('datetime').timedelta(days=1)
        if not segs or segs[-1]['start']!=start:
            segs.append({'start':start,'open':hourly[hour]['open'],'close':hourly[hour]['close'],'last':hour})
        else:
            segs[-1]['close']=hourly[hour]['close'];segs[-1]['last']=hour
    return [s for s in segs if (s['last']-s['start']).total_seconds()>=11*3600]

def run(segs,cost):
    rows=[]
    position=0
    for i in range(1,len(segs)-1):
        prior,current,nxt=segs[i-1],segs[i],segs[i+1]
        prior_ret=prior['close']/prior['open']-1
        # If the last matching 12-hour session fell, hold spot long; otherwise stay in cash.
        # Session bins alternate day/night; compare only with the prior same type.
        if i>=2: prior_ret=segs[i-2]['close']/segs[i-2]['open']-1
        target=int(prior_ret<0)
        holding_ret=nxt['open']/current['open']-1
        fee=abs(target-position)*cost
        rows.append({'date':current['start'].strftime('%Y-%m-%d'),'month':current['start'].strftime('%Y-%m'),'return':(1+target*holding_ret)*(1-fee)-1,'target':target,'fee':fee})
        position=target
    return rows

def summary(rows):
    eq=peak=1.;dd=0.
    for x in rows:
        eq*=1+x['return'];peak=max(peak,eq);dd=min(dd,eq/peak-1)
    mean=statistics.mean([x['return'] for x in rows]) if rows else 0
    vol=statistics.stdev([x['return'] for x in rows]) if len(rows)>1 else 0
    return {'segments':len(rows),'return_percent':100*(eq-1),'max_drawdown_percent':100*dd,'annualized_sharpe':mean/vol*math.sqrt(365*2) if vol else 0,'position_fraction':sum(x['target'] for x in rows)/max(1,len(rows)),'entries':sum(x['target'] and (i==0 or not rows[i-1]['target']) for i,x in enumerate(rows))}

def main():
    out={'source':'Binance spot 1-minute klines, July-September 2026; segments aligned UTC 08:00/20:00','rule':'for each 12-hour session, go long spot for the following session only when the previous matching session (day or night) had a negative return; otherwise cash. Compare with buy-and-hold. This is a simple long-only adaptation; prior signal known at session close.','cost':'10 bps per side on each position change; no shorting, no leverage, no cash yield','caveats':'Only three months of retained minute data. Not a forward holdout. Session cutoffs are literature-informed; this is a narrow exploratory replication.','assets':{}}
    for symbol in SYMBOLS:
        segs=load_segments(symbol); rows=run(segs,.001)
        out['assets'][symbol]={'periods':{'all':summary(rows),**{m:summary([r for r in rows if r['month']==m]) for m in sorted(set(r['month'] for r in rows))}},'buy_hold':{m:summary([{'return':segs[i+1]['open']/segs[i]['open']-1,'target':1} for i in range(len(segs)-1) if segs[i]['start'].strftime('%Y-%m')==m]) for m in sorted(set(s['start'].strftime('%Y-%m') for s in segs))}}
    OUT.write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(out['assets'],indent=2))
if __name__=='__main__':main()
