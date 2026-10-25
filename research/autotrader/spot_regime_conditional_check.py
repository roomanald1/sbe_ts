#!/usr/bin/env python3
"""Condition retained daily spot strategy returns on a causal BTC 200d regime."""
import json
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DATA = ROOT / 'data/binance_spot_1d_api_2019_2026'
SYMBOLS = ['BTCUSDT','ETHUSDT','BNBUSDT','XRPUSDT','SOLUSDT','ADAUSDT','DOGEUSDT']
COST = 0.0015

def load():
    data={}
    for s in SYMBOLS:
        rows=json.loads((DATA/f'{s}-1d.json').read_text())
        data[s]={datetime.fromtimestamp(r[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'o':float(r[1]),'h':float(r[2]),'l':float(r[3]),'c':float(r[4])} for r in rows}
    days=sorted(set.intersection(*(set(data[s]) for s in SYMBOLS)))
    return days,data

def metrics(rs):
    if not rs:return {'days':0,'compound_return_pct':None,'mean_daily_bp':None,'positive_day_pct':None,'sharpe':None}
    wealth=1.0
    for r in rs: wealth*=1+r
    avg=sum(rs)/len(rs)
    var=sum((r-avg)**2 for r in rs)/max(1,len(rs)-1)
    return {'days':len(rs),'compound_return_pct':100*(wealth-1),'mean_daily_bp':10000*avg,'positive_day_pct':100*sum(r>0 for r in rs)/len(rs),'sharpe':avg/(var**.5)*365**.5 if var else 0}

def run(days,data):
    start=202
    regime={}
    for i in range(start,len(days)-1):
        prev=days[i-1]
        sma=sum(data['BTCUSDT'][days[k]]['c'] for k in range(i-200,i))/200
        regime[days[i]]='BTC_above_SMA200' if data['BTCUSDT'][prev]['c']>sma else 'BTC_below_SMA200'
    strategies={}
    # Shared-universe families, matching spot_regime_strategy_test.py.
    fams=['buy_and_hold','trend_50_200','donchian_55','donchian_20_filter','oversold_rebound','relative_strength_rotation']
    hold_left={s:0 for s in SYMBOLS}; weights={s:0.0 for s in SYMBOLS}
    for family in fams:
        weights={s:0.0 for s in SYMBOLS}; hold_left={s:0 for s in SYMBOLS}; daily=[]
        for i in range(start,len(days)-1):
            d,prev=days[i],days[i-1]; target={s:0. for s in SYMBOLS}
            if family=='relative_strength_rotation':
                eligible=[]
                for s in SYMBOLS:
                    avg=sum(data[s][days[k]]['c'] for k in range(i-200,i))/200
                    if data[s][prev]['c']>avg:
                        mom=data[s][prev]['c']/data[s][days[i-91]]['c']-1
                        eligible.append((mom,s))
                if eligible: target[max(eligible)[1]]=1.
            else:
                for s in SYMBOLS:
                    closes=[data[s][days[k]]['c'] for k in range(i-200,i)]
                    c=data[s][prev]['c']
                    if family=='buy_and_hold': signal=True
                    elif family=='trend_50_200': signal=c>sum(closes[-50:])/50 and c>sum(closes)/200
                    elif family=='donchian_55': signal=c>max(data[s][days[k]]['h'] for k in range(i-56,i-1))
                    elif family=='donchian_20_filter': signal=c>max(data[s][days[k]]['h'] for k in range(i-21,i-1)) and c>sum(closes)/200
                    elif family=='oversold_rebound':
                        changes=[data[s][days[k]]['c']-data[s][days[k-1]]['c'] for k in (i-2,i-1)]
                        gains=sum(max(x,0) for x in changes)/2; losses=sum(max(-x,0) for x in changes)/2
                        rsi=100. if losses==0 else 100-100/(1+gains/losses)
                        if hold_left[s]>0 and c>sum(closes)/200 and rsi<70:
                            hold_left[s]-=1; signal=True
                        elif c>sum(closes)/200 and rsi<=10:
                            hold_left[s]=4; signal=True
                        else: hold_left[s]=0; signal=False
                    target[s]=float(signal)/len(SYMBOLS)
                target={s:float(target[s]>0)/len(SYMBOLS) for s in SYMBOLS}
            turnover=sum(abs(target[s]-weights[s]) for s in SYMBOLS)
            gross=sum(target[s]*(data[s][days[i+1]]['o']/data[s][d]['o']-1) for s in SYMBOLS)
            daily.append((d,gross-COST*turnover)); weights=target
        strategies[family]=daily
    # Per-asset trend systems and the three-sleeve trend mix, each at the same daily cadence.
    for s,n in [('BTCUSDT',50),('ETHUSDT',50),('BNBUSDT',50),('BNBUSDT',100)]:
        pos=0; daily=[]
        for i in range(start,len(days)-1):
            d,prev=days[i],days[i-1]
            avg=sum(data[s][days[k]]['c'] for k in range(i-n,i))/n
            target=int(data[s][prev]['c']>avg)
            ret=data[s][days[i+1]]['o']/data[s][d]['o']-1
            daily.append((d,target*ret-COST*abs(target-pos))); pos=target
        strategies[f'{s}_SMA{n}']=daily
    # Reproduce simple equal-capital three-sleeve mix and BTC-SMA200-gated variant.
    for gated in [False,True]:
        equity={s:1/3 for s in ['BTCUSDT','ETHUSDT','BNBUSDT']}; pos={s:0 for s in equity}; daily=[]
        for i in range(start,len(days)-1):
            d,prev=days[i],days[i-1]; total=sum(equity.values()); new=0.
            btcavg=sum(data['BTCUSDT'][days[k]]['c'] for k in range(i-200,i))/200
            on=int(data['BTCUSDT'][prev]['c']>btcavg)
            for s,n in [('BTCUSDT',50),('ETHUSDT',50),('BNBUSDT',100)]:
                avg=sum(data[s][days[k]]['c'] for k in range(i-n,i))/n
                target=int(data[s][prev]['c']>avg)*(on if gated else 1)
                sleeve=equity[s]*(1-COST*abs(target-pos[s]))
                equity[s]=sleeve*(1+target*(data[s][days[i+1]]['o']/data[s][d]['o']-1))
                new+=equity[s];pos[s]=target
            daily.append((d,new/total-1))
        strategies['trend_mix_BTC200_gate' if gated else 'trend_mix_ungated']=daily
    # Validation-selected weekly/monthly rotation, dip-buy and BTC-confirmation candidates.
    # These are fixed configurations sourced from the existing search results, not reselected on this regime split.
    # Monthly 30d momentum, SMA50 filter; rebalance on the first calendar day of each month.
    rotation_pos={s:0. for s in SYMBOLS}; daily=[]
    for i in range(1,len(days)-1):
        d,prev=days[i],days[i-1]
        if d[8:10]=='01' and i>51:
            ranking=[]
            for s in SYMBOLS:
                close=data[s][prev]['c']; old=data[s][days[i-1-30]]['c']
                trend=sum(data[s][days[k]]['c'] for k in range(i-50,i))/50
                if close>trend: ranking.append((close/old-1,s))
            target={s:0. for s in SYMBOLS}
            if ranking: target[max(ranking)[1]]=1.
            turnover=sum(abs(target[s]-rotation_pos[s]) for s in SYMBOLS)
            rotation_pos=target
        else: turnover=0.
        gross=sum(rotation_pos[s]*(data[s][days[i+1]]['o']/data[s][d]['o']-1) for s in SYMBOLS)
        daily.append((d,gross-COST*turnover))
    strategies['monthly_mom30_SMA50_rotation_validation_winner']=daily
    # Validation-selected dip candidate: SOL, 200 bp one-day dip, 100 bp target, 14-day max hold.
    s='SOLUSDT'; exposure=0; entry_open=0.; entry_i=0; daily=[]
    for i in range(2,len(days)-1):
        d,prev=days[i],days[i-1]
        net=exposure*(data[s][d]['o']/data[s][days[i-1]]['o']-1)
        if exposure and (data[s][prev]['c']/entry_open-1>=.01 or i-entry_i>=14):
            exposure=0; net-=COST
        elif not exposure and data[s][prev]['c']/data[s][days[i-2]]['c']-1<=-.02:
            exposure=1; entry_open=data[s][d]['o']; entry_i=i; net-=COST
        daily.append((d,net))
    strategies['SOL_dip200_TP100_14d_validation_winner']=daily
    # Validation-selected BNB SMA100 gated by BTC SMA100, inspected in the later period.
    pos=0; daily=[]
    for i in range(101,len(days)-1):
        d,prev=days[i],days[i-1]
        bnbsma=sum(data['BNBUSDT'][days[k]]['c'] for k in range(i-100,i))/100
        btcsma=sum(data['BTCUSDT'][days[k]]['c'] for k in range(i-100,i))/100
        target=int(data['BNBUSDT'][prev]['c']>bnbsma and data['BTCUSDT'][prev]['c']>btcsma)
        ret=data['BNBUSDT'][days[i+1]]['o']/data['BNBUSDT'][d]['o']-1
        daily.append((d,target*ret-COST*abs(target-pos))); pos=target
    strategies['BNB_SMA100_BTC_SMA100_confirm']=daily
    out={}
    for name,series in strategies.items():
        buckets={'BTC_above_SMA200':[],'BTC_below_SMA200':[]}
        for d,r in series:
            if d in regime:buckets[regime[d]].append(r)
        out[name]={k:metrics(v) for k,v in buckets.items()}
    result={'regime_definition':'BTC prior daily close above/below trailing 200-day close SMA; same common 7-asset calendar; signal state uses only information available before the return interval','evaluation':'Net open-to-next-open daily strategy return, 15 bps per side on exposure/weight changes; regime-conditioned returns compound only days assigned to that BTC state, so returns are not full-path account returns. Daily observations are serially dependent.','date_range':[days[start],days[-2]],'strategies':out}
    path=ROOT/'spot_regime_conditional_results.json';path.write_text(json.dumps(result,indent=2));print(json.dumps(result,indent=2));print('saved',path)
if __name__=='__main__':
    d,x=load();run(d,x)
