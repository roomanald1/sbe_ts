# Passive crypto market-making strategy review

**Prepared:** 23 October 2026  
**Purpose:** Compare passive market-making hypotheses with historical local data. This is an exploratory study, not a profitability claim or a live-trading recommendation.

## Findings

No candidate tested here demonstrated positive net returns. Passive entry reduced simulated losses versus taker entry, but the holdout remained negative even when every posted order was assumed to fill. A trade-through fill proxy was also negative. Static book features did not turn local costed markouts positive.

The local historical capture is sparse evaluation/book snapshots, not a sequenced limit-order-book event archive. It cannot credibly resolve queue priority, partial fills, cancel/replace timing, or the inventory path of a two-sided market maker. These results are screening tests for market-making ingredients and passive entry, not a full market-making strategy backtest.

## Candidate research points and local tests

| Research point | Test performed on retained history | Result | What it establishes |
|---|---|---|---|
| Spread capture from joining the touch | Post a buy at best bid for bullish evaluations and a sell at best ask for bearish evaluations; exit at the horizon's executable opposite quote. Chronological 70/30 split by horizon. | Final holdout with assumed fills: 1m −0.168% (n=122), 5m −0.153% (n=51), 15m −0.307% (n=28), 60m −0.121% (n=27). All negative after modeled 0.08% maker entry and 0.10% taker exit fees. | Crossing less spread and paying a lower assumed entry fee helped versus a taker entry, but did not overcome adverse movement and costs. Rates are a fee scenario, not the account's verified rate. |
| Fill selection / adverse selection | Count a fill only when recorded path low passes a resting buy or path high passes a resting sell. | Holdout means: 1m −0.209% (70 fills), 5m −0.192% (33), 15m −0.413% (20), 60m −0.192% (25). | This coarse trade-through proxy performed worse than the optimistic fill-all case at 1m, 5m and 60m. It is consistent with adverse selection, but extrema do not prove a queue fill. |
| Imbalance or microprice signal | Use the sign of stored microprice offset or depth imbalance to choose direction, then score the resulting recorded costed outcomes. | In the 21 October session, 5m mean net was −0.277% (microprice, n=1,682) and −0.248% (depth imbalance, n=1,609). At 15m: −0.279% (n=1,679) and −0.224% (n=1,608). All lost to the matched short-only means of −0.213% and −0.147%, respectively. | Static imbalance direction did not provide a usable markout filter in this sample. This is not a queue-aware fill conditioned on imbalance. |
| Aggressive flow / toxicity filter | Compare signed 5-second, 15-second and 60-second trade-flow signs with costed markouts. | Best local proxy was 5-second flow: −0.216% at 5m (n=1,439) and −0.208% at 15m (n=1,432), still below matched short-only. Longer flow windows were more negative. | These simple flow signs did not yield positive direction. A calibrated toxicity model could still be useful for abstention, but needs event-level queue and fill data. |
| Depth / liquidity regime | Compare within-symbol shallow and deep book quartiles and cost-beating move frequency. | At 5m, shallow/deep median absolute moves were 0.102%/0.090%, median modeled costs 0.224%/0.228%, and moves exceeded modeled costs 24.9%/24.3% (n=401/391). At 15m, cost-beating rates were 34.5%/32.8% (n=400/390). | Depth buckets did not materially separate tradeable opportunities. The return outcomes remain taker-oriented markouts, not maker P&L. |
| Inventory-skewed quoting (Avellaneda–Stoikov family) | Checked whether available records support a historical replay of reservation-price skew and inventory limits. | Not identifiable from retained labels: no continuous inventory state, quote update/cancel sequence, fill event, or queue rank. Related static microprice/depth signals above were negative. | No valid P&L test of inventory-skewed two-sided quoting can be reported from this dataset. |

Detailed source results and methods are in [MAKER_REPLAY.md](./MAKER_REPLAY.md), [LITERATURE_TRIAL.md](./LITERATURE_TRIAL.md), and [ADDED_PAPER_TESTS.md](./ADDED_PAPER_TESTS.md). The passive-entry capture and 21 October feature study are different sessions and cost assumptions; their numbers should not be pooled.

