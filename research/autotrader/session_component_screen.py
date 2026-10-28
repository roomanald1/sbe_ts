#!/usr/bin/env python3
"""Decompose published BTC/ETH 12h session effects into long-only spot sleeves."""
import json
from session_reversal_multiyear_screen import CACHE, session_records, stats, SYMBOLS
OUT='research/autotrader/session_component_results.json'
RANGES={'2019-2020':lambda y:2019<=y<=2020,'2021-2022':lambda y:2021<=y<=2022,'2023-2024':lambda y:2023<=y<=2024,'2025-2026':lambda y:y>=2025}

def test(records,symbol,policy,cost):
    prior={};held=0;rows=[]
    for r in records:
        old=prior.get(r['kind']);prior[r['kind']]=r
        if old is None: continue
        down=old['close']/old['open']<1
        if policy=='full_reversal': target=int(down)
        elif policy=='day_reversal': target=int(r['kind']=='day' and down)
        elif policy=='night_reversal': target=int(r['kind']=='night' and down)
        elif policy=='night_long': target=int(r['kind']=='night')
        elif policy=='day_long': target=int(r['kind']=='day')
        elif policy=='combo': target=int(r['kind']=='night' or (r['kind']=='day' and down))
        else: raise ValueError(policy)
        if r['next_open'] is None: continue
        gross=r['next_open']/r['open']-1
        rows.append({'year':r['year'],'return':(1+target*gross)*(1-cost*abs(target-held))-1,'target':target})
        held=target
    return rows

def main():
    raw=json.loads(CACHE.read_text()); policies=('full_reversal','day_reversal','night_reversal','night_long','day_long','combo');out={}
    for bp in (10,15,20):
        out[str(bp)]={}
        for s in SYMBOLS:
            rec=session_records(raw[s],8 if s=='BTCUSDT' else 5);out[str(bp)][s]={}
            for policy in policies:
                rows=test(rec,s,policy,bp/10000)
                out[str(bp)][s][policy]={name:stats([r for r in rows if pred(r['year'])]) for name,pred in RANGES.items()}
    open(OUT,'w').write(json.dumps(out,indent=2)+'\n')
    for bp in ('10','15','20'):
        print('\nCOST',bp)
        for s,pols in out[bp].items():
            for p,splits in pols.items():
                print(s,p,{k:round(v['return_percent'],1) for k,v in splits.items()})
if __name__=='__main__':main()
