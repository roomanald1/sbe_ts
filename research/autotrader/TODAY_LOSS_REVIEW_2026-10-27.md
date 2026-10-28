# 27 October 2026: why the paper AutoTrader lost

Snapshot cutoff: **22:58:00 UTC**. The position ledger and Insights audit were read while the stack was running. All figures here are paper results, not broker fills. The ledger replay uses the position service's average-cost and fee-allocation logic, with the rates in the persisted AutoTrader state. Re-run the scripts below for current values.

## Loss decomposition

The paper ledger contains 16,359 fill rows dated 27 October before the cutoff. There were 1,467 closing fill rows across symbols. Their realized result was **-$11.154** on about **$4,923 of closed entry notional**:

| Component | USD | Share of loss |
|---|---:|---:|
| Price movement before allocated fees | -1.361 | 12% |
| Allocated entry and closing fees | -9.793 | 88% |
| Realized net | **-11.154** | 100% |

The effective allocated fee drag was **0.199% of closed entry notional**, close to a 10 bp fee on each side. The gross price result was **-0.028%** and net was **-0.227%** of that notional. This is why adding more short-horizon data has not solved the loss: the strategy is repeatedly buying and selling moves that are smaller than its round-trip cost. Raw fees charged on all fills dated today were $9.804; allocated fees on today's closes differ slightly because some entry fees belong to positions opened earlier or still open.

Largest net loss contributors were ARB-USDT (-$2.302), AAVE-USDT (-$1.540), SOL-USDT (-$1.483), BCH-USDT (-$1.274), ADA-USDT (-$1.250), and AVAX-USDT (-$1.229). BTC-USDT was +$0.040 after costs, but choosing it after observing today's outcome is not a trading rule. The ledger replay agrees with the state's daily realized P&L to rounding at the same time.

The entry audit contained 409 decisions, 233 attributed to live-trade-flow; 141 of those reported a positive filled quantity. Among 275 decision IDs with exit audits, 161 were attributed to live-trade-flow. These audit events are incomplete relative to the ledger and can refer to the same position more than once, so the ledger is the P&L source of truth. The concentration still points to the live trade-flow/scout loop as the main turnover generator.

## Does today's signal data reveal a better intraday direction?

The read-only [signal replay](./today_signal_replay.py) sampled every 97th recorded executable outcome and excluded overlapping observations within each symbol and horizon. It split today at 12:00 UTC. The default bullish/bearish signal lost after the stored bid/ask and fee model in **both** periods:

| Horizon | 00:00-12:00 mean net | 12:00-cutoff mean net | Median cost in later period | Median absolute move in later period |
|---|---:|---:|---:|---:|
| 1 minute | -0.250% (804) | -0.254% (714) | 0.227% | 0.030% |
| 5 minutes | -0.223% (600) | -0.232% (617) | 0.224% | 0.088% |
| 15 minutes | -0.259% (394) | -0.309% (411) | 0.223% | 0.179% |
| 60 minutes | -0.318% (152) | -0.345% (136) | 0.226% | 0.302% |

Parentheses show directional observation counts. The [frozen replay output](./today_signal_replay_results.json) has exact figures. At an hour, the typical absolute move exceeds cost, but the signal selected the wrong direction often enough that its net mean remains negative. Observations remain correlated across symbols and time, so this is a diagnostic screen, not independent validation. A further scan of always-long, always-short, reversed signal, liquid coins, and lower-cost subsets did not produce a stable positive result in both periods. Do not turn an ex-post winning symbol into a claimed strategy.

## What would have worked?

**The only strategy with a directly measured positive relative outcome today is a no-trade cost gate:** hold cash rather than submit these short-horizon directional orders. It would have avoided the measured $11.154 paper loss, before any interest on cash. This is a counterfactual on the observed ledger, not a claim that cash produces an absolute investment return.

For a positive-return *research candidate*, the strongest existing test is the [cost-gated BTC/ETH quarterly cash-and-carry review](./NEW_STRATEGY_REVIEW_2026-10-27.md). Its fixed 60-day entry screen bought spot and shorted the matched USD-margined quarterly future only when the entry premium exceeded a 0.5% modeled fee/slippage budget. **18 of 18 eligible asset trades over nine expiry dates** from 2024 through March 2026 were positive in that model; the June 2026 trades failed the gate and would have lost if forced. A depth check at 10:52:47 UTC today found an indicative 0.8674% BTC entry premium, or 0.3674% after that cost budget. That quote is stale and the December future has not settled, so it is not a realized win today. The model omits margin financing, liquidation, venue risk, some index tracking error, and actual historical order-book fills. Its selection after several candidate screens makes 18/18 discovery evidence rather than clean out-of-sample proof.

The [55-day BTC-200-gated daily breakout](./spot_regime_gate_test.py) is the best directional alternative already tested over a longer period: +16.8% in 2021-22 training, +42.4% in 2023-24 validation, and +11.9% from 2025 through 22 October 2026 at 15 bp per side on portfolio-weight turnover. It trades much less often, but today's full daily candle and its matching live fill path were not in that frozen test. It cannot be called a 27 October winner.

## Additional source checks

