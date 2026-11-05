fn recent_net_pnl_edge(outcomes: &LearningStats) -> Option<f64> {
    if outcomes.recent_trade_pnls.is_empty() {
        return None;
    }
    let now_ms = current_time_ms();
    let (net, gross, effective_samples) = outcomes.recent_trade_pnls.iter().fold(
        (0.0, 0.0, 0.0),
        |(net, gross, samples), (timestamp, pnl)| {
            let age_ms = now_ms.saturating_sub(*timestamp);
            let weight = 0.5_f64.powf(age_ms as f64 / LEARNING_HALF_LIFE_MS as f64);
            (net + pnl * weight, gross + pnl.abs() * weight, samples + weight)
        },
    );
    if gross <= 1e-12 {
        return Some(0.0);
    }
    let confidence = effective_samples / (effective_samples + 2.0);
    Some((net / gross * confidence).clamp(-1.0, 1.0))
}

fn learned_strategy_size_multiplier(outcomes: &LearningStats) -> f64 {
    if outcomes.samples == 0 || outcomes.positive > outcomes.samples {
        return 1.0;
    }
    let edge = recent_net_pnl_edge(outcomes)
        .unwrap_or_else(|| confident_win_rate_edge(outcomes.positive, outcomes.samples));
    // Net outcomes include allocated fees. This reduces size for frequent-small-
    // win strategies whose larger losses make the recent book unprofitable.
    let target = if edge < 0.0 {
        1.0 + edge * (1.0 - POOR_STRATEGY_SIZE_MULTIPLIER)
    } else {
        1.0 + edge * (MAX_LEARNED_STRATEGY_SIZE_MULTIPLIER - 1.0)
    };
    target.clamp(POOR_STRATEGY_SIZE_MULTIPLIER, MAX_LEARNED_STRATEGY_SIZE_MULTIPLIER)
}

fn strategy_learning_stats_across_symbols(settings: &Settings, strategy: Strategy) -> LearningStats {
    settings.learned_profiles.values()
        .filter(|profile| profile.strategy == strategy.as_str())
        .fold(LearningStats::default(), |mut total, profile| {
            total.samples = total.samples.saturating_add(profile.learning_samples);
            total.positive = total.positive.saturating_add(profile.positive_samples);
            total.realized_pnl += profile.realized_pnl;
            total
        })
}

fn daily_strategy_effective_budget(settings: &Settings, strategy: &str) -> f64 {
    let base_budget = daily_strategy_budget(settings, strategy);
    let Some(kind) = Strategy::from_name(strategy)
        .filter(|kind| !kind.is_live_insight() && !kind.is_dated_carry())
    else {
        return 0.0;
    };
    base_budget * settings.learned_strategy_size_multipliers
        .get(kind.as_str()).copied().unwrap_or(1.0)
}

fn intraday_strategy_size_multiplier(settings: &Settings, strategy: Strategy) -> f64 {
    settings.learned_strategy_size_multipliers
        .get(strategy.as_str()).copied().unwrap_or(1.0).min(1.0)
}

