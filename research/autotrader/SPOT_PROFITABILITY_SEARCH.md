# Crypto spot profitability search

Updated: 2026-10-23

## Finding

The search produced a historically profitable candidate, but it is a long-only trend filter, not a passive market maker: hold ETH when the prior daily close is above its trailing 50-day simple moving average; otherwise hold cash. Signals are formed at the close and acted on at the next daily open. The test compounds open-to-next-open returns and charges 15 bps whenever exposure changes: Binance's current regular spot fee of 10 bps per side plus an assumed 5 bps per side for slippage.

On the chronological 2025-01-01 through 2026-10-22 period, this rule returned +132.6% net, annualized Sharpe 1.27, and maximum drawdown -29.8%. ETH buy-and-hold over the same 660 daily intervals returned -25.9% before costs (about -26.1% after a 15 bps entry and exit assumption). This is an encouraging historical result, not a promise or proof it will keep working. The tested holdout was also used to inspect a 43-variant strategy set, so the best observed strategy is subject to selection bias. Treat it as a candidate requiring a new forward paper test, not a validated edge.

## Test design

- Data: Binance public Spot REST GET /api/v3/klines, 1-day bars. Seven USDT pairs: BTC, ETH, BNB, XRP, SOL, ADA, DOGE. Downloaded rows are retained in research/autotrader/data/binance_spot_1d_api_2019_2026/.
- Shared analysis calendar starts when SOL has data, 2020-08-11, and ends 2026-10-22. The earlier 2019 bars remain in the raw files but are excluded by the common-calendar analysis.
- Timing: prior close signal, trade at next open, hold open-to-next-open. Exposure is 0 or 1; no leverage, shorting, or compounding of idle cash.
- Cost: 10 bps fee plus 5 bps assumed slippage on each entry/exit; costs are charged on changes in exposure. This is a conservative taker-style model for Binance regular spot, not a measured fill-cost estimate.
- Splits: train through 2022, validation 2023–2024, and holdout 2025 through 2026-10-22. Parameters were a small, explicit set: 50/100/200-day moving-average filters and 30/90/180-day time-series momentum across seven pairs, plus an equal-weight 90-day momentum portfolio.
- Annualized Sharpe uses daily net returns and 365 days. Drawdown is computed from compounded daily returns.

## Candidate results

| Rule | Train 2020-08–2022 | Validation 2023–24 | Holdout 2025–2026-10-22 |
|---|---:|---:|---:|
| ETH above 50-day SMA | +299.6%, Sharpe 1.20, max DD -54.9% | +88.4%, Sharpe 0.94, max DD -37.9% | +132.6%, Sharpe 1.27, max DD -29.8% |
| BTC above 50-day SMA | +193.9%, Sharpe 1.18, max DD -58.4% | +318.5%, Sharpe 1.98, max DD -26.8% | +17.3%, Sharpe 0.47, max DD -26.2% |
| BNB above 50-day SMA | +1215.3%, Sharpe 1.62, max DD -41.5% | +115.2%, Sharpe 1.07, max DD -30.4% | +36.0%, Sharpe 0.66, max DD -33.0% |
| Equal-weight 90-day momentum | See full results JSON | See full results JSON | -15.3%, Sharpe -0.33, max DD -30.1% |

The very high altcoin training returns underscore regime dependence and the danger of selecting winners after looking at the data. Full results for all 43 variants and the exact assumptions are in research/autotrader/spot_strategy_search_results.json.

## Older-history validation (outside the original 2019–2026 dataset)

I fetched ETHUSDT daily Binance klines from 2017-08-17 through 2019-01-01 and ran the same 50-day rule, next-open execution, open-to-next-open holding, and 15 bps per-side fee/slippage assumption. Buy-and-hold is measured over exactly the same intervals, including the SMA warm-up exclusion.

| Period | Strategy net return | ETH buy-and-hold net return | Strategy max drawdown | Exposure |
|---|---:|---:|---:|---:|
| 2017-10-06–2017-12-31 (after SMA warm-up) | +66.8% | +147.9% | -31.0% | 81.6%
| 2018 | -7.6% | -82.1% | -51.2% | 20.3%
| Combined | +54.1% | -55.5% | -51.2% | 32.1%

So the rule did hold up as a way to reduce exposure through the 2018 bear market, but it badly lagged the 2017 bull run and still experienced a 51% peak-to-trough drawdown in the combined window. That is regime protection, not stable absolute returns. The pre-2019 sample has only about 16 months after warm-up, so it is limited evidence. Reproduce with research/autotrader/eth_sma50_older_validation.py; the downloaded API rows are retained in research/autotrader/data/binance_spot_1d_api_2017_2018/.

