# Historical strategy results by market regime

**Updated:** 23 October 2026  
**Purpose:** Collect the historical situations in which each researched strategy family did or did not work, and label the market regime those periods appear to represent. This is a synthesis of existing tests, not a new backtest or a live strategy recommendation.

## Regime labels used here

The labels are descriptive, based on broad crypto price behavior and the benchmark results in the cited tests. They are not point-in-time regime classifications computed by a single model. Periods such as 2025–October 2026 contain multiple conditions and should not be treated as one uniform regime.

| Label | Working description | Historical example in these tests |
|---|---|---|
| Bull / persistent uptrend | Broad prices rise and winners can run for weeks or months. | 2017 ETH run; 2023–24 broad altcoin rally in the seven-coin comparison. |
| Bear / prolonged downtrend | Broad prices fall for months, with rallies that may fail. | 2018 ETH decline; 2022 crypto bear market. |
| Choppy / mixed or leadership-rotation | Direction changes or leadership is narrow; trend exposure can switch off and relative strength can reverse. | 2025–22 Oct 2026 comparison window, which includes multiple conditions and ends with a weak equal-weight buy-and-hold result. |
| Short bearish episode | Several hours of falling prices; a short baseline can look unusually strong. | 21 Oct 2026 evening signal capture. |

## Regime-conditioned recheck on daily spot data

I re-ran the principal daily strategy families using a causal market-state label: BTC's prior daily close above or below its trailing 200-day simple moving average. The common seven-asset calendar and 15 bps per-side trade-cost assumption are retained. This restricts the comparison to 1 March 2021–22 October 2026, when the 200-day state is available.

The table reports the strategy's average net return per calendar day assigned to each state (basis points) and annualized daily Sharpe within that state. Flat strategy days are included. These are descriptive, serially dependent observations, not independent trials. A filtered compound return is also saved in the result JSON, but is not presented here because compounding only selected regime-days skips intervening days and does not represent an investable account path.

| Strategy | BTC above SMA200: mean bp/day (Sharpe) | BTC below SMA200: mean bp/day (Sharpe) | Read against its intended regime |
|---|---:|---:|---|
| Seven-asset buy-and-hold | +22.1 (1.19) | +2.7 (0.13) | Uptrend exposure fit; materially weaker below trend. |
| 50/200-day trend filter | +17.8 (1.40) | +1.5 (0.43) | Stronger above trend; lower exposure below trend helped, though small positive mean partly reflects cash days. |
| 55-day breakout | +5.9 (1.35) | -0.2 (-0.29) | Supports the directional-market tag; essentially no positive average in BTC-below state. |
| 20-day breakout plus 200-day asset filter | +5.5 (1.21) | +0.2 (0.28) | Mostly consistent with the uptrend tag; below-state mean is near zero. |
| 90-day relative-strength rotation | +56.5 (1.76) | -8.9 (-0.52) | Strongly regime-dependent: worked in BTC-above state and failed in BTC-below state. |
| RSI(2) oversold rebound | +1.3 (0.16) | -0.1 (-0.01) | No convincing rebound edge even in the intended bull-pullback state. |
| BTC above SMA50 | +14.0 (1.22) | +0.4 (0.06) | Trend participation held up in BTC-up state; near flat below it. |
| ETH above SMA50 | +14.2 (0.89) | +6.6 (0.66) | Positive in both benchmark states in this slice; ETH can decouple from BTC, so BTC regime is an imperfect tag. |
| BNB above SMA100 | +19.8 (1.11) | -4.4 (-0.57) | Fits rising-market tag and did not hold up below BTC's long trend. |
| Three-sleeve BTC/ETH/BNB trend mix | +14.6 (1.11) | +1.0 (0.15) | Most return accrued above BTC's trend; little evidence of meaningful bear-regime gains. |
| Same trend mix with BTC-200 gate | +14.9 (1.14) | -0.2 (-2.38) | The gate does what it is designed to do: avoid exposure below trend. The negative mean reflects transition-day costs; a zero-return state is not guaranteed. |
| Monthly 30-day momentum / SMA50 rotation, validation winner | +43.1 (1.59) | -1.1 (-0.07) | Confirms strong bull/leadership dependence, but the configuration was selected on validation and the holdout periods were weak. |
| SOL 200 bp dip / 100 bp target / 14-day exit, validation winner | +29.1 (1.53) | -30.1 (-1.20) | Contradicts the idea that this dip-buy rule is safe in bear regimes; positive-state result is highly sensitive to the selected candidate and path. |
| BNB above SMA100 with BTC above SMA100 confirmation | +25.9 (1.61) | +2.5 (0.53) | Mostly consistent with a rising-market filter; this was a post-hoc configuration, so treat as exploratory. |

