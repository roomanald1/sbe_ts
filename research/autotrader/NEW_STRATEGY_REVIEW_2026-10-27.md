# Strategy review: cost-gated dated futures cash and carry

Prepared 27 October 2026. Research result only. No orders, live strategy settings, or backend services were changed.

## Implemented paper workflow

The [Insights carry signal worker](../../src/rust/insights/src/dated_carry_signals.rs) now owns Binance market-data requests and computes the executable BTC/ETH quarterly basis near 60 days to expiry. It records generated entry and settlement signals in the Insights audit database and sends them to AutoTrader's internal carry-signal endpoint. AutoTrader's [dated-carry receiver](../../src/rust/auto_trader/src/auto_trader/dated_carry.rs) consumes only those signals, applies the selected-strategy and paper-size gates, persists paired paper positions, and records paper entry/exit audits. AutoTrader makes no Binance market-data calls for this strategy. The strategy is a checkbox in the [AutoTrader settings view](../../src/typescript/workspace/apps/react-app/src/views/AutoTrader.tsx), and the separate [carry widget](../../src/typescript/workspace/apps/react-app/src/views/DatedCarry.tsx) shows signals, positions, settlement sources and paper P&L. It is disabled by default and has no authenticated order submission or real collateral account.

## Daily strategy signal boundary

The older daily-candle path was also migrated. [Insights daily strategy signals](../../src/rust/insights/src/daily_strategy_signals.rs) now fetches closed candles from FeedHandlers, reads AutoTrader's selected strategies and current parameter profiles, computes the six daily strategy entry and active-state rules, records evaluations in the Insights audit database, and publishes signal batches. The [AutoTrader receiver](../../src/rust/auto_trader/src/auto_trader/daily_strategy_receiver.rs) stores the latest bounded-freshness state for exit decisions and queues only selected signals for entries. The AutoTrader daily market-data fetch and in-process daily evaluation files were removed from its compiled path. Execution and position maintenance still obtain order and position inputs from the position service; they do not generate daily strategy signals.

## Recommendation

The best new candidate found is a **cost-gated BTC/ETH dated-futures cash-and-carry trade**. Buy spot and short the same coin quantity in the USD-margined quarterly future roughly 60 calendar days before expiry. Enter only when the executable future bid minus executable spot ask exceeds all entry, settlement, and spot-exit costs plus an explicit risk buffer. Hold the future to cash settlement; sell spot as an equal-size, 30-minute TWAP during the venue's 07:30-08:00 UTC settlement-index window. Keep margin available on the futures leg and cap size by the weaker of spot and quarterly depth. If the entry spread does not pass the gate, hold cash.

This is absent from the current [AutoTrader strategy catalog](../../src/rust/auto_trader/src/auto_trader/strategies/mod.rs). A source search also found no quarterly-delivery, cash-and-carry, or basis-trade implementation in AutoTrader or Execution. It needs a paired spot/futures order lifecycle, futures collateral and settlement accounting, and a timed spot-exit scheduler. The existing directional spot strategies and VWAP orders are different mechanisms.

## Why the existing signal/exit database did not justify another short-horizon signal

The read-only [database audit](./current_signal_exit_audit.py) sampled 27,673 of about 2.65 million signal evaluations. The underlying market span was only about 22.5 hours on 26-27 October 2026. In non-overlapping symbol/horizon observations, the median absolute 1-minute move was 0.028% and the median modeled round-trip cost was 0.230%. At 5 minutes the values were 0.092% and 0.224%. The original directional signals averaged negative modeled net returns at 1, 5, 15, and 60 minutes. The [audit output](./current_signal_exit_audit_results.json) gives exact counts and estimates; the live database is still changing.

The exit audit had about 200 unique entry decision IDs with exit events, but only 39 complete single-fill price paths; 38 of these were take-profit paths and none were ordinary stop-loss paths. This is too one-sided to show that changing a stop would improve losing trades. The stored external funding records also cover only BTC/ETH for roughly the same short window, not a usable cross-section or expiry-cycle history.

