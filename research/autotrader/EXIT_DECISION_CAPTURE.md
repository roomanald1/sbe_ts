# Exit-policy decision capture

Prepared: 25 October 2026
Question for next review: Are fixed stop and target exits cutting daily research-strategy trades off too early, and would strategy-aware exits improve net results?

## Current decision state

- Persisted Auto Trader state at src/rust/position_management/auto_trader_state.json had enabled true, generic signals disabled, and six research strategies configured: donchian-55-btc200, trend-sleeves-btc200, relative-strength-90-sma200, rsi2-oversold-rebound, trend-50-200, and donchian-20-sma200.
- Root auto_trader_state.json is a different state snapshot: disabled, generic signals enabled, only donchian-55-btc200. Do not treat it as active without checking AUTO_TRADER_STATE_FILE or the running service API.
- The position-management state had global stop loss and take profit both at 0.5%. Per-position overrides and policy adaptation exist; use each entry/exit audit's effective values.
- In src/rust/auto_trader/src/auto_trader/position_exits.rs, exit evaluation checks strategy disablement, then stop loss, then take profit, then research-regime exit. Fixed thresholds can therefore close a position before its daily eligibility rule is checked.
- The 55-day breakout backtest evaluates yesterday's close, enters at today's open, and earns open-to-next-open returns while its daily signal remains active. That differs from the Auto Trader's fixed P&L thresholds. A matched daily-bar replay below supports the mismatch as a likely source of the gap, subject to the listed assumptions.

## Evidence recorded

### Persisted outcomes

The 180-entry closed-trade window in src/rust/position_management/auto_trader_state.json was current through 25 October 2026 20:48 UTC.

| Measure | Observed |
|---|---:|
| Closed trades | 180 |
| Winners / losers | 41 / 139 |
| Win rate | 22.8% |
| Mean return | -0.269% per trade |
| Median return | -0.408% per trade |
| Mean winning return | +0.851% |
| Mean losing return | -0.599% |
| Realized P&L in window | USD 22.84 loss |
| Recorded fees in window | USD 51.77 |

At observed average win/loss sizes, breakeven requires about a 41.3% win rate. At the observed 22.8% win rate, average wins need to be about +2.03% if average losses remain -0.599%. This is arithmetic on a mixed-strategy window, not an exit-policy counterfactual.

The stored 10,000-order ring contained 97 filled protective stop orders and 25 filled take-profit orders. These counts are indicative only: they do not map one-to-one to the 180 closed trades and include multiple strategies, partial fills, and external reconciliation.

### Daily breakout comparison

The fixed seven-asset Binance daily breakout test in research/autotrader/spot_regime_gate_test.py uses BTC, ETH, BNB, XRP, SOL, ADA, DOGE; 1/7 account weight per qualifying symbol; prior 55-day highs; next-open execution; open-to-next-open returns; and 15 bps per side on portfolio-weight turnover. With BTC prior close above its SMA200:

| Split | Net compound return | Max drawdown | Days |
|---|---:|---:|---:|
| 2021-2022 training | +16.8% | -12.7% | 671 |
| 2023-2024 validation | +42.4% | -8.2% | 731 |
| 2025-22 Oct 2026 holdout | +11.9% | -4.4% | 660 |

An independent cost stress calculation remained positive in the holdout: +9.1% at 30 bps per side and +5.5% at 50 bps per side. It is not a live fill model and should not be attributed to the Auto Trader without matching sizing, timing, and exits.

### Audit coverage and limits

- The current src/rust/insights/signals.sqlite audit contained 240 entry events and 122 exit events at review time. These differ from the state window because retention and event/reconciliation types differ.
- Only 3 of 81 paired entry/exit records in the initial audit join contained a nonempty price path; 78 had none. This is insufficient to compare exit alternatives.
- Protective-stop and external-ledger-close events may describe the same position close. Deduplicate by decision/position and use the final ledger fill; do not count both as separate trades.
- Price paths are sampled about every five seconds and capped at 4,096 samples. They can omit early portions and do not establish exact intrabar trigger order or fill quality when samples are missing.
- The closed-trade window combines multiple research strategies and historical generic-signal activity. Segment by strategy, symbol, entry cohort, exit reason, and effective thresholds before drawing conclusions about Donchian exits.

## Next-review procedure

