"""Screen fixed 60-day-to-expiry BTC/ETH quarterly basis from Binance archives."""
import calendar,csv,io,json,urllib.request,zipfile
from datetime import date,timedelta
from pathlib import Path
from funding_cross_section_replay import CACHE,DAY_MS

ASSETS=("BTC","ETH")
BASE="https://data.binance.vision/data"
ROUND_TRIP_COST_PERCENT=0.50

def expiry(year,month):
    last=calendar.monthrange(year,month)[1]
    day=date(year,month,last)
    while day.weekday()!=4:day-=timedelta(days=1)
    return day

def open_price(symbol,day,kind):
    stamp=day.strftime("%Y-%m")
    if kind=="spot":
        name=f"spot-{symbol}-1d-{stamp}.zip"
        url=f"{BASE}/spot/monthly/klines/{symbol}/1d/{symbol}-1d-{stamp}.zip"
    else:
        name=f"{symbol}-1d-{stamp}.zip"
        url=f"{BASE}/futures/um/monthly/klines/{symbol}/1d/{symbol}-1d-{stamp}.zip"
    path=CACHE/name
    if path.exists():raw=path.read_bytes()
    else:
        with urllib.request.urlopen(url,timeout=30) as r:raw=r.read()
        CACHE.mkdir(parents=True,exist_ok=True);path.write_bytes(raw)
    with zipfile.ZipFile(io.BytesIO(raw)) as z:
        with z.open(z.namelist()[0]) as stream:
            rows=list(csv.reader(io.TextIOWrapper(stream,encoding="utf-8")))
    for row in rows:
        if not row[0].isdigit():continue
        stamp=int(row[0]);actual=date(1970,1,1)+timedelta(days=stamp//(DAY_MS*(1000 if stamp>10**14 else 1)))
        if actual==day:return float(row[1])
    raise ValueError(f"missing {kind} {symbol} {day}")

rows=[]
for year in (2024,2025,2026):
    for month in (3,6,9,12):
        end=expiry(year,month)
        if end>date(2026,10,27):continue
        entry=end-timedelta(days=60)
        for asset in ASSETS:
            spot_symbol=asset+"USDT"
            dated_symbol=spot_symbol+f"_{year%100:02d}{month:02d}{end.day:02d}"
            try:
                spot=open_price(spot_symbol,entry,"spot")
                future=open_price(dated_symbol,entry,"future")
            except Exception as exc:
                rows.append(dict(asset=asset,expiry=end.isoformat(),entry=entry.isoformat(),error=str(exc)))
                continue
            basis=100*(future/spot-1)
            rows.append(dict(asset=asset,expiry=end.isoformat(),entry=entry.isoformat(),spot_open=spot,future_open=future,
                             basis_percent=basis,annualized_basis_percent=basis*365/60,
                             conservative_net_percent=basis-ROUND_TRIP_COST_PERCENT,
                             eligible=basis>ROUND_TRIP_COST_PERCENT))

def settlement_prices(asset):
    url=f"https://fapi.binance.com/futures/data/delivery-price?pair={asset}USDT"
    with urllib.request.urlopen(url,timeout=20) as response:values=json.load(response)
    return {date(1970,1,1)+timedelta(days=int(v["deliveryTime"])//DAY_MS):float(v["deliveryPrice"]) for v in values}

def spot_exit_open(asset,expiry_day):
    symbol=asset+"USDT";stamp=expiry_day.strftime("%Y-%m")
    name=f"spot-{symbol}-1h-{stamp}.zip";path=CACHE/name
    if path.exists():raw=path.read_bytes()
    else:
        url=f"{BASE}/spot/monthly/klines/{symbol}/1h/{symbol}-1h-{stamp}.zip"
        with urllib.request.urlopen(url,timeout=30) as response:raw=response.read()
        CACHE.mkdir(parents=True,exist_ok=True);path.write_bytes(raw)
    target=(expiry_day-date(1970,1,1)).days*DAY_MS+8*3600000
    with zipfile.ZipFile(io.BytesIO(raw)) as z:
        with z.open(z.namelist()[0]) as stream:
            for row in csv.reader(io.TextIOWrapper(stream,encoding="utf-8")):
                if row[0].isdigit():
                    t=int(row[0]);t=t//1000 if t>10**14 else t
                    if t==target:return float(row[1])
    raise ValueError(f"missing exit {symbol} {expiry_day}")

settle={asset:settlement_prices(asset) for asset in ASSETS}
for row in rows:
    if "error" in row:continue
    end=date.fromisoformat(row["expiry"])
    if end not in settle[row["asset"]]:
        row["realized_error"]="delivery price unavailable"
        continue
    try:exit_spot=spot_exit_open(row["asset"],end)
    except Exception as exc:
        row["realized_error"]=str(exc)
        continue
    s0=row["spot_open"];f0=row["future_open"];p=settle[row["asset"]][end];s1=exit_spot
    gross=f0-s0+s1-p
    fees=.001*s0+.0005*f0+.0005*p+.001*s1
    slippage=.0005*(s0+f0+p+s1)
    row["settlement_price"]=p;row["spot_exit_08_open"]=s1
    row["realized_gross_percent"]=100*gross/s0
    row["realized_net_percent"]=100*(gross-fees-slippage)/s0


def spot_exit_twap(asset,expiry_day):
    symbol=asset+"USDT";stamp=expiry_day.strftime("%Y-%m-%d")
    path=CACHE/f"spot-{symbol}-1m-{stamp}.zip"
    if path.exists():raw=path.read_bytes()
    else:
        url=f"{BASE}/spot/daily/klines/{symbol}/1m/{symbol}-1m-{stamp}.zip"
        with urllib.request.urlopen(url,timeout=30) as response:raw=response.read()
        CACHE.mkdir(parents=True,exist_ok=True);path.write_bytes(raw)
    start=(expiry_day-date(1970,1,1)).days*DAY_MS+7*3600000+30*60000
    end=start+30*60000
    prices=[]
    with zipfile.ZipFile(io.BytesIO(raw)) as z:
        with z.open(z.namelist()[0]) as stream:
            for row in csv.reader(io.TextIOWrapper(stream,encoding="utf-8")):
                if row[0].isdigit():
                    t=int(row[0]);t=t//1000 if t>10**14 else t
                    if start<=t<end:prices.append(float(row[1]))
    if len(prices)!=30:raise ValueError(f"missing TWAP bars {symbol} {expiry_day}: {len(prices)}")
    return sum(prices)/30

for row in rows:
    if "settlement_price" not in row:continue
    end=date.fromisoformat(row["expiry"])
    try:s1=spot_exit_twap(row["asset"],end)
    except Exception as exc:
        row["twap_error"]=str(exc);continue
    s0=row["spot_open"];f0=row["future_open"];p=row["settlement_price"]
    gross=f0-s0+s1-p
    fees=.001*s0+.0005*f0+.0005*p+.001*s1
    slippage=.0005*(s0+f0+p+s1)
    row["spot_exit_30min_twap"]=s1
    row["twap_realized_gross_percent"]=100*gross/s0
    row["twap_realized_net_percent"]=100*(gross-fees-slippage)/s0

output=dict(source=BASE,rule="Enter 60 calendar days before quarterly expiry if basis exceeds all modeled costs; hold to cash settlement",
            cost_percent_of_spot_notional=ROUND_TRIP_COST_PERCENT,rows=rows)
Path(__file__).with_name("dated_basis_entry_results.json").write_text(json.dumps(output,indent=2)+"\n")
print(json.dumps(output,indent=2))
