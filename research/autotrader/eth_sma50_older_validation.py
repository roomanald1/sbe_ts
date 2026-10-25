#!/usr/bin/env python3
"""Validate the ETH 50-day SMA rule on Binance history before 2019."""
import json, math
from datetime import datetime, timezone
from pathlib import Path
ROOT=Path(__file__).resolve().parent
FILES=[ROOT/'data/binance_spot_1d_api_2017_2018/ETHUSDT-1d.json',ROOT/'data/binance_spot_1d_api_2019_2026/ETHUSDT-1d.json']
COST=0.0015

def main():
    prices={}
    for path in FILES:
        for row in json.loads(path.read_text()):
            day=datetime.fromtimestamp(row[0]/1000,timezone.utc).strftime('%Y-%m-%d')
            prices[day]=(float(row[1]),float(row[4]))
    days=sorted(prices); returns=[]; previous_exposure=0
    for i in range(50,len(days)-1):
        day=days[i]
        signal=int(prices[days[i-1]][1] > sum(prices[days[k]][1] for k in range(i-50,i))/50)
        change=abs(signal-previous_exposure)
        net=(prices[days[i+1]][0]/prices[day][0]-1)*signal-change*COST
        returns.append((day,net,signal)); previous_exposure=signal
    for name,lo,hi in [('2017 post-warmup','2017-01-01','2018-01-01'),('2018','2018-01-01','2019-01-01'),('2017-18','2017-01-01','2019-01-01')]:
        sample=[r for r in returns if lo<=r[0]<hi]
        wealth=peak=1.; drawdown=0.
        for _,r,_ in sample:
            wealth*=1+r; peak=max(peak,wealth); drawdown=min(drawdown,wealth/peak-1)
        first,last=sample[0][0],sample[-1][0]
        # Match buy-and-hold to the exact same open-to-next-open interval.
        end_index=days.index(last)+1
        benchmark=prices[days[end_index]][0]/prices[first][0]*(1-COST)**2-1
        exposure=sum(x[2] for x in sample)/len(sample)
        print(f'{name}: {first} to {last}; n={len(sample)}; strategy={100*(wealth-1):.2f}%; buy_hold={100*benchmark:.2f}%; max_dd={100*drawdown:.2f}%; exposure={100*exposure:.1f}%')
if __name__=='__main__':main()
