#!/usr/bin/env python3
"""Test regime-gated and return-residualized weekly order-flow rotation."""
import json
from datetime import datetime,timezone
from pathlib import Path
from weekly_orderflow_rotation_screen import load,stats,SYMBOLS
ROOT=Path(__file__).resolve().parent;OUT=ROOT/'weekly_orderflow_variants_results.json'
SPANS={'2020-2022':lambda d:'2020-01-01'<=d<'2023-01-01','2023-2024':lambda d:'2023-01-01'<=d<'2025-01-01','2025':lambda d:'2025-01-01'<=d<'2026-01-01','2026':lambda d:d>='2026-01-01'}
def run(dates,data,cost,mode):
 weights={s:0. for s in SYMBOLS};rows=[]
 for i in range(201,len(dates)-1):
  day,prior=dates[i],dates[i-1];turn=0
  if datetime.strptime(day,'%Y-%m-%d').weekday()==0:
   flow={};momentum={}
   for s in SYMBOLS:
    q=sum(data[s][dates[k]]['quote'] for k in range(i-7,i));b=sum(data[s][dates[k]]['buy_quote'] for k in range(i-7,i))
    flow[s]=(2*b-q)/q if q else -1
    momentum[s]=data[s][prior]['close']/data[s][dates[i-8]]['close']-1
   if mode=='residual_flow':
    mx=sum(momentum.values())/len(SYMBOLS);my=sum(flow.values())/len(SYMBOLS)
    den=sum((momentum[s]-mx)**2 for s in SYMBOLS)
    beta=sum((momentum[s]-mx)*(flow[s]-my) for s in SYMBOLS)/den if den else 0
    score={s:flow[s]-my-beta*(momentum[s]-mx) for s in SYMBOLS}
   else: score=flow
   btc_mean=sum(data['BTCUSDT'][dates[k]]['close'] for k in range(i-200,i))/200
   risk_on=data['BTCUSDT'][prior]['close']>btc_mean
   selected={s:0. for s in SYMBOLS}
   ranked=sorted(((score[s],s) for s in SYMBOLS),reverse=True)[:2]
   gate=(mode!='flow_btc200' or risk_on)
   if gate and ranked[0][0]>0:
    for _,s in ranked:selected[s]=.5
   turn=sum(abs(selected[s]-weights[s]) for s in SYMBOLS);weights=selected
  gross=sum(weights[s]*(data[s][dates[i+1]]['open']/data[s][day]['open']-1) for s in SYMBOLS)
  rows.append((day,gross-cost*turn))
 return rows
def main():
 dates,data=load();out={}
 for mode in ('flow_btc200','residual_flow'):
  for bp in (10,15):
   rows=run(dates,data,bp/10000,mode);out[f'{mode}_{bp}bp']={name:stats([v for d,v in rows if pred(d)]) for name,pred in SPANS.items()}
 print(json.dumps(out,indent=2));OUT.write_text(json.dumps(out,indent=2)+'\n')
if __name__=='__main__':main()