Reproduction: [regime-conditioned script](./spot_regime_conditional_check.py) and [result data](./spot_regime_conditional_results.json). The check reproduces representative rules and the validation-selected rotation/dip candidates; it does not disaggregate all 43 original single-asset variants or all 144 dip-buy configurations. Those broad searches still require candidate-by-candidate regime attribution before claiming that every parameter setting has been checked.

This check supports several broad tags, but it also qualifies them: 55-day and filtered 20-day breakouts had their best evidence above BTC's long trend; 90-day rotation and dip buying were especially fragile below it; RSI(2) rebound did not show a reliable within-regime result; and the ETH SMA50 sleeve remained positive in both buckets. The BTC-200 label is a broad-market proxy, not an asset-specific regime classifier, so it can mislabel ETH or altcoin conditions.

## Strategies with historically positive scenarios

| Strategy | Scenario where it worked in the tested history | Evidence | Regime tag and read |
|---|---|---|---|
| ETH close above 50-day SMA, otherwise cash | Positive across 2020–22 train (+299.6%), 2023–24 validation (+88.4%), and 2025–22 Oct 2026 holdout (+132.6% net). In the short 2018 sample it lost 7.6% while ETH buy-and-hold lost 82.1%. | [Spot profitability search](./SPOT_PROFITABILITY_SEARCH.md) | **Bull trend participation; bear-market exposure reduction.** The rule also lagged the 2017 bull run (+66.8% vs +147.9% buy-and-hold) and had a 51.2% drawdown in the older combined sample. Interesting candidate, but holdout selection bias remains. |
| BTC/ETH/BNB equal-capital trend mix (50-day BTC/ETH, 100-day BNB) | +1,967% full-period vs +1,283% static basket; lost 46.0% in 2022 while the BTC-200-day-gated mix was flat. 2018 window loss was 25.0% vs 57.8% for matched buy-and-hold. | [Spot profitability search](./SPOT_PROFITABILITY_SEARCH.md) | **Bull trend participation, with risk-off protection in bear regimes.** Maximum drawdown remained 64.5% ungated / 60.5% gated. Full history informed strategy construction, so this is not confirmation. |
| BTC-200-day risk-off gate on the trend mix | Flat in 2022 and in cash through its warmed-up 2018 bear window; reduced max drawdown from 64.5% to 60.5%. It also returned less than the ungated mix in strong periods (2025: +21.8% vs +29.7%). | [Spot profitability search](./SPOT_PROFITABILITY_SEARCH.md) | **Prolonged bear / below-trend regime.** Protection has an opportunity cost when markets recover or rally. |
| 20–55-day Donchian breakout family | Positive in train, validation and holdout for the 55-day rule (+14.9%, +44.3%, +9.9%). Holdout remained positive at 20, 40 and 55 days even when modeled cost rose to 50 bps/side (+4.2%, +5.5%, +2.6%). | [Spot regime comparison](./SPOT_REGIME_STRATEGY_REVIEW.md) | **Persistent directional / breakout regime.** Only modest holdout gains; tested universe is small and survivorship-biased. Variant sensitivity was inspected after seeing holdout results. |
| 55-day breakout | Only add-on positive in all three periods; full-period max drawdown 15.4%, lowest among the compared add-ons. | [Spot regime comparison](./SPOT_REGIME_STRATEGY_REVIEW.md) | **Sustained directional markets.** The strongest across-period consistency in that comparison, but not a proven edge. |
| 20-day breakout plus 200-day trend filter | Positive in 2019–22 (+19.9%), 2023–24 (+45.3%), and holdout (+3.6%). | [Spot regime comparison](./SPOT_REGIME_STRATEGY_REVIEW.md) | **Selective rising trend / early breakout.** Less effective than the 55-day breakout in the holdout. |
| BNB above 100-day SMA | +18.1% in 2025 and +9.8% through 22 Oct 2026; in older 2018 sample lost 20.7% vs -32.3% buy-and-hold. | [Spot profitability search](./SPOT_PROFITABILITY_SEARCH.md) | **Positive trend participation and partial bear protection.** Its 2019–26 total return lagged BNB buy-and-hold and drawdown was about 78%; the dates were not a pristine holdout. |

