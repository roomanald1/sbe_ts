#!/usr/bin/env python3
"""Combine tested spot strategy sleeves over a common weekly window."""
import json
from datetime import datetime, timezone, timedelta
from pathlib import Path
ROOT=Path(__file__).resolve().parent
DATA=ROOT/'data/binance_spot_1d_api_2019_2026'
PIT=ROOT/'spot_point_in_time_weekly_results.json'
SYMBOLS=['BTCUSDT','ETHUSDT','BNBUSDT','XRPUSDT','SOLUSDT','ADAUSDT','DOGEUSDT']
COST=.0015

def stats(rs):
    rs=list(rs)
    wealth=peak=1.;dd=0.
    for r in rs:
        wealth*=1+r;peak=max(peak,wealth);dd=min(dd,wealth/peak-1)
    avg=sum(rs)/len(rs) if rs else 0.;var=sum((x-avg)**2 for x in rs)/max(1,len(rs)-1) if rs else 0.
    return {'return_pct':100*(wealth-1),'sharpe':avg/var**.5*52**.5 if var else 0,'max_dd_pct':100*dd,'weeks':len(rs)}

def load():
    data={}
    for s in SYMBOLS:
        rows=json.loads((DATA/f'{s}-1d.json').read_text())
        data[s]={datetime.fromtimestamp(x[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'o':float(x[1]),'h':float(x[2]),'c':float(x[4])} for x in rows}
    days=sorted(set.intersection(*(set(data[s]) for s in SYMBOLS)))
    return days,data

def simulate_daily(days,data,start,end):
    start_i=days.index(start); end_i=days.index(end)
    daily={}
    names=['buy_hold_7','trend_50_200','donchian_55','breakout_20_sma200','oversold_rsi2','rotation_90_sma200_monthly','btc_gated_donchian55']
    positions={n:{s:0. for s in SYMBOLS} for n in names}
    held={s:0 for s in SYMBOLS}
    for i in range(start_i,end_i):
        day,prev=days[i],days[i-1]
        # Prior-close BTC risk gate.
        btc_sma=sum(data['BTCUSDT'][days[k]]['c'] for k in range(i-200,i))/200
        btc_on=data['BTCUSDT'][prev]['c']>btc_sma
        targets={n:{s:0. for s in SYMBOLS} for n in names}
        targets['buy_hold_7']={s:1/len(SYMBOLS) for s in SYMBOLS}
        rotation_rebalance=day[8:10]=='01'
        for s in SYMBOLS:
            c=data[s][prev]['c']
            sma50=sum(data[s][days[k]]['c'] for k in range(i-50,i))/50
            sma200=sum(data[s][days[k]]['c'] for k in range(i-200,i))/200
            high55=max(data[s][days[k]]['h'] for k in range(i-56,i-1))
            high20=max(data[s][days[k]]['h'] for k in range(i-21,i-1))
            targets['trend_50_200'][s]=float(c>sma50 and c>sma200)/len(SYMBOLS)
            targets['donchian_55'][s]=float(c>high55)/len(SYMBOLS)
            targets['breakout_20_sma200'][s]=float(c>high20 and c>sma200)/len(SYMBOLS)
            targets['btc_gated_donchian55'][s]=float(c>high55 and btc_on)/len(SYMBOLS)
            changes=[data[s][days[k]]['c']-data[s][days[k-1]]['c'] for k in (i-2,i-1)]
            gains=sum(max(x,0) for x in changes)/2;losses=sum(max(-x,0) for x in changes)/2
            rsi2=100. if losses==0 else 100-100/(1+gains/losses)
            if held[s]>0 and c>sma200 and rsi2<70:
                held[s]-=1; mr=True
            elif c>sma200 and rsi2<=10:
                held[s]=4;mr=True
            else:
                held[s]=0;mr=False
            targets['oversold_rsi2'][s]=float(mr)/len(SYMBOLS)
        # Monthly 90d relative-strength rotation, filtered by each asset's 200d trend.
        if rotation_rebalance:
            eligible=[]
            for s in SYMBOLS:
                c=data[s][prev]['c'];sma=sum(data[s][days[k]]['c'] for k in range(i-200,i))/200
                if c>sma:eligible.append((c/data[s][days[i-91]]['c']-1,s))
            if eligible: targets['rotation_90_sma200_monthly'][max(eligible)[1]]=1.
        else:
            targets['rotation_90_sma200_monthly']=positions['rotation_90_sma200_monthly'].copy()
        for name in names:
            old=positions[name];new=targets[name]
            turnover=sum(abs(new[s]-old[s]) for s in SYMBOLS)
            gross=sum(new[s]*(data[s][days[i+1]]['o']/data[s][day]['o']-1) for s in SYMBOLS)
            daily.setdefault(day,{})[name]=gross-COST*turnover
            positions[name]=new
    # Existing three-sleeve trend mix as a separate strategy sleeve.
    mixeq={s:1/3 for s in ['BTCUSDT','ETHUSDT','BNBUSDT']};mixpos={s:0 for s in mixeq};mixdaily={}
    for i in range(start_i,end_i):
        day,prev=days[i],days[i-1];newtotal=0.;oldtotal=sum(mixeq.values())
        for s,n in [('BTCUSDT',50),('ETHUSDT',50),('BNBUSDT',100)]:
            avg=sum(data[s][days[k]]['c'] for k in range(i-n,i))/n
            target=int(data[s][prev]['c']>avg)
            sleeve=mixeq[s]*(1-COST*abs(target-mixpos[s]))
            r=data[s][days[i+1]]['o']/data[s][day]['o']-1
            mixeq[s]=sleeve*(1+target*r);mixpos[s]=target;newtotal+=mixeq[s]
        mixdaily[day]=newtotal/oldtotal-1
    for day,r in mixdaily.items():daily[day]['btc_eth_bnb_trend_mix']=r
    return daily

def main():
    days,data=load()
    pit=json.loads(PIT.read_text())['variants']
    pit_logs={name:{x['date']:x['net_return'] for x in pit[name]['sample_trade_log']} for name in pit}
    sundays=[d for d in days if datetime.strptime(d,'%Y-%m-%d').weekday()==6 and d>='2021-03-01']
    pit_dates=set.intersection(*(set(m) for m in pit_logs.values()))
    sundays=[d for d in sundays if d in pit_dates and d in days]
    # A week uses all daily open-to-open returns from Sunday through Saturday.
    daily=simulate_daily(days,data,sundays[0],sundays[-1])
    components=['btc_eth_bnb_trend_mix','buy_hold_7','trend_50_200','donchian_55','breakout_20_sma200','oversold_rsi2','rotation_90_sma200_monthly','btc_gated_donchian55','pit_top_1_equal_weight','pit_top_5_equal_weight','pit_top_10_equal_weight']
    series={k:[] for k in components}
    for j,(d,nxt) in enumerate(zip(sundays,sundays[1:])):
        begin=days.index(d);finish=days.index(nxt)
        for name in components[:8]:
            w=1.
            for day in days[begin:finish]:w*=1+daily[day][name]
            series[name].append(w-1)
        for name,logs in pit_logs.items():series['pit_'+name].append(logs[d])
    # Each strategy receives equal initial capital in its own sleeve, compounds
    # independently, and no sleeve is rebalanced to restore its original weight.
    def combined(names):
        equity={n:1/len(names) for n in names};rs=[]
        for t in range(len(sundays)-1):
            old=sum(equity.values())
            for n in names:equity[n]*=1+series[n][t]
            rs.append(sum(equity.values())/old-1)
        return rs
    groups={'all_11_sleeves':components,
            'all_except_point_in_time_rotations':components[:8],
            'trend_breakout_and_mix_only':[x for x in components if x in ['btc_eth_bnb_trend_mix','trend_50_200','donchian_55','breakout_20_sma200','btc_gated_donchian55']]}
    def splits(rs):
        out={'train_2021_2022':[],'validation_2023_2024':[],'holdout_2025_2026':[]}
        for d,r in zip(sundays[:-1],rs):
            k='train_2021_2022' if d<'2023-01-01' else 'validation_2023_2024' if d<'2025-01-01' else 'holdout_2025_2026'
            out[k].append(r)
        return {k:stats(v) for k,v in out.items()}
    results={'range':[sundays[0],(datetime.strptime(sundays[-1],'%Y-%m-%d')+timedelta(days=7)).strftime('%Y-%m-%d')],'weekly_periods':len(sundays)-1,
      'construction':'Equal initial capital in independent strategy sleeves; sleeve equity compounds separately; no leverage; cash stays inside each strategy; portfolio is not rebalanced back to equal weights. Daily strategies are aggregated into Sunday-to-Sunday returns. The point-in-time rotation sleeves use CoinMarketCap snapshot prices. Costs are as estimated in each component test.',
      'components':{n:{'full':stats(series[n]),'holdout':stats(series[n][i] for i,d in enumerate(sundays[:-1]) if d>='2025-01-01')} for n in components},
      'combined':{g:{'full':stats(combined(ns)),'splits':splits(combined(ns)),'sleeves':ns} for g,ns in groups.items()},
      'limits':'This combines heterogeneous strategy backtests and price sources, with imperfect daily fee/execution estimates. The component rules were selected after prior research, which creates selection bias. The combined results are not a fresh untouched test and do not imply future profitability.'}
    out=ROOT/'spot_all_strategy_portfolio_results.json';out.write_text(json.dumps(results,indent=2));print(json.dumps(results,indent=2))
if __name__=='__main__':main()
