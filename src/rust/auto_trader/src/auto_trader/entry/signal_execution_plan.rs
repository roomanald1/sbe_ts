fn signal_strategy_order_size_multiplier(
    strategy: Option<Strategy>,
    decision: Option<&serde_json::Value>,
) -> Result<f64, String> {
    match strategy {
        Some(Strategy::RiskAdjustedMonthlyMomentum) => decision
            .and_then(|decision| decision.get("strategy_order_size_multiplier"))
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0 && *value <= 1.0)
            .ok_or_else(|| "Monthly momentum signal is missing a valid volatility size multiplier".to_owned()),
        _ => Ok(1.0),
    }
}

struct SignalExecutionPlan {
    effective_stop_loss_percent: f64,
    effective_take_profit_percent: f64,
    horizon_take_profit_percent: Option<f64>,
    notional: f64,
    minimum_book_order: f64,
    stop_headroom_multiplier: f64,
    deep_book: Option<DeepBookAssessment>,
    deep_book_ms: f64,
    side: &'static str,
    order_type: AutoOrderType,
    profile: usize,
    execution_params: ExecutionParameters,
    learning_context: String,
}

async fn prepare_signal_execution_plan(
    state: &AutoTraderState,
    mut settings: tokio::sync::MutexGuard<'_, Settings>,
    signal: &Signal,
    dashboard: &Dashboard,
    market: &serde_json::Value,
    now_ms: u64,
    quote_rate: f64,
    exploratory: bool,
    scout: bool,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
) -> Result<SignalExecutionPlan, String> {
    let strategy = signal.strategy.as_deref().and_then(Strategy::from_name);
    let strategy_profile =
        strategy.map(|strategy| strategy_parameter_profile(&settings, &signal.symbol, strategy));
    let strategy_order_size_multiplier =
        signal_strategy_order_size_multiplier(strategy, signal.strategy_decision.as_ref())?;
    let stop_parameter = strategy_profile
        .as_ref()
        .and_then(|profile| strategy_profile_value(profile, "stop_loss_percent"));
    let target_parameter = strategy_profile
        .as_ref()
        .and_then(|profile| strategy_profile_value(profile, "take_profit_percent"));
    let explicit_stop = strategy.is_some_and(|kind| {
        settings
            .strategy_parameter_overrides
            .contains_key(&strategy_parameter_key(
                &signal.symbol,
                kind,
                "stop_loss_percent",
            ))
    });
    let explicit_target = strategy.is_some_and(|kind| {
        settings
            .strategy_parameter_overrides
            .contains_key(&strategy_parameter_key(
                &signal.symbol,
                kind,
                "take_profit_percent",
            ))
    });
    let learned_exit_policy = learned_exit_policy(signal, &dashboard.walk_forward);
    let per_strategy_history_is_mature = strategy_profile
        .as_ref()
        .is_some_and(|profile| profile.learning_samples >= STRATEGY_PARAMETER_MIN_SAMPLES);
    let baseline_stop = stop_parameter.unwrap_or(counterfactual_stop_loss_percent);
    let baseline_target = target_parameter.unwrap_or(counterfactual_take_profit_percent);
    let bounded_learned_policy = learned_exit_policy.map(|(stop, target, samples)| {
        let widening_tiers = (samples / MIN_VALIDATED_EXIT_POLICY_SAMPLES)
            .clamp(1, MAX_STOP_WIDENING_TIERS) as f64;
        let max_widening = STOP_WIDENING_PER_VALIDATION_TIER_PERCENT * widening_tiers;
        let stop = if stop > baseline_stop {
            stop.min(baseline_stop + max_widening)
        } else {
            stop
        };
        (stop, target)
    });
    // Respect either manual exit override. Otherwise a validated walk-forward
    // policy can widen the stop, including after the strategy profile matures,
    // only when held-out results beat the conservative baseline.
    let explicit_exit_override = explicit_stop || explicit_target;
    let validated_wider_policy = (!explicit_exit_override)
        .then_some(bounded_learned_policy)
        .flatten()
        .filter(|(stop, _)| *stop > baseline_stop + 1e-9);
    let configured_stop = if explicit_exit_override {
        baseline_stop
    } else if let Some((stop, _)) = validated_wider_policy {
        stop
    } else if per_strategy_history_is_mature {
        baseline_stop
    } else {
        bounded_learned_policy
            .map(|(stop, _)| stop)
            .unwrap_or(baseline_stop)
    };
    let configured_target = if explicit_exit_override {
        baseline_target
    } else if let Some((_, target)) = validated_wider_policy {
        target
    } else if per_strategy_history_is_mature {
        baseline_target
    } else {
        bounded_learned_policy
            .map(|(_, target)| target)
            .unwrap_or(baseline_target)
    };
    let learned_or_configured_stop = configured_stop;
    let effective_stop_loss_percent = volatility_adjusted_stop_loss_percent(
        learned_or_configured_stop,
        signal.components.volatility_1m_percent,
    );
    let baseline_type = choose_order_type(&signal, settings.risk_appetite);
    let risk_fraction = if settings.risk_limited_sizing {
        0.25
    } else {
        1.0
    };
    let risk_size_scale = risk_size_scale(settings.risk_appetite);
    let exploration_cap = if scout {
        settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION * risk_size_scale
    } else if exploratory {
        settings.max_order_notional * (settings.exploration_percent / 100.0) * risk_size_scale
    } else {
        settings.max_order_notional
    };
    let risk_order_cap = exploration_cap
        .min(settings.max_order_notional)
        .min(settings.max_order_notional * risk_fraction);
    let stop_headroom_multiplier = if strategy.is_some_and(|kind| !kind.uses_protective_stops()) {
        1.0
    } else if matches!(baseline_type, AutoOrderType::Vwap | AutoOrderType::Twap) {
        2.0
    } else {
        1.0
    };
    let category = strategy_category(signal.strategy.as_deref());
    let total_allocated = allocated_notional(&settings);
    let category_allocated = allocated_notional_for_category(&settings, Some(category));
    let remaining_budget = (settings.daily_notional_limit - total_allocated)
        .min(category_notional_limit(&settings, category) - category_allocated)
        .max(0.0);
    let mut notional = risk_order_cap.min(remaining_budget / stop_headroom_multiplier);
    // Size against the unlearned stop so a wider validated stop does not
    // silently increase the dollar risk of the position.
    if strategy.is_some_and(|kind| kind.uses_protective_stops()) {
        notional *= volatility_size_scale(baseline_stop, effective_stop_loss_percent);
    }
    notional *= strategy_order_size_multiplier;
    if notional <= 0.0 {
        return Err("Strategy category or total notional capacity is fully allocated".into());
    }
    if exploratory {
        if !settings.last_risk_note.is_empty() {
            settings.last_risk_note.push_str(" · ");
        }
        settings
            .last_risk_note
            .push_str(if scout {
                "scout order capped at 5% of max order; no sufficiently negative same-side walk-forward evidence"
            } else {
                "small paper-exploration size; no reliable same-side walk-forward edge yet"
            });
    }
    let baseline_params = ExecutionParameters {
        vwap_duration_minutes: settings.vwap_duration_minutes,
        vwap_participation_rate: settings.vwap_participation_percent / 100.0,
        limit_offset_bps: settings.limit_offset_bps,
        trigger_offset_bps: settings.trigger_offset_bps,
        trailing_distance_percent: settings.trailing_distance_percent,
    };
    let minimum_book_order = if scout {
        MIN_SCOUT_DEEP_BOOK_ORDER_NOTIONAL
    } else {
        MIN_DEEP_BOOK_ORDER_NOTIONAL
    };
    let deep_book_started = Instant::now();
    let deep_book = assess_deep_book(
        market,
        &signal.symbol,
        signal.direction == "bullish",
        notional * quote_rate,
        minimum_book_order * quote_rate,
    );
    let deep_book_ms = deep_book_started.elapsed().as_secs_f64() * 1000.0;
    let (effective_take_profit_percent, horizon_take_profit_percent) =
        if validated_wider_policy.is_some() {
            // Keep the target paired with the wider stop that passed walk-forward
            // validation; unrelated target heuristics would invalidate that result.
            (configured_target, None)
        } else {
            let horizon_take_profit_percent =
                horizon_adjusted_take_profit_percent(signal, &dashboard.walk_forward, configured_target);
            let effective_take_profit_percent = confidence_adjusted_take_profit_percent(
                &settings.closed_trade_returns_percent,
                horizon_take_profit_percent.unwrap_or(configured_target),
            );
            (effective_take_profit_percent, horizon_take_profit_percent)
        };
    let context = market_context(&signal);
    decay_learning_map(&mut settings.learning, now_ms);
    let learned_type = if scout {
        baseline_type
    } else if let Some(order_type) = adapt_order_type(
        baseline_type,
        &context,
        &settings.learning,
        settings.risk_appetite,
    ) {
        order_type
    } else {
        return Err(
            "Recent closed auto-trades show no profitable order type with enough samples".into(),
        );
    };
    let side_depth = deep_book
        .map(|assessment| {
            if signal.direction == "bullish" {
                assessment.ask_depth_10bps
            } else {
                assessment.bid_depth_10bps
            }
        })
        .unwrap_or(0.0);
    let depth_ratio = if side_depth > 0.0 {
        notional * quote_rate / side_depth
    } else {
        0.0
    };
    let (schedule_depth_threshold, schedule_threshold_samples) =
        learned_schedule_depth_threshold(&settings.learning, &context);
    let depth_ratio_bucket = schedule_depth_ratio_bucket(depth_ratio);
    let order_type = score_execution_order_type(
        signal,
        settings.risk_appetite,
        notional * quote_rate,
        baseline_type,
        learned_type,
        deep_book,
        schedule_depth_threshold,
    );
    let profile = choose_parameter_profile(order_type, &context, &settings.learning, exploratory);
    let execution_params = parameter_profile(baseline_params, profile);
    *state.latest_execution_parameters.lock().await = Some(LatestExecutionParameters {
        timestamp: now_ms,
        symbol: signal.symbol.clone(),
        context: context.clone(),
        baseline_order_type: format!("{baseline_type:?}"),
        selected_order_type: format!("{order_type:?}"),
        profile,
        vwap_duration_minutes: execution_params.vwap_duration_minutes,
        vwap_participation_percent: execution_params.vwap_participation_rate * 100.0,
        schedule_depth_threshold,
        proposed_depth_ratio: depth_ratio,
        schedule_threshold_samples,
        limit_offset_bps: execution_params.limit_offset_bps,
        trigger_offset_bps: execution_params.trigger_offset_bps,
        trailing_distance_percent: execution_params.trailing_distance_percent,
    });
    let learning_context = if scout {
        format!("scout:{context}:P{profile}:depth:{depth_ratio_bucket}")
    } else {
        format!("{context}:P{profile}:depth:{depth_ratio_bucket}")
    };
    drop(settings);
    let side = if signal.direction == "bullish" {
        "buy"
    } else {
        "sell"
    };
    Ok(SignalExecutionPlan {
        effective_stop_loss_percent,
        effective_take_profit_percent,
        horizon_take_profit_percent,
        notional,
        minimum_book_order,
        stop_headroom_multiplier,
        deep_book,
        deep_book_ms,
        side,
        order_type,
        profile,
        execution_params,
        learning_context,
    })
}


#[cfg(test)]
mod risk_adjusted_monthly_momentum_sizing_tests {
    use super::*;

    #[test]
    fn strategy_uses_volatility_multiplier_and_fails_closed_when_missing() {
        let strategy = Some(Strategy::RiskAdjustedMonthlyMomentum);
        assert_eq!(signal_strategy_order_size_multiplier(strategy, Some(&serde_json::json!({
            "strategy_order_size_multiplier": 0.4
        }))).unwrap(), 0.4);
        assert!(signal_strategy_order_size_multiplier(strategy, None).is_err());
        assert!(signal_strategy_order_size_multiplier(strategy, Some(&serde_json::json!({
            "strategy_order_size_multiplier": 2.0
        }))).is_err());
        assert_eq!(signal_strategy_order_size_multiplier(Some(Strategy::Donchian55Btc200), None).unwrap(), 1.0);
    }
}
