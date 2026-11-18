# Strategy ROI Analysis & Learning Improvement Plan

**Date:** 2026-11-17  
**Repo:** /home/ronnie/dev/sbe_ts  
**Status:** Analysis complete, ready for implementation

---

## Problem Statement

ROI is bad — no net positive days. Need to review strategies, thresholds, and signals to find patterns of positive net for a strategy, a param, a symbol, and suggest improvements to learning so it can spot those patterns better.

---

## 1. Current Architecture Overview

### Live Intraday Strategies (5)

| Strategy | Signal Component | Threshold Parameter | Default |
|---|---|---|---|
| `live-momentum` | `momentum_10s_percent` or `trend_5m_volatility_units` | `minimum_momentum` | (catalog) |
| `live-trade-flow` | `trade_flow_imbalance` | `minimum_flow_imbalance` | (catalog) |
| `live-order-book` | `book_imbalance` | `minimum_book_imbalance` | (catalog) |
| `live-vwap-dislocation` | `mispricing_vs_vwap_bps` | `minimum_vwap_dislocation_bps` | 2.0 bps |
| `live-absorption-pressure` | `pressure_without_price_move` (negated) | `minimum_absorption_pressure` | 0.2 |

Each evaluator:
- Requires a directional signal (long/short; neutral → Reject)
- Compares directional component against a **learned** threshold
- Requires walk-forward evidence: `sample_count >= 10`, `average_net_return_percent > 0`, `positive_net_rate >= 0.45`
- Uses **shared protective stops**: stop-loss 0.5%, take-profit 0.8%, profit-protection activation 0.15%, giveback 0.10%

### Daily/Research Strategies

Donchian-10/20/55, trend-50-200, relative-strength-90, rsi2-oversold-rebound, trend-sleeves, risk-adjusted portfolios, dated-carry. These use BTC trend gates and breakout rules.
### Learning System

- Outcomes attributed to `symbol:strategy:parameter` key (`profile_id`)
- `confident_win_rate_edge()` uses Beta(1,1) posterior: `(positive+1)/(samples+2)`, shrunk toward 0
- `learned_strategy_size_multiplier()` scales position size 0.25x–1.5x based on edge
- `recent_net_pnl_edge()` uses exponentially weighted PnL with half-life
- Scouts test parameter variants (`MIN_LIVE_SCOUT_PARAMETER_VARIANT_SAMPLES = 6`)
- Scout params include risk parameters (stop, target, profit-protection) and entry thresholds
- `StrategyWalkForwardEvidence` is a flat aggregate: `{sample_count, positive_net_rate, average_net_return_percent, last_sample_timestamp_ms}` — **no symbol, regime, or condition breakdown**

### Research Backtests (2021-03 to 2026-10)

From `research/autotrader/spot_all_strategy_portfolio_results.json`:

| Strategy | Full Return | Holdout 2025-2026 |
|---|---|---|
| trend_50_200 | +499% | **-16.5%** |
| donchian_55 | +82% | +9.9% |
| breakout_20_sma200 | +80% | +3.6% |
| oversold_rsi2 | -11.8% | -16.8% |
| All non-PIT portfolio | +230% | **-9.4%** |

**Pattern:** Everything that worked historically is negative in the current regime. The most robust single strategy historically (`donchian_55`) is barely positive (+9.9%) in holdout. The BTC-gated donchian (`btc_gated_donchian55`: +86% full) was the standout — confirming **regime/context awareness is the missing ingredient**.

### Runtime Logs

- `auto_trader_state.json`: `enabled: false`, `realized_pnl: 0.0` — auto-trader currently off
- `auto_trader.log`: Heavy errors — post-only limit rejections ("would take liquidity"), 503s ("No fresh market mark for PI-USDT", "Account equity cannot be valued")
## 2. Root Causes of Bad ROI

| # | Problem | Impact |
|---|---|---|
| A | **Walk-Forward Gate Death Spiral** | Entry requires `average_net_return_percent > 0`. When nothing is positive, no live strategy can enter → no trades → no feedback → no adaptation → stuck forever with stale negative evidence. |
| B | **Walk-Forward Evidence Not Symbol-Conditioned** | Single aggregate per profile. Cannot discover that momentum works on BTC but fails on SHIB. |
| C | **No Regime/Context Awareness in Entry Thresholds** | Same threshold in trending, ranging, and volatile regimes. `market_regime` and `ranging_probability` fields exist on `StrategyMarketSignal` but **no live evaluator uses them**. |
| D | **One-Dimensional Threshold Learning** | `learning_direction` is just -1/0/1 (increase/decrease/neutral). No contextual bandit, no regime-conditioned parameter selection. |
| E | **No Signal Strength → Outcome Correlation** | Entry requires only `side * value > threshold`. A signal at 0.001% enters as readily as one at 2.0%. Weak signals crossing the threshold are likely the losers. |
| F | **Shared/Fixed Protective Stops** | Stop-loss 0.5%, take-profit 0.8% identical across all strategies/symbols. Too tight for high-vol symbols, too loose for low-vol ones. Not learned per context. |
| G | **Scout Minimum Samples = 2** | `MINIMUM_SCOUT_PARAMETER_SAMPLES = 2` — parameters promoted on just 2 observations, indistinguishable from noise. |
| H | **Scouts Trade Obscure Symbols, Observation-Only** | USDC-PLN, SHIB-USDT, XMSTR-USDT — illiquid symbols where execution costs dominate. Many `observation_only=true`, so no real feedback. |
## 3. Recommended Improvements to Learning (Prioritized)

### Priority 1: Fix the Walk-Forward Death Spiral
Change the gate so `average_net_return_percent > 0` is required **only when evidence is mature** (`sample_count >= minimum_samples`). Allow immature evidence to enter. Add a "regime-cooled" mode where the threshold is relaxed if recent outcomes are negative across all strategies.

