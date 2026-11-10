fn diagnostic_strategy_blocker(diagnostic: &StrategyDiagnostic) -> Option<StrategyBlocker> {
    let (symbol, reason) = if diagnostic.btc_gate_required
        && diagnostic.btc_gate_passed == Some(false)
    {
        let window = diagnostic
            .btc_window_days
            .map(|days| format!("SMA{days}"))
            .unwrap_or_else(|| "trend average".to_owned());
        (
            "BTC-USDT".to_owned(),
            match (diagnostic.btc_close, diagnostic.btc_sma) {
                (Some(close), Some(sma)) => format!(
                    "BTC trend gate not passed: close {close:.2} vs {window} {sma:.2}"
                ),
                _ => format!("BTC trend gate not passed ({window})"),
            },
        )
    } else {
        let failed = diagnostic.symbols.iter().find_map(|item| {
            if !item.evaluated || item.signal_active {
                return None;
            }
            if !item.eligible {
                return Some((
                    item.symbol.clone(),
                    "not eligible for this strategy".to_owned(),
                ));
            }
            if !item.primary_passed {
                let unit = item.primary_metric_unit.as_deref().unwrap_or("");
                let observed = item
                    .primary_metric_value
                    .map(|value| format!("{value:.2} {unit}"))
                    .unwrap_or_else(|| "unavailable".to_owned());
                return Some((
                    item.symbol.clone(),
                    format!(
                        "{} not met: observed {}, threshold {:.2} {}; distance {:.2}%",
                        item.primary_rule,
                        observed,
                        item.primary_level,
                        unit,
                        item.primary_distance_percent
                    ),
                ));
            }
            if item.secondary_passed == Some(false) {
                return Some((
                    item.symbol.clone(),
                    format!(
                        "{} not met (distance {:.2}%)",
                        item.secondary_rule.as_deref().unwrap_or("secondary rule"),
                        item.secondary_distance_percent.unwrap_or_default()
                    ),
                ));
            }
            None
        })?;
        failed
    };
    Some(StrategyBlocker {
        strategy: diagnostic.strategy.clone(),
        symbol,
        reason,
        timestamp_ms: diagnostic.as_of_ms,
    })
}

#[cfg(test)]
mod strategy_blocker_tests {
    use super::*;
    use strategies::daily::StrategySymbolDiagnostic;

    #[test]
    fn reports_the_failed_strategy_rule_and_observed_metric() {
        let diagnostic = StrategyDiagnostic {
            strategy: "trend-50-200".to_owned(),
            as_of_ms: 123,
            btc_close: None,
            btc_sma: None,
            btc_distance_percent: None,
            btc_gate_required: false,
            btc_gate_passed: None,
            btc_window_days: None,
            symbols: vec![StrategySymbolDiagnostic {
                symbol: "ETH-USDT".to_owned(),
                close: 90.0,
                eligible: true,
                primary_rule: "price above SMA50".to_owned(),
                primary_level: 100.0,
                primary_distance_percent: -10.0,
                primary_metric_value: Some(90.0),
                primary_metric_unit: Some("price".to_owned()),
                primary_passed: false,
                secondary_rule: None,
                secondary_level: None,
                secondary_distance_percent: None,
                secondary_passed: None,
                evaluated: true,
                signal_active: false,
                leader_gap_percent: None,
                relative_strength_gap_pp: None,
                rsi_exit_level: None,
                maximum_hold_days: None,
                trend_return_windows: None,
            }],
        };

        let blocker = diagnostic_strategy_blocker(&diagnostic).unwrap();
        assert_eq!(blocker.strategy, "trend-50-200");
        assert_eq!(blocker.symbol, "ETH-USDT");
        assert!(blocker.reason.contains("observed 90.00 price"));
        assert!(blocker.reason.contains("threshold 100.00 price"));
        assert_eq!(blocker.timestamp_ms, 123);
    }
}

