fn score_execution_order_type(
    signal: &Signal,
    risk_appetite: f64,
    notional: f64,
    baseline: AutoOrderType,
    learned: AutoOrderType,
    book: Option<DeepBookAssessment>,
    schedule_depth_threshold: f64,
) -> AutoOrderType {
    let side_depth = book.map(|assessment| {
        if signal.direction == "bullish" { assessment.ask_depth_10bps } else { assessment.bid_depth_10bps }
    }).unwrap_or(0.0);
    let depth_ratio = if side_depth > 0.0 { notional / side_depth } else { 0.0 };
    // Large parent orders use the scheduler to work child slices against volume.
    let large_order = depth_ratio >= schedule_depth_threshold;
    let urgency = (signal.components.momentum_10s_percent.unwrap_or(0.0).abs() / 0.30)
        .max(signal.components.pressure_without_price_move.unwrap_or(0.0).abs())
        .clamp(0.0, 1.0);
    let ranging = signal.ranging_probability.unwrap_or(0.0).clamp(0.0, 1.0);
    let trend = (signal.components.trend_5m_volatility_units.unwrap_or(0.0).abs() / 3.0).clamp(0.0, 1.0);
    let fee_bps = configured_taker_fee_rate() * 10_000.0;
    let impact_bps = book.map(|assessment| assessment.impact_bps).filter(|value| value.is_finite()).unwrap_or(0.0);
    let scaled_impact_bps = if side_depth > 0.0 {
        impact_bps * (notional / (side_depth * DEEP_BOOK_MAX_PARTICIPATION).max(1.0)).clamp(0.25, 4.0)
    } else { impact_bps };
    let candidates = [baseline, learned, AutoOrderType::Vwap, AutoOrderType::Twap,
        AutoOrderType::PostOnlyLimit, AutoOrderType::Gtd, AutoOrderType::Ioc, AutoOrderType::Market];
    let mut best = learned;
    let mut best_score = f64::NEG_INFINITY;
    for (index, candidate) in candidates.into_iter().enumerate() {
        if candidates[..index].contains(&candidate) { continue; }
        if large_order && !matches!(candidate, AutoOrderType::Vwap | AutoOrderType::Twap) { continue; }
        let maker = matches!(candidate, AutoOrderType::PostOnlyLimit | AutoOrderType::Gtd);
        let impact_multiplier = match candidate {
            AutoOrderType::Vwap | AutoOrderType::Twap => 0.45,
            AutoOrderType::PostOnlyLimit | AutoOrderType::Gtd => 0.10,
            _ => 1.0,
        };
        let expected_cost_bps = (if maker { configured_fee_rates_maker_bps() } else { fee_bps })
            + scaled_impact_bps * impact_multiplier
            + if maker { 0.0 } else { signal.components.aggressor_cost_bps.unwrap_or(0.0).abs() * 0.25 };
        let mut score = if candidate == baseline { 1.0 } else { 0.0 };
        if candidate == learned { score += 0.35; }
        score += match candidate {
            AutoOrderType::Vwap => signal.strength.clamp(0.0, 1.0) * 0.6,
            AutoOrderType::Twap => ranging * 0.7,
            AutoOrderType::PostOnlyLimit | AutoOrderType::Gtd => ranging * 0.8 - urgency * 0.7,
            AutoOrderType::Market | AutoOrderType::Ioc => urgency * 0.7 + trend * 0.2,
            _ => 0.0,
        };
        if matches!(candidate, AutoOrderType::Vwap | AutoOrderType::Twap) {
            score += ((depth_ratio - MIN_SCHEDULE_DEPTH_RATIO)
                / (schedule_depth_threshold - MIN_SCHEDULE_DEPTH_RATIO).max(0.01))
                .clamp(0.0, 1.0) * 2.0;
            score -= urgency * (risk_appetite / 100.0) * 0.7;
        }
        // Cost is a meaningful score component, but its bounded weight cannot
        // override a strong signal/regime fit by itself.
        score -= (expected_cost_bps / 10.0).clamp(0.0, 2.0) * 0.35;
        if score > best_score { best = candidate; best_score = score; }
    }
    best
}


fn strategy_post_stop_cooldown_ms(strategy: &str, orders: &[TraderOrder]) -> u64 {
    let outcomes = orders
        .iter()
        .rev()
        .filter(|order| {
            order.strategy.as_deref() == Some(strategy)
                && matches!(
                    order.purpose.as_str(),
                    "stop-loss" | "take-profit" | "profit-protection" | "shadow-profit-protection" | "strategy-disabled" | "research-regime-exit"
                )
                && order.status == "filled"
                && order.filled_quantity > 0.0
                && order.realized_pnl.is_finite()
        })
        .take(POST_STOP_OUTCOME_WINDOW)
        .collect::<Vec<_>>();

    if outcomes.len() < MIN_POST_STOP_OUTCOMES_FOR_ADAPTATION {
        return DEFAULT_POST_STOP_REENTRY_COOLDOWN_MS;
    }

    let win_rate = outcomes
        .iter()
        .filter(|order| order.realized_pnl > 0.0)
        .count() as f64
        / outcomes.len() as f64;
    let loss_pressure = ((0.55 - win_rate) / 0.35).clamp(0.0, 1.0);
    MIN_POST_STOP_REENTRY_COOLDOWN_MS
        + ((MAX_POST_STOP_REENTRY_COOLDOWN_MS - MIN_POST_STOP_REENTRY_COOLDOWN_MS) as f64
            * loss_pressure) as u64
}
