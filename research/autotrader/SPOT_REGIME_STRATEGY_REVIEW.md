# Spot regime strategy comparison

## Test design

This study compares five additional long-only strategy families against equal-weight buy-and-hold on the same seven-symbol Binance spot daily-candle universe: BTC, ETH, BNB, XRP, SOL, ADA and DOGE. The full daily period available across all symbols begins in 2019 and ends 22 October 2026. Signals are computed from the prior close, trades rebalance at the next open, and each day's return is open-to-next-open. Costs are 15 bps per side on traded portfolio weight (10 bps assumed fee plus 5 bps slippage). Cash earns zero. No leverage is used.

The fixed holdout is 2025 through 22 October 2026. Rules were specified before evaluating the holdout; no optimization was performed on it. The existing BTC/ETH/BNB trend mix had a different construction and universe, so it is not presented as a directly comparable benchmark here.

## Results

Compounded account returns after modeled costs; drawdown is peak-to-trough within each segment.

| Strategy | Train 2019–22 | Validation 2023–24 | Holdout 2025–Oct 22 2026 | Full-period max drawdown |
|---|---:|---:|---:|---:|
| Equal-weight buy and hold | +0.8% | +514.0% | -39.8% | -79.4% |
| 50/200-day trend filter | +182.7% | +151.7% | -16.6% | -36.5% |
| 55-day high breakout | +14.9% | +44.3% | +9.9% | -15.4% |
| 20-day high breakout + 200-day trend filter | +19.9% | +45.3% | +3.6% | -18.0% |
| RSI(2) oversold rebound, above 200-day average, 5-day max hold | -3.6% | +11.9% | -14.7% | -36.0% |
| 90-day relative-strength rotation, above 200-day average | +242.7% | +571.1% | -5.9% | -84.4% |

Sharpe ratios and segment-level drawdowns are in [spot_regime_strategy_results.json](./spot_regime_strategy_results.json).

## When each may fit

- **55-day breakout:** candidate for sustained directional markets. It was the only tested add-on positive in all three periods and had the smallest observed full-period drawdown here, but its returns were modest and it still depends on a small, survivorship-biased coin universe.
- **20-day breakout plus 200-day trend filter:** a more selective breakout candidate for rising markets; weaker than the 55-day rule in the holdout.
- **50/200-day trend:** may participate in broad uptrends and reduce exposure during prolonged declines. It lost money in the holdout, so the historical validation gains did not persist there.
- **Relative-strength rotation:** can concentrate in the latest leader during strong altcoin cycles. It produced the highest historical return, alongside an 84% drawdown and negative holdout return; the backtest gives no evidence it is a robust standalone allocation.
- **RSI(2) rebound:** intended for short-lived oversold bounces in a longer-term uptrend. It lost money in training and holdout; this version is not supported by the results.
- **Buy and hold:** simplest exposure in broad bull markets, but the holdout and drawdown show the large downside risk of staying fully invested.

## Limits and next step

These are daily-bar simulations, not executable strategy proofs. The test does not model intraday path, stop order, queue position, partial fills, market impact, taxes or custody. The coin list includes assets selected for present-day relevance, creating survivorship and availability bias. October 2026 is partial. Returns and Sharpe estimates are sensitive to the chosen historical window and assumed cost. Treat the breakout result as a candidate for wider-universe, walk-forward and higher-resolution validation, not as a profitability claim.

## Breakout sensitivity check

I also varied the Donchian lookback and increased modeled cost while keeping the holdout fixed. Holdout net returns were:

| Breakout lookback | 15 bps/side | 30 bps/side | 50 bps/side |
|---|---:|---:|---:|
| 20 days | +18.7% | +12.2% | +4.2% |
| 40 days | +14.7% | +10.7% | +5.5% |
| 55 days | +9.9% | +6.7% | +2.6% |
| 90 days | +1.3% | -0.8% | -3.5% |

The result is not isolated to exactly 55 days: 20–55 day breakouts stayed positive in this holdout even at 50 bps per side. The 90-day rule did not. Since comparing variants after seeing holdout results introduces selection bias, these numbers are robustness diagnostics, not a new strategy selection. A stronger next test is a rolling walk-forward with a wider, historically point-in-time coin universe and delisted assets included.

## Point-in-time universe walk-forward

