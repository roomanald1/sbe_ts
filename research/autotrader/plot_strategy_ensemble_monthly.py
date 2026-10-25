#!/usr/bin/env python3
"""Export monthly net returns and a comparison chart for the trend mix."""
import csv,json
from datetime import datetime,timezone
from pathlib import Path
ROOT=Path(__file__).resolve().parent;DATA=ROOT/'data/binance_spot_1d_api_2019_2026'

# Recreate the strategy daily returns from raw retained klines.
from spot_strategy_ensemble import RULES,portfolio

def main():
 data={}
 for s,_ in RULES:
  data[s]={datetime.fromtimestamp(r[0]/1000,timezone.utc).strftime('%Y-%m-%d'):{'o':float(r[1]),'c':float(r[4])} for r in json.loads((DATA/f'{s}-1d.json').read_text())}
 days=sorted(set.intersection(*(set(x) for x in data.values())))
 mix,_=portfolio(days,data,False);gated,_=portfolio(days,data,True)
 monthly={}
 for (day,r),(day2,r2) in zip(mix,gated):
  month=day[:7];monthly.setdefault(month,{'mix':1.,'gated':1.,'days':0})
  monthly[month]['mix']*=1+r;monthly[month]['gated']*=1+r2;monthly[month]['days']+=1
 out=ROOT/'spot_strategy_ensemble_monthly.csv'
 with out.open('w',newline='') as f:
  w=csv.writer(f);w.writerow(['month','mix_net_return_pct','btc_gated_net_return_pct','daily_intervals','complete_month'])
  for m,v in monthly.items():
   w.writerow([m,round((v['mix']-1)*100,6),round((v['gated']-1)*100,6),v['days'],str(v['days'] in (28,29,30,31)).lower()])
 months=list(monthly);mx=[(monthly[m]['mix']-1)*100 for m in months];gt=[(monthly[m]['gated']-1)*100 for m in months]
 # Draw an SVG directly so the export has no plotting-library dependency.
 width_px,height_px=1800,760;left,right,top,bottom=76,28,74,92
 pw=width_px-left-right;ph=height_px-top-bottom
 vals=mx+gt;lim=max(10.,max(abs(x) for x in vals));step=25 if lim<=125 else 50 if lim<=250 else 100
 lim=((int(lim/step)+1)*step);ymin,ymax=-lim,lim
 def y(v):return top+(ymax-v)/(ymax-ymin)*ph
 spacing=pw/len(months);barw=min(7.,spacing*.36)
 lines=[]
 lines.append(f'<svg xmlns="http://www.w3.org/2000/svg" width="{width_px}" height="{height_px}" viewBox="0 0 {width_px} {height_px}">')
 lines.append('<rect width="100%" height="100%" fill="white"/>')
 lines.append(f'<text x="{width_px/2}" y="35" text-anchor="middle" font-family="Arial" font-size="25" font-weight="bold">Monthly returns: BTC / ETH / BNB trend mix</text>')
 for val in range(-lim,lim+1,step):
  yy=y(val);lines.append(f'<line x1="{left}" y1="{yy:.1f}" x2="{width_px-right}" y2="{yy:.1f}" stroke="#dddddd"/>')
  lines.append(f'<text x="{left-10}" y="{yy+5:.1f}" text-anchor="end" font-family="Arial" font-size="13" fill="#444">{val}%</text>')
 zero=y(0);lines.append(f'<line x1="{left}" y1="{zero:.1f}" x2="{width_px-right}" y2="{zero:.1f}" stroke="#333" stroke-width="1.4"/>')
 for i,m in enumerate(months):
  center=left+(i+.5)*spacing
  for off,val,color in [(-barw/2,mx[i],'#18845f' if mx[i]>=0 else '#b64747'),(barw/2,gt[i],'#72bda0' if gt[i]>=0 else '#d99393')]:
   yy=y(val);height=abs(zero-yy);topbar=min(zero,yy)
   lines.append(f'<rect x="{center+off-barw/2:.2f}" y="{topbar:.2f}" width="{barw:.2f}" height="{max(height,.6):.2f}" fill="{color}"/>')
  if m.endswith('-01') or i==0:
   lines.append(f'<text x="{center:.1f}" y="{height_px-bottom+25}" text-anchor="middle" font-family="Arial" font-size="13" fill="#444">{m[:4]}</text>')
 lines.append(f'<text x="{left}" y="{height_px-45}" font-family="Arial" font-size="14" fill="#333">Monthly net return (%)</text>')
 # Legend.
 lx=width_px-390;ly=55
 for j,(label,color) in enumerate([('Trend mix','#18845f'),('BTC risk-off gated','#72bda0')]):
  xx=lx+j*190;lines.append(f'<rect x="{xx}" y="{ly-15}" width="16" height="16" fill="{color}"/><text x="{xx+23}" y="{ly-2}" font-family="Arial" font-size="14" fill="#333">{label}</text>')
 partial=months[-1] if monthly[months[-1]]['days'] not in (28,29,30,31) else None
 note='15 bps per side; prior-close signal, next-open execution; open-to-open returns.'
 if partial:note+=f' Final month {partial} is partial ({monthly[partial]["days"]} intervals).'
 lines.append(f'<text x="{left}" y="{height_px-15}" font-family="Arial" font-size="12" fill="#555">{note}</text>')
 lines.append('</svg>')
 svg=ROOT/'spot_strategy_ensemble_monthly.svg';svg.write_text('\n'.join(lines))
 print(svg);print(out);print('months',len(months),'last',months[-1],monthly[months[-1]]['days'],'intervals')
if __name__=='__main__':main()
