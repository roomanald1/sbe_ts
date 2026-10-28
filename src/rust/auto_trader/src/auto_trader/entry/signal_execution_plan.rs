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
    let configured_stop = if explicit_stop || per_strategy_history_is_mature {
        stop_parameter.unwrap_or(counterfactual_stop_loss_percent)
    } else {
        learned_exit_policy
            .map(|(stop, _)| stop)
            .unwrap_or(counterfactual_stop_loss_percent)
    };
    let configured_target = if explicit_target || per_strategy_history_is_mature {
        target_parameter.unwrap_or(counterfactual_take_profit_percent)
    } else {
        learned_exit_policy
            .map(|(_, target)| target)
            .unwrap_or(counterfactual_take_profit_percent)
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
    let stop_headroom_multiplier =
        if matches!(baseline_type, AutoOrderType::Vwap | AutoOrderType::Twap) {
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
    notional *= volatility_size_scale(configured_stop, effective_stop_loss_percent);
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
    let horizon_take_profit_percent =
        horizon_adjusted_take_profit_percent(signal, &dashboard.walk_forward, configured_target);
    let effective_take_profit_percent = confidence_adjusted_take_profit_percent(
        &settings.closed_trade_returns_percent,
        horizon_take_profit_percent.unwrap_or(configured_target),
    );
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
