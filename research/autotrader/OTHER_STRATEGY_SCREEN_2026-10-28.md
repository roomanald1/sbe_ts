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

## Reviewed longer-horizon portfolio ideas (31 October 2026)

Three additional monthly/quarterly concepts were screened after the previous review. All returns below are compounded within each date block. Costs are modeled at 15 or 30 bp per side on traded weight; they combine a fee scenario and assumed slippage, not verified OKX fills.

| Strategy and implementation tested | Period | Net return, 15 bp/side | Net return, 30 bp/side | Decision |
|---|---|---:|---:|---|
| Quarterly rebalance to equal BTC/ETH/BNB weights | 2018–19 / 2020–22 / 2023–24 / 2025–26 to Oct 28 | -29.1% / +916.5% / +268.5% / -8.6% | -29.3% / +913.6% / +268.2% / -8.7% | Rejected: it lagged the matched non-rebalanced equal-initial-weight basket in three of four blocks. It reduced the basket's 2023–24 drawdown (about -40% to -32%) while improving that block's return, but did not transfer to the other blocks. |
| Drawdown ladder on equal BTC/ETH/BNB sleeves: target 25% exposure per completed 10% drawdown from the running all-time high; return to cash at a new high | 2018–19 / 2020–22 / 2023–24 / 2025–26 to Oct 28 | -32.3% / +135.0% / +204.3% / -1.1% | -33.7% / +124.9% / +198.9% / -2.8% | Rejected: lower recent drawdown than the matched buy-and-hold basket (-40% versus -58% in the latest block), but it lost money in 2018–19 and the latest block and gave up most 2020–22 gains. This is a drawdown-triggered exposure rule, not recurring-contribution DCA. |
| Quarterly rebalance into equal-weight top five assets ranked in the point-in-time CoinMarketCap top 100 by trailing 26-week return; stablecoins excluded; missing next top-500 marks treated as total loss | 2020–22 / 2023–24 / 2025–26 to Oct 18 | +480.8% / +102.2% / -45.4% | +467.2% / +98.8% / -46.3% | Rejected: strong earlier blocks did not persist. Two holdings were missing from the next top-500 snapshot; only 12, 8 and 7 quarter observations were available in the respective blocks. |

The BTC/ETH/BNB market-data screens use today's surviving assets. The point-in-time momentum screen reduces that particular selection bias, but snapshot marks are not executable prices and do not resolve token identity, liquidity or delisting uncertainty. None of these results justifies adding a selectable paper strategy. The original scripts for the in-session comparison were not retained; the values are exploratory and should be rerun from a versioned screen before being used as a benchmark.

## Additional strategy ideas screened (31 October 2026)

These long-horizon ideas were screened against the equal-initial-weight BTC/ETH/BNB buy-and-hold basket. Returns are net of modeled costs at 15 or 30 bp per side; the figures are exploratory, and no strategy was added to AutoTrader.

| Idea tested | Periods tested | Net return at 15 bp/side | Net return at 30 bp/side | Review |
|---|---|---:|---:|---|
| Quarterly rebalance to one-third BTC, ETH and BNB | 2018–19 / 2020–22 / 2023–24 / 2025–26 to Oct 28 | -29.1% / +916.5% / +268.5% / -8.6% | -29.3% / +913.6% / +268.2% / -8.7% | Beat the non-rebalanced basket only in 2023–24; rejected as inconsistent. |
| All-time-high drawdown ladder: raise each BTC/ETH/BNB sleeve by 25% per completed 10% drawdown, return to cash on a new high | Same periods | -32.3% / +135.0% / +204.3% / -1.1% | -33.7% / +124.9% / +198.9% / -2.8% | Reduced the latest-block drawdown, but lost in 2018–19 and the latest block and gave up most 2020–22 gains. This is not recurring-contribution DCA. |
| Quarterly top-five momentum from a point-in-time top-100 universe, ranked on trailing 26-week returns | 2020–22 / 2023–24 / 2025–26 to Oct 18 | +480.8% / +102.2% / -45.4% | +467.2% / +98.8% / -46.3% | Large later loss; two selected assets lacked a next top-500 mark and were treated as total losses. Only 12, 8 and 7 quarter observations in these periods. |

