fn prepare_directional_signal_candidates(
    state: &AutoTraderState,
    dashboard: &Dashboard,
    now: u64,
    risk_appetite: f64,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
) -> PreparedSignals {
    let mut signals = dashboard.best.clone();
    for signal in &mut signals {
        let multiplier =
            longer_horizon_score_multiplier(signal.score, &dashboard.missed_opportunity_bands);
        if multiplier > 1.0 {
            signal.score *= multiplier;
        }
    }
    let neutral_direction_floor = NEUTRAL_SCORE_DIRECTION_FLOOR * risk_floor_scale(risk_appetite);
    let mut inferred_direction_count = 0;
    for signal in &mut signals {
        if signal.direction == "neutral"
            && signal.ranging_probability.unwrap_or(0.0) < NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY
            && signal.score.is_finite()
            && signal.score.abs() >= neutral_direction_floor
        {
            signal.direction = if signal.score > 0.0 {
                "bullish".into()
            } else {
                "bearish".into()
            };
            inferred_direction_count += 1;
        }
    }
    // Record borderline neutral scores as directional shadow probes only. These
    // are evaluated on every scan and never feed the order-selection path.
    for candidate in signals.iter().filter(|candidate| {
        !candidate.stale
            && candidate.direction == "neutral"
            && now.saturating_sub(candidate.timestamp / 1000) <= MAX_SIGNAL_AGE_SECONDS
            && candidate.score.is_finite()
            && candidate.score.abs() >= SHADOW_MIN_ABS_SCORE
            && candidate.score.abs() < neutral_direction_floor
            && candidate.ranging_probability.unwrap_or(0.0)
                < NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY
    }) {
        let mut shadow = candidate.clone();
        shadow.direction = if shadow.score > 0.0 {
            "bullish"
        } else {
            "bearish"
        }
        .into();
        let evidence = dashboard.walk_forward.iter().find(|item| {
            item.symbol == shadow.symbol && is_entry_evidence_horizon(item.horizon_seconds)
        });
        let payload = signal_audit_payload(
            &shadow,
            "decision",
            now.saturating_mul(1000),
            "skipped",
            Some(
                "No tradeable bias: shadow-only neutral score probe below live direction floor; no order submitted",
            ),
            Some(true),
            counterfactual_stop_loss_percent,
            counterfactual_take_profit_percent,
            evidence,
        );
        queue_auto_trader_audit(state, payload);
    }
    PreparedSignals {
        signals,
        inferred_direction_count,
        neutral_direction_floor,
    }
}

