"""Offline, dependency-free chronological rule study of recorded signal outcomes."""
import io, gzip, struct, json, hashlib, math, collections
from pathlib import Path
from datetime import datetime, timezone
ROOT=Path(__file__).resolve().parents[2]
NAMES=['trend1','momentum','flow','book','volatility','trend5','trend5_units','vwap','pressure','aggressor_cost']
class Reader:
 def __init__(self,b): self.b=b; self.i=0
 def val(self,f):
  v=struct.unpack_from('<'+f,self.b,self.i)[0]; self.i+=struct.calcsize('<'+f); return v
 def s(self):
  n=self.val('Q'); v=self.b[self.i:self.i+n].decode(); self.i+=n; return v
 def opt(self,f='d'): return self.val(f) if self.val('B') else None
 def record(self):
  kind=self.val('I')
  if kind==0:
   d=dict(symbol=self.s(),t=self.val('Q'),direction=self.s(),score=self.val('d'),strength=self.val('d'))
   d.update({k:self.opt() for k in NAMES})
   for _ in range(self.val('Q')): self.s()
   d.update(stale=self.val('B'),ranging=self.opt(),lower=self.opt(),upper=self.opt())
  elif kind==1:
   self.s(); d=dict(symbol=self.s(),t=self.val('Q'),end=self.val('Q'),h=self.val('Q'),ret=self.val('d'),match=self.val('B'),direction=self.s(),strength=self.val('d'))
   d.update(net=self.opt(),cost=self.opt(),trend_units=self.opt(),trend_match=self.opt('B'),trend_net=self.opt())
   if self.val('B'): d['features']=[self.val('d') for _ in range(4)]
  else: raise ValueError(kind)
  assert self.i==len(self.b),(kind,self.i,len(self.b))
  return kind,d

