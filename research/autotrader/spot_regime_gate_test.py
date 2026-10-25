#!/usr/bin/env python3
"""Test a BTC 200-day risk gate on a diversified 55-day high breakout."""
import json
from datetime import datetime, timezone
from pathlib import Path
ROOT = Path(__file__).resolve().parent
DATA = ROOT / 'data/binance_spot_1d_api_2019_2026'
SYMBOLS = ['BTCUSDT','ETHUSDT','BNBUSDT','XRPUSDT','SOLUSDT','ADAUSDT','DOGEUSDT']
COST = .0015

def load():
    data={}
    for s in SYMBOLS:
        rows=json.loads((DATA/f'{s}-1d.json').read_text())
        data[s]={datetime.fromtimestamp(x[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'o':float(x[1]),'h':float(x[2]),'c':float(x[4])} for x in rows}
    dates=sorted(set.intersection(*(set(data[s]) for s in SYMBOLS)))
    return dates,data

def stats(rs):
    wealth=peak=1.;dd=0.
    for r in rs:
        wealth*=1+r;peak=max(peak,wealth);dd=min(dd,wealth/peak-1)
    avg=sum(rs)/len(rs) if rs else 0
    var=sum((x-avg)**2 for x in rs)/max(1,len(rs)-1) if rs else 0
    return {'return_pct':100*(wealth-1),'sharpe':avg/var**.5*365**.5 if var else 0,'max_dd_pct':100*dd,'days':len(rs)}

def main():
    dates,data=load(); daily=[]; prev_raw={s:0 for s in SYMBOLS}; prev_gated={s:0 for s in SYMBOLS}
    for i in range(202,len(dates)-1):
        day,prev=dates[i],dates[i-1]
        btc_sma=sum(data['BTCUSDT'][dates[k]]['c'] for k in range(i-200,i))/200
        bull=data['BTCUSDT'][prev]['c']>btc_sma
        eligible=[]
        for s in SYMBOLS:
            prior_close=data[s][prev]['c']
            prior_high=max(data[s][dates[k]]['h'] for k in range(i-56,i-1))
            if prior_close>prior_high: eligible.append(s)
        raw={s:(1/len(SYMBOLS) if s in eligible else 0.) for s in SYMBOLS}
        gated={s:(raw[s] if bull else 0.) for s in SYMBOLS}
        turns=sum(abs(raw[s]-prev_raw[s]) for s in SYMBOLS)
        gate_turns=sum(abs(gated[s]-prev_gated[s]) for s in SYMBOLS)
        gross=sum(raw[s]*(data[s][dates[i+1]]['o']/data[s][day]['o']-1) for s in SYMBOLS)
        gate_gross=sum(gated[s]*(data[s][dates[i+1]]['o']/data[s][day]['o']-1) for s in SYMBOLS)
        daily.append({'date':day,'bull':bull,'raw':gross-COST*turns,'gated':gate_gross-COST*gate_turns,'breakout_assets':eligible})
        prev_raw=raw; prev_gated=gated
    def summarize(key):
        by_split={'train_2021_2022':[],'validation_2023_2024':[],'holdout_2025_2026':[]}
        years={};regimes={'BTC_above_SMA200':[],'BTC_below_SMA200':[]}
        for x in daily:
            r=x[key]
            sp='train_2021_2022' if x['date']<'2023-01-01' else 'validation_2023_2024' if x['date']<'2025-01-01' else 'holdout_2025_2026'
            by_split[sp].append(r);years.setdefault(x['date'][:4],[]).append(r)
            regimes['BTC_above_SMA200' if x['bull'] else 'BTC_below_SMA200'].append(r)
        return {'splits':{k:stats(v) for k,v in by_split.items()},'by_year':{k:stats(v) for k,v in years.items()},'by_btc_regime_daily_samples':{k:stats(v) for k,v in regimes.items()},'full':stats([x[key] for x in daily])}
    result={'source':'Existing local Binance daily spot candles','date_range':[daily[0]['date'],daily[-1]['date']],
      'rules':'Each of 7 assets gets 1/7 account weight on a close above prior 55-day high; gated version takes all breakout positions only when BTC prior close is above its trailing SMA200; otherwise cash. Signals at prior close, entry current open, open-to-next-open returns.',
      'cost':'15 bps per side on absolute portfolio-weight turnover; cash earns 0.',
      'out_of_sample':'Common seven-symbol data begins 2021-03-01; fixed split: train through 2022, validation 2023-24, holdout 2025 through 2026-10-22. No parameter search in this test.',
      'ungated':summarize('raw'),'btc_sma200_gated':summarize('gated')}
    p=ROOT/'spot_regime_gate_results.json';p.write_text(json.dumps(result,indent=2));print(json.dumps(result,indent=2))
if __name__=='__main__':main()