1. Establish live state from the running Auto Trader API and configured state-file path. Record enabled state, strategies, generic-signal flag, global and per-position settings, process start/restart time, open positions, and ledger reconciliation.
2. Build complete round trips. Join each entry to all partial fills and final exit fills by decision ID plus position/ledger identity. Preserve signed quantity, side, prices, fees, timestamps, funding if relevant, and realized USD P&L. Deduplicate paired stop/external-close events.
3. Capture complete price paths. Persist executable bid/ask snapshots from entry until flat at a defined cadence without truncating the start. Retain daily candles used by the strategy, signal-state transitions, and first touch of stop, target, and strategy invalidation. Keep quote triggers distinct from actual fills.
4. Record exit provenance: reason (stop-loss, take-profit, strategy invalidation, BTC regime off, time stop, manual/external), configured and effective stop/target, trigger quote/time, order type, fill price/time, slippage, fees, partial fills, and whether the strategy signal was still active.
5. Run matched counterfactuals on identical entries and executable quote paths: current 0.5% stop/target; strategy invalidation only (Donchian loses breakout eligibility; BTC gate off exits gated positions); and one predeclared structural/trailing exit. Charge actual side-specific fees, spread, slippage, and realistic execution delay. Do not tune on the evaluation window.
6. Use chronological splits. Select variants on training data, freeze on validation, and report an untouched later holdout. Report net expectancy, win rate, average win/loss, profit factor, turnover, hold duration, drawdown, time in market, missed upside after early exits, and adverse excursion after delayed exits. Bootstrap by day or trade cluster because observations overlap.
7. Decide by strategy. Adopt a strategy-aware exit only if it improves net holdout expectancy and drawdown versus matched current behavior across multiple chronological periods and remains positive under higher cost assumptions. If path coverage is incomplete, keep the decision unresolved and gather more data.

## Files to inspect

- Active-looking state snapshot: src/rust/position_management/auto_trader_state.json
- Alternate root state snapshot: auto_trader_state.json
- Strategy rules: src/rust/auto_trader/src/auto_trader/research_market_data.rs
- Exit evaluation: src/rust/auto_trader/src/auto_trader/position_exit_rules.rs; protective-stop maintenance: src/rust/auto_trader/src/auto_trader/protective_stop_maintenance.rs
- Exit adaptation and thresholds: src/rust/auto_trader/src/auto_trader/execution.rs and src/rust/auto_trader/src/auto_trader/constants.rs
- Daily breakout backtest/results: research/autotrader/spot_regime_gate_test.py and research/autotrader/spot_regime_gate_results.json
- Audit database: src/rust/insights/signals.sqlite (read-only during review)



## Counterfactual completed on 25 October 2026

A reproducible replay was added at research/autotrader/exit_policy_counterfactual.py, with output in research/autotrader/exit_policy_counterfactual_results.json. It uses the same seven Binance daily series, prior 55-day breakout, BTC SMA200 gate, fixed 1/7 weights, next-open entry, and 15 bps per-side baseline cost. It compares strategy-signal exits with net fixed brackets. For daily bars that touch both bracket levels, it assumes the stop hit first; gaps exit at the worse open price.

| Exit policy | 2021-2022 | 2023-2024 | 2025-22 Oct 2026 | Holdout max drawdown |
|---|---:|---:|---:|---:|
| Daily strategy invalidation | +16.79% | +42.36% | +11.88% | -4.41% |
| Net stop 0.5%, target 0.5% | -6.90% | -12.46% | -5.70% | -5.70% |
| Net stop 0.5%, target 1.0% | -6.90% | -11.58% | -5.29% | -5.29% |
| Net stop 0.5%, target 1.5% | -6.90% | -10.69% | -5.07% | -5.07% |
| Net stop 1.0%, target 1.5% | -12.72% | -15.30% | -6.56% | -6.56% |

Signal-exit holdout returns remained +9.09% at 30 bps per side and +5.49% at 50 bps per side. Fixed-bracket results are only a daily OHLC approximation; intraday stop/target order is unknown, and stop-first treatment is conservative. The strategy-signal result reproduces the prior daily backtest convention and is not an execution-matched test of live positions.

The source change is limited to positions tagged donchian-55-btc200: their exit is now based on that strategy's own active/inactive status, instead of another configured strategy keeping the symbol eligible. While Donchian 55 BTC200 remains enabled, fixed stop/target checks are bypassed and its protective stop barriers are cancelled. Disabling the strategy still triggers an exit. This means those positions can carry larger losses and is not validated by live fills; the runtime has not been rebuilt or restarted.

## Provisional conclusion

The matched daily-bar replay favors strategy invalidation over the tested fixed brackets in all chronological splits. That supports using the strategy's own active state as the Donchian 55 BTC200 exit. It does not establish that the live implementation will make money: actual fill paths remain sparse, the counterfactual uses daily OHLC, and the broader realized sample mixes strategies. The source logic was changed, but settings were not edited and the running service was not rebuilt or restarted.
