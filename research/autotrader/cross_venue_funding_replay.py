"""Cross-venue fixed-direction funding spread screen: short HL, long Binance perps."""
import json, math, statistics, urllib.request
from datetime import date, timedelta
from pathlib import Path
import funding_cross_section_replay as f

ROOT=Path("/tmp/sbe_ts_perp_funding_study/data/full")
CACHE=Path("/tmp/sbe_ts_funding_cross_section")
COINS=("BTC","ETH","SOL")
START=date(2024,1,1)
END=date(2026,9,12)
DAY=f.DAY_MS
START_MS=int((START-date(1970,1,1)).days*DAY)
END_MS=int((END-date(1970,1,1)).days*DAY)

def hl_prices(coin):
    path=CACHE/f"hl-{coin}-1d-through-2026-09-13.json"
    if path.exists(): rows=json.loads(path.read_text())
    else:
        req={"type":"candleSnapshot","req":{"coin":coin,"interval":"1d","startTime":START_MS-DAY,"endTime":END_MS+DAY}}
        payload=json.dumps(req).encode()
        with urllib.request.urlopen(urllib.request.Request("https://api.hyperliquid.xyz/info",data=payload,headers={"Content-Type":"application/json"}),timeout=30) as response:
            rows=json.load(response)
        path.write_text(json.dumps(rows))
    return {int(r["t"])//DAY:float(r["o"]) for r in rows if float(r["v"])>0}

def funding(path, timekey, ratekey):
    total={}; counts={}
    for line in path.open():
        r=json.loads(line)
        t=int(r[timekey]); day=(t-300000)//DAY if t%DAY<300000 else t//DAY
        total[day]=total.get(day,0.0)+float(r[ratekey])
        counts[day]=counts.get(day,0)+1
    return total,counts

def load():
    bprices={}; hprices={}; bf={}; hf={}; bc={}; hc={}
    for coin in COINS:
        symbol=coin+"USDT"
        bprices[coin]={}
        for y in range(2023,2027):
            for m in range(1,13):
                if (y,m)>(2026,9):continue
                _,_,rows=f.fetch(symbol,y,m,"price")
                for r in rows:bprices[coin][int(r[0])//DAY]=float(r[1])
        hprices[coin]=hl_prices(coin)
        bf[coin],bc[coin]=funding(ROOT/f"binance-funding-{symbol}.jsonl","fundingTime","fundingRate")
        hf[coin],hc[coin]=funding(ROOT/f"funding-{coin}.jsonl","time","fundingRate")
    return bprices,hprices,bf,hf,bc,hc

def replay(bp,hp,bf,hf,bc,hc,cost):
    days=sorted(set.intersection(*(set(bp[s])&set(hp[s])&set(bf[s])&set(hf[s]) for s in COINS)))
    dayset=set(days); units={s:0.0 for s in COINS}
    equity=peak=1.0;dd=0.0;rows=[];trades=0;missing=[]
    for day in days:
        if day+1 not in dayset or day<START_MS//DAY or day>END_MS//DAY:continue
        if any(bc[s].get(day)!=3 or hc[s].get(day)!=24 for s in COINS):
            missing.append(str(date(1970,1,1)+timedelta(days=day)))
        today=date(1970,1,1)+timedelta(days=day); before=equity;fee=0.0
        if not rows or today.day==1:
            target={s:equity*(0.5/len(COINS))/hp[s][day] for s in COINS}
            turnover=sum(abs(target[s]-units[s])*(bp[s][day]+hp[s][day]) for s in COINS)
            fee=turnover*cost; equity-=fee
            units={s:target[s]*equity/before for s in COINS}
            trades+=1
        price=sum(units[s]*(bp[s][day+1]-bp[s][day]-hp[s][day+1]+hp[s][day]) for s in COINS)
        carry=sum(units[s]*(hp[s][day]*hf[s][day]-bp[s][day]*bf[s][day]) for s in COINS)
        equity+=price+carry;peak=max(peak,equity);dd=min(dd,equity/peak-1)
        rows.append(dict(day=today.isoformat(),price=price/before,funding=carry/before,cost=fee/before,net=equity/before-1))
    last=(date.fromisoformat(rows[-1]["day"])-date(1970,1,1)).days+1
    close=cost*sum(abs(units[s])*(bp[s][last]+hp[s][last]) for s in COINS)
    before=equity;equity-=close;dd=min(dd,equity/peak-1)
    rows[-1]["cost"]+=close/before;rows[-1]["net"]=(1+rows[-1]["net"])*(1-close/before)-1
    years={}
    for y in sorted({r["day"][:4] for r in rows}):
        subset=[r for r in rows if r["day"].startswith(y)]
        years[y]={k:100*(math.prod(1+r["net"] for r in subset)-1) if k=="return" else 100*sum(r[k] for r in subset) for k in ("return","price","funding","cost")}
    net=[r["net"] for r in rows]
    return dict(days=len(rows),monthly_rebalances=trades,missing_settlement_days=len(missing),missing_day_examples=missing[:10],
                return_percent=100*(equity-1),max_drawdown_percent=100*dd,sharpe=statistics.mean(net)/statistics.stdev(net)*math.sqrt(365),by_year=years)

if __name__=="__main__":
    bp,hp,bf,hf,bc,hc=load()
    result=dict(assets=COINS,start=START.isoformat(),end=END.isoformat(),source_funding="https://github.com/molinamanlio/perp-funding-study/tree/main/data/full",
                counts={s:dict(binance_prices=len(bp[s]),hl_prices=len(hp[s]),binance_funding_days=len(bf[s]),hl_funding_days=len(hf[s])) for s in COINS})
    for name,cost in (("cost_10bps",.001),("cost_6bps",.0006)):
        result[name]=replay(bp,hp,bf,hf,bc,hc,cost)
    Path(__file__).with_name("cross_venue_funding_results.json").write_text(json.dumps(result,indent=2)+"\n")
    print(json.dumps(result,indent=2))
