# Alternative strategy screen: risk-adjusted rotation candidate found

**Prepared:** 28 October 2026  
**Scope:** A distinct strategy for the current spot-oriented AutoTrader. No strategy or runtime settings were changed in this research.

## Finding

An additional paper candidate has now passed the local chronological screen: monthly long-only rotation by **90-day return divided by 60-day realized volatility**, gated by BTC above its 200-day SMA and each candidate above its own 100-day SMA. With monthly signal exits and no fixed stop/target, a 40% annualized volatility cap kept net return positive in every split at 15 and 25 bp per side, with materially lower drawdowns than the uncapped version. Replays using current fixed stop/target settings lost in every split, so the candidate requires a strategy-specific signal exit with protective brackets disabled. This is a backtest candidate, not a claim that future trades will profit; the latest period has fewer than 300 trading days and the historical asset universe is fixed.

The strongest *externally documented live* result remains cross-exchange spot arbitrage. A 2026 IEEE study reports a real-capital engine trading 430,000+ spot transactions across eight venues and 32 pairs from 2021 to 2024, with 2.0% average monthly net profit over its deployment. That is a meaningful mechanism, but it needs synchronized multi-venue order books, accounts and pre-funded inventory, two-sided execution, cancel/reconciliation logic and venue-specific fee/latency controls. The current AutoTrader execution path is built around its single OKX paper ledger; this is a separate execution system, not a strategy-only addition.

A read-only **$1,000 depth-walk snapshot** of Binance and OKX books at 15:24 UTC today found the largest gross cross-venue spread was 0.56 bp for BTC and 0.17 bp for ETH. Other sampled pairs were negative. Even a 10 bp taker fee on each venue is 20 bp per entry before unwind costs, so none of these current observations clears costs. The live-study result supports researching this architecture; it does not imply a profitable trade is available now.

## Local results

The tests use retained Binance candles or a point-in-time weekly top-100 universe, prior information only, and fees/slippage in the returns. The most recent daily bar is 27 October; the point-in-time CMC-derived snapshot set ends 25 October.

| Candidate tested | Train | Validation | Later 2025Oct 2026 | Decision |
|---|---:|---:|---:|---|
| Monthly bottom-10 volatility from a point-in-time top-100 universe, 15 bp/side | +181.6% (201922), max DD -80.6% | +107.9% (202324), max DD -44.9% | **-2.2%**, max DD -25.7% | Did not keep its gains in the recent period; at 50 bp/side later return was -6.2%. |
| BTC/ETH 60-day ratio mean reversion, |z|e2, exit at mean or 10 days; 5% annual short borrow | -35.9% (202122) | +5.7% (202324) | **-18.3%** | Not a useful spot-only daily pair rule; it loses in both early and recent windows. |
| Weekly buy-two-losers reversal with BTC SMA200 gate, 15 bp/side | -10.2% | -38.7% | **-27.6%** | Rejected. |
| BTC short breakdowns below BTC SMA200 with borrow cost, 15 bp/side | -38.9% (20-day) / -7.2% (55-day) | -3.5% / -1.8% | **-5.7% / -2.9%** | Neither mirror short rule paid in these splits. |
| Turn-of-month BTC with BTC SMA200 gate, 15 bp/side | +13.8% | -23.5% | +14.1% | Regime-specific, not repeatable across periods. |

The point-in-time low-volatility test selects the bottom ten realized-volatility assets from the active top 100 at each monthly rebalance, using eight prior weekly returns; it treats a missing next snapshot mark as a total loss. This provides a broader universe than the seven-coin archive, but delistings and snapshot coverage still affect the result. The cited 2026 low-volatility paper reports a low-vol premium on a much broader spot universe, but the older 2021 paper found no significant premium. Our latest-period check does not support adding it here.

The calendar result is also conflicted by the literature: one 2022 paper reports a positive turn-of-month pattern through August 2021, while a 2024 seasonality study finds no robust return seasonalities and says the positive BTC Monday effect did not persist after 2015. This local test likewise did not show consistent returns.