def main():
 raw=(ROOT/'src/rust/insights/signals.jsonl').read_bytes()
 data=gzip.GzipFile(fileobj=io.BytesIO(raw)).read() if raw[:2]==b'\x1f\x8b' else raw
 assert data[:8]==b'SIGBIN01'
 snapshots={}; outcomes=[]; i=8; counts=collections.Counter()
 while i<len(data):
  n=struct.unpack_from('<I',data,i)[0]; i+=4
  kind,d=Reader(data[i:i+n]).record(); i+=n; counts[kind]+=1
  if kind==0: snapshots[d['symbol'],d['t']]=d
  else: outcomes.append(d)
 day='2026-10-19'; start=int(datetime.fromisoformat(day).replace(tzinfo=timezone.utc).timestamp()*1000)-3600000; finish=start+86400000
 today=[o for o in outcomes if start<=o['t']<finish]
 rows=[]
 for o in today:
  s=snapshots.get((o['symbol'],o['t']))
  if s and not s['stale'] and o['cost'] is not None and o['net'] is not None and o['end']>=o['t'] and all(math.isfinite(o[k]) for k in ['ret','cost','net']): rows.append(dict(s,**{k:v for k,v in o.items() if k not in ['strength']}))
 lo=min(r['t'] for r in today); hi=max(r['end'] for r in today); b1=lo+(hi-lo)*.5; b2=lo+(hi-lo)*.75
 parts=[[r for r in rows if a<=r['t']<b and r['end']<b] for a,b in [(lo,b1),(b1,b2),(b2,hi+1)]]
 parts=[sorted(p,key=lambda x:(x['t'],x['symbol'])) for p in parts]
 def stamp(t): return datetime.fromtimestamp(t/1000,timezone.utc).isoformat()
 meta=dict(sha256=hashlib.sha256(raw).hexdigest(),records=dict(counts),today_outcomes=len(today),matched_costed=len(rows),symbols=len(set(r['symbol'] for r in rows)),horizons=dict(collections.Counter(r['h'] for r in rows)),utc_boundaries=list(map(stamp,[lo,b1,b2,hi])),partition_rows=list(map(len,parts)),missing_cost=sum(o['cost'] is None for o in today))
 # Both sides assessed using recorded same-side net; opposite uses same observed execution-cost proxy.
 cache={}
 def evaluate(part,rule):
  h,feat,threshold,polarity,gate=rule; next_t={}; trades=[]
  key=(id(part),h)
  if key not in cache: cache[key]=[r for r in part if r['h']==h]
  for r in cache[key]:
   if r['h']!=h or r['t']<next_t.get(r['symbol'],0): continue
   if feat=='baseline': side=1 if r['direction']=='bullish' else -1 if r['direction']=='bearish' else 0
   else:
    v=r.get(feat)
    if v is None or not math.isfinite(v) or abs(v)<threshold: continue
    side=(1 if v>0 else -1)*polarity
   if not side: continue
   if gate and (r.get('volatility') is None or r['volatility']<gate): continue
   same=side==(1 if r['direction']=='bullish' else -1)
   net=r['net'] if same else side*r['ret']-r['cost']
   trades.append((r['t'],r['end'],r['symbol'],net,same,r['net'])); next_t[r['symbol']]=r['end']
  vals=[t[3] for t in trades]; n=len(vals); total=sum(vals); mean=total/n if n else 0
  sd=(sum((v-mean)**2 for v in vals)/(n-1))**.5 if n>1 else 0
  # Descriptive cumulative equal-notional P&L, ordered by exit; no portfolio compounding.
  equity=peak=dd=0
  for t in sorted(trades,key=lambda x:x[1]): equity+=t[3]; peak=max(peak,equity); dd=max(dd,peak-equity)
  by_symbol={}
  for t in trades:
   z=by_symbol.setdefault(t[2],dict(n=0,usd_at_100=0)); z['n']+=1; z['usd_at_100']+=t[3]
  return dict(by_symbol=by_symbol,matched_original_usd_at_100=sum(t[5] for t in trades),n=n,mean_pct=mean,sum_pct=total,usd_at_100=total,win_pct=100*sum(v>0 for v in vals)/n if n else 0,drawdown_usd_at_100=dd,opposite_proxy=sum(not t[4] for t in trades),naive_se=sd/n**.5 if n else 0)
 rules=[]
 for h in sorted(set(r['h'] for r in rows)):
  for f in ['score']+NAMES:
   values=sorted(abs(r[f]) for r in parts[0] if r['h']==h and r.get(f) is not None and math.isfinite(r[f]))
   if not values: continue
   for q in [0,.5,.8,.95]:
    threshold=values[min(len(values)-1,int(q*len(values)))]
    for polarity in [-1,1]:
     for gate in [0,.1,.25]: rules.append((h,f,threshold,polarity,gate))
 results=[]
 for rule in rules:
  train=evaluate(parts[0],rule)
  if train['n']>=30: results.append(dict(rule=rule,train=train))
 # Freeze shortlist using training alone, select one using validation alone; final test only once.
 shortlist=sorted(results,key=lambda x:x['train']['mean_pct']-x['train']['naive_se'],reverse=True)[:20]
 for x in shortlist: x['validation']=evaluate(parts[1],x['rule'])
 eligible=[x for x in shortlist if x['validation']['n']>=15]
 selected=max(eligible,key=lambda x:x['validation']['mean_pct']-x['validation']['naive_se']) if eligible else None
 if selected: selected=dict(selected,test=evaluate(parts[2],selected['rule']))
 output=dict(meta=meta,candidate_count=len(rules),eligible_train=len(results),selected=selected,shortlist=shortlist,baselines={str(h):[evaluate(p,(h,'baseline',0,1,0)) for p in parts] for h in sorted(set(r['h'] for r in rows))},costs=dict(mean=sum(r['cost'] for r in rows)/len(rows),negative=sum(r['cost']<0 for r in rows)))
 (Path(__file__).parent/'results.json').write_text(json.dumps(output,indent=2))
 print(json.dumps({k:v for k,v in output.items() if k not in ['shortlist','baselines']},indent=2))
if __name__=='__main__': main()
