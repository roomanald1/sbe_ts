"""Spot versus perpetual carry replay on public Binance daily opens."""
import csv, io, json, math, statistics, urllib.request, zipfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import date, timedelta
from pathlib import Path
import funding_cross_section_replay as f

f.SYMBOLS = ("BTCUSDT", "ETHUSDT", "SOLUSDT")
f.MONTHS = [(y,m) for y in range(2023,2027) for m in range(1,13) if (y,m)<=(2026,9)]
S = f.SYMBOLS
BASE = "https://data.binance.vision/data/spot/monthly/klines"

def get_spot(s,y,m):
    stamp=f"{y}-{m:02d}"
    path=f.CACHE / f"spot-{s}-1d-{stamp}.zip"
    if path.exists(): raw=path.read_bytes()
    else:
        with urllib.request.urlopen(f"{BASE}/{s}/1d/{s}-1d-{stamp}.zip",timeout=30) as r: raw=r.read()
        f.CACHE.mkdir(parents=True,exist_ok=True); path.write_bytes(raw)
    with zipfile.ZipFile(io.BytesIO(raw)) as z:
        with z.open(z.namelist()[0]) as stream:
            rows=list(csv.reader(io.TextIOWrapper(stream,encoding="utf-8")))
    if rows and not rows[0][0].isdigit(): rows.pop(0)
    out={}
    for row in rows:
        stamp=int(row[0]); out[stamp//(f.DAY_MS*(1000 if stamp>10**14 else 1))]=float(row[1])
    return s,out

def load():
    perp,funding,errors=f.load_panel()
    spot={s:{} for s in S}
    with ThreadPoolExecutor(max_workers=12) as pool:
        jobs={pool.submit(get_spot,s,y,m):(s,y,m) for s in S for y,m in f.MONTHS}
        for future in as_completed(jobs):
            try: s,part=future.result(); spot[s].update(part)
            except Exception as e: errors.append(str((jobs[future],e)))
    return spot,perp,funding,errors

def replay(spot,perp,funding,cost,filtered):
    days=sorted(set.intersection(*(set(spot[s])&set(perp[s])&set(funding[s]) for s in S)))
    dayset=set(days)
    units={s:0 for s in S}
    equity=peak=1.0; dd=0.0; rows=[]; trade_days=0
    for day in days:
        if day+1 not in dayset or day<days[0]+7: continue
        today=date(1970,1,1)+timedelta(days=day)
        before=equity; fee=0.0
        if (not rows) or (filtered and today.weekday()==0) or ((not filtered) and today.day==1):
            target={}
            for s in S:
                history=[funding[s].get(p) for p in range(day-7,day)]
                active=not filtered or (None not in history and sum(history)/7>(0.0002 if units[s] else 0.0007))
                target[s]=(0.5/len(S))*equity/spot[s][day] if active else 0.0
            turnover=sum(abs(target[s]-units[s])*(spot[s][day]+perp[s][day]) for s in S)
            fee=turnover*cost; equity-=fee
            units={s:target[s]*equity/before for s in S}
            if turnover: trade_days+=1
        price=sum(units[s]*(spot[s][day+1]-spot[s][day]-perp[s][day+1]+perp[s][day]) for s in S)
        cash=sum(units[s]*perp[s][day]*funding[s][day] for s in S)
        equity+=price+cash; peak=max(peak,equity); dd=min(dd,equity/peak-1)
        rows.append(dict(day=today.isoformat(),price=price/before,funding=cash/before,cost=fee/before,net=equity/before-1))
    close=cost*sum(abs(units[s])*(spot[s][days[-1]]+perp[s][days[-1]]) for s in S)
    before=equity; equity-=close; dd=min(dd,equity/peak-1)
    rows[-1]["cost"]+=close/before; rows[-1]["net"]=(1+rows[-1]["net"])*(1-close/before)-1
    years={}
    for year in sorted({r["day"][:4] for r in rows}):
        subset=[r for r in rows if r["day"].startswith(year)]
        years[year]={k:100*(math.prod(1+r["net"] for r in subset)-1) if k=="return" else 100*sum(r[k] for r in subset) for k in ("return","price","funding","cost")}
    net=[r["net"] for r in rows]
    return dict(days=len(rows),trade_days=trade_days,return_percent=100*(equity-1),max_drawdown_percent=100*dd,
                sharpe=statistics.mean(net)/statistics.stdev(net)*math.sqrt(365),by_year=years)

if __name__=="__main__":
    spot,perp,funding,errors=load()
    result=dict(symbols=S,errors=errors,counts={s:{"spot":len(spot[s]),"perp":len(perp[s]),"funding":len(funding[s])} for s in S})
    for name,filtered,cost in (("monthly_10bps",False,.001),("filtered_10bps",True,.001),("monthly_6bps",False,.0006),("filtered_6bps",True,.0006)):
        result[name]=replay(spot,perp,funding,cost,filtered)
    Path(__file__).with_name("spot_perp_carry_results.json").write_text(json.dumps(result,indent=2)+"\n")
    print(json.dumps(result,indent=2))