## Sources

- Henker et al., [*Executing Arbitrage at Scale: Empirical Evidence on Crypto Spot Market Efficiency*](https://doi.org/10.1109/ICBC67748.2026.11575495), reports results from real-capital, multi-exchange spot execution through 2024. It is evidence for the mechanism, not a prediction for today's spreads.
- Pyo and Jang, [*Revisiting the low-volatility anomaly in cryptocurrency markets*](https://doi.org/10.1016/j.frl.2026.109851), report low-volatility portfolios outperforming high-volatility portfolios on their broad sample; Burggraf and Rudolf, [*Cryptocurrencies and the low volatility anomaly*](https://doi.org/10.1016/j.frl.2020.101683), report no significant premium in an earlier sample. The different periods and universes make point-in-time replication important.
- The 2022 [turn-of-the-month study](https://doi.org/10.1108/MF-02-2022-0084) and 2024 [seasonality study](https://doi.org/10.1016/j.frl.2024.105429) disagree on whether a usable calendar return effect persists.
- OKX fee documentation says actual maker/taker rates vary by instrument and account tier: [OKX fee rules](https://www.okx.com/en-gb/help/trading-fee-rules-faq). The current book probe's fee floor is a scenario, not an authenticated account quote.

## Reproduction

From the WSL repository root:

    python3 research/autotrader/point_in_time_low_volatility_screen.py
    python3 research/autotrader/btc_eth_pair_reversion_screen.py
    python3 research/autotrader/weekly_reversal_screen.py
    python3 research/autotrader/turn_of_month_screen.py
    python3 research/autotrader/spot_bear_breakdown_screen.py
    python3 research/autotrader/spot_cross_venue_probe.py

The saved result files and frozen 25 October point-in-time snapshot are beside the scripts. These screens are exploratory; the research sample overlaps earlier reviewed periods and is not a clean forward holdout.

## Additional screen: volatility-managed time-series momentum

A BTC-only, long-or-cash rule was tested against its unscaled counterpart. Both use a 60-day positive-return signal from the prior close and enter at the next open. The risk-managed variant scales exposure toward 50% annualized volatility from the prior 30 daily returns, capped at 100%. Exposure-change costs were charged at 15/30/50 bp per dollar turned over.

| Cost | 2021–22 unscaled / managed | 2023–24 unscaled / managed | 2025–26 unscaled / managed |
|---|---:|---:|---:|
| 15 bp | -18.8% / -34.2% | +229.0% / +212.2% | +13.1% / +12.8% |
| 30 bp | -22.5% / -37.2% | +209.4% / +192.9% | +9.2% / +9.0% |
| 50 bp | -27.2% / -40.9% | +184.9% / +169.0% | +4.3% / +4.1% |

Volatility scaling slightly improved the 2023–24 Sharpe and drawdown, but did not improve returns; it underperformed in both other splits. This is not a new candidate to implement. The result is consistent with literature caution: Yang (2025) reports improved momentum performance under risk management, while Grobys et al. (2025) document severe crash and tail risk in crypto momentum, and Fieberg et al. (2024) find that large-cap momentum's abnormal returns are concentrated in bull markets and fade over time. See [Yang (2025)](https://doi.org/10.1016/j.frl.2025.107879), [Grobys et al. (2025)](https://link.springer.com/article/10.1007/s11408-025-00474-9), and [Fieberg et al. (2024)](https://doi.org/10.1016/j.irfa.2024.103218).

Reproduce with:

    python3 research/autotrader/vol_managed_momentum_screen.py

## Search conclusion

The next strategies with plausible external evidence are either already represented in the app's research (trend, funding/carry, mean reversion) or require a different execution/data stack (cross-venue arbitrage, multi-asset spot shorting, minute-level microstructure). The local results do not justify turning a strategy on live. The new risk-adjusted rotation is a paper candidate only, and the bracket check below identifies a required exit-policy change. These are historical screens with overlapping periods, not out-of-sample profit guarantees.


## Candidate found: risk-adjusted monthly momentum rotation

The current AutoTrader already has a raw 90-day relative-strength rule ([strategy contract](../../src/rust/auto_trader/src/auto_trader/strategies/strategy_relative_strength_90_sma200/contract.rs)). This candidate changes the ranking and portfolio control: on the first daily bar of each month, require BTC above its 200-day SMA, require each candidate above its own 100-day SMA with positive 90-day return, then select the highest 90-day return divided by trailing 60-day realized volatility. Hold one spot asset or cash until the next monthly rebalance. Size the selected position to a 40% annualized-volatility target, capped at 100%. Signals use prior closes and execute at the next open.

| Cost per side | 2021-22 (from Mar 2021) | 2023-24 | 2025 | 2026 through Oct 22 |
|---|---:|---:|---:|---:|
| 15 bp | **+10.5%**, DD -23.5% | **+234.7%**, DD -30.9% | **+12.9%**, DD -24.8% | **+5.6%**, DD -9.7% |
| 25 bp | **+10.1%**, DD -23.5% | **+229.5%**, DD -31.1% | **+12.3%**, DD -24.8% | **+5.3%**, DD -9.7% |

This is positive across the available chronological blocks and remains positive under a 25 bp per-side cost stress. The uncapped risk-adjusted ranking also remained positive in each block, but had a 62% drawdown in 2021-22 and 47% in 2023-24. The 40% exposure cap reduced those drawdowns. For comparison, the same gated monthly rotation ranked by raw 90-day return at 15 bp returned +51.1%, +180.2%, +29.5%, and +1.5% across the same blocks: raw momentum did better in 2021-22 and 2025, while risk-adjusted momentum did better in 2023-24 and 2026. The candidate is therefore a distinct tradeoff, not a universal improvement.

The daily candle sample is 7 liquid symbols (BTC, ETH, BNB, XRP, SOL, ADA, DOGE), from 11 August 2020 to 23 October 2026; 200-day warm-up means the first evaluated signal is 1 March 2021. The universe is fixed rather than point-in-time and the results do not include slippage beyond the modeled 15/25 bp, taxes, stop/target behavior, partial fills, or live capacity. The backtest is price-only and keeps cash at zero return. A strong 2023-24 block drives much of the cumulative gain. The candidate should be treated as a paper-trading hypothesis and revalidated on a fresh forward period before risking capital.

Published research supports testing risk-adjusted momentum but does not validate this exact rule. Yang (2025) reports better return and Sharpe from volatility-managed crypto momentum, using a 2-week long-short formation/holding design; Grobys et al. (2025) find severe crash and tail risk in crypto momentum despite improvements from risk management; Fieberg et al. (2024) find large-cap momentum is costly and much of its abnormal return comes in bull markets. Sources: [Yang (2025)](https://doi.org/10.1016/j.frl.2025.107879), [Grobys et al. (2025)](https://link.springer.com/article/10.1007/s11408-025-00474-9), and [Fieberg et al. (2024)](https://doi.org/10.1016/j.irfa.2024.103218).

Reproduce with:

    python3 research/autotrader/risk_adjusted_momentum_rotation.py

The detailed sensitivity results are saved in [risk_adjusted_momentum_results.json](./risk_adjusted_momentum_results.json). No AutoTrader implementation or service settings were changed in this research.

### Compatibility check: protective brackets

The positive candidate backtest assumes monthly signal-driven exits and **no fixed stop or take-profit**. A separate daily OHLC replay using the current default brackets (0.5% stop / 0.8% target) produced negative returns in all four chronological periods at 15 bp per side; using the strategy's wider allowed bracket settings (1.5% stop / 3.0% target) also lost in three of four periods. The signal-exit version therefore only remains a candidate if implemented with its own monthly exit contract and the per-strategy protective-stop opt-out supported by the AutoTrader. This is a material dependency, not a minor tuning detail. See [bracket sensitivity results](./risk_adjusted_momentum_bracket_results.json) and [bracket replay](./risk_adjusted_momentum_bracket_check.py).