- [OKX fee rules](https://www.okx.com/en-gb/help/trading-fee-rules-faq) confirm that maker/taker rates vary by instrument, account tier, and order execution. Verify the actual fee schedule before claiming a cost gate has cleared.
- [Svogun and Bazan-Palomino (2022)](https://www.sciencedirect.com/science/article/pii/S1042443122000816) explicitly compare daily and one-minute crypto technical rules with transaction costs and find costs and market state matter. Their results motivate slower, costed validation; they do not validate this AutoTrader.
- [Chi et al. (2023)](https://onlinelibrary.wiley.com/doi/full/10.1002/fut.22425) find futures basis a stronger cross-sectional factor than momentum in their sample. That supports investigating basis as a different return mechanism, not assuming a particular spread will persist.
- [BIS crypto carry research](https://www.bis.org/publications/working-paper-1087-crypto-carry) describes spot-long/futures-short carry and emphasizes margin spikes and liquidation risk. [Binance delivery rules](https://www.binance.com/en-AU/support/faq/detail/a3401595e1734084959c61491bc0dbe3) specify the 07:30-08:00 UTC index averaging and settlement fee used by the existing screen.

## Data-quality finding

The state's `closed_trade_fees_usd` window is **not** a per-trade fee record for externally reconciled closes. [position_maintenance_worker.rs](../../src/rust/auto_trader/src/auto_trader/workers/position_maintenance_worker.rs) passes `source_position.fees_paid`, a cumulative fee total for the entire symbol position, to `record_closed_trade_outcome`. The last 200 stored outcome rows summed to about $59.4 in reported fees while the *entire* day's ledger charged about $9.8. The same path divides a symbol-level realized delta by the prior residual quantity; three stored "returns" were below -400%, including -2406%, on sub-$1 losses. Those percentages and fee totals must not be used for strategy selection or adaptive exit learning. The ledger-level dollar P&L decomposition above is unaffected by this reporting problem.

## Decision

Pause new live-signal/scout entries in paper research until a cost gate demonstrates positive **net** outcomes on a later, untouched period. Keep the current dated-carry workflow paper-only and disabled by default; measure synchronized executable depth, account-specific fees, financing and collateral, and actual settlement tracking before considering a live paired trade. Repair the cumulative-fee and residual-denominator outcome metrics before interpreting adaptive policy statistics. No settings, orders, or running services were changed in this review.

### Reproduction

From the WSL repository root:

    python3 research/autotrader/today_loss_replay.py --as-of 2026-10-27T22:58:00Z
    python3 research/autotrader/today_signal_replay.py --as-of 2026-10-27T22:58:00Z

The [ledger snapshot output](./today_loss_replay_results.json) and [signal snapshot output](./today_signal_replay_results.json) were saved at this cutoff. The live SQLite database and ledger continue changing, and USD conversion rates in the state are current at replay time rather than versioned historically.


## Follow-up on 27 October at 23:08 UTC

The carry implementation was present and had **already opened two synthetic paper pairs** at 11:41 UTC: BTCUSDT_261225 and ETHUSDT_261225. The initial review's phrasing that it was "disabled by default" described the code default, not the active session. The running AutoTrader API returned `enabled: true`, both positions `open`, and fresh eligible BTC/ETH candidates. Insights re-scans approximately every five minutes. There are no authenticated Binance spot or futures order calls in this workflow, so no real cash/futures trade was placed. Settlement cannot be reported until the 25 December expiry. The persisted carry book's `enabled: false` field is stale; the API derives the effective enabled status from AutoTrader's global switch and selected strategies.

To stop the measured fee churn while retaining existing exit management and the paper carry worker, the AutoTrader intraday notional limit was set to **0 USDT** through its settings API. The daily strategy limit remained 500 USDT and carry remained selected. The full stack was restarted through the repository scripts after the code fix; ports 3001-3006 and the UI were healthy, and no new entry orders appeared after restart.

The source fix removes closed-trade samples from symbol-level external reconciliation, where a residual quantity and cumulative `fees_paid` cannot describe one round trip. It records tracked exit outcomes using the strategy lot's own P&L and both entry and exit fees. A versioned startup migration cleared the already corrupted closed-trade sample window while preserving ledger P&L. The AutoTrader Rust suite passed 12/12 tests. No live exchange trading capability was added.


## Follow-up on 28 October: daily entries

The 08:00 UTC Insights daily scan marked BNB-USDT active for `trend-sleeves-btc200`, and SOL-USDT active for both `relative-strength-90-sma200` and `rsi2-oversold-rebound`. AutoTrader selected BNB and SOL but submitted no daily entry on that first pass while local books were warming after startup. At 08:05, the receiver had a fresh daily snapshot but no corresponding daily decisions appeared: its pending queue was keyed only by symbol and high-frequency live updates could overwrite the daily work.

The [pending queue](../../src/rust/auto_trader/src/auto_trader/state.rs) now gives daily and live signals separate per-symbol slots and processes daily slots first. After the full-stack restart, the 08:10 scan produced daily decisions and an accepted SOL RSI(2) VWAP order, which began filling. The BNB sleeve was selected but had not produced an entry order at the verification cutoff. The AutoTrader Rust suite passed 13 tests, including a regression test that a live update cannot replace the pending daily signal for the same symbol.
