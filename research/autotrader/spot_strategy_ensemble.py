#!/usr/bin/env python3
"""Evaluate equal initial capital sleeves with proper changing sleeve values."""
import json
from datetime import datetime,timezone
from pathlib import Path
ROOT=Path(__file__).resolve().parent;DATA=ROOT/'data/binance_spot_1d_api_2019_2026';COST=.0015
RULES=[('BTCUSDT',50),('ETHUSDT',50),('BNBUSDT',100)]

def metric(rs):
 if not rs:return {'return_pct':0,'sharpe':0,'max_dd_pct':0,'days':0}
 wealth=peak=1.;dd=0.
 for r in rs:wealth*=1+r;peak=max(peak,wealth);dd=min(dd,wealth/peak-1)
 avg=sum(rs)/len(rs);var=sum((r-avg)**2 for r in rs)/max(1,len(rs)-1)
 return {'return_pct':100*(wealth-1),'sharpe':avg/(var**.5)*365**.5 if var else 0,'max_dd_pct':100*dd,'days':len(rs)}

def portfolio(days,data,gated):
 equity={s:1/len(RULES) for s,_ in RULES};position={s:0 for s,_ in RULES};rets=[];regimes=[]
 for i in range(201,len(days)-1):
  day=days[i];prev=days[i-1]
  btc_sma200=sum(data['BTCUSDT'][days[k]]['c'] for k in range(i-200,i))/200
  risk_on=int(data['BTCUSDT'][prev]['c']>btc_sma200)
  old_total=sum(equity.values());new_total=0.
  for s,n in RULES:
   avg=sum(data[s][days[k]]['c'] for k in range(i-n,i))/n
   target=int(data[s][prev]['c']>avg)
   if gated:target*=risk_on
   sleeve=equity[s]*(1-COST*abs(target-position[s]))
   ret=data[s][days[i+1]]['o']/data[s][day]['o']-1
   equity[s]=sleeve*(1+target*ret)
   new_total+=equity[s];position[s]=target
  netret=new_total/old_total-1
  rets.append((day,netret));regimes.append((day,risk_on,netret))
 return rets,regimes

def main():
 data={}
 for s,_ in RULES:
  data[s]={datetime.fromtimestamp(r[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'o':float(r[1]),'c':float(r[4])} for r in json.loads((DATA/f'{s}-1d.json').read_text())}
 days=sorted(set.intersection(*(set(x) for x in data.values())))
 mixed,regimes=portfolio(days,data,False);gated,_=portfolio(days,data,True)
 # Buy and hold three equal dollar sleeves, including initial purchase and final liquidation costs.
 bh_equity={s:(1/len(RULES))*(1-COST) for s,_ in RULES};bh_returns=[]
 for i in range(201,len(days)-1):
  old=sum(bh_equity.values())
  for s,_ in RULES:bh_equity[s]*=data[s][days[i+1]]['o']/data[s][days[i]]['o']
  new=sum(bh_equity.values())
  if i==len(days)-2:new*=1-COST
  bh_returns.append((days[i],new/old-1))
 bucket={k:[] for k in ['BTC_above_SMA200','BTC_below_SMA200']};yearly={}
 for d,on,r in regimes:
  bucket['BTC_above_SMA200' if on else 'BTC_below_SMA200'].append(r)
  yearly.setdefault(d[:4],{'mix':[],'btc_gated':[]})['mix'].append(r)
 for d,r in gated:yearly[d[:4]]['btc_gated'].append(r)
 result={'construction':'three independent sleeves, each starts with one third of capital: BTC above SMA50, ETH above SMA50, BNB above SMA100; uninvested sleeve value stays cash','gated_construction':'same sleeves, each disabled when BTC prior close is below its trailing SMA200','accounting':'sleeve equity compounds independently; fee is debited on sleeve equity at each position change; aggregate daily return is change in total account equity','timing':'prior-close signal, rebalance at next open, open-to-next-open returns','cost':'15 bps per side on sleeve notional changes','range':[mixed[0][0],mixed[-1][0]],'portfolio':metric([r for _,r in mixed]),'equal_weight_buy_and_hold':metric([r for _,r in bh_returns]),'btc_regime_gated':metric([r for _,r in gated]),'by_btc_regime':{k:metric(v) for k,v in bucket.items()},'by_year':{y:{k:metric(v) for k,v in values.items()} for y,values in yearly.items()}}
 path=ROOT/'spot_strategy_ensemble_results.json';path.write_text(json.dumps(result,indent=2));print(json.dumps(result,indent=2))
if __name__=='__main__':main()