fn status(state: &AutoTraderState, settings: &Settings) -> StatusResponse {
    if let Ok(mut summary) = state.gap_summary.lock() {
        let day = utc_day();
        if summary.utc_day != day {
            *summary = GapSummary {
                utc_day: day,
                ..GapSummary::default()
            };
        }
    }
    let mut latest_strategy_blockers = state
        .latest_strategy_blockers
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let diagnostics = {
        state
            .strategy_diagnostic
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    };
    for diagnostic in diagnostics.iter() {
        let existing = latest_strategy_blockers.get(&diagnostic.strategy);
        if existing.is_some_and(|blocker| blocker.timestamp_ms > diagnostic.as_of_ms) {
            continue;
        }
        if let Some(blocker) = diagnostic_strategy_blocker(diagnostic) {
            latest_strategy_blockers.insert(diagnostic.strategy.clone(), blocker);
        } else {
            latest_strategy_blockers.remove(&diagnostic.strategy);
            }
    }
    StatusResponse {
        enabled: state.enabled.load(Ordering::Relaxed),
        audit_healthy: state.audit_health.load(Ordering::Relaxed) == 3
            && !state.audit_failure_latched.load(Ordering::Relaxed),
        audit_failure_latched: state.audit_failure_latched.load(Ordering::Relaxed),
        decision_audit_queue_depth: state.audit_tx.depth(),
        execution_audit_queue_depth: state.execution_audit_tx.depth(),
        audit_coalesced_decisions: state.audit_tx.coalesced.load(Ordering::Relaxed),
        audit_dropped_events: state.audit_tx.dropped.load(Ordering::Relaxed)
            + state.execution_audit_tx.dropped.load(Ordering::Relaxed),
        strategy: settings.strategies.first().copied().unwrap_or_default().as_str().to_owned(),
        strategies: settings.strategies.iter().map(|strategy| strategy.as_str().to_owned()).collect(),
        signal_strategies_enabled: settings.signal_strategies_enabled,
        max_order_notional: settings.max_order_notional,
        daily_notional_limit: settings.daily_notional_limit,
        daily_realized_loss_limit: settings.daily_realized_loss_limit,
        exploration_percent: settings.exploration_percent,
        risk_appetite: settings.risk_appetite,
        max_open_positions: settings.max_open_positions,
        max_intraday_open_positions: settings.max_intraday_open_positions,
        max_daily_open_positions: settings.max_daily_open_positions,
        intraday_notional_limit: settings.intraday_notional_limit,
        daily_strategy_notional_limit: settings.daily_strategy_notional_limit,
        daily_strategy_budgets: settings
            .strategies
            .iter()
            .copied()
            .filter(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
            .map(|strategy| {
                // Match the recent-outcome profile used to calculate this multiplier and cap.
                let learning = strategy_learning_stats_across_symbols(settings, strategy);
                StrategyBudgetStatus {
                    strategy: strategy.as_str().to_owned(),
                    base_budget: daily_strategy_budget(settings, strategy.as_str()),
                    effective_budget: daily_strategy_effective_budget(settings, strategy.as_str()),
                    used_notional: allocated_notional_for_strategy(settings, strategy.as_str()),
                    learning_multiplier: learned_strategy_size_multiplier(&learning),
                    learning_samples: learning.samples,
                    positive_rate: (learning.samples > 0)
                        .then_some(learning.positive as f64 / learning.samples as f64),
                    realized_pnl: learning.realized_pnl,
                }
            })
            .collect(),
        intraday_strategy_budgets: settings
            .strategies
            .iter()
            .copied()
            .filter(|strategy| strategy.is_live_insight() && !strategy.is_legacy_composite())
            .map(|strategy| {
                let learning = strategy_learning_stats_across_symbols(settings, strategy);
                StrategyBudgetStatus {
                    strategy: strategy.as_str().to_owned(),
                    base_budget: intraday_strategy_budget(settings, strategy.as_str()),
                    effective_budget: intraday_strategy_effective_budget(settings, strategy.as_str()),
                    used_notional: allocated_notional_for_strategy(settings, strategy.as_str()),
                    learning_multiplier: intraday_strategy_size_multiplier(settings, strategy),
                    learning_samples: learning.samples,
                    positive_rate: (learning.samples > 0)
                        .then_some(learning.positive as f64 / learning.samples as f64),
                    realized_pnl: learning.realized_pnl,
                }
            })
            .collect(),
        intraday_used_notional: allocated_notional_for_category(&settings, Some(StrategyCategory::Intraday)),
        daily_strategy_used_notional: allocated_notional_for_category(&settings, Some(StrategyCategory::Daily)),
        max_entries_per_auto_position: settings.max_entries_per_auto_position,
        stop_loss_percent: settings.stop_loss_percent,
        take_profit_percent: settings.take_profit_percent,
        vwap_duration_minutes: settings.vwap_duration_minutes,
        vwap_participation_percent: settings.vwap_participation_percent,
        limit_offset_bps: settings.limit_offset_bps,
        trigger_offset_bps: settings.trigger_offset_bps,
        trailing_distance_percent: settings.trailing_distance_percent,
        closed_trade_window: settings.closed_trade_window,
        minimum_scout_parameter_samples: settings.minimum_live_scout_parameter_samples,
        used_notional_today: settings.used_notional_today,
        available_notional: (settings.daily_notional_limit - settings.used_notional_today).max(0.0),
        last_decision: settings.last_decision.clone(),
        last_decision_at: settings.last_decision_at,
        decisions_since_trade: state
            .gap_summary
            .lock()
            .map(|summary| summary.decisions)
            .unwrap_or_default(),
        gap_reasons: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .reasons
                    .iter()
                    .map(|(reason, count)| GapReason {
                        reason: reason.clone(),
                        count: *count,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        gate_hits: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .gate_hits
                    .iter()
                    .map(|(gate, count)| GateHit {
                        gate: gate.clone(),
                        count: *count,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        stale_signal_candidates_total: state
            .stale_signal_candidates_total
            .load(Ordering::Relaxed),
        stale_signal_cycles_total: state
            .stale_signal_cycles_total
            .load(Ordering::Relaxed),
        symbol_gate_hits: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .symbol_gate_hits
                    .iter()
                    .map(|((symbol, direction, gate), count)| SymbolGateHit {
                        symbol: symbol.clone(),
                        direction: direction.clone(),
                        gate: gate.clone(),
                        count: count.count,
                        last_detail: count.last_detail.clone(),
                        distance_to_pass: count.distance_to_pass,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        latest_signal_diagnostic: state
            .gap_summary
            .lock()
            .map(|summary| summary.latest_detail.clone())
            .unwrap_or_default(),
        strategy_diagnostics: state
            .strategy_diagnostic
            .lock()
            .ok()
            .map(|diagnostics| diagnostics.clone())
            .unwrap_or_default(),
        latest_strategy_blockers: latest_strategy_blockers.into_values().collect(),
    }
}
