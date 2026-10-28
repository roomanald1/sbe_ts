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
    StatusResponse {
        enabled: state.enabled.load(Ordering::Relaxed),
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
    }
}