**Files:** `src/rust/strategy/src/lib.rs` — `evaluate()` walk-forward gate

### Priority 2: Signal Strength Minimum for Entry
Add `MinimumSignalStrength` parameter to each live strategy. Entry requires `signal.strength >= minimum_signal_strength` in addition to threshold crossing. Filters out noise-crossings.

**Files:** `core/contracts.rs`, `parameter_catalog.rs`, each `strategy_live_*/evaluate.rs`

### Priority 3: Increase Scout Minimum Samples
Raise `MINIMUM_SCOUT_PARAMETER_SAMPLES` from 2 to 10. Require `lower_90_one_sided_improvement_percent > 0` before promotion.

**Files:** `src/rust/learning/src/feedback.rs`

### Priority 4: Symbol-Conditioned Walk-Forward Evidence
Extend `StrategyWalkForwardEvidence` (or add parallel map) to track per-symbol and per-regime buckets. Evaluator looks up evidence keyed by `symbol + regime`, falling back to aggregate.

**Files:** `input.rs`, `lib.rs`, `cycle_entry.rs`

### Priority 5: Regime-Conditioned Entry Thresholds
Store separate thresholds per regime (trending/ranging/volatile). Evaluator reads `signal.market_regime` and selects matching threshold.

**Files:** `contracts.rs`, `parameter_catalog.rs`, each `strategy_live_*/evaluate.rs`, `learning/src/`

### Priority 6: Per-Symbol/Per-Strategy Protective Stops
Learn `stop_loss_percent` and `take_profit_percent` per `symbol:strategy` profile. Exit rules already read from profile — ensure learning updates them from `TradeOutcome` feedback.

**Files:** `learning/src/profiles.rs`, `auto_trader/exits/position_exit_rules.rs`

### Priority 7: Scout Liquid Symbols, Not Observation-Only
Restrict scout symbol selection to liquid symbols (BTC, ETH, major pairs). Set `observation_only=false` so scouts generate real `TradeOutcome` feedback.

**Files:** `strategy/src/scouts/`, `auto_trader/profile_settings_adapter.rs`

### Priority 8: Time-Decay on Learned Parameters
Add exponential decay to learned parameter values and profile stats so the system adapts to regime change.

**Files:** `learning/src/profiles.rs`

---

## 4. Suggested Implementation Order

1. Priority 1 — Fix death spiral (nothing trades without this)
2. Priority 2 — Signal strength minimum (quick win, filters noise)
3. Priority 3 — Scout minimum samples (prevents noise-based promotion)
4. Priority 4 — Symbol-conditioned evidence (enables per-symbol learning)
5. Priority 5 — Regime-conditioned thresholds (biggest ROI lever)
6. Priority 6 — Per-symbol stops/targets (adapts to symbol volatility)
7. Priority 8 — Time-decay (keeps learning current)
7. Priority 7 — Scout liquid symbols (improves feedback quality)

---

## 5. Implementation Log

| Priority | Task | File(s) | Status |
|---|---|---|---|
| 1 | Walk-forward gate fix | `src/rust/strategy/src/lib.rs` | **DONE** |
| 2 | Signal strength minimum | `contracts.rs`, `parameter_catalog.rs`, evaluators | **DONE** |
| 3 | Scout min samples | `learning/src/feedback.rs`, `learning/src/lib.rs`, `learning/src/profiles.rs` | **DONE** |
| 4 | Symbol-conditioned evidence | `input.rs`, `lib.rs`, `cycle_entry.rs` | **Pending** |
| 5 | Regime-conditioned thresholds | `contracts.rs`, `parameter_catalog.rs`, evaluators, learning | **DONE** |
| 6 | Per-symbol stops/targets | `learning/src/profiles.rs`, `position_exit_rules.rs` | **DONE** |
| 7 | Scout liquid symbols | `scouts/service.rs` | **DONE** |
| 8 | Time-decay on learned params | `learning/src/profiles.rs` | **DONE** |

---

---



## Implementation Status

| Priority | Task | Status |
|---|---|---|
| 1 | Walk-forward gate fix (dead spiral prevention) | **DONE** |
| 2 | Signal strength minimum (noise filtering) | **DONE** |
| 3 | Scout minimum samples (noise prevention) | **DONE** |
| 4 | Symbol-conditioned evidence | **Pending** |
| 5 | Regime-conditioned thresholds | **DONE** |
| 6 | Per-symbol stops/targets | **DONE** |
| 7 | Scout liquid symbols | **DONE** |
| 8 | Time-decay on learned params | **DONE** |

---
*End of analysis. Proceeding to implementation, one priority at a time.*
- `insights.log`: Scouts testing on obscure/illiquid symbols (USDC-PLN, SHIB-USDT, XMSTR-USDT, HYPE-USDC), many `observation_only=true` (not actually trading)

### Priority 6 implementation note

Protective stop and target values are part of each strategy's parameter profile and are explored by paired scout comparisons. Learning promotes a changed risk parameter only after the configured minimum paired sample count and a positive one-sided confidence bound. AutoTrader reads the active profile at entry and stores the effective stop and target on the position; exit rules use those position values. Stop-loss exit-timing replays can additionally select a profile-and-market-context stop when enough replay evidence supports it. Post-exit observations tune take-profit distance per symbol, strategy, and direction after the minimum sample threshold.

### Priority 7 implementation note

Live and daily scout inputs are limited to BTC, ETH, and major USDT pairs; dated-carry scouts use the same allowlist. Horizon markouts remain observation-only measurements. Actual live scout probes use the trade-outcome path and are not converted into horizon observations.
