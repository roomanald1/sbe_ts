# Daily breakout and trend candidates

Prepared 30 October 2026. This is a reproducible research screen; neither candidate was enabled in AutoTrader.

## Result

I independently replayed a 20-day breakout with a 200-day BTC regime gate at two position sizes, with and without a three-session minimum hold, and a separate 16-coin trend portfolio. The backtest uses a hard 100% gross exposure cap, next-open entries, open-to-next-open returns, and 15 bp per side. These explicit cash-account constraints differ from a simulation that assigns 25% to every active asset without limiting aggregate exposure.

| Rule | 2018-19 | 2020-22 | 2023-24 | 2025-26 to Oct 29 |
|---|---:|---:|---:|---:|
| Existing-size reference: 1/7 per coin, no hold minimum | +13.2% | +208.8% | +44.9% | +21.7% |
| A: 25% maximum per coin, no hold minimum | +17.1% | +382.5% | +52.4% | +11.0% |
| **A2: A plus three-session minimum hold** | **+26.7%** | **+2298.3%** | **+110.9%** | **-0.1%** |
| B: diversified trend score, volatility weighted | -24.0% | +1439.0% | +164.6% | -22.8% |

A2's very large 2020-22 result comes from compounding during that period. It does not persist into 2025-26, where the result is flat at 15 bp per side and -9.1% at 30 bp per side. Its period maximum drawdown at 15 bp was -21.0% in 2025-26 and -30.2% in 2020-22. The larger position size increased exposure and drawdown in the earlier periods; the 3-day hold materially changes results, so it should be treated as a parameter selected after looking at history.

B is not a robust candidate in this implementation: it lost in 2018-19 and 2025-26 and its maximum drawdown was about 47-51% in each period. I would not add either rule to the active paper strategy catalog on this evidence. A2 could be reconsidered as a separately monitored paper experiment only with a capped cash-account implementation and frozen forward rules; B needs a materially different risk design before reconsideration.

## Exact rules and portfolio mechanics

- **Data and universe:** Binance Spot public daily klines for BTC, ETH, BNB, XRP, ADA, SOL, DOGE, LTC, TRX, LINK, BCH, XLM, ETC, FIL, AVAX, and DOT quoted in USDT. The universe is fixed to coins selected today, so it has survivorship bias. Data spans 17 August 2017 through 29 October 2026; each coin only contributes after its Binance listing and required lookback.
- **Signal timing:** indicators use the prior completed UTC daily candle. Target weights take effect at the current UTC open. P&L runs from that open to the next UTC open. Cash earns zero.
- **Breakout:** close above the preceding 20 completed daily highs and BTC close above its trailing 200-close SMA. A2 holds each entry for at least three open-to-open sessions, even if the signal turns off during that minimum.
- **Sizing:** the reference uses at most 1/7 per asset. A/A2 use at most 25% per asset. Active positions are capped at 100% aggregate exposure; unused cash stays idle. When more than four names signal, all are scaled equally so the total stays at 100%.
- **B trend score:** average of each asset's 10, 20, 40, 80 and 160-session close-to-close returns. Only positive scores are included while BTC is above its SMA200. Raw weights are score divided by 30-session realized volatility, normalized across eligible names and capped at 25% per name, with iterative redistribution. Cash remains idle if no name qualifies.
- **Costs:** fee and assumed slippage are represented together as 15 or 30 bp per side on absolute weight turnover. This is not an account-specific OKX fee or a fill simulation. No lending, leverage, interest, funding, spread, market impact or partial-fill effects are modeled.

Returns in each column are compounded from a fresh 1.0 starting portfolio at the start of that period. Max drawdown and average exposure are also measured inside each period. Results use the same 16 present-day assets across periods, but older periods naturally contain only the coins already listed then. That is not point-in-time universe selection, and 2025-26 has already been explored in earlier repository work, so it is not an untouched holdout.

## Reproduction

Run from the repository root:

```sh
python3 research/autotrader/daily_candidate_screen.py
```

The script downloads missing candles from Binance's public Spot REST endpoint into `research/autotrader/data/binance_spot_candidates_16_daily/`, reuses that local cache on later runs, and writes [the detailed result JSON](./daily_candidate_screen_results.json). The downloaded candles are included with [SHA256 checksums](./data/binance_spot_candidates_16_daily/SHA256SUMS); verify them from the repository root with `sha256sum -c research/autotrader/data/binance_spot_candidates_16_daily/SHA256SUMS`. Delete the cache to fetch current public data again. Because the endpoint can revise historical candles and is not versioned, preserve the downloaded cache and its checksums if an exact rerun is needed.

The estimates above are exploratory only. Binance OHLC does not reproduce OKX order-book execution, and present-day coin selection excludes delisted assets. No result here is a production or profitability claim.