I tested weekly 12-week relative-strength momentum using point-in-time CoinMarketCap ranking snapshots served by CryptWatch. Each rebalance selects only from the assets ranked in that week's top 100, excludes stablecoins, and holds the top 1, 5 or 10 names equally for one week. The fetched history has 403 Sunday snapshots from 6 January 2019 through 18 October 2026; prices for exits are taken from the next snapshot's top 500. The source says its snapshots are weekly before 18 December 2021 and more frequent afterward. The strategy was not tuned on the holdout.

| Equal-weight holdings | Train 2019–22 | Validation 2023–24 | Holdout 2025–Oct 18 2026 | Full-period max drawdown |
|---|---:|---:|---:|---:|
| Top 1 | -96.4% | +48.4% | -99.9% | -100.0% |
| Top 5 | +407.5% | +113.8% | -49.5% | -86.1% |
| Top 10 | +48.9% | +124.5% | -64.2% | -91.0% |

The point-in-time universe did not rescue relative-strength rotation: all three variants lost heavily in the holdout, with severe historical drawdowns. Top 5 had large prior gains but still lost about half its value during the holdout. This is stronger evidence against treating the earlier current-universe rotation result as a dependable strategy.

The source data are rankings and snapshot prices, not executable exchange prices. Missing next-snapshot marks are conservatively valued at zero; that occurred six times for top 10 and zero times for top 1/top 5. It can overstate loss if a coin merely fell below rank 500. The source's history cadence changed from weekly to daily; this test samples Sundays to keep the interval weekly. CoinMarketCap data coverage, historical price identity, contract redenominations, liquidity and trading costs remain limitations. See the full per-week trade log and exact metrics in [spot_point_in_time_weekly_results.json](./spot_point_in_time_weekly_results.json), reproducible from [spot_point_in_time_weekly.py](./spot_point_in_time_weekly.py).

## BTC trend gate on the 55-day breakout

I tested a simple market-regime rule on the seven-symbol daily universe: take equal 1/7 account weights in each coin that closes above its prior 55-day high only while BTC is above its trailing 200-day average; otherwise hold cash. The common seven-coin daily sample begins 1 March 2021, so the training period here is 2021–22, not 2019–22. Signals use prior closes and enter at the next open. Costs remain 15 bps per side.

| Version | Train 2021–22 | Validation 2023–24 | Holdout 2025–Oct 22, 2026 | Full sample | Max drawdown |
|---|---:|---:|---:|---:|---:|
| 55-day breakout, ungated | +14.9% | +44.3% | +9.9% | +82.2% | -15.4% |
| Breakout gated by BTC above SMA200 | +16.8% | +42.4% | +11.9% | +86.0% | -14.3% |

The BTC filter reduced exposure in the 2022 bear market (the gated strategy was flat that year) and modestly improved the combined result, but it did not eliminate losses within holding periods or establish a rule that will always work. The test covers only about five and a half years, with one main bear cycle, and the universe is seven assets.

See [spot_regime_gate_results.json](./spot_regime_gate_results.json) and [spot_regime_gate_test.py](./spot_regime_gate_test.py) for the full regime/year breakdown and reproduction.

## Equal-capital portfolio of all tested strategy sleeves

I combined 11 distinct strategy sleeves over the common 7 March 2021–18 October 2026 weekly window, giving each the same initial capital and letting each sleeve compound independently. The sleeves include buy-and-hold, the prior BTC/ETH/BNB trend mix, trend and breakout systems, RSI(2) rebound, monthly relative-strength rotation, and the three point-in-time top-1/5/10 rotation portfolios. Daily Binance-based strategies are compounded into Sunday-to-Sunday returns; point-in-time rotation sleeves use their weekly CoinMarketCap snapshot returns. Modeled costs are those used in each constituent test.

| Portfolio construction | 2021–22 | 2023–24 | 2025–Oct 18, 2026 | Full period | Full-period max drawdown |
|---|---:|---:|---:|---:|---:|
| All 11 sleeves, equal initial capital | +4.1% | +182.1% | -12.7% | +156.4% | -59.3% |
| Core trend/breakout/mix sleeves only (5) | +42.4% | +104.8% | +3.8% | +202.8% | -25.3% |

So the all-strategy portfolio did return a positive amount over the full sample, but it lost during the most recent holdout. Removing the rotation and mean-reversion sleeves improves the historical profile here, but that subset was chosen after seeing prior test results, so its return is especially vulnerable to selection bias. Neither combination establishes a durable or guaranteed edge. Results also combine different price sources and simplified cost models.

Full component and portfolio metrics are in [spot_all_strategy_portfolio_results.json](./spot_all_strategy_portfolio_results.json); reproduce with [spot_all_strategy_portfolio.py](./spot_all_strategy_portfolio.py).
