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
    // Record borderline neutral scores as directional scout probes only. These
    // are evaluated on every scan and never feed the order-selection path.
    for candidate in signals.iter().filter(|candidate| {
        !candidate.stale
            && candidate.direction == "neutral"
            && now.saturating_sub(candidate.timestamp / 1000) <= MAX_SIGNAL_AGE_SECONDS
            && candidate.score.is_finite()
            && candidate.score.abs() >= SCOUT_PROBE_MIN_ABS_SCORE
            && candidate.score.abs() < neutral_direction_floor
            && candidate.ranging_probability.unwrap_or(0.0)
                < NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY
    }) {
        let mut scout = candidate.clone();
        scout.direction = if scout.score > 0.0 {
            "bullish"
        } else {
            "bearish"
        }
        .into();
        let evidence = entry_walk_forward_for_strategy(
            &dashboard.walk_forward,
            &scout.symbol,
            scout.strategy.as_deref(),
            now.saturating_mul(1000),
        );
        let payload = signal_audit_payload(
            &scout,
            "decision",
            now.saturating_mul(1000),
            "skipped",
            Some(
                "No tradeable bias: scout-only neutral score probe below live direction floor; no order submitted",
            ),
            Some(true),
            counterfactual_stop_loss_percent,
            counterfactual_take_profit_percent,
            evidence,
        );
        queue_auto_trader_audit(state, payload);
        if let Some(market) = dashboard.markets.get(&scout.symbol) {
            let bid = market
                .get("bids")
                .and_then(serde_json::Value::as_array)
                .and_then(|levels| levels.first())
                .and_then(|level| level.get("price"))
                .and_then(serde_json::Value::as_f64);
            let ask = market
                .get("asks")
                .and_then(serde_json::Value::as_array)
                .and_then(|levels| levels.first())
                .and_then(|level| level.get("price"))
                .and_then(serde_json::Value::as_f64);
            if let Some(entry_price) = bid
                .zip(ask)
                .map(|(bid, ask)| (bid + ask) * 0.5)
                .filter(|price| price.is_finite() && *price > 0.0)
            {
                let request = serde_json::json!({
                    "symbol": scout.symbol,
                    "timestamp": scout.timestamp,
                    "direction": scout.direction,
                    "entry_price": entry_price,
                });
                tokio::spawn(async move {
                    let _ = crate::service_rpc::insights_request(
                        "POST",
                        "/internal/signals/counterfactual",
                        Some(request),
                    ).await;
                });
            }
        }
    }
    PreparedSignals {
        signals,
        inferred_direction_count,
        neutral_direction_floor,
    }
}

