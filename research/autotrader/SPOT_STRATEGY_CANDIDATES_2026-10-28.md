# Spot strategy candidates for the current AutoTrader

Prepared 28 October 2026. Research only; no strategy was activated or traded for this review.

## Decision

The strongest distinct **paper candidate** found is a long-only **20-day breakout with a BTC 200-day risk gate and next-day signal exit**. It combines indicator components already used separately by AutoTrader. On seven present-day Binance USDT pairs, 15 bp per side and separate non-rebalanced sleeves, it returned +18.6% in 2021-22, +36.4% in 2023-24, and +28.1% in 2025 through 27 October 2026. The later-period maximum drawdown was -7.8%; the result stayed +20.2% when cost rose to 50 bp per side. It had 84 later-period entries across seven assets.

This is **discovery evidence**, not proof of a deployable profit. The later data have already been inspected in earlier repository research, the universe selects assets that survive today, and Binance signal prices are not OKX fills. The backtest assumes the strategy exits when its daily signal becomes inactive. A matched daily-OHLC bracket screen showed that a 0.5% net stop and 0.5% net target would instead have lost 8.9% in the later period. The current AutoTrader's generic intraday brackets would therefore invalidate the profitable backtest convention. A new rule should stay paper-only until a matching exit implementation, complete execution costs, and untouched forward period are evaluated.

## What was tested

The [replay script](./spot_trailing_breakout_screen.py) uses the existing seven-pair Binance daily archive plus a [frozen 28 October REST snapshot](./data/binance_spot_recent_2026-10-28.json). It forms signals from the previous closed daily candle, trades at the next open, and measures open-to-next-open returns. Each asset starts with equal capital in its own unleveraged sleeve and remains in cash when its rule is inactive. Fees plus assumed slippage are charged on each entry and exit at 15, 30 and 50 bp per side. There is no daily sleeve rebalancing or interest on cash. The trading calendar runs from 1 March 2021 to 27 October 2026. [Full results](./spot_trailing_breakout_results.json) include years, drawdowns, Sharpe, entries and turnover.

The BTC gate is the previous BTC close above its trailing 200-close SMA. The new candidate holds the sleeve after a close above that asset's **prior 20-day high** while the gate is on; otherwise it holds cash. Consecutive breakouts can extend a hold. This is daily breakout eligibility, not the classic Turtle trailing-channel hold.

| Long-only seven-asset rule | 2021-22 | 2023-24 | 2025-27 Oct 2026 | Later max DD | Later at 50 bp/side |
|---|---:|---:|---:|---:|---:|
| **20-day pulse, BTC SMA200 gate** | **+18.6%** | **+36.4%** | **+28.1%** | -7.8% | +20.2% |
| 20-day pulse, own SMA200 gate (current signal) | +18.8% | +38.9% | +0.2% | -10.5% | -6.8% |
| 55-day pulse, BTC SMA200 gate (current signal) | +20.4% | +39.3% | +8.1% | -5.3% | +2.9% |
| Seven-asset buy-and-hold | -20.0% | +514.8% | -42.7% | -68.0% | -42.7% |

The 20-day BTC-gated pulse was +14.1% in 2025 and +12.3% in 2026 through 27 October at 15 bp per side. It was flat in 2022 because BTC failed the gate. The 55-day BTC-gated rule was also positive in the three broad periods, which supports the **breakout plus risk gate family** more than any one exact lookback. The 20-day result is still selection-biased because it was compared after earlier 2025-26 results were known.

An [older three-asset check](./spot_trailing_breakout_results.json) used BTC, ETH and BNB from 27 May 2018 onward, when all three had 200 prior daily bars. The 20-day BTC-gated rule stayed in cash during the remaining 2018 bear window while matched buy-and-hold lost 59.8%. It made +6.0% in 2019 and +14.9% in 2020 at 15 bp per side, but those two years turned slightly negative at 50 bp per side. That is a useful older-regime check with a different asset universe, not a clean replication of the seven-asset portfolio.

## Other candidates and rejects

| Rule | Historical result | Fit and judgment |
|---|---|---|
| Standalone ETH above SMA50, otherwise cash | +88.3% in 2023-24 and +132.5% in 2025-27 Oct 2026 at 15 bp/side; +116.8% later at 50 bp/side. It lost 30.8% in 2022 and had a 55% full-period drawdown. | A distinct daily spot rule that current Insights data can calculate. A standalone ETH exit must follow SMA invalidation; the current BTC-gated trend sleeve and generic stop/target are different exposures. Promising only as a small paper sleeve with substantial downside. |
| Standalone BNB above SMA100 | +32.1% later, but -28.9% in 2021-22 and -77.9% training drawdown. | Easy daily-data fit; weak across regimes. |
| Equal-capital BTC/ETH independent SMA50 | +63.2% later, but -11.9% in 2021-22. | Less concentrated than ETH alone, yet not positive across broad periods. |
| Weekly ETH SMA50 check | +85.3% later, but -12.6% in 2021-22 and worse drawdown than daily ETH SMA50. | Lower turnover did not improve robustness. |
| Classic 55-day entry / 20-day low exit with BTC gate | +63.1% in 2021-22 and +175.3% in 2023-24, then -18.5% later; -35.4% later drawdown. | A standard trailing trend system, but it failed the more recent period. |
| Classic 20-day entry / 10-day low exit with BTC gate | +107.7% in 2021-22 and +146.2% in 2023-24, then -3.2% later. | Earlier gains did not persist. |
| ETH SMA50 entry / SMA100 exit | +58.7% later but -15.4% in 2021-22; 48 later entries versus 10 for the simple ETH SMA50 rule. | This hysteresis choice created extra churn in the tested regime. |

