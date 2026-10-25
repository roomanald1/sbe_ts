# Research leads for the signal stack

**Prepared:** 22 October 2026  
**Purpose:** Translate external research into testable trading hypotheses, and check the strongest fit against the retained signal outcomes. This is research, not a live-strategy recommendation.

## Best lead: cross-asset return spillovers

Jia et al. document a crypto “seesaw” effect: lagged returns from large coins negatively predict returns in smaller coins. Their rolling LASSO cross-sectional strategy reports positive five-minute returns on Bitfinex and Huobi after its transaction-cost assumptions. Their findings are from 2018–2021 and the active instruments and fee schedules do not match this OKX setup. [Published article and abstract](https://doi.org/10.1016/j.jempfin.2023.101428)

### Local proxy check

I tested the paper’s directional intuition, not a replica of its rolling LASSO portfolio: if BTC’s prior five-minute return is positive, short each other asset; if negative, long it. Outcomes were exact-joined to each asset’s synchronized research feature timestamp. BTC and ETH targets were excluded. I retained one outcome per asset per holding horizon to reduce overlap, and used the recorded long/short gross and net return fields. The later UTC date is a chronological confirmation period, but the two short sessions are not enough for a final untouched validation.

| Hold | Session | N | Mean gross | Mean net | Median net |
|---|---|---:|---:|---:|---:|
| 5 min | 21 Oct, 19:40–23:59 UTC | 4,162 | -0.008% | -0.285% | -0.240% |
| 5 min | 22 Oct, 00:00–06:02 UTC | 6,057 | -0.021% | -0.310% | -0.250% |
| 15 min | 21 Oct, 19:40–23:59 UTC | 1,297 | +0.012% | -0.263% | -0.232% |
| 15 min | 22 Oct, 00:00–06:02 UTC | 2,049 | -0.054% | -0.328% | -0.224% |

This fixed-sign proxy did not transfer: its gross mean was near zero or negative and the 15-minute result changed sign. Recorded costs then made each slice clearly negative. This does not disprove the paper’s rolling LASSO portfolio; it says the much simpler “fade BTC” rule is not enough here. A proper follow-up would construct an as-of cross-section of each coin’s own and peer returns, fit regularized coefficients on a rolling training window, rank forecasts, charge the actual venue/side/turnover cost, and evaluate on several later days. Current research features expose BTC/ETH one- and five-minute returns to each symbol, but not a complete historical cross-section of every peer return. Snapshot trends may fill that gap if their timestamp and feature definitions are verified.

## Strong second lead: cross-venue order-flow leadership

Albers et al. study order books and trades across 14 Bitcoin markets at sub-second resolution. Their linear models explain 10–37% of 500 ms return variation, and their paper explicitly compares taker PnL under exchange fees; fee regimes affect which venues lead or lag. They also report a live maker experiment because passive fills are difficult to test from historical data. [Full paper, with methods and results](https://arxiv.org/abs/2108.09750)

This points to a potential advantage from **where** flow arrives, rather than more thresholds on a single OKX signal. The current stack’s OKX book/trade feed cannot identify a cross-venue leader. To test it, collect synchronized trades and best bid/ask (ideally full L2) from at least Binance and OKX, estimate leader/follower delays out of sample, and replay follower-side orders with measured fees, spread, latency and fill probability. Kaiko documents tick-level trades and L2 streams plus historical/replay data for cross-venue research, though deeper history is subscription data. [Kaiko data dictionary](https://docs.kaiko.com/explore-our-data/data-dictionary)

## Medium-horizon lead: crypto carry and funding

The BIS finds that crypto futures carry is large and variable, averaging above 10% annualized in its sample and sometimes much higher; elevated carry predicts future crash risk. This supports a **crowding / downside-risk filter** over longer horizons, rather than assuming a momentary positive funding rate is an intraday buy or sell signal. [BIS Working Paper 1087](https://www.bis.org/publications/working-paper-1087-crypto-carry)

A separate model of perpetual futures derives no-arbitrage price bounds with trading costs and finds deviations from those bounds can persist and co-move across coins. That is a better basis for a spot/perp carry or relative-value test than raw funding direction. [Fundamentals of Perpetual Futures](https://arxiv.org/abs/2212.06888)

Available local Binance/OKX basis and funding history could support a first descriptive test, but the current retained external sample covers only one evening and the preceding test found mixed, unstable relationships. For a serious carry test, acquire synchronized spot and perpetual quotes, funding payment timestamps, borrow/collateral costs, margin requirements and liquidation-aware PnL across multiple regimes. Never infer a risk-free trade from basis alone.

## On-chain flow lead

Chi et al. report that intraday USDT net inflows to exchanges positively forecast BTC and ETH returns (strongest at one hour), ETH’s own exchange inflow predicts lower ETH returns, and BTC’s own inflow does not significantly predict BTC returns. This is a useful, specific hypothesis: test **stablecoin exchange inflow separately from coin inflow**, and test ETH coin inflow with the opposite sign. The paper studies exchange flow aggregates and options strategies; this is not evidence that a directional spot/perp strategy will be profitable after this system’s fees. [Paper and abstract](https://arxiv.org/abs/2411.06327)

Potential data sources include CryptoQuant’s exchange-flow API (entity-labelled inflow, outflow and netflow endpoints; its on-chain metrics require the Professional tier) and Glassnode’s exchange metrics. Check metric resolution, history, labeling changes, and point-in-time availability before paying or backtesting. [CryptoQuant API](https://cryptoquant.com/apis), [Glassnode exchange metrics](https://docs.glassnode.com/basic-api/endpoints/distribution)

## Recommendation

The next highest-value experiment is not a larger sweep of funding thresholds. It is a rolling cross-sectional return model, using peer and own lagged returns with a frozen feature set and realistic turnover costs. In parallel, begin recording a second venue’s trades/quotes to test whether cross-venue flow adds predictive value. Keep on-chain stablecoin-flow as a separate one-hour study because its source data and holding period differ from the current second-level signal engine.

No tested result in the current short history establishes positive net expectancy. Keep all three ideas in shadow/paper evaluation until they survive several chronological holdouts with actual fills and fees.