## Strategies screened before the recommendation

| Candidate | Evidence from the replay | Decision |
|---|---|---|
| Weekly 14-coin long-low/short-high funding cross-section | At 10 bps cost per traded dollar: -10.2% in 2023, +2.2% in 2024, +11.3% in 2025, +20.6% Jan-Sep 2026; -24.3% maximum drawdown. Leaving out one coin moves full-period return from -7.0% to +46.4%. | Too sensitive to universe and a losing year. |
| Monthly equal-coin long spot/short Binance perp (BTC/ETH/SOL) | At 10 bps per traded dollar: +2.5% in 2023, +4.6% in 2024, +1.2% in 2025, +0.1% Jan-Sep 2026. A funding filter returned only +1.2% over the whole period. | Recent carry too thin. |
| Monthly long Binance/short Hyperliquid perpetuals (BTC/ETH/SOL) | At 10 bps per traded dollar: +6.5% in 2024, +2.4% in 2025, +0.6% Jan-Sep 2026. Daily-price replay, one incomplete funding day, no intraday liquidation replay. | Positive but recent spread too thin for two-venue operational risk. |
| 60-day BTC/ETH spot versus dated futures with a cost gate | 18 of 18 closed eligible asset trades positive after modeled fees and slippage through March 2026. June 2026 failed the gate and the two hypothetical trades lost money. | Most concrete candidate, with the exit tied to settlement. |

See the [funding cross-section replay](./funding_cross_section_replay.py) and [results](./funding_cross_section_14_results.json), [spot/perp carry replay](./spot_perp_carry_replay.py) and [results](./spot_perp_carry_results.json), and [cross-venue replay](./cross_venue_funding_replay.py) and [results](./cross_venue_funding_results.json). The cross-venue funding files are a third-party frozen archive whose SHA256 manifest verified locally; its price leg uses the exchanges' daily opens. The 2026 period is partial. All of these are model screens, not broker statements.

## Dated futures rule and realized historical screen

The reproducible [dated-basis screen](./dated_basis_entry_screen.py) uses Binance's public spot and USD-M delivery-futures archives, historical official delivery prices, and spot 1-minute candles. For every BTC and ETH quarterly expiry from March 2024 through September 2026, it records the spot and futures daily opens exactly 60 calendar days before the expiry. It calls the setup eligible when the futures premium over spot exceeds 0.5% of spot notional. This 0.5% is a cost budget: regular-user spot fees 0.1% on entry and exit, futures taker fee 0.05% on entry and at settlement, and 0.05% assumed slippage on each of the four transactions. In the realized calculation the fees and slippage scale with the actual entry and exit prices; the 0.5% gate is an entry-time approximation.

The modeled dollar profit per coin equals future entry price minus spot entry price plus the 07:30-08:00 spot TWAP minus the official futures settlement price, less the above costs. The TWAP uses equal quantities at the 30 archived one-minute opens. No order book from those historical dates is available, so the 5-bp per-transaction slippage is an assumption. The [complete trade-level results](./dated_basis_entry_results.json) include both entry premiums and settlement outcomes.

| Expiry year | Closed eligible trades | Positive net | Mean modeled net per eligible trade | Lowest modeled net |
|---|---:|---:|---:|---:|
| 2024 | 8 | 8 | +0.986% | +0.241% |
| 2025 | 8 | 8 | +0.819% | +0.164% |
| 2026 through March | 2 | 2 | +0.222% | +0.142% |

There are 18 asset trades but only nine expiry dates, with BTC/ETH outcomes correlated. The 2026 June BTC and ETH entry premiums were just 0.080% and 0.038%, so the rule skipped both. Had they been entered, modeled net returns would have been -0.367% and -0.381%. BTC for September 2026 passed the entry gate, but the official delivery-price endpoint did not return that settlement, so September is excluded from the realized table. This missing point is a limitation, not a win.

The exit schedule matters. For March 2024 ETH, selling spot at the 08:00 open would have modeled -0.489% net even though the entry premium was 1.618%. Selling as a 30-minute TWAP over the exchange's settlement-index window modeled +0.241%. This is a concrete exit change supported by the replay, though minute opens plus assumed slippage are not actual fills.

