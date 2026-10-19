"""Audit recorded cost coverage and hindsight opportunity; not a trading strategy."""
from replay import *
raw=(ROOT/'src/rust/insights/signals.jsonl').read_bytes(); data=gzip.GzipFile(fileobj=io.BytesIO(raw)).read(); i=8; rows=[]; snaps={}
while i<len(data):
 n=struct.unpack_from('<I',data,i)[0]; i+=4; kind,d=Reader(data[i:i+n]).record(); i+=n
 if kind==1 and 1792364400000<=d['t']<1792450800000: rows.append(d)
 elif kind==0: snaps[d['symbol'],d['t']]=d
out={'directions':{},'horizons':{},'feature_coverage':{}}
for direction in sorted(set(r['direction'] for r in rows)):
 rs=[r for r in rows if r['direction']==direction]; out['directions'][direction]={'n':len(rs),'costed':sum(r['cost'] is not None for r in rs)}
for h in sorted(set(r['h'] for r in rows)):
 rs=[r for r in rows if r['h']==h and r['cost'] is not None]; next_t={}; picked=[]
 for r in sorted(rs,key=lambda r:(r['t'],r['symbol'])):
  if r['t']<next_t.get(r['symbol'],0): continue
  picked.append(r); next_t[r['symbol']]=r['end']
 if not picked: continue
 costs=sorted(r['cost'] for r in picked); moves=sorted(abs(r['ret']) for r in picked)
 out['horizons'][h]={'n':len(picked),'median_cost_pct':costs[len(costs)//2],'median_abs_move_pct':moves[len(moves)//2], 'oracle_positive_pct':100*sum(abs(r['ret'])>max(0,r['cost']) for r in picked)/len(picked),'negative_costs':sum(r['cost']<0 for r in rs),'late_over_30s_pct':100*sum(r['end']-r['t']>h*1000+30000 for r in picked)/len(picked)}
for f in NAMES:
 ss=[snaps.get((r['symbol'],r['t'])) for r in rows]; out['feature_coverage'][f]=sum(s is not None and s[f] is not None for s in ss)
Path(__file__).with_name('coverage.json').write_text(json.dumps(out,indent=2)); print(json.dumps(out,indent=2))