fn signal_weighted_strategy_budget(
    settings: &Settings,
    dashboard: &Dashboard,
    strategy: Strategy,
    category: StrategyCategory,
    now_ms: u64,
) -> f64 {
    let (enabled, pool, configured) = match category {
        StrategyCategory::Daily => (
            settings.strategies.iter().copied()
                .filter(|kind| !kind.is_live_insight() && !kind.is_dated_carry())
                .collect::<Vec<_>>(),
            settings.daily_strategy_notional_limit,
            !settings.daily_strategy_budgets.is_empty(),
        ),
        StrategyCategory::Intraday => (
            settings.strategies.iter().copied()
                .filter(|kind| kind.is_live_insight() && !kind.is_legacy_composite())
                .collect::<Vec<_>>(),
            settings.intraday_notional_limit,
            !settings.intraday_strategy_budgets.is_empty(),
        ),
    };
    if configured || enabled.is_empty() || !enabled.contains(&strategy) {
        return match category {
            StrategyCategory::Daily => daily_strategy_effective_budget(settings, strategy.as_str()),
            StrategyCategory::Intraday => intraday_strategy_effective_budget(settings, strategy.as_str()),
        };
    }

    // Half the pool is an equal opportunity floor. The rest follows current
    // signal confidence, expected opportunity frequency, and learned net results.
    const GUARANTEED_SHARE: f64 = 0.5;
    let floor = pool * GUARANTEED_SHARE / enabled.len() as f64;
    let mut weights = BTreeMap::<String, f64>::new();
    for signal in dashboard.best.iter().filter(|signal| {
        signal.strategy_candidate
            && !signal.stale
            && matches!(signal.direction.as_str(), "bullish" | "bearish")
            && now_ms.saturating_sub(signal.timestamp) <= signal_max_age_seconds(signal) * 1_000
    }) {
        let Some(kind) = signal.strategy.as_deref().and_then(Strategy::from_name)
            .filter(|kind| enabled.contains(kind)) else { continue; };
        let learning = match category {
            StrategyCategory::Daily => settings.learned_strategy_size_multipliers
                .get(kind.as_str()).copied().unwrap_or(1.0),
            StrategyCategory::Intraday => intraday_strategy_size_multiplier(settings, kind),
        };
        let confidence = if signal.strength.is_finite() { signal.strength.clamp(0.0, 1.0) } else { 0.0 };
        *weights.entry(kind.as_str().to_owned()).or_default() += confidence * learning;
    }
    // sqrt(count) in the aggregate gives genuinely frequent strategies more
    // room while keeping a broad signal universe from overwhelming the floor.
    let weights = weights.into_iter().map(|(kind, sum)| {
        let count = dashboard.best.iter().filter(|signal| {
            signal.strategy.as_deref() == Some(kind.as_str())
                && signal.strategy_candidate && !signal.stale
                && matches!(signal.direction.as_str(), "bullish" | "bearish")
                && now_ms.saturating_sub(signal.timestamp) <= signal_max_age_seconds(signal) * 1_000
        }).count().max(1) as f64;
        (kind, sum / count.sqrt())
    }).collect::<BTreeMap<_, _>>();
    let total_weight = weights.values().sum::<f64>();
    let variable = if total_weight > 0.0 {
        (pool - floor * enabled.len() as f64) * weights.get(strategy.as_str()).copied().unwrap_or(0.0) / total_weight
    } else { 0.0 };
    (floor + variable).max(0.0)
}

fn intraday_strategy_effective_budget(settings: &Settings, strategy: &str) -> f64 {
    let base_budget = intraday_strategy_budget(settings, strategy);
    let Some(kind) = Strategy::from_name(strategy).filter(|kind| kind.is_live_insight()) else {
        return 0.0;
    };
    base_budget * intraday_strategy_size_multiplier(settings, kind)
}

fn signal_strategy_order_size_multiplier(
    strategy: Option<Strategy>,
    decision: Option<&serde_json::Value>,
) -> Result<f64, String> {
    match strategy {
        Some(Strategy::RiskAdjustedMonthlyMomentum | Strategy::Donchian20Btc200Hold3 | Strategy::RiskAdjustedTrendPortfolio) => decision
            .and_then(|decision| decision.get("strategy_order_size_multiplier"))
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0 && *value <= 1.0)
            .ok_or_else(|| "Research strategy signal is missing a valid size multiplier".to_owned()),
        _ => Ok(1.0),
    }
}