## Current executable spread check

At 10:52:47 UTC on 27 October 2026, the Binance BTCUSDT spot ask was 80,012.81 USDT and the BTCUSDT December 25 delivery-future bid was 80,706.90 USDT. Walking the public books for a roughly 10,000 USDT spot purchase and equal-coin futures short gave a 0.8674% entry premium. The same 0.5% round-trip cost budget leaves an indicative +0.3674% of spot notional through expiry, before collateral financing, venue/counterparty risk, taxes, and any settlement tracking error beyond the slippage budget. This is a timestamped opportunity check, not an order or a live quote now. A new executable-depth and account-fee check is required immediately before any trade.

## Why the mechanism is plausible, and where it can fail

A dated future converges to its cash-settlement index at expiry, so the premium observed on a matched spot/future entry is a more defined payoff than forecasting the next directional signal or future perpetual funding. Binance's [quarterly contract rules](https://www.binance.com/en-AE/support/faq/detail/3ae441db4ae740e19af3fe9228eb6619) and [delivery rules](https://www.binance.com/en-AU/support/faq/detail/a3401595e1734084959c61491bc0dbe3) specify settlement on the last Friday of the quarter at 08:00 UTC, with a delivery price built from the preceding 30 minutes' index values and a settlement fee. [Binance's spot fee explanation](https://www.binance.com/en-NZ/support/faq/detail/e85d6e703b874674840122196b89780a) gives the regular-user 0.1% example, and its [futures fee explanation](https://www.binance.com/en-NG/support/faq/detail/360033544231) gives a 0.05% regular-user taker example. Account-specific fees, promotions, and legal access must be checked at execution. The [public data archive](https://github.com/binance/binance-public-data/blob/master/README.md) documents the historical candle data.

The paired price legs do not remove margin risk: the short future can face a severe mark-to-market loss while the spot gain is in another wallet. Forced liquidation, exchange failure, basis widening before expiry, spot/index mismatch, execution gaps, and the cost of idle collateral can turn the observed spread into a loss or make it unattractive versus cash. [BIS research on crypto carry](https://www.bis.org/publications/working-paper-1087-crypto-carry) emphasizes margin and liquidation risk. A public [BTC/ETH cash-and-carry index](https://www.compassft.com/indice/spkcarry/) independently follows the same broad spot-versus-future mechanism, but its returns do not validate this implementation. A [study of CEX/DEX carry risks](https://link.springer.com/article/10.1007/s42521-026-00213-3) also shows why fees, collateral, and forced exits must be modeled rather than treating hedged carry as risk free.

## Decision standard before implementation

This screen found a new and mechanically motivated rule with positive modeled closed trades across nine selected expiry dates, plus an observable premium on 27 October. It does **not** establish a guaranteed profit or an investable return on total collateral. Because this candidate was selected after examining several alternatives, the historical sample is discovery evidence, not a clean holdout. Preserve the fixed 60-day rule and cost gate, then paper-trade the next expiry with synchronized executable books, actual account fees, capital tied up in spot and futures margin, intraday marks, and a real 30-minute spot-exit schedule. Compare its net return on total committed capital with cash and the cost of a failure to complete either leg. Only enable trading if the executable premium clears those costs and a collateral-specific margin buffer.

### Reproduction

From the WSL repository root:

    python3 research/autotrader/current_signal_exit_audit.py
    python3 research/autotrader/funding_cross_section_replay.py --universe 14 --start-year 2023 --leave-one-out
    python3 research/autotrader/spot_perp_carry_replay.py
    python3 research/autotrader/cross_venue_funding_replay.py
    python3 research/autotrader/dated_basis_entry_screen.py

The cross-venue script expects the frozen funding study at /tmp/sbe_ts_perp_funding_study; the other scripts download public data into /tmp/sbe_ts_funding_cross_section. Public archives can be revised. No ZIP checksums were pinned for the Binance files.