The BTC/ETH/BNB tests use today's surviving assets. Point-in-time momentum reduces survivorship bias, but snapshot prices are not executable and do not resolve token identity or liquidity changes. Fee/slippage rates are scenarios, not account-specific OKX execution costs. The comparison scripts used for this screen were temporary and were not retained; rerun and version the source before using these figures as a baseline.

## Options-premium and implied-volatility sizing review (31 October 2026)

Deribit BTC daily DVOL history is available from 24 March 2021 in the public volatility-index endpoint. The index is an implied-volatility measure, not an executable option-chain history. The current test joins it to retained Binance BTC spot candles.

| Idea tested | Net result at 15 bp/side | Net result at 30 bp/side | Review |
|---|---|---|---|
| Sell option premium when implied volatility exceeds subsequent realized volatility | Not established | Not established | The descriptive proxy from 66 month-end observations is unchanged: mean DVOL 58.7%, mean following 30-day realized volatility 52.1%, mean spread +6.6 volatility points (median +8.9; positive in 65.2% of observations). A subsequent 30-day ATM inverse iron-fly replay used Deribit historical option trade candles, the month's last-Friday expiry, approximately 5%-OTM wings, historical instrument taker fees and published delivery prices. Only **1 of 66** monthly entry dates had first-trade candles for all four legs. That one sample returned +0.66% of BTC notional on opening-price fills, but -0.90% under an adverse within-day OHLC fill stress. One trade is not evidence of profitability. A naked short-straddle diagnostic also had only one priced trade and lost 24.3% of notional; it is not a defined-risk result and is not suitable for a conclusion. Do not implement. |
| Size a long-only BTC 60-day time-series-momentum sleeve to 50% annualized volatility using prior-day DVOL; cap exposure at 100% | 2021-03-24–2022: -26.8%; 2023–24: +184.5%; 2025–26 to Oct 28: +11.8% | -29.5%; +164.4%; +7.9% | Risk overlay only: reduced 2021–22 maximum drawdown from -60.6% fixed exposure to -46.8%, but returned less than fixed exposure in 2023–24 and the later block. It did not establish a separate return edge. |

For comparison, fixed BTC exposure returned -30.0%, +229.0%, and +13.1% in those periods at 15 bp per side; sizing from trailing realized volatility returned -38.4%, +212.2%, and +12.8%. At 30 bp per side those matched-period results were -33.3%, +209.4%, +9.2% fixed and -41.2%, +192.9%, +9.0% realized-vol sized. The first block begins on the first date with prior-day DVOL available (24 March 2021), so it is shorter than the existing BTC trend screen's 2021–22 block.

Reproduce the DVOL analysis with `python3 research/autotrader/dvol_risk_sizing_screen.py` and the option trade-candle probe with `python3 research/autotrader/deribit_iron_fly_history_screen.py`. The latter saves [the result JSON](./deribit_iron_fly_history_results.json) and [entry candles](./data/deribit_ironfly_entry_candles.json). Deribit documents its historical instrument/trade API and delivery prices; option trade candles are not historical bid/ask quotes. The sample is too sparse for a meaningful spread P&L estimate, so neither options idea was implemented.


## Candidate found: risk-adjusted monthly momentum rotation

The current AutoTrader already has a raw 90-day relative-strength rule ([strategy contract](../../src/rust/strategies/src/strategy_relative_strength_90_sma200/contract.rs)). This candidate changes the ranking and portfolio control: on the first daily bar of each month, require BTC above its 200-day SMA, require each candidate above its own 100-day SMA with positive 90-day return, then select the highest 90-day return divided by trailing 60-day realized volatility. Hold one spot asset or cash until the next monthly rebalance. Size the selected position to a 40% annualized-volatility target, capped at 100%. Signals use prior closes and execute at the next open.

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