fn signal_agreement_size_multiplier(
    strategy: Option<Strategy>,
    decision: Option<&serde_json::Value>,
) -> Result<f64, String> {
    let Some(strategy) = strategy
        .filter(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
    else {
        return Ok(1.0);
    };
    let multiplier = decision
        .and_then(|decision| decision.get("strategy_agreement_size_multiplier"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(1.0);
    if multiplier.is_finite() && (1.0..=1.2).contains(&multiplier) {
        Ok(multiplier)
    } else {
        Err(format!(
            "{} has an invalid strategy agreement size multiplier",
            strategy.as_str()
        ))
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

async fn execution_learning_decision(
    client: &reqwest::Client,
    context: &str,
    baseline: AutoOrderType,
    signal: &Signal,
    risk_appetite: f64,
    allow_adaptation: bool,
    exploratory: bool,
    learning: &BTreeMap<String, ExecutionLearningStats>,
    import_legacy_learning: bool,
) -> (strategies::ExecutionLearningResponse, bool) {
    let order_types = ALL_ORDER_TYPES.iter().filter_map(|kind| {
        serde_json::to_value(kind).ok()?.as_str().map(str::to_owned)
    }).collect::<Vec<_>>();
    let explore_profile_order_types = ALL_ORDER_TYPES.iter().filter(|kind| {
        exploratory || should_explore_execution_profile(signal, **kind)
    }).filter_map(|kind| serde_json::to_value(kind).ok()?.as_str().map(str::to_owned)).collect();
    let request = strategies::ExecutionLearningRequest {
        context: context.to_owned(),
        baseline_order_type: serde_json::to_value(baseline).ok()
            .and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default(),
        order_types,
        profile_count: PARAMETER_PROFILE_COUNT,
        risk_appetite,
        allow_order_type_adaptation: allow_adaptation,
        explore_profile_order_types,
        learning: if import_legacy_learning {
            learning.iter().map(|(key, stats)| (key.clone(), strategies::ExecutionLearningStat {
                samples: stats.samples,
                filled_samples: stats.filled_samples,
                fill_rate_total: stats.fill_rate_total,
                cost_notional: stats.cost_notional,
                all_in_cost_notional_bps: stats.all_in_cost_notional_bps,
                updated_at_ms: stats.updated_at_ms,
            })).collect()
        } else { BTreeMap::new() },
    };
    let base = std::env::var("LEARNING_SERVICE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3010".into());
    let result = client.post(format!("{}/execution-decision", base.trim_end_matches('/')))
        .timeout(std::time::Duration::from_millis(250))
        .json(&request)
        .send().await
        .map_err(|error| error.to_string())
        .and_then(|response| response.error_for_status().map_err(|error| error.to_string()));
    let response = match result {
        Ok(response) => response.json::<strategies::ExecutionLearningResponse>().await.ok(),
        Err(error) => {
            crate::service_log!("WARN", "Learning service unavailable for execution profile: {error}");
            None
        }
    };
    let imported = response.is_some() && import_legacy_learning;
    (response.unwrap_or_else(|| strategies::ExecutionLearningResponse {
        learned_order_type: Some(request.baseline_order_type),
        schedule_depth_threshold: DEFAULT_SCHEDULE_DEPTH_RATIO,
        schedule_threshold_samples: 0,
        parameter_profiles: BTreeMap::new(),
    }), imported)
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
    let strategy_profile = strategy
        .map(|strategy| learned_strategy_parameter_profile(&settings, &signal.symbol, strategy));
    if let (Some(strategy), Some(profile)) = (strategy, strategy_profile.as_ref()) {
        queue_strategy_profile_registration(&mut settings, &signal.symbol, strategy, profile);
    }
    let is_portfolio_rebalance = strategy == Some(Strategy::RiskAdjustedTrendPortfolio)
        && signal.strategy_decision.as_ref()
        .and_then(|decision| decision.get("portfolio_rebalance_delta_usd"))
        .and_then(serde_json::Value::as_f64)
        .is_some_and(|delta| delta.is_finite() && delta > 0.0);
    let strategy_order_size_multiplier = if is_portfolio_rebalance { 1.0 } else {
        signal_strategy_order_size_multiplier(strategy, signal.strategy_decision.as_ref())?
    };
    let agreement_size_multiplier =
        signal_agreement_size_multiplier(strategy, signal.strategy_decision.as_ref())?;
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
        .is_some_and(|profile| profile.learning_samples >= WALK_FORWARD_POLICY_MIN_SAMPLES);
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
    let strategy_size_learning = strategy
        .filter(|kind| kind.is_dated_carry())
        .map(|kind| {
            settings
                .learned_profiles
                .get(&format!("{}:{}", signal.symbol, kind.as_str()))
                .map(|profile| LearningStats {
                    samples: profile.learning_samples,
                    positive: profile.positive_samples,
                    realized_pnl: profile.realized_pnl,
                    ..LearningStats::default()
                })
                .unwrap_or_default()
        });
    let learned_size_multiplier = if !scout && !exploratory {
        if strategy.is_some_and(|kind| !kind.is_dated_carry()) {
            // Daily and intraday strategies apply pooled learning to their
            // strategy-specific budget caps below.
            1.0
        } else {
            strategy_size_learning
                .as_ref()
                .map(learned_strategy_size_multiplier)
                .unwrap_or(1.0)
        }
    } else {
        1.0
    };
    let normal_order_cap = if strategy.is_some() && !scout && !exploratory {
        settings.max_order_notional * STRATEGY_BASE_ORDER_FRACTION
    } else {
        settings.max_order_notional
    };
    let exploration_cap = if scout {
        settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION * risk_size_scale
    } else if exploratory {
        settings.max_order_notional * (settings.exploration_percent / 100.0) * risk_size_scale
    } else {
        normal_order_cap
    };
    let rebalance_delta = strategy
        .filter(|kind| *kind == Strategy::RiskAdjustedTrendPortfolio)
        .and_then(|_| signal.strategy_decision.as_ref())
        .and_then(|decision| decision.get("portfolio_rebalance_delta_usd"))
        .and_then(serde_json::Value::as_f64)
        .filter(|delta| delta.is_finite() && *delta > 0.0);
    let risk_order_cap = (exploration_cap
        .min(settings.max_order_notional)
        .min(settings.max_order_notional * risk_fraction)
        * learned_size_multiplier
        * strategy_order_size_multiplier
        * if rebalance_delta.is_some() { 1.0 } else { agreement_size_multiplier })
        .min(rebalance_delta.unwrap_or(f64::INFINITY))
        .min(settings.max_order_notional);
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
    let strategy_remaining_budget = if rebalance_delta.is_some() { f64::INFINITY } else { signal
        .strategy
        .as_deref()
        .and_then(Strategy::from_name)
        .map(|kind| match strategy_category(Some(kind.as_str())) {
            StrategyCategory::Daily if !kind.is_live_insight() && !kind.is_dated_carry() => {
                signal_weighted_strategy_budget(&settings, dashboard, kind, StrategyCategory::Daily, now_ms)
                    - allocated_notional_for_strategy(&settings, kind.as_str())
            }
            StrategyCategory::Intraday if kind.is_live_insight() => {
                signal_weighted_strategy_budget(&settings, dashboard, kind, StrategyCategory::Intraday, now_ms)
                    - allocated_notional_for_strategy(&settings, kind.as_str())
            }
            _ => f64::INFINITY,
        })
        .unwrap_or(f64::INFINITY) };
    let remaining_budget = if rebalance_delta.is_some() && signal.direction == "bearish" {
        f64::INFINITY
    } else {
        (settings.daily_notional_limit - total_allocated)
            .min(category_notional_limit(&settings, category) - category_allocated)
            .min(strategy_remaining_budget)
            .max(0.0)
    };
    let mut notional = risk_order_cap.min(remaining_budget / stop_headroom_multiplier);
    // Size against the unlearned stop so a wider validated stop does not
    // silently increase the dollar risk of the position.
    if strategy.is_some_and(|kind| kind.uses_protective_stops()) {
        notional *= volatility_size_scale(baseline_stop, effective_stop_loss_percent);
    }
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
        post_only_timeout_minutes: signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(|parameters| parameters.get("post_only_timeout_minutes"))
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && (1.0..=15.0).contains(value))
            .or_else(|| strategy_profile.as_ref().and_then(|profile| strategy_profile_value(profile, "post_only_timeout_minutes")))
            .unwrap_or(5.0) as u64,
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
                effective_stop_loss_percent,
            );
            (effective_take_profit_percent, horizon_take_profit_percent)
        };
    let context = market_context(&signal);
    let (execution_learning, imported_execution_learning) = execution_learning_decision(
        &state.client,
        &context,
        baseline_type,
        signal,
        settings.risk_appetite,
        !scout,
        exploratory,
        &settings.execution_learning,
        !settings.execution_learning_imported,
    ).await;
    if imported_execution_learning {
        settings.execution_learning_imported = true;
    }
    let learned_type = execution_learning.learned_order_type.as_deref()
        .and_then(|name| serde_json::from_value::<AutoOrderType>(serde_json::json!(name)).ok())
        .unwrap_or(baseline_type);
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
    let schedule_depth_threshold = execution_learning.schedule_depth_threshold;
    let schedule_threshold_samples = execution_learning.schedule_threshold_samples;
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
    let order_type_name = serde_json::to_value(order_type).ok()
        .and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default();
    let profile = execution_learning.parameter_profiles.get(&order_type_name).copied().unwrap_or(0);
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

    #[test]
    fn recent_strategy_learning_includes_scout_shadow_outcomes_across_symbols() {
        let strategy = Strategy::Donchian55Btc200;
        let mut settings = Settings::default();
        for (symbol, samples, positive, realized_pnl) in [
            ("BTC-USDT", 1, 1, 2.0),
            ("ETH-USDT", 1, 0, -1.0),
            ("SOL-USDT", 1, 1, 0.5),
        ] {
            let parameters = BTreeMap::new();
            let profile_id = strategies::profile_id(symbol, strategy.as_str(), &parameters);
            settings.learned_profiles.insert(
                format!("{symbol}:{}", strategy.as_str()),
                strategies::StrategyProfile {
                    profile_id,
                    version: 1,
                    symbol: symbol.to_owned(),
                    strategy: strategy.as_str().to_owned(),
                    parameters,
                    learning_samples: samples,
                    positive_samples: positive,
                    realized_pnl,
                },
            );
        }

        let stats = strategy_learning_stats_across_symbols(&settings, strategy);
        assert_eq!(stats.samples, 3);
        assert_eq!(stats.positive, 2);
        assert_eq!(stats.realized_pnl, 1.5);
    }
}

#[cfg(test)]
mod signal_execution_budget_tests {
    use super::*;

    fn signal(strategy: &str, strength: f64, timestamp: u64) -> Signal {
        Signal {
            symbol: "ETH-USDT".into(), direction: "bullish".into(), strength,
            timestamp, stale: false, score: strength, ranging_probability: None,
            components: SignalComponents::default(), strategy_candidate: true,
            strategy: Some(strategy.into()), strategy_decision: None,
            market_regime: None, news_attribution: None, entry_gate: None,
        }
    }

    fn dashboard(best: Vec<Signal>) -> Dashboard {
        Dashboard {
            best, walk_forward: Arc::new(Vec::new()), missed_opportunity_bands: Vec::new(),
            markets: Arc::new(BTreeMap::new()), queued_at: None, queue_wait_ms: None,
        }
    }

    #[test]
    fn recent_net_edge_and_learned_size_multiplier_handle_empty_profitable_and_losing_samples() {
        assert_eq!(recent_net_pnl_edge(&LearningStats::default()), None);
        let zero = LearningStats { recent_trade_pnls: vec![(current_time_ms(), 0.0)], ..LearningStats::default() };
        assert_eq!(recent_net_pnl_edge(&zero), Some(0.0));
        let wins = LearningStats { samples: 10, positive: 8, recent_trade_pnls: vec![(current_time_ms(), 5.0), (current_time_ms(), 3.0)], ..LearningStats::default() };
        let losses = LearningStats { samples: 10, positive: 2, recent_trade_pnls: vec![(current_time_ms(), -8.0), (current_time_ms(), 1.0)], ..LearningStats::default() };
        assert!(recent_net_pnl_edge(&wins).unwrap() > 0.0);
        assert!(learned_strategy_size_multiplier(&wins) > 1.0);
        assert!(recent_net_pnl_edge(&losses).unwrap() < 0.0);
        assert!(learned_strategy_size_multiplier(&losses) < 1.0);
        assert_eq!(learned_strategy_size_multiplier(&LearningStats::default()), 1.0);
        assert_eq!(learned_strategy_size_multiplier(&LearningStats { samples: 2, positive: 3, ..LearningStats::default() }), 1.0);
    }

    #[test]
    fn signal_weighted_daily_budget_keeps_a_floor_and_rewards_eligible_signals() {
        let mut settings = Settings::default();
        settings.strategies = vec![Strategy::Donchian20Btc200, Strategy::Donchian55Btc200];
        settings.daily_notional_limit = 500.0;
        settings.daily_strategy_notional_limit = 500.0;
        let no_signals = dashboard(Vec::new());
        assert_eq!(signal_weighted_strategy_budget(&settings, &no_signals, Strategy::Donchian20Btc200, StrategyCategory::Daily, 100_000), 125.0);
        let signals = dashboard(vec![signal("donchian-20-btc200", 1.0, 100_000), signal("donchian-20-btc200", 1.0, 100_000)]);
        let favored = signal_weighted_strategy_budget(&settings, &signals, Strategy::Donchian20Btc200, StrategyCategory::Daily, 100_000);
        let other = signal_weighted_strategy_budget(&settings, &signals, Strategy::Donchian55Btc200, StrategyCategory::Daily, 100_000);
        assert!(favored > other);
        assert!(other >= 125.0);
        assert!((favored + other - 500.0).abs() < 1e-9);

        let stale = dashboard(vec![signal("donchian-20-btc200", 1.0, 1)]);
        assert_eq!(signal_weighted_strategy_budget(&settings, &stale, Strategy::Donchian20Btc200, StrategyCategory::Daily, 100_000_000), 125.0);
    }

    #[test]
    fn configured_budgets_bypass_signal_weighting_and_order_multipliers_validate_contracts() {
        let mut settings = Settings::default();
        settings.strategies = vec![Strategy::Donchian20Btc200, Strategy::Donchian55Btc200];
        settings.daily_strategy_budgets.insert("donchian-20-btc200".into(), 200.0);
        let signals = dashboard(vec![signal("donchian-55-btc200", 1.0, 100_000)]);
        assert_eq!(signal_weighted_strategy_budget(&settings, &signals, Strategy::Donchian20Btc200, StrategyCategory::Daily, 100_000), 200.0);
        assert_eq!(daily_strategy_effective_budget(&settings, "live-momentum"), 0.0);
        assert_eq!(intraday_strategy_effective_budget(&settings, "donchian-20-btc200"), 0.0);

        let strategy = Some(Strategy::RiskAdjustedMonthlyMomentum);
        assert_eq!(signal_strategy_order_size_multiplier(strategy, Some(&serde_json::json!({"strategy_order_size_multiplier":0.5}))).unwrap(), 0.5);
        assert!(signal_strategy_order_size_multiplier(strategy, Some(&serde_json::json!({"strategy_order_size_multiplier":0.0}))).is_err());
        assert_eq!(signal_strategy_order_size_multiplier(None, None).unwrap(), 1.0);
        assert_eq!(signal_agreement_size_multiplier(Some(Strategy::Donchian20Btc200), None).unwrap(), 1.0);
        assert_eq!(signal_agreement_size_multiplier(Some(Strategy::Donchian20Btc200), Some(&serde_json::json!({"strategy_agreement_size_multiplier":1.1}))).unwrap(), 1.1);
        assert!(signal_agreement_size_multiplier(Some(Strategy::Donchian20Btc200), Some(&serde_json::json!({"strategy_agreement_size_multiplier":1.3}))).is_err());
    }
}