## Continued strategy search

I tested two more daily-bar families with the same 15 bps per-side assumption: (1) weekly/monthly cross-sectional momentum rotation across BTC, ETH, BNB, XRP, SOL, ADA, and DOGE, gated by a moving average; and (2) long-only dip buying on BTC, ETH, BNB, and SOL, with fixed take-profit and time exits. Neither family produced a compelling 2026 result. The best rotation result in 2026 was only +1.3% net (after +5.5% in 2025), while the best dip-buy result lost 0.8% in 2026; most configurations lost substantially more. Results are retained in research/autotrader/spot_rotation_search_results.json and research/autotrader/spot_dip_rebound_results.json, with reproducible scripts beside them.

A simpler candidate emerged in the BTC-confirmation search: hold BNB when its prior close is above its 100-day SMA; otherwise hold cash. This was the top configuration by 2023–24 validation return within that small trend-filter set. It returned +18.1% in 2025 and +9.8% in 2026 through the latest full daily bar, after modeled costs. However, this same BNB 100-day rule had already appeared in the earlier parameter sweep, so these dates are not a pristine holdout. Over its 2019–2026 backtest it compounded to about +2,960%, but BNB buy-and-hold returned about +3,897% on matched dates; the strategy's maximum drawdown was about -78%. On older 2018 data, after its 100-day warm-up, it lost 20.7% versus -32.3% for BNB buy-and-hold. The absolute backtest is profitable, but it does not establish a reliable or superior edge.

One cross-confirmed BNB variation (BNB above SMA50 and BTC above SMA100) returned +29.9% in 2025 and +22.6% in 2026 to date, but that configuration was noticed after looking at those periods and is more exposed to selection bias. It should not be treated as confirmatory evidence. Full trend-confirmation results are in research/autotrader/spot_btc_confirm_results.json.

## Strategy mix by market scenario

I combined three simple sleeves: one third of starting capital in BTC while BTC is above its 50-day SMA, one third in ETH while ETH is above its 50-day SMA, and one third in BNB while BNB is above its 100-day SMA. The optional risk-off version turns all sleeves off when BTC is below its 200-day SMA. Each sleeve compounds its own capital; fees are charged on position changes. The strategy is evaluated from 2019-07-21 through 2026-10-22, once the longest indicator is available.

| Portfolio | Net cumulative return | Annualized Sharpe | Max drawdown |
|---|---:|---:|---:|
| Equal-capital trend mix | +1,967% | 1.14 | -64.5% |
| Same mix, BTC 200-day risk-off gate | +1,703% | 1.12 | -60.5% |
| Static equal-weight BTC/ETH/BNB buy-and-hold | +1,283% | 0.88 | -74.2% |

The gated mix was flat in 2022, when the ungated mix lost 46.0%. On the earlier 2018 window, the ungated mix lost 25.0% from 2018-02-14 through year end versus 57.8% for the matched buy-and-hold basket; the 200-day BTC gate stayed in cash after its warm-up. The gated mix then gave up some gains in bull years, but returned +21.8% in 2025 and +15.0% through 2026-10-22. The ungated mix returned +29.7% and +15.9% in those periods.

This is the clearest historical scenario mix so far: trend sleeves participate when their asset is strong, and the BTC regime gate avoids the prolonged 2022 decline. It still has a very large 60.5% maximum drawdown, lost money in 2019, and these components were chosen after inspecting the same historical data. The reported 2018 window is short and begins after the indicator warm-up. Treat this as a paper-trading candidate, not a validated profit claim. Reproduce with research/autotrader/spot_strategy_ensemble.py; detailed daily/year results are in research/autotrader/spot_strategy_ensemble_results.json.

## What this says about maker vs taker

This candidate is not evidence that passive maker quotes are profitable. Daily OHLC candles cannot establish whether a resting limit order would fill, how much queue priority it had, or whether it was adversely selected. The earlier 92-day passive-quote study remained negative under the assumed regular-user fees; its retained data and report are research/autotrader/data/binance_spot_1m_2026q3/ and research/autotrader/PASSIVE_MM_REVIEW.md. This new search points to a separate possibility: avoid holding spot during weak trends, with taker-like executions on the signal changes.

## Reproduction

Run python3 research/autotrader/spot_strategy_search.py. It reuses the retained daily JSON files and rewrites research/autotrader/spot_strategy_search_results.json.

## Sources

- Binance documents data-api.binance.vision as a public market-data-only REST host and lists GET /api/v3/klines: https://github.com/binance/binance-spot-api-docs/blob/master/faqs/market_data_only.md
- Binance's current spot fee schedule lists Regular User maker/taker at 0.100%/0.100%: https://www.binance.com/en/fee/trading