The trailing-channel and slower-SMA screens were small, named mechanism checks, not an exhaustive parameter search. Their negative later results are why the report recommends the 20-day **one-day signal** instead of extending every breakout into a long hold.

## Exit and execution fit

The [matched 20-day exit screen](./spot20_exit_policy_check.py) uses the same previous-close signal but fixed 1/7 weights, as the existing exit-policy research did. Its [results](./spot20_exit_policy_results.json) are directly comparable *within that screen*:

| Exit policy, 15 bp per side | 2021-22 | 2023-24 | 2025-27 Oct 2026 |
|---|---:|---:|---:|
| Exit when daily signal invalidates | +18.4% | +42.6% | +22.4% |
| Net stop 0.5%, target 0.5% | -8.5% | -17.5% | -8.9% |
| Net stop 0.5%, target 1.5% | -8.4% | -15.0% | -8.3% |

Daily OHLC cannot reveal whether a stop or target touched first within a day. The screen assumes the stop first if both levels touched and exits gaps at the worse open. It is a conservative approximation, not a fill replay. Its fixed daily weights also differ from the separate-sleeve portfolio table, so do not compare the two tables' exact returns as if they were the same portfolio.

To make the 20-day BTC-gated candidate fit the existing app, Insights would combine its current 20-day high calculation with its BTC SMA200 state in [daily_strategy_signals.rs](../../src/rust/insights/src/daily_strategy_signals.rs). The strategy catalog and UI would expose the new rule. AutoTrader would need to manage that position by its own daily active/inactive status, like the current 55-day BTC-gated rule, and remove the 0.5% bracket from *that strategy's paper positions*. The [exit rules](../../src/rust/auto_trader/src/auto_trader/exits/position_exit_rules.rs) currently give this special handling only to the 55-day variant. Execution would still need synchronized OKX executable books, actual fill fees and realistic capacity; no SBE schema change is needed.

## External evidence and limits

- [Svogun and Bazan-Palomino (2022)](https://www.sciencedirect.com/science/article/pii/S1042443122000816) studied daily and one-minute crypto moving-average and breakout rules with costs, and found more daily than one-minute rules retained positive excess returns after costs. That supports testing slower spot rules, not any specific rule here.
- [Hudson and Urquhart (2021)](https://doi.org/10.1007/s10479-019-03357-1) found technical-rule profitability in several cryptocurrencies while noting no out-of-sample Bitcoin predictability in their sample. Their multiple-testing work reinforces the need for an untouched period.
- The [2016 Donchian breakout thesis](https://open.uct.ac.za/items/3b7b16c9-ee94-428c-96ca-747f67e1c623/full) documents the standard 20/10 and 55/20 entry/exit families screened here; the local later-period losses show that conventional settings do not transfer automatically.
- The [Binance public-data specification](https://github.com/binance/binance-public-data/blob/master/README.md) documents the OHLC structure and timestamp format of its archives. The recent snapshot was read from its market-data REST host. Archive rows can be revised and the recent snapshot has no pinned checksum.
- [OKX fee documentation](https://www.okx.com/en-gb/help/trading-fee-rules-faq) says maker/taker rates depend on the instrument and account. The 15-50 bp cost scenarios are stress assumptions, not verified account-specific OKX execution costs.

The present-day seven-coin universe excludes assets that delisted or lost liquidity, creating survivorship bias. Binance open prices and daily OHLC do not reproduce OKX order-book fill prices, partial fills, stops, scheduling, or market impact. These backtests were examined after substantial earlier research on overlapping dates; their later period is **not** an untouched holdout. The practical next test is a frozen paper implementation with the matched daily exit and real execution audit, followed by a future period that remains unseen during rule selection.

### Reproduction

From the WSL repository root:

    python3 research/autotrader/spot_trailing_breakout_screen.py
    python3 research/autotrader/spot20_exit_policy_check.py

The first script reads the retained archive and frozen recent snapshot. The 28 October bar contributes only its opening price for the final return; its incomplete high, low and close are never used to form a signal. The second reuses its loader and the existing exit-policy engine.


## Implementation update, 28 October 2026

The researched rule is now implemented as the selectable paper strategy `donchian-20-btc200`. [Insights daily signals](../../src/rust/insights/src/daily_strategy_signals.rs) fix the lookback at 20 closed days and the BTC regime at 200 closed days. [The strategy contract](../../src/rust/auto_trader/src/auto_trader/strategies/strategy_donchian_20_btc200/contract.rs) uses its own active/inactive daily state for exits. AutoTrader does not apply its shared 0.5% stop or target to this strategy, and [protective-stop maintenance](../../src/rust/auto_trader/src/auto_trader/exits/protective_stop_maintenance.rs) does not arm a fixed barrier for its positions. This matches the signal-exit replay but permits larger losses between daily evaluations; the UI label states this explicitly.

The new strategy is available in AutoTrader settings and order labels. Existing selected strategies and persisted paper positions were not changed. The services were not restarted, so the running binaries do not yet use this source change. Validation passed: the Insights rule test, 14 AutoTrader tests, both Rust builds, React typecheck, and ESLint on the three changed React files. The repository-wide React lint still has pre-existing errors outside those files.
