#!/usr/bin/env python3
"""Test whether BTC trend confirmation improves simple spot trend filters."""
import json
from datetime import datetime,timezone
from pathlib import Path
R=Path(__file__).resolve().parent; D=R/'data/binance_spot_1d_api_2019_2026'
SYMS=['BTCUSDT','ETHUSDT','BNBUSDT']; COST=.0015

def stats(rs):
 w=peak=1.;dd=0.
 for r in rs:w*=1+r;peak=max(peak,w);dd=min(dd,w/peak-1)
 avg=sum(rs)/len(rs);var=sum((x-avg)**2 for x in rs)/max(1,len(rs)-1)
 return {'return_pct':100*(w-1),'sharpe':avg/(var**.5)*365**.5 if var else 0,'max_dd_pct':100*dd}

def main():
 data={}
 for s in SYMS:
  data[s]={datetime.fromtimestamp(x[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'o':float(x[1]),'c':float(x[4])} for x in json.loads((D/f'{s}-1d.json').read_text())}
 ds=sorted(set.intersection(*(set(data[s]) for s in SYMS)));out=[]
 for asset in ['ETHUSDT','BNBUSDT']:
  for own in [50,100,200]:
   for btc in [None,50,100,200]:
    for confirm in ([False] if btc is None else [True]):
     prev=0;rr=[]
     for i in range(1,len(ds)-1):
      sig=0
      if i>(max(own,btc) if confirm else own):
       p=ds[i-1]
       ownma=sum(data[asset][ds[k]]['c'] for k in range(i-own,i))/own
       btcma=(sum(data['BTCUSDT'][ds[k]]['c'] for k in range(i-btc,i))/btc) if confirm else 0
       sig=int(data[asset][p]['c']>ownma and (not confirm or data['BTCUSDT'][p]['c']>btcma))
      cost=COST*abs(sig-prev)
      ret=(data[asset][ds[i+1]]['o']/data[asset][ds[i]]['o']-1)*sig-cost
      rr.append((ds[i],ret));prev=sig
     splits={k:[] for k in ['train_2019_2022','validation_2023_2024','selection_2025','holdout_2026']}
     for d,r in rr:
      k='train_2019_2022' if d<'2023-01-01' else 'validation_2023_2024' if d<'2025-01-01' else 'selection_2025' if d<'2026-01-01' else 'holdout_2026'
      splits[k].append(r)
     key=f'{asset}_sma{own}'+(f'_btc_sma{btc}' if confirm else '_no_btc_gate')
     out.append({'strategy':key,'confirm':confirm,'splits':{k:stats(v) for k,v in splits.items()}})
 out.sort(key=lambda x:x['splits']['validation_2023_2024']['return_pct'],reverse=True)
 path=R/'spot_btc_confirm_results.json';path.write_text(json.dumps({'cost_bps_per_side':15,'execution':'prior-close signal; next open; open-to-next-open','results':out},indent=2))
 for x in out:
  sp=x['splits'];print(x['strategy'],*[round(sp[k]['return_pct'],1) for k in ['train_2019_2022','validation_2023_2024','selection_2025','holdout_2026']],round(sp['holdout_2026']['max_dd_pct'],1))
 print('saved',path)
if __name__=='__main__':main()