## Strategies that only looked good in particular regimes or failed to transfer

| Strategy | Observed scenario | Evidence | Regime tag and conclusion |
|---|---|---|---|
| 50/200-day trend filter, seven-asset portfolio | Strong train/validation (+182.7%, +151.7%); holdout -16.6%. | [Spot regime comparison](./SPOT_REGIME_STRATEGY_REVIEW.md) | **Broad bull / recovery; weak in mixed or leadership-rotation holdout.** Gains did not persist in the latest segment. |
| 90-day relative-strength rotation above 200-day average | +242.7% train and +571.1% validation; -5.9% holdout, full max drawdown -84.4%. | [Spot regime comparison](./SPOT_REGIME_STRATEGY_REVIEW.md) | **Strong, concentrated altcoin leadership cycles.** Severe crash and reversal exposure; not robust as a standalone allocation. |
| Equal-weight 90-day momentum | -15.3% in 2025–22 Oct 2026 holdout (original search). | [Spot profitability search](./SPOT_PROFITABILITY_SEARCH.md) | **Did not work in the tested latest window.** Momentum leadership can reverse; this test does not establish which sub-regime caused the loss. |
| RSI(2) oversold rebound, above 200-day average | +11.9% in validation but -3.6% train and -14.7% holdout. | [Spot regime comparison](./SPOT_REGIME_STRATEGY_REVIEW.md) | **Intended for short-lived pullbacks within a bull trend; not supported by results.** The positive validation was not durable. |
| Dip-buy rebound family | Best tested configuration lost 0.8% in 2026; most configurations lost more. | [Spot profitability search](./SPOT_PROFITABILITY_SEARCH.md) | **Failed in the tested latest regime.** Not enough evidence to identify a profitable dip-buying regime. |
| Weekly/monthly cross-sectional rotation | Best 2026 result only +1.3%, after +5.5% in 2025. | [Spot profitability search](./SPOT_PROFITABILITY_SEARCH.md) | **Weak in the latest mixed/rotation window.** No compelling candidate. |
| BNB 50-day plus BTC 100-day confirmation | +29.9% in 2025 and +22.6% in 2026 to date. | [Spot profitability search](./SPOT_PROFITABILITY_SEARCH.md) | **Apparently strong rising-market filter**, but discovered after examining these periods; treat as post-hoc, not a validated regime result. |
| Buy-and-hold | +514.0% in 2023–24 validation, then -39.8% in 2025–22 Oct 2026; full max drawdown -79.4%. | [Spot regime comparison](./SPOT_REGIME_STRATEGY_REVIEW.md) | **Strong broad bull; painful bear/reversal.** Simple, but not regime-robust. |

## Intraday, flow and market-making research