## What the literature suggests testing next

1. **Inventory-aware reservation price and quote width.** Avellaneda and Stoikov derive a reservation price skewed against inventory and a spread tied to volatility and fill intensity. A historical test needs quote/position/fill replay and a fitted fill-intensity curve, not just directional markouts. [Original paper](https://math.nyu.edu/~avellane/HighFrequencyTrading.pdf)
2. **Fill probability versus post-fill return.** Recent Binance BTC perpetual live-experiment research finds a trade-off between fill likelihood and post-fill returns; it motivates a queue-aware contrarian-touch strategy, while warning that naïve joining can be unprofitable. [Albers et al., *The Market Maker's Dilemma*](https://papers.ssrn.com/sol3/papers.cfm?abstractid=5074873)
3. **Adverse selection as a risk cost.** Crypto order/trade data show statistically significant adverse-selection costs; this supports measuring markout after actual passive fills, rather than treating spread revenue as profit. [Tiniç, *Adverse selection in cryptocurrency markets*](https://doi.org/10.1111/jfir.12317)

These sources motivate hypotheses; they do not establish that an OKX spot strategy will earn a profit. OKX states that fee rates depend on account tier and instrument and directs users to the account's pair-specific fee display/API. The scenario rate in the passive-entry replay must be replaced by a timestamped account rate before decision-quality economics. [OKX trading fee rules](https://www.okx.com/en-us/help/trading-fee-rules-faq)

## Recommended next historical experiment

Capture a multi-day, event-sequenced L2 book and trades for a small liquid universe on the intended venue and instrument. Preserve exchange and receive timestamps, sequence numbers, all add/change/delete events, and actual maker/taker fee rates. Replay a short frozen set of policies: (a) symmetric touch, (b) inventory-skewed reservation price with hard inventory caps, and (c) touch quotes gated by a predeclared flow-toxicity threshold. Model queue-ahead depletion conservatively and report fill rates, realized spread, post-fill markout, inventory P&L, fees, turnover and drawdown on chronological holdouts. Until that replay exists, no tested candidate supports claiming a profitable passive market maker.

## Reproduction and limitations

The passive-entry figures are reproduced in [MAKER_REPLAY.md](./MAKER_REPLAY.md). Related feature and depth results come from the read-only SQLite replay documented in [LITERATURE_TRIAL.md](./LITERATURE_TRIAL.md) and [ADDED_PAPER_TESTS.md](./ADDED_PAPER_TESTS.md). No order was placed and no live settings were changed.

The passive-entry sample covers about one hour of labels, while the feature study covers 4h20m on another session. Samples are short and highly correlated across symbols. Trade-through is only a price-path proxy; all-fill is optimistic. Results omit realistic partial fills, queue rank, cancel latency, market impact, persistent inventory mark-to-market and the account's actual historical fee tier. They cannot establish long-run expected returns.

## Expanded free-data test — Binance spot, 1 July to 30 September 2026

To extend the local short captures, I downloaded Binance's official monthly 1-minute spot kline archives for BTCUSDT, ETHUSDT and SOLUSDT. The archive provides public daily/monthly files and per-file checksums; these files were verified against their published SHA-256 values before parsing. [Binance Public Data repository](https://github.com/binance/binance-public-data)

### Test design

The new reproducible script is [passive_mm_public_backtest.py](./passive_mm_public_backtest.py). It downloads and verifies the monthly archives, then uses one long-only spot inventory unit per symbol: while flat, post a bid a fixed number of basis points below the previous minute's close; when long, post an ask the same distance above the previous close. A touch counts as a full maker fill only if the candle opens on the non-marketable side and its low/high then crosses the limit. Test quote offsets were 2, 5, 10, 20, 30 and 50 bp. Volatility variants skip new bid entries when the current one-minute range exceeds a causal approximation to the prior 24-hour 90th percentile (prior mean + 1.282 standard deviations); existing inventory exit quotes remain active.

Fees are modeled at 10 bp on each side, matching Binance's displayed regular-user spot maker rate for standard pairs at the time checked; account tier, pair and discount can change the actual rate. [Binance fee schedule](https://www.binance.com/en/fee/trading) The sample is split chronologically 70/30 by elapsed time. No parameter was chosen using holdout results.

### Results

Returns are arithmetic mean per completed round trip. “N” is round trips, not independent observations. Holdout is the final 30% of the three-month interval pooled across symbols.

| Policy | Training N | Training gross | Training net | Holdout N | Holdout gross | Holdout net | Holdout wins |
|---|---:|---:|---:|---:|---:|---:|---:|
| Fixed 2 bp | 62,126 | −0.023% | −0.223% | 28,459 | −0.025% | −0.225% | 0.9% |
| Fixed 5 bp | 30,064 | −0.018% | −0.218% | 14,365 | −0.022% | −0.221% | 1.9% |
| Fixed 10 bp | 9,243 | −0.015% | −0.215% | 4,803 | −0.020% | −0.220% | 10.2% |
| Fixed 20 bp | 1,624 | +0.011% | −0.189% | 784 | −0.015% | −0.215% | 32.7% |
| Fixed 30 bp | 474 | +0.098% | −0.102% | 199 | +0.027% | −0.173% | 44.2% |
| Fixed 50 bp | 96 | +0.557% | +0.356% | 39 | −0.211% | −0.411% | 41.0% |
| Skip high-vol, 5 bp | 25,802 | −0.011% | −0.211% | 12,140 | −0.014% | −0.214% | 0.5% |
| Skip high-vol, 10 bp | 6,106 | +0.000% | −0.200% | 3,082 | +0.000% | −0.200% | 9.3% |
| Skip high-vol, 20 bp | 537 | +0.051% | −0.149% | 108 | +0.148% | −0.052% | 42.6% |

No holdout policy was profitable net of the fee scenario. The 20 bp volatility-gated policy was closest to flat in holdout, but it was negative in training, had only 108 holdout round trips, and its fill assumption is optimistic. Wider fixed quotes were sparse and did not consistently improve gross returns. Thus this extension does not identify a profitable policy.

### Fee sensitivity and the only positive holdout case

I repriced the same fill events for the 20 bp quote with the high-volatility entry pause at lower maker-fee scenarios. At 10 bp per side, training/holdout net means were −0.149%/−0.052% (537/108 trades); at 7.5 bp they were −0.099%/−0.002%; at 5 bp, −0.049%/+0.048%; and at 3 bp, −0.009%/+0.088%. The 3 bp case is a possible Binance VIP3 maker rate with the displayed BNB discount, but requires a qualifying account tier; it is not the regular-user rate. [Binance fee schedule](https://www.binance.com/en/fee/trading)

That gives a narrow descriptive claim: **under the optimistic candle-touch model and a 3 bp maker fee per side, this policy averaged +0.088% net per round trip over 108 holdout trades.** It is not a credible profitability claim yet: the training mean was slightly negative, BTC generated no holdout fills, the 20 bp volatility-gated rule was selected after inspecting a grid, and the fill model does not account for queue priority. Treat it as a hypothesis requiring a newly collected, untouched holdout and realistic maker-fill replay.

### Interpretation limits

These are one-minute candle-touch simulations, not observed queue fills: a bar high/low says nothing about queue rank, whether the touch traded enough size, or whether the order would have been canceled first. The simulation also assumes stable fee rates, no partial fills, no market impact, and no outages. Each symbol holds at most one long unit at a time; short inventory and true simultaneous two-sided quoting are not modeled. The summed USD 100-per-round-trip figures in the JSON output are arithmetic diagnostics, not a capital-constrained portfolio P&L. Candle data cannot test microprice, order-flow toxicity or inventory-skew signals, so those still require sequenced quote/trade history.

Run from the repository root:

