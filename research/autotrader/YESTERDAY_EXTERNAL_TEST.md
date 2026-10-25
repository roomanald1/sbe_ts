# Test of external-market features against yesterday's signals

Date analyzed: 2026-10-21 UTC. Analysis is descriptive and exploratory; it is not evidence of a profitable strategy.

## Data available

The local Insights SQLite database has signal records from 19:40:01 to 23:59:59 UTC on 2026-10-21, or about 4 hours 20 minutes of that day. It contains 261 BTC and 261 ETH OKX external-market snapshots. Funding, open interest and perp basis are populated on all snapshots. Options ATM IV and 25-delta skew are null on all 522 snapshots.

Binance has retained 5-minute derivatives history, so I separately fetched its BTCUSDT/ETHUSDT open-interest statistics and basis history for 21 October from its public API and joined those records to the same signal outcomes. Binance returned 289 five-minute points per metric and asset for the full UTC day. The database did not capture these fields locally; the retained history comes from Binance's public [open-interest statistics](https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/rest-api/market-data#open-interest-statistics) and [basis](https://developers.binance.com/en/docs/derivatives/usds-margined-futures/market-data) endpoints. Other candidates still lack same-window inputs here: Deribit cross-venue features, Bybit liquidation events, Hyperliquid data, on-chain ETH/USDT flows, stablecoin supply, CFTC positioning, ETF flows, token unlocks, macro-event probabilities and prediction-market prices. Fetching current values cannot reconstruct what was available yesterday.

## Method

For each BTC-USDT and ETH-USDT signal outcome, joined the most recent OKX external-market sample available at or before the signal timestamp, allowing a maximum 90-second staleness. To prevent repeating a one-minute external observation against dozens of per-second outcomes, selected the first exact-pair outcome within 15 seconds after each external observation. This yielded 240–260 paired observations per asset and horizon. Horizons are 60, 300, 900 and 3600 seconds.

Reported correlations are Pearson correlations of each feature with subsequent raw underlying return, not risk-adjusted strategy returns. The 5-minute OI change is computed as percentage change in open-interest currency from the sample five snapshots earlier. “Funding fade” is a deliberately naive gross-only check: short when the recorded funding rate is positive and long when negative; its mean return is before fees, spread, slippage, funding cash flows, or position and turnover constraints.

## Results

| Pair | Horizon | N | Funding vs return r | Basis vs return r | 5m OI change vs return r | Naive funding-fade mean gross return |
|---|---:|---:|---:|---:|---:|---:|
| BTC-USDT | 60s | 260 | +0.093 | -0.035 | -0.046 | -0.0021% |
| BTC-USDT | 300s | 256 | +0.229 | -0.132 | -0.150 | -0.0093% |
| BTC-USDT | 900s | 246 | +0.394 | -0.277 | -0.131 | -0.0211% |
| BTC-USDT | 3600s | 241 | +0.642 | -0.254 | -0.073 | -0.1194% |
| ETH-USDT | 60s | 259 | -0.054 | -0.079 | -0.037 | +0.0042% |
| ETH-USDT | 300s | 255 | -0.157 | -0.129 | -0.183 | +0.0148% |
| ETH-USDT | 900s | 245 | -0.170 | -0.011 | -0.216 | +0.0381% |
| ETH-USDT | 3600s | 240 | -0.242 | -0.093 | -0.002 | +0.1616% |

All raw mean forward returns were negative in this short captured interval: BTC -0.0020%, -0.0089%, -0.0233% and -0.1027% at the respective horizons; ETH -0.0042%, -0.0148%, -0.0381% and -0.1616%. The apparent ETH funding-fade gain is therefore consistent with simply being short during a falling window. The same rule lost on BTC. Basis-fade (long when basis is negative, as it was on average for both) would have lost gross. These outcomes do not establish that any input adds an edge over a short-only or market-regime baseline. Even ETH's 1-hour mean funding-fade return of 16.2 bps gross is below the roughly 21 bps median execution-cost estimate in [REPORT.md](./REPORT.md); the shorter-horizon means are much smaller.

## Binance retained-history check

At the Binance 5-minute timestamps during the local signal window, joined Binance open-interest value and basis with one exact-pair signal outcome per horizon. Binance basis was converted to basis points as price basis divided by index price, multiplied by 10,000. Cross-venue basis spread is Binance basis bps minus the contemporaneous stored OKX basis bps. OI change is Binance USD open-interest value percent change versus the prior 5-minute observation. These are Pearson correlations with subsequent raw return, not net P&L.

| Pair | Horizon | N | Binance basis r | Binance-minus-OKX basis r | Binance 5m OI change r |
|---|---:|---:|---:|---:|---:|
| BTC-USDT | 60s | 51 | -0.012 | -0.089 | -0.048 |
| BTC-USDT | 300s | 50 | -0.192 | -0.121 | -0.105 |
| BTC-USDT | 900s | 48 | -0.150 | +0.006 | -0.056 |
| BTC-USDT | 3600s | 48 | +0.015 | +0.174 | -0.055 |
| ETH-USDT | 60s | 51 | -0.165 | -0.096 | -0.187 |
| ETH-USDT | 300s | 50 | -0.317 | -0.210 | -0.191 |
| ETH-USDT | 900s | 48 | -0.076 | -0.035 | -0.230 |
| ETH-USDT | 3600s | 48 | +0.069 | +0.166 | -0.179 |

The sign and size shift across horizons and assets; these results do not establish stable predictive value. The 5-minute-bin sample is still one late-day episode with overlapping 15- and 60-minute outcomes.

## Limits and conclusion

The sample starts late in the UTC day, spans only one market episode, and has strongly overlapping forward returns. The 3600-second observations, in particular, are not 240 independent experiments; adjacent outcomes share most of the same return window. Feature values also have serial dependence, and the stored external timestamp is local receive time after the API response, not the exchange's source-event time. Correlations are not significance tests and can reflect the common price trend. There is no robust net-P&L claim from these numbers.

These checks provide no evidence to promote funding, OI or basis into the live signal. They justify preserving and timestamping these fields for a longer forward shadow test with regime controls, non-overlapping evaluation blocks, execution costs, and a comparison against simple long-only/short-only baselines. For the other candidates, historical inputs must be collected or licensed with reliable point-in-time timestamps before a yesterday-style test is possible. Binance history was pulled retrospectively from the exchange archive, so its availability now does not by itself establish the feed would have been available with the same latency to the live strategy.