| Strategy / signal family | Historical situation | Regime tag and conclusion |
|---|---|---|
| AutoTrader rule search (trend, momentum, flow, imbalance, volatility, VWAP, pressure; follow/fade) | A 15-minute score fade was the best validation-ranked shortlisted rule, but validation mean was -0.0009% and final holdout mean -0.1895% across 217 entries. Other rules that made money in training failed validation. | **No demonstrated regime where it worked after costs.** The 19 Oct session did not support a profitable rule; market move sizes were usually below estimated costs. See [REPORT](./REPORT.md). |
| 21–22 Oct signal score fade / BTC-to-alt spillover proxy | Score fade remained negative across the two sessions. Fixed-sign BTC spillover had near-zero or negative gross means and negative net results; sign changed by horizon/session. | **Short bearish episode effects are confounded with the market baseline.** No repeatable regime tag or edge supported. See [EDGE_SEARCH](./EDGE_SEARCH.md) and [RESEARCH_LEADS](./RESEARCH_LEADS.md). |
| Funding, basis and open-interest filters | On 21 Oct, raw correlations changed by asset and horizon; naive funding fade looked positive for ETH during a falling window, but gross was below typical execution costs and the same rule lost on BTC. | **Short bearish episode / confounded directional baseline.** No stable regime association established. See [YESTERDAY_EXTERNAL_TEST](./YESTERDAY_EXTERNAL_TEST.md). |
| Passive touch / maker entry | Passive entry reduced modeled loss versus taker entry, but holdout stayed negative even assuming every order filled; trade-through proxy was worse. | **No profitable regime identified.** Results are consistent with adverse selection, but the capture cannot model queue or actual fills. See [PASSIVE_MM_REVIEW](./PASSIVE_MM_REVIEW.md) and [MAKER_REPLAY](./MAKER_REPLAY.md). |
| Static imbalance, microprice, depth and simple trade-flow signs | In the one-evening test, these features had negative net means and did not beat matched short-only. | **No regime-specific edge identified.** One short window is insufficient to infer regime behavior. See [PASSIVE_MM_REVIEW](./PASSIVE_MM_REVIEW.md). |
| Literature-inspired intraday features (clock-time, activity, news, external derivatives and other proxies) | The local 21 Oct replay found no robust positive net expectancy; several literature mechanisms were not identifiable from retained point-in-time data. | **Regime unknown or untestable locally.** Results are exploratory proxy checks, not replications. See [LITERATURE_TRIAL](./LITERATURE_TRIAL.md) and [ADDED_PAPER_TESTS](./ADDED_PAPER_TESTS.md). |

## Practical regime map from current evidence

| Current condition | Strategies worth keeping as research candidates | What the history says |
|---|---|---|
| Sustained broad uptrend | 20–55-day breakouts; per-asset SMA trend sleeves | Best relative fit in daily tests. Returns vary by asset and still include large drawdowns. |
| Prolonged bear / broad weakness | Cash when below long moving-average gates; BTC-200-day gate | Historically reduced losses in 2018/2022 samples, but can miss rapid recoveries and does not guarantee capital preservation. |
| Sharp pullback inside a long-term uptrend | RSI(2) rebound / dip-buying | The tested versions did not transfer; do not tag this regime as successful yet. |
| Altcoin leadership / high dispersion | Relative-strength rotation | Can compound strongly when leadership persists, but the same family had an 84% drawdown and negative latest holdout. |
| Choppy, mixed or rapidly rotating market | No intraday signal or spot family demonstrated reliable positive net results | Most tactical features were cost-negative; breakout family was only modestly positive in the latest segment. |
| Short bearish intraday episode | Short-only benchmark may appear favorable | Feature/funding/fade findings are not separable from simply being short during the declining window. |

## How to use these tags

These tags describe **where historical performance appeared**, not when a strategy should be switched on. Most evidence uses small present-day asset universes, modeled costs, overlapping or short-duration observations, and inspected holdouts. The next useful step is a predeclared regime classifier plus frozen strategy rules, evaluated on point-in-time assets (including delisted coins where possible), multiple chronological holdouts, and actual venue fees/fills. Keep intraday and maker strategies in shadow research until they show positive net results across separate market regimes.

## Source index

- [Spot profitability search](./SPOT_PROFITABILITY_SEARCH.md)
- [Spot regime strategy comparison](./SPOT_REGIME_STRATEGY_REVIEW.md)
- [Passive market-making review](./PASSIVE_MM_REVIEW.md)
- [Maker entry replay](./MAKER_REPLAY.md)
- [Historical signal model review](./REPORT.md)
- [Cross-session edge search](./EDGE_SEARCH.md)
- [External market feature check](./YESTERDAY_EXTERNAL_TEST.md)
- [Research leads](./RESEARCH_LEADS.md)
- [Literature trial](./LITERATURE_TRIAL.md)
- [Additional paper tests](./ADDED_PAPER_TESTS.md)
