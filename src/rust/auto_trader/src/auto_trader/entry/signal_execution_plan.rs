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
            (
                net + pnl * weight,
                gross + pnl.abs() * weight,
                samples + weight,
            )
        },
    );
    if gross <= 1e-12 {
        return Some(0.0);
    }
    let confidence = effective_samples / (effective_samples + 2.0);
    Some((net / gross * confidence).clamp(-1.0, 1.0))
}

fn effective_entry_stop(strategy: Option<Strategy>, stop: f64, volatility: Option<f64>) -> f64 {
    if strategy.is_some() {
        stop
    } else {
        volatility_adjusted_stop_loss_percent(stop, volatility)
    }
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
    target.clamp(
        POOR_STRATEGY_SIZE_MULTIPLIER,
        MAX_LEARNED_STRATEGY_SIZE_MULTIPLIER,
    )
}

fn signal_confidence_size_multiplier(signal: &Signal) -> f64 {
    let strength = if signal.strength.is_finite() {
        signal.strength.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let directionally_aligned_score = if signal.score.is_finite()
        && ((signal.direction == "bullish" && signal.score >= 0.0)
            || (signal.direction == "bearish" && signal.score <= 0.0))
    {
        signal.score.abs().clamp(0.0, 1.0)
    } else {
        0.0
    };
    // Signal strength and score capture the setup itself. Market-state
    // confidence adds case-specific context when the classifier has evidence;
    // it must not replace the signal's own confidence.
    let signal_confidence = (strength * directionally_aligned_score).sqrt();
    let confidence = signal
        .market_state
        .as_ref()
        .map(|state| {
            let state_confidence = if state.confidence.is_finite() {
                state.confidence.clamp(0.0, 1.0)
            } else {
                0.0
            };
            0.75 * signal_confidence + 0.25 * state_confidence
        })
        .unwrap_or(signal_confidence);
    MIN_SIGNAL_STRENGTH_SIZE_MULTIPLIER
        + (MAX_SIGNAL_STRENGTH_SIZE_MULTIPLIER - MIN_SIGNAL_STRENGTH_SIZE_MULTIPLIER) * confidence
}

fn directional_budget_confidence(signal: &Signal) -> f64 {
    let strength = if signal.strength.is_finite() {
        signal.strength.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let directionally_aligned_score = if signal.score.is_finite()
        && ((signal.direction == "bullish" && signal.score > 0.0)
            || (signal.direction == "bearish" && signal.score < 0.0))
    {
        signal.score.abs().clamp(0.0, 1.0)
    } else {
        0.0
    };
    let setup_confidence = (strength * directionally_aligned_score).sqrt();
    if setup_confidence <= 0.0 {
        return 0.0;
    }
    let confidence = signal
        .market_state
        .as_ref()
        .map(|state| {
            let state_confidence = if state.confidence.is_finite() {
                state.confidence.clamp(0.0, 1.0)
            } else {
                0.0
            };
            0.75 * setup_confidence + 0.25 * state_confidence
        })
        .unwrap_or(setup_confidence);
    if confidence.is_finite() {
        confidence.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn strategy_learning_stats_across_symbols(
    settings: &Settings,
    strategy: Strategy,
) -> LearningStats {
    settings
        .learned_profiles
        .values()
        .filter(|profile| profile.strategy == strategy.as_str())
        .fold(LearningStats::default(), |mut total, profile| {
            total.samples = total.samples.saturating_add(profile.learning_samples);
            total.positive = total.positive.saturating_add(profile.positive_samples);
            total.realized_pnl += profile.realized_pnl;
            total
        })
}

fn daily_shared_pool_ceiling(settings: &Settings, strategy: &str) -> f64 {
    let Some(kind) = Strategy::from_name(strategy)
        .filter(|kind| !kind.is_live_insight() && !kind.is_dated_carry())
    else {
        return 0.0;
    };
    if !settings.strategies.contains(&kind) {
        return 0.0;
    }
    settings.daily_notional_limit
        * settings
            .learned_strategy_size_multipliers
            .get(kind.as_str())
            .copied()
            .unwrap_or(1.0)
}

fn paper_learning_exploration_active(settings: &Settings, now_ms: u64) -> bool {
    now_ms < settings.paper_learning_exploration_until_ms
}

fn intraday_strategy_size_multiplier(settings: &Settings, strategy: Strategy, now_ms: u64) -> f64 {
    if strategy.is_live_insight() {
        // Live strategies use symbol/direction risk at the entry, while strict
        // promotion still requires validated Scout evidence.
        return if paper_learning_exploration_active(settings, now_ms) {
            0.20
        } else {
            1.0
        };
    }
    settings
        .learned_strategy_size_multipliers
        .get(strategy.as_str())
        .copied()
        .unwrap_or(1.0)
        .min(1.0)
}

fn learned_strategy_entry_paused(settings: &Settings, strategy: Strategy, now_ms: u64) -> bool {
    if strategy.is_live_insight() {
        return false;
    }
    if strategy.is_live_insight() && paper_learning_exploration_active(settings, now_ms) {
        return false;
    }
    settings
        .learned_strategy_size_multipliers
        .get(strategy.as_str())
        .is_some_and(|multiplier| multiplier.is_finite() && *multiplier <= 0.0)
}

fn is_portfolio_rebalance_signal(signal: &Signal, strategy: Option<Strategy>) -> bool {
    strategy == Some(Strategy::RiskAdjustedTrendPortfolio)
        && signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("portfolio_rebalance_delta_usd"))
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|delta| delta.is_finite() && delta > 0.0)
}

fn learned_pause_block_reason(
    settings: &Settings,
    strategy: Option<Strategy>,
    signal: &Signal,
    is_portfolio_rebalance: bool,
    direction: &str,
    now_ms: u64,
) -> Option<String> {
    if is_portfolio_rebalance && direction == "bearish" {
        return None;
    }
    let kind = strategy?;
    if !learned_strategy_entry_paused(settings, kind, now_ms) {
        return None;
    }
    validated_strategy_entry_edge(signal, kind).err()
}

fn validated_strategy_entry_edge(signal: &Signal, strategy: Strategy) -> Result<f64, String> {
    validated_strategy_entry_edge_at(signal, strategy, current_time_ms())
}

fn validated_strategy_entry_edge_at(
    signal: &Signal,
    strategy: Strategy,
    now_ms: u64,
) -> Result<f64, String> {
    let parameters = signal_strategy_parameters(signal);
    let input = StrategySignal::entry(signal, &parameters);
    let (minimum, maximum_age_seconds) = strategy_scout_entry_limits(&parameters, strategy)?;
    let evidence = input.walk_forward_evidence.ok_or_else(|| {
        crate::service_log!(
            "INFO",
            "Walk-forward missing trace: strategy={} strategy_candidate={} daily_cold_start={} parameter_probe={} rule_evidence_available={} rule_evidence_eligible={} selection_mode={}",
            strategy.as_str(),
            signal.strategy_candidate,
            is_daily_cold_start_exploration_candidate(signal),
            is_daily_parameter_exploration_probe(signal),
            daily_cold_start_exploration_checks(signal).0,
            daily_cold_start_exploration_checks(signal).1,
            signal
                .entry_gate
                .as_ref()
                .and_then(|gate| gate.get("selection_mode"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("missing"),
        );
        "walk_forward_missing: no prior Scout outcomes match this strategy, symbol, and parameter profile".to_owned()
    })?;
    if signal.timestamp > now_ms {
        return Err(format!(
            "walk_forward_signal_in_future: signal {} is after current time {}",
            signal.timestamp, now_ms
        ));
    }
    if evidence.last_sample_timestamp_ms >= signal.timestamp {
        return Err(format!(
            "walk_forward_not_point_in_time: latest evidence {} is not before signal {}",
            evidence.last_sample_timestamp_ms, signal.timestamp
        ));
    }
    if evidence.last_sample_timestamp_ms > now_ms {
        return Err(format!(
            "walk_forward_evidence_in_future: evidence {} is after current time {}",
            evidence.last_sample_timestamp_ms, now_ms
        ));
    }
    let age_ms = now_ms - evidence.last_sample_timestamp_ms;
    if age_ms > maximum_age_seconds.saturating_mul(1_000) {
        return Err(format!(
            "walk_forward_stale: evidence is {}s old; maximum is {}s",
            age_ms / 1_000,
            maximum_age_seconds
        ));
    }
    if evidence.sample_count < minimum {
        return Err(format!(
            "walk_forward_insufficient_samples: {} outcomes; requires at least {}",
            evidence.sample_count, minimum
        ));
    }
    let effective_samples = evidence
        .effective_sample_size
        .unwrap_or(evidence.sample_count as f64);
    if !effective_samples.is_finite() || effective_samples < minimum as f64 {
        return Err(format!(
            "walk_forward_insufficient_effective_samples: {} outcomes, effective n {:.1}; requires at least {}",
            evidence.sample_count, effective_samples, minimum
        ));
    }
    let lower_bound = evidence
        .expectancy_ci_low_percent
        .filter(|value| value.is_finite())
        .ok_or_else(|| {
            format!(
                "walk_forward_interval_unavailable: {} outcomes have no valid expectancy lower bound",
                evidence.sample_count
            )
        })?;
    if lower_bound <= 0.0 {
        return Err(format!(
            "walk_forward_expectancy_not_proven: lower 95% confidence bound {:.3}% requires > 0.000% (n={}, effective n={:.1})",
            lower_bound, evidence.sample_count, effective_samples
        ));
    }
    Ok(lower_bound)
}

fn allow_bounded_strategy_exploration(signal: &Signal, strategy: Strategy) -> Result<(), String> {
    let parameters = signal_strategy_parameters(signal);
    let (minimum_samples, maximum_age_seconds) =
        strategy_scout_entry_limits(&parameters, strategy)?;
    let input = StrategySignal::entry(signal, &parameters);
    let Some(evidence) = input.walk_forward_evidence else {
        return Ok(());
    };
    // Exploration is for profiles without enough prior observations. Never
    // use outcomes from after this signal, or continue probing a profile that
    // already has enough samples and negative net evidence.
    if evidence.sample_count < minimum_samples
        || evidence.last_sample_timestamp_ms >= signal.timestamp
    {
        return Ok(());
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    if now_ms.saturating_sub(evidence.last_sample_timestamp_ms)
        > maximum_age_seconds.saturating_mul(1_000)
    {
        return Ok(());
    }
    let effective_samples = evidence
        .effective_sample_size
        .unwrap_or(evidence.sample_count as f64);
    if !effective_samples.is_finite() || effective_samples < minimum_samples as f64 {
        return Ok(());
    }
    let Some(lower_bound) = evidence
        .expectancy_ci_low_percent
        .filter(|value| value.is_finite())
    else {
        return Ok(());
    };
    if lower_bound <= 0.0 {
        return Err(format!(
            "walk_forward_expectancy_not_proven: lower 95% confidence bound {:.3}% requires > 0.000% (n={}, effective n={:.1})",
            lower_bound, evidence.sample_count, effective_samples
        ));
    }
    Ok(())
}

fn strategy_entry_walk_forward_gate(
    settings: &Settings,
    signal: &Signal,
    strategy: Option<Strategy>,
    is_portfolio_rebalance: bool,
    paper_learning_exploration: bool,
    exploratory: bool,
    now_ms: u64,
) -> Result<(), String> {
    if !exploratory {
        if let Some(reason) = learned_pause_block_reason(
            settings,
            strategy,
            signal,
            is_portfolio_rebalance,
            &signal.direction,
            now_ms,
        ) {
            return Err(format!("Strategy entry paused: {reason}"));
        }
    }
    if let Some(kind) = strategy.filter(|kind| !kind.is_dated_carry()) {
        if !(is_portfolio_rebalance && signal.direction == "bearish") {
            if paper_learning_exploration {
                // Reset Learning starts a temporary paper-only sampling window.
            } else if exploratory {
                allow_bounded_strategy_exploration(signal, kind)?;
            } else {
                validated_strategy_entry_edge(signal, kind)?;
            }
        }
    }
    Ok(())
}

fn exploratory_notional_cap(settings: &Settings, risk_size_scale: f64) -> f64 {
    settings.max_order_notional * (settings.exploration_percent / 100.0) * risk_size_scale
}

fn risk_fraction(settings: &Settings) -> f64 {
    if settings.risk_limited_sizing {
        0.25
    } else {
        1.0
    }
}

fn risk_bounded_order_cap(settings: &Settings, initial_cap: f64) -> f64 {
    initial_cap
        .min(settings.max_order_notional)
        .min(settings.max_order_notional * risk_fraction(settings))
}

/// Bound notional against the risk allowance established before a profile exit
/// overlay is selected. Fees are included in both the reference allowance and
/// selected-stop cost, while the caller's notional remains a hard upper bound.
fn stop_risk_bounded_notional(
    notional_cap: f64,
    reference_stop_percent: f64,
    selected_stop_percent: f64,
    round_trip_cost_bps: f64,
) -> f64 {
    if !notional_cap.is_finite()
        || notional_cap <= 0.0
        || !reference_stop_percent.is_finite()
        || reference_stop_percent <= 0.0
        || !selected_stop_percent.is_finite()
        || selected_stop_percent <= 0.0
        || !round_trip_cost_bps.is_finite()
        || round_trip_cost_bps < 0.0
    {
        return 0.0;
    }

    let round_trip_cost_percent = round_trip_cost_bps / 100.0;
    let reference_loss_per_notional = reference_stop_percent + round_trip_cost_percent;
    let selected_loss_per_notional = selected_stop_percent + round_trip_cost_percent;
    if !reference_loss_per_notional.is_finite()
        || !selected_loss_per_notional.is_finite()
        || selected_loss_per_notional <= 0.0
    {
        return 0.0;
    }

    let allowed_loss = notional_cap * reference_loss_per_notional / 100.0;
    (allowed_loss * 100.0 / selected_loss_per_notional)
        .min(notional_cap)
        .max(0.0)
}

fn minimum_book_order_for_entry(exploratory: bool, notional: f64) -> f64 {
    if exploratory {
        notional.min(MIN_DEEP_BOOK_ORDER_NOTIONAL)
    } else {
        MIN_DEEP_BOOK_ORDER_NOTIONAL
    }
}

fn strategy_scout_entry_limits(
    parameters: &BTreeMap<String, f64>,
    strategy: Strategy,
) -> Result<(usize, u64), String> {
    let specs = strategy.parameter_specs();
    let bounded = |key: StrategyParameterKey| {
        let spec = specs.iter().find(|spec| spec.key == key)?;
        let value = strategy_parameter_value(parameters, key)?;
        let position = (value - spec.min) / spec.step;
        (spec.step > 0.0
            && value >= spec.min
            && value <= spec.max
            && (position - position.round()).abs() <= 1e-6)
            .then_some(value)
    };
    let minimum_samples = bounded(StrategyParameterKey::ScoutMinimumWalkForwardSamples)
        .ok_or_else(|| {
            "Strategy entry is missing its bounded minimum Scout sample count".to_owned()
        })?
        .round() as usize;
    let maximum_age_seconds = bounded(StrategyParameterKey::ScoutMaximumEvidenceAgeSeconds)
        .ok_or_else(|| {
            "walk_forward_invalid_config: maximum evidence age is missing or out of bounds"
                .to_owned()
        })?
        .round() as u64;
    Ok((minimum_samples, maximum_age_seconds))
}

fn signal_weighted_strategy_budget(
    settings: &Settings,
    dashboard: &Dashboard,
    strategy: Strategy,
    category: StrategyCategory,
    now_ms: u64,
) -> f64 {
    let _ = category;
    shared_candidate_allocations(settings, dashboard, now_ms, None)
        .iter()
        .filter(|(key, _)| key.0 == strategy.as_str())
        .map(|(_, allocation)| allocation.effective_allocation)
        .sum()
}

fn allocation_horizon(strategy: Strategy) -> &'static str {
    match strategy_category(Some(strategy.as_str())) {
        StrategyCategory::Daily => "daily",
        StrategyCategory::Intraday => "intraday",
    }
}

fn signal_candidate_budget(
    state: &AutoTraderState,
    settings: &Settings,
    dashboard: &Dashboard,
    strategy: Strategy,
    signal: &Signal,
    exploratory_authorized: bool,
    now_ms: u64,
) -> f64 {
    let candidates = shared_candidate_allocations(
        settings,
        dashboard,
        now_ms,
        Some((signal, exploratory_authorized)),
    );
    if let Ok(mut snapshot) = state.capital_allocation_snapshot.try_lock() {
        snapshot.as_of_ms = now_ms;
        snapshot.pool_limit = settings.daily_notional_limit;
        snapshot.candidates = candidates.clone();
    }
    candidates
        .get(&(
            strategy.as_str().to_owned(),
            signal.symbol.clone(),
            allocation_horizon(strategy).to_owned(),
        ))
        .map(|snapshot| snapshot.effective_allocation)
        .unwrap_or(0.0)
}

fn shared_candidate_allocations(
    settings: &Settings,
    dashboard: &Dashboard,
    now_ms: u64,
    selected_candidate: Option<(&Signal, bool)>,
) -> BTreeMap<(String, String, String), CapitalAllocationCandidateSnapshot> {
    const GUARANTEED_SHARE: f64 = 0.5;
    const MAX_CANDIDATES: usize = 256;
    let available = (settings.daily_notional_limit - allocated_notional(settings)).max(0.0);
    if available <= 0.0 {
        return BTreeMap::new();
    }
    let enabled = settings
        .strategies
        .iter()
        .copied()
        .filter(|kind| !kind.is_legacy_composite() && !kind.is_dated_carry())
        .filter(|kind| {
            if kind.is_live_insight() {
                intraday_strategy_size_multiplier(settings, *kind, now_ms) > 0.0
            } else {
                settings
                    .learned_strategy_size_multipliers
                    .get(kind.as_str())
                    .is_none_or(|multiplier| multiplier.is_finite() && *multiplier > 0.0)
            }
        })
        .collect::<Vec<_>>();
    let mut weights = BTreeMap::<(String, String, String), (f64, bool)>::new();
    let mut signals = dashboard.best.iter().collect::<Vec<_>>();
    if let Some((selected, _)) = selected_candidate {
        // Keep the selected clone because it carries the authoritative current
        // strict/exploratory selection mode; the dashboard copy may be older.
        signals.push(selected);
    }
    for signal in signals.into_iter().filter(|signal| {
        signal.strategy_candidate
            && !signal.stale
            && matches!(signal.direction.as_str(), "bullish" | "bearish")
            && signal.timestamp <= now_ms
            && now_ms - signal.timestamp <= signal_max_age_seconds(signal) * 1_000
    }) {
        let Some(kind) = signal
            .strategy
            .as_deref()
            .and_then(Strategy::from_name)
            .filter(|kind| enabled.contains(kind))
        else {
            continue;
        };
        let key = (
            kind.as_str().to_owned(),
            signal.symbol.clone(),
            allocation_horizon(kind).to_owned(),
        );
        let edge = validated_strategy_entry_edge_at(signal, kind, now_ms)
            .ok()
            .filter(|edge| edge.is_finite() && *edge > 0.0);
        let selected_exploratory = selected_candidate.is_some_and(|(selected, allowed)| {
            allowed && selected.symbol == signal.symbol && selected.strategy == signal.strategy
        });
        let selected_current = selected_candidate.is_some_and(|(selected, _)| {
            selected.symbol == signal.symbol && selected.strategy == signal.strategy
        });
        let entry_gate_mode = signal
            .entry_gate
            .as_ref()
            .and_then(|gate| gate.get("selection_mode"))
            .and_then(serde_json::Value::as_str);
        let strict_eligible = edge.is_some()
            && ((selected_current && !selected_exploratory) || entry_gate_mode == Some("strict"));
        let exploratory_eligible =
            !strict_eligible && (selected_exploratory || entry_gate_mode == Some("exploratory"));
        if !strict_eligible && !exploratory_eligible {
            continue;
        }
        let learning = if kind.is_live_insight() {
            intraday_strategy_size_multiplier(settings, kind, now_ms)
        } else {
            settings
                .learned_strategy_size_multipliers
                .get(kind.as_str())
                .copied()
                .unwrap_or(1.0)
        };
        let weight = edge
            .filter(|_| strict_eligible)
            .map(|edge| {
                edge / (1.0 + edge)
                    * directional_budget_confidence(signal)
                    * learning.clamp(0.0, MAX_LEARNED_STRATEGY_SIZE_MULTIPLIER)
            })
            .filter(|weight| weight.is_finite() && *weight > 0.0)
            .unwrap_or(0.0);
        if weight > 0.0 {
            weights
                .entry(key)
                .and_modify(|existing| existing.0 = existing.0.max(weight))
                .or_insert((weight, true));
        } else {
            weights.entry(key).or_insert((0.0, false));
        }
    }
    let active_count = weights.len().min(MAX_CANDIDATES);
    if active_count == 0 {
        return BTreeMap::new();
    }
    let mut result = BTreeMap::new();
    let floor = available * GUARANTEED_SHARE / active_count as f64;
    let total_weight = weights.values().map(|(weight, _)| *weight).sum::<f64>();
    let variable_pool = (available - floor * active_count as f64).max(0.0);
    for (key, (weight, strict_eligible)) in weights.into_iter().take(MAX_CANDIDATES) {
        let variable = if total_weight.is_finite() && total_weight > 0.0 {
            variable_pool * weight / total_weight
        } else {
            0.0
        };
        let committed = allocated_notional_for_candidate(settings, &key.0, &key.1);
        result.insert(
            key,
            CapitalAllocationCandidateSnapshot {
                effective_allocation: committed + floor + variable,
                evidence_eligible: Some(strict_eligible),
                state: if strict_eligible {
                    "strict".to_owned()
                } else {
                    "exploratory".to_owned()
                },
                reason: if strict_eligible {
                    "Validated positive edge; receives evidence-weighted shared-pool allocation"
                        .to_owned()
                } else {
                    "Strict evidence is unavailable; selected under existing exploration eligibility and receives only the bounded exploratory allocation floor"
                        .to_owned()
                },
            },
        );
    }
    result
}

fn intraday_shared_pool_ceiling(settings: &Settings, strategy: &str, now_ms: u64) -> f64 {
    let Some(kind) = Strategy::from_name(strategy).filter(|kind| kind.is_live_insight()) else {
        return 0.0;
    };
    if !settings.strategies.contains(&kind) {
        return 0.0;
    }
    settings.daily_notional_limit * intraday_strategy_size_multiplier(settings, kind, now_ms)
}

fn signal_strategy_order_size_multiplier(
    strategy: Option<Strategy>,
    decision: Option<&serde_json::Value>,
) -> Result<f64, String> {
    match strategy {
        Some(
            Strategy::RiskAdjustedMonthlyMomentum
            | Strategy::Donchian20Btc200Hold3
            | Strategy::RiskAdjustedTrendPortfolio,
        ) => decision
            .and_then(|decision| decision.get("strategy_order_size_multiplier"))
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0 && *value <= 1.0)
            .ok_or_else(|| {
                "Research strategy signal is missing a valid size multiplier".to_owned()
            }),
        _ => Ok(1.0),
    }
}

fn signal_agreement_size_multiplier(
    strategy: Option<Strategy>,
    decision: Option<&serde_json::Value>,
) -> Result<f64, String> {
    let Some(strategy) =
        strategy.filter(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
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
    exploratory: bool,
    effective_stop_loss_percent: f64,
    effective_take_profit_percent: f64,
    exit_parameter_provenance: serde_json::Value,
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
    execution_learning_ms: f64,
    imported_execution_learning: bool,
    exploration_risk_adjustment: ExplorationRiskAdjustment,
}

fn exit_parameter_source(
    settings: &Settings,
    signal: &Signal,
    strategy_profile: Option<&StrategyParameterProfile>,
    strategy: Option<Strategy>,
    parameter_key: &str,
    validated_policy_supplies_value: bool,
) -> &'static str {
    let Some(profile) = strategy_profile else {
        return "global_fallback";
    };
    let Some(parameter) = profile
        .parameters
        .iter()
        .find(|parameter| parameter.key == parameter_key)
    else {
        return "global_fallback";
    };
    if parameter.configured_value.is_some() {
        return "configured_profile_override";
    }
    if validated_policy_supplies_value {
        return "validated_exit_policy";
    }
    let profile_has_learned_value = strategy.is_some_and(|strategy| {
        settings
            .learned_profiles
            .get(&format!("{}:{}", signal.symbol, strategy.as_str()))
            .is_some_and(|profile| {
                profile.learning_samples > 0 && profile.parameters.contains_key(parameter_key)
            })
    });
    if profile_has_learned_value {
        "learned_strategy_profile"
    } else {
        "strategy_default_profile"
    }
}

fn exit_parameter_provenance(
    settings: &Settings,
    signal: &Signal,
    strategy_profile: Option<&StrategyParameterProfile>,
    strategy: Option<Strategy>,
    active_policy: Option<&strategy::LearnedExitProtectionPolicy>,
    contextual_stop_percent: Option<f64>,
    selected_stop_loss_percent: f64,
    effective_stop_loss_percent: f64,
    selected_take_profit_percent: f64,
    effective_take_profit_percent: f64,
) -> serde_json::Value {
    let stop_key = StrategyParameterKey::StopLossPercent.as_str();
    let target_key = StrategyParameterKey::TakeProfitPercent.as_str();
    let profile_values = strategy_profile
        .map(|profile| {
            profile
                .parameters
                .iter()
                .map(|parameter| (parameter.key.clone(), parameter.value))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let selected_parameter_profile_id = signal
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("learning_profile_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let selected_parameter_profile_version = signal
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("learning_profile_version"))
        .and_then(serde_json::Value::as_u64);
    let profile_id = selected_parameter_profile_id
        .clone()
        .or_else(|| strategy_profile.and_then(|profile| profile.profile_id.clone()))
        .or_else(|| {
            strategy.map(|strategy| {
                strategy::profile_id(&signal.symbol, strategy.as_str(), &profile_values)
            })
        });
    let profile_version = selected_parameter_profile_version
        .or_else(|| strategy_profile.and_then(|profile| profile.profile_version));
    let stop_source = if contextual_stop_percent.is_some() {
        "contextual_stop_overlay"
    } else {
        exit_parameter_source(
            settings,
            signal,
            strategy_profile,
            strategy,
            stop_key,
            active_policy.is_some_and(|policy| policy.stop_loss_percent.is_some()),
        )
    };
    let parameter_support = |key: &str| profile_values.contains_key(key);
    let maximum_hold_days_spec = captured_maximum_hold_days_specification(strategy);
    serde_json::json!({
        "schema_version": 1,
        "selected_at_ms": current_time_ms(),
        "profile_id": profile_id,
        "profile_version": profile_version,
        "source_profile_id": strategy_profile.and_then(|profile| profile.profile_id.as_deref()),
        "source_profile_version": strategy_profile.and_then(|profile| profile.profile_version),
        "selected_parameter_profile_id": selected_parameter_profile_id,
        "selected_parameter_profile_version": selected_parameter_profile_version,
        "exit_policy_profile_id": active_policy.and_then(|policy| policy.profile_id.as_deref()),
        "exit_policy_profile_version": active_policy.and_then(|policy| policy.profile_version),
        "exit_policy_as_of_ms": active_policy.map(|policy| policy.as_of_ms),
        "maximum_holding_duration": {
            "source": if active_policy.is_some_and(|policy| policy.maximum_holding_duration_ms.is_some()) {
                "validated_exit_policy"
            } else if profile_values.contains_key(StrategyParameterKey::MaximumHoldDays.as_str()) {
                "strategy_profile"
            } else {
                "strategy_default_or_unavailable"
            },
            "selected_days": signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("strategy_parameters"))
                .and_then(|parameters| parameters.get(StrategyParameterKey::MaximumHoldDays.as_str()))
                .and_then(serde_json::Value::as_f64),
            "selected_ms": signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("strategy_parameters"))
                .and_then(|parameters| parameters.get(StrategyParameterKey::MaximumHoldDays.as_str()))
                .and_then(serde_json::Value::as_f64)
                .map(|days| (days * 86_400_000.0).ceil() as u64),
            "effective_ms": active_policy
                .and_then(|policy| policy.maximum_holding_duration_ms)
                .or_else(|| signal.strategy_decision.as_ref()
                    .and_then(|decision| decision.get("strategy_parameters"))
                    .and_then(|parameters| parameters.get(StrategyParameterKey::MaximumHoldDays.as_str()))
                    .and_then(serde_json::Value::as_f64)
                    .filter(|days| days.is_finite() && *days > 0.0)
                    .map(|days| (days * 86_400_000.0).ceil() as u64)),
        },
        "profile_parameters": profile_values,
        "profile_parameter_support": {
            "maximum_hold_days": parameter_support(StrategyParameterKey::MaximumHoldDays.as_str()),
            "minimum_hold_days": parameter_support(StrategyParameterKey::MinimumHoldDays.as_str()),
            "profit_protection_activation_percent": parameter_support(StrategyParameterKey::ProfitProtectionActivationPercent.as_str()),
            "profit_protection_giveback_percent": parameter_support(StrategyParameterKey::ProfitProtectionGivebackPercent.as_str()),
            "profit_protection_minimum_lock_percent": parameter_support(StrategyParameterKey::ProfitProtectionMinimumLockPercent.as_str()),
            "trailing_activation_percent": parameter_support(StrategyParameterKey::TrailingActivationPercent.as_str()),
            "trailing_distance_percent": parameter_support(StrategyParameterKey::TrailingDistancePercent.as_str()),
            "breakeven_activation_percent": parameter_support(StrategyParameterKey::BreakevenActivationPercent.as_str()),
        },
        "strategy_parameter_specification": {
            "schema_version": 1,
            "maximum_hold_days": maximum_hold_days_spec
        },
        "unsupported_profile_exit_parameters": ["server_side_trailing_barrier_amendment", "server_side_breakeven_barrier"],
        "volatility_stop_adjustment": {
            "applied": (effective_stop_loss_percent - selected_stop_loss_percent).abs() > 1e-12,
            "volatility_1m_percent": signal.components.volatility_1m_percent,
            "selected_percent": selected_stop_loss_percent,
            "effective_percent": effective_stop_loss_percent,
            "source": match (
                strategy.is_some(),
                (effective_stop_loss_percent - selected_stop_loss_percent).abs() > 1e-12,
            ) {
                (true, true) => "volatility_adjusted_strategy_profile",
                (true, false) => "strategy_profile_unmodified",
                (false, true) => "volatility_adjusted_global_fallback",
                (false, false) => "global_fallback_unmodified",
            },
        },
        "stop_loss": {
            "source": stop_source,
            "selected_percent": selected_stop_loss_percent,
            "effective_percent": effective_stop_loss_percent,
            "profile_percent": profile_values.get(stop_key),
            "contextual_stop_percent": contextual_stop_percent,
            "global_fallback_percent": settings.stop_loss_percent,
        },
        "take_profit": {
            "source": exit_parameter_source(
                settings, signal, strategy_profile, strategy, target_key,
                active_policy.is_some_and(|policy| policy.take_profit_percent.is_some()),
            ),
            "selected_percent": selected_take_profit_percent,
            "effective_percent": effective_take_profit_percent,
            "profile_percent": profile_values.get(target_key),
            "global_fallback_percent": settings.take_profit_percent,
            "confidence_or_policy_adjustment_applied":
                (effective_take_profit_percent - selected_take_profit_percent).abs() > 1e-12,
        },
        "trailing_protection": {
            "source": exit_parameter_source(
                settings, signal, strategy_profile, strategy,
                StrategyParameterKey::TrailingActivationPercent.as_str(),
                active_policy.is_some_and(|policy| policy.trailing_activation_percent.is_some()),
            ),
            "activation_percent": signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("strategy_parameters"))
                .and_then(|parameters| parameters.get(StrategyParameterKey::TrailingActivationPercent.as_str()))
                .and_then(serde_json::Value::as_f64),
            "distance_percent": signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("strategy_parameters"))
                .and_then(|parameters| parameters.get(StrategyParameterKey::TrailingDistancePercent.as_str()))
                .and_then(serde_json::Value::as_f64),
            "runtime_action": "AutoTrader_monitor_ioc_exit",
            "server_side_barrier": "initial_static_stop_remains_armed",
        },
        "profit_protection": {
            "source": exit_parameter_source(
                settings, signal, strategy_profile, strategy,
                StrategyParameterKey::ProfitProtectionActivationPercent.as_str(),
                active_policy.is_some(),
            ),
            "activation_percent": signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("strategy_parameters"))
                .and_then(|parameters| parameters.get(StrategyParameterKey::ProfitProtectionActivationPercent.as_str()))
                .and_then(serde_json::Value::as_f64),
            "giveback_percent": signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("strategy_parameters"))
                .and_then(|parameters| parameters.get(StrategyParameterKey::ProfitProtectionGivebackPercent.as_str()))
                .and_then(serde_json::Value::as_f64),
            "minimum_lock_percent": signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("strategy_parameters"))
                .and_then(|parameters| parameters.get(StrategyParameterKey::ProfitProtectionMinimumLockPercent.as_str()))
                .and_then(serde_json::Value::as_f64),
        },
        "breakeven_protection": {
            "source": exit_parameter_source(
                settings, signal, strategy_profile, strategy,
                StrategyParameterKey::BreakevenActivationPercent.as_str(),
                active_policy.is_some_and(|policy| policy.breakeven_activation_percent.is_some()),
            ),
            "activation_percent": signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("strategy_parameters"))
                .and_then(|parameters| parameters.get(StrategyParameterKey::BreakevenActivationPercent.as_str()))
                .and_then(serde_json::Value::as_f64),
            "runtime_action": "AutoTrader_monitor_ioc_exit_at_net_flat",
            "server_side_barrier": "initial_static_stop_remains_armed",
        },
        "risk_appetite_at_selection": settings.risk_appetite,
    })
}

fn captured_maximum_hold_days_specification(strategy: Option<Strategy>) -> serde_json::Value {
    let specification = strategy.and_then(|kind| {
        kind.parameter_specs()
            .into_iter()
            .find(|spec| spec.key == StrategyParameterKey::MaximumHoldDays)
            .map(|spec| {
                serde_json::json!({
                    "key": StrategyParameterKey::MaximumHoldDays.as_str(),
                    "default": spec.default,
                    "min": spec.min,
                    "max": spec.max,
                    "step": spec.step,
                    "unit": spec.unit
                })
            })
    });
    serde_json::json!({
        "schema_version": 1,
        "maximum_hold_days": specification
    })
}

fn execution_learning_decision(
    context: &str,
    baseline: AutoOrderType,
    signal: &Signal,
    _risk_appetite: f64,
    allow_adaptation: bool,
    exploratory: bool,
    learning: &BTreeMap<String, ExecutionLearningStats>,
    pushed_learning: &BTreeMap<String, strategy::ExecutionLearningStat>,
) -> strategy::ExecutionLearningResponse {
    let mut learning = learning.clone();
    for (key, stats) in pushed_learning {
        if learning
            .get(key)
            .is_none_or(|existing| existing.updated_at_ms < stats.updated_at_ms)
        {
            learning.insert(
                key.clone(),
                ExecutionLearningStats {
                    samples: stats.samples,
                    filled_samples: stats.filled_samples,
                    fill_rate_total: stats.fill_rate_total,
                    cost_notional: stats.cost_notional,
                    all_in_cost_notional_bps: stats.all_in_cost_notional_bps,
                    updated_at_ms: stats.updated_at_ms,
                },
            );
        }
    }
    let learned_order_type =
        learned_execution_order_type(context, baseline, &learning, allow_adaptation);
    let (schedule_depth_threshold, schedule_threshold_samples) =
        learned_schedule_depth_threshold(&learning, context);
    let parameter_profiles = ALL_ORDER_TYPES
        .iter()
        .map(|kind| {
            let name = serde_json::to_value(kind)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default();
            (
                name,
                choose_parameter_profile(
                    *kind,
                    context,
                    &learning,
                    exploratory || should_explore_execution_profile(signal, *kind),
                ),
            )
        })
        .collect();
    strategy::ExecutionLearningResponse {
        learned_order_type,
        schedule_depth_threshold,
        schedule_threshold_samples,
        parameter_profiles,
    }
}

async fn prepare_signal_execution_plan(
    state: &AutoTraderState,
    settings: Settings,
    signal: &Signal,
    dashboard: &Dashboard,
    market: &serde_json::Value,
    now_ms: u64,
    quote_rate: f64,
    exploratory: bool,
    force_ioc_fallback: bool,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
) -> Result<SignalExecutionPlan, String> {
    let strategy = signal.strategy.as_deref().and_then(Strategy::from_name);
    let paper_learning_exploration = paper_learning_exploration_active(&settings, now_ms)
        && strategy.is_some_and(|kind| kind.is_live_insight());
    let mut exploratory = exploratory || paper_learning_exploration;
    // Stale walk-forward data cannot justify a full-size strict entry, but it
    // should not veto a currently selected signal outright. Treat it as a
    // bounded exploration so the strategy can keep trading while fresh
    // outcomes are collected.
    let stale_walk_forward_reason = (!exploratory)
        .then(|| {
            strategy
                .filter(|kind| !kind.is_dated_carry())
                .and_then(|kind| validated_strategy_entry_edge(signal, kind).err())
                .filter(|reason| reason.starts_with("walk_forward_stale:"))
        })
        .flatten();
    if let Some(reason) = stale_walk_forward_reason.as_deref() {
        exploratory = true;
        crate::service_log!(
            "WARN",
            "Stale walk-forward evidence downgraded selected signal to bounded exploration: symbol={} strategy={} reason={}",
            signal.symbol,
            strategy.map(Strategy::as_str).unwrap_or("unknown"),
            reason,
        );
    }
    let strategy_profile = strategy
        .map(|strategy| learned_strategy_parameter_profile(&settings, &signal.symbol, strategy));
    let is_portfolio_rebalance = is_portfolio_rebalance_signal(signal, strategy);
    // An explicitly selected exploratory entry is bounded by the exploration
    // notional cap and its own evidence/context checks below. Do not let a
    // learned-pause walk-forward freshness check veto that capped probe after
    // candidate selection has already authorized it.
    strategy_entry_walk_forward_gate(
        &settings,
        signal,
        strategy,
        is_portfolio_rebalance,
        paper_learning_exploration,
        exploratory,
        now_ms,
    )?;
    let strategy_order_size_multiplier = if is_portfolio_rebalance {
        1.0
    } else {
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
    let baseline_stop = stop_parameter.unwrap_or(counterfactual_stop_loss_percent);
    let baseline_target = target_parameter.unwrap_or(counterfactual_take_profit_percent);
    // Generic signal walk-forward is no longer an exit-policy source. Stop and
    // target values come from the selected strategy profile or configured
    // defaults, with Learning-owned post-exit feedback applied below.
    let configured_stop = baseline_stop;
    let configured_target = baseline_target;
    let active_policy = strategy.and_then(|kind| {
        let selected_profile_id = strategy_profile
            .as_ref()
            .and_then(|profile| profile.profile_id.as_deref())
            .or_else(|| {
                signal
                    .strategy_decision
                    .as_ref()
                    .and_then(|decision| decision.get("learning_profile_id"))
                    .and_then(serde_json::Value::as_str)
            });
        let selected_profile_version = strategy_profile
            .as_ref()
            .and_then(|profile| profile.profile_version)
            .or_else(|| {
                signal
                    .strategy_decision
                    .as_ref()
                    .and_then(|decision| decision.get("learning_profile_version"))
                    .and_then(serde_json::Value::as_u64)
            });
        active_exit_protection_policy(
            &settings,
            &signal.symbol,
            kind.as_str(),
            selected_profile_id,
            selected_profile_version,
        )
    });
    let learned_exit_policy = active_policy.is_some();
    let contextual_stop = (!explicit_stop && !learned_exit_policy)
        .then(|| {
            let kind = strategy?;
            let parameters = signal_strategy_parameters(signal);
            let profile_id = strategy::profile_id(&signal.symbol, kind.as_str(), &parameters);
            settings
                .contextual_stop_loss_percents
                .get(&format!("{profile_id}|{}", market_context(signal)))
                .copied()
                .filter(|value| value.is_finite())
                .filter(|value| {
                    kind.parameter_specs()
                        .iter()
                        .find(|spec| spec.key.as_str() == "stop_loss_percent")
                        .is_some_and(|spec| *value >= spec.min && *value <= spec.max)
                })
        })
        .flatten();
    let selected_stop_loss_percent = contextual_stop.unwrap_or(configured_stop);
    let effective_stop_loss_percent = effective_entry_stop(
        strategy,
        selected_stop_loss_percent,
        signal.components.volatility_1m_percent,
    );
    let baseline_type = if force_ioc_fallback {
        AutoOrderType::Ioc
    } else if exploratory {
        AutoOrderType::PostOnlyLimit
    } else {
        choose_order_type(&signal, settings.risk_appetite)
    };
    let risk_size_scale = risk_size_scale(settings.risk_appetite);
    let strategy_size_learning = strategy.filter(|kind| kind.is_dated_carry()).map(|kind| {
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
    let learned_size_multiplier = if !exploratory {
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
    let normal_order_cap = if strategy.is_some() && !exploratory {
        settings.max_order_notional * STRATEGY_BASE_ORDER_FRACTION
    } else {
        settings.max_order_notional
    };
    let exploration_cap = if exploratory {
        exploratory_notional_cap(&settings, risk_size_scale)
    } else {
        normal_order_cap
    };
    let rebalance_delta = strategy
        .filter(|kind| *kind == Strategy::RiskAdjustedTrendPortfolio)
        .and_then(|_| signal.strategy_decision.as_ref())
        .and_then(|decision| decision.get("portfolio_rebalance_delta_usd"))
        .and_then(serde_json::Value::as_f64)
        .filter(|delta| delta.is_finite() && *delta > 0.0);
    let exploration_risk_adjustment = if exploratory {
        contextual_exploration_risk_adjustment(signal, &settings, now_ms)
    } else {
        ExplorationRiskAdjustment::neutral()
    };
    let loss_cluster_scale = if !(is_portfolio_rebalance && signal.direction == "bearish")
        && has_recent_profile_loss_cluster(signal, &settings, now_ms)
    {
        LOSS_CLUSTER_SIZE_MULTIPLIER
    } else {
        1.0
    };
    let entry_risk_scale = exploration_risk_adjustment.scale.min(loss_cluster_scale);
    let risk_order_cap = (risk_bounded_order_cap(&settings, exploration_cap)
        * signal_confidence_size_multiplier(&signal)
        * learned_size_multiplier
        * entry_risk_scale
        * strategy_order_size_multiplier
        * if rebalance_delta.is_some() {
            1.0
        } else {
            agreement_size_multiplier
        })
    .min(rebalance_delta.unwrap_or(f64::INFINITY))
    .min(settings.max_order_notional);
    let stop_headroom_multiplier = if strategy.is_some_and(|kind| !kind.uses_protective_stops()) {
        1.0
    } else if matches!(baseline_type, AutoOrderType::Vwap | AutoOrderType::Twap) {
        2.0
    } else {
        1.0
    };
    let total_allocated = allocated_notional(&settings);
    let strategy_remaining_budget = if rebalance_delta.is_some() {
        f64::INFINITY
    } else {
        signal
            .strategy
            .as_deref()
            .and_then(Strategy::from_name)
            .map(|kind| {
                if kind.is_dated_carry() {
                    f64::INFINITY
                } else {
                    signal_candidate_budget(
                        state,
                        &settings,
                        dashboard,
                        kind,
                        &signal,
                        exploratory,
                        now_ms,
                    ) - allocated_notional_for_candidate(&settings, kind.as_str(), &signal.symbol)
                }
            })
            .unwrap_or(f64::INFINITY)
    };
    let remaining_budget = if rebalance_delta.is_some() && signal.direction == "bearish" {
        f64::INFINITY
    } else {
        (settings.daily_notional_limit - total_allocated)
            .min(strategy_remaining_budget)
            .max(0.0)
    };
    let mut notional = risk_order_cap.min(remaining_budget / stop_headroom_multiplier);
    // The pre-overlay stop establishes the per-entry monetary-risk allowance.
    // A selected profile can widen its stop without increasing that allowance;
    // a tighter stop never raises notional above the existing capital/risk cap.
    if strategy.is_some_and(|kind| kind.uses_protective_stops()) {
        let round_trip_cost_bps = 2.0 * configured_taker_fee_rate() * 10_000.0
            + signal
                .components
                .aggressor_cost_bps
                .filter(|cost| cost.is_finite() && *cost > 0.0)
                .unwrap_or(0.0);
        notional = stop_risk_bounded_notional(
            notional,
            counterfactual_stop_loss_percent,
            effective_stop_loss_percent,
            round_trip_cost_bps,
        );
    }
    if notional <= 0.0 {
        return Err("Strategy category or total notional capacity is fully allocated".into());
    }
    let baseline_params = ExecutionParameters {
        post_only_timeout_minutes: signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(|parameters| parameters.get("post_only_timeout_minutes"))
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && (1.0..=15.0).contains(value))
            .or_else(|| {
                strategy_profile.as_ref().and_then(|profile| {
                    strategy_profile_value(profile, "post_only_timeout_minutes")
                })
            })
            .unwrap_or(5.0) as u64,
        vwap_duration_minutes: settings.vwap_duration_minutes,
        vwap_participation_rate: settings.vwap_participation_percent / 100.0,
        limit_offset_bps: settings.limit_offset_bps,
        trigger_offset_bps: settings.trigger_offset_bps,
        trailing_distance_percent: settings.trailing_distance_percent,
    };
    // Cold-start paper probes are intentionally smaller than normal entries.
    // Let their actual capped notional define the minimum; otherwise the
    // $10 deep-book floor rejects a $2.50–$5 exploratory order before the
    // slippage and cost gates can evaluate it.
    let minimum_book_order = minimum_book_order_for_entry(exploratory, notional);
    let deep_book_started = Instant::now();
    let deep_book = assess_deep_book(
        market,
        &signal.symbol,
        signal.direction == "bullish",
        notional * quote_rate,
        minimum_book_order * quote_rate,
    );
    let deep_book_ms = deep_book_started.elapsed().as_secs_f64() * 1000.0;
    let horizon_take_profit_percent = None;
    let effective_take_profit_percent = if learned_exit_policy {
        configured_target
    } else {
        confidence_adjusted_take_profit_percent(
            &closed_trade_learning_returns(&settings),
            configured_target,
            effective_stop_loss_percent,
        )
    };
    let exit_parameter_provenance = exit_parameter_provenance(
        &settings,
        signal,
        strategy_profile.as_ref(),
        strategy,
        active_policy,
        contextual_stop,
        selected_stop_loss_percent,
        effective_stop_loss_percent,
        configured_target,
        effective_take_profit_percent,
    );
    let context = market_context(&signal);
    let execution_learning_started = Instant::now();
    let execution_learning = execution_learning_decision(
        &context,
        baseline_type,
        signal,
        settings.risk_appetite,
        true,
        exploratory,
        &settings.execution_learning,
        &settings.learned_execution_stats,
    );
    let imported_execution_learning = false;
    let execution_learning_ms = execution_learning_started.elapsed().as_secs_f64() * 1_000.0;
    let learned_type = execution_learning
        .learned_order_type
        .as_deref()
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
    let learned_order_type = if force_ioc_fallback {
        AutoOrderType::Ioc
    } else if exploratory {
        // Exploratory entries stay passive unless a stronger selected signal
        // supersedes its own still-unfilled maker order through IOC fallback.
        AutoOrderType::PostOnlyLimit
    } else {
        score_execution_order_type(
            signal,
            settings.risk_appetite,
            notional * quote_rate,
            baseline_type,
            learned_type,
            deep_book,
            schedule_depth_threshold,
        )
    };
    let order_type = learned_order_type;
    let order_type_name = serde_json::to_value(order_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    let profile = execution_learning
        .parameter_profiles
        .get(&order_type_name)
        .copied()
        .unwrap_or(0);
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
    let learning_context = format!("{context}:P{profile}:depth:{depth_ratio_bucket}");
    drop(settings);
    let side = if signal.direction == "bullish" {
        "buy"
    } else {
        "sell"
    };
    Ok(SignalExecutionPlan {
        exploratory,
        effective_stop_loss_percent,
        effective_take_profit_percent,
        exit_parameter_provenance,
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
        execution_learning_ms,
        imported_execution_learning,
        exploration_risk_adjustment: ExplorationRiskAdjustment {
            scale: entry_risk_scale,
            ..exploration_risk_adjustment
        },
    })
}

#[cfg(test)]
mod captured_maximum_hold_specification_tests {
    use super::*;

    #[test]
    fn entry_provenance_captures_registered_hold_days_bounds_only() {
        let specification =
            captured_maximum_hold_days_specification(Some(Strategy::Rsi2OversoldRebound));
        let captured = &specification["maximum_hold_days"];
        let registered = Strategy::Rsi2OversoldRebound
            .parameter_specs()
            .into_iter()
            .find(|spec| spec.key == StrategyParameterKey::MaximumHoldDays)
            .unwrap();
        assert_eq!(specification["schema_version"], 1);
        assert_eq!(captured["key"], "maximum_hold_days");
        assert_eq!(captured["unit"], "days");
        assert_eq!(captured["default"], registered.default);
        assert_eq!(captured["min"], registered.min);
        assert_eq!(captured["max"], registered.max);
        assert_eq!(captured["step"], registered.step);
        assert_eq!(
            captured_maximum_hold_days_specification(None)["maximum_hold_days"],
            serde_json::Value::Null
        );
    }
}

#[cfg(test)]
mod risk_adjusted_monthly_momentum_sizing_tests {
    use super::*;

    #[test]
    fn strategy_uses_volatility_multiplier_and_fails_closed_when_missing() {
        let strategy = Some(Strategy::RiskAdjustedMonthlyMomentum);
        assert_eq!(
            signal_strategy_order_size_multiplier(
                strategy,
                Some(&serde_json::json!({
                    "strategy_order_size_multiplier": 0.4
                }))
            )
            .unwrap(),
            0.4
        );
        assert!(signal_strategy_order_size_multiplier(strategy, None).is_err());
        assert!(
            signal_strategy_order_size_multiplier(
                strategy,
                Some(&serde_json::json!({
                    "strategy_order_size_multiplier": 2.0
                }))
            )
            .is_err()
        );
        assert_eq!(
            signal_strategy_order_size_multiplier(Some(Strategy::Donchian55Btc200), None).unwrap(),
            1.0
        );
    }

    #[test]
    fn recent_strategy_learning_includes_scout_outcomes_across_symbols() {
        let strategy = Strategy::Donchian55Btc200;
        let mut settings = Settings::default();
        for (symbol, samples, positive, realized_pnl) in [
            ("BTC-USDT", 1, 1, 2.0),
            ("ETH-USDT", 1, 0, -1.0),
            ("SOL-USDT", 1, 1, 0.5),
        ] {
            let parameters = BTreeMap::new();
            let profile_id = strategy::profile_id(symbol, strategy.as_str(), &parameters);
            settings.learned_profiles.insert(
                format!("{symbol}:{}", strategy.as_str()),
                strategy::StrategyProfile {
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

    fn exploration_signal(timestamp: u64, walk_forward: Option<serde_json::Value>) -> Signal {
        let strategy = Strategy::Donchian20Btc200;
        let parameters = strategy
            .parameter_specs()
            .into_iter()
            .filter(|spec| {
                matches!(
                    spec.key,
                    StrategyParameterKey::ScoutMinimumWalkForwardSamples
                        | StrategyParameterKey::ScoutMaximumEvidenceAgeSeconds
                )
            })
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let mut signal = signal(strategy.as_str(), 1.0, timestamp);
        let mut decision = serde_json::json!({"strategy_parameters": parameters});
        if let Some(evidence) = walk_forward {
            decision["strategy_walk_forward"] = evidence;
        }
        signal.strategy_decision = Some(decision);
        signal
    }

    #[test]
    fn selected_exploration_bypasses_absent_or_stale_evidence_but_strict_entry_does_not() {
        let now_ms = current_time_ms();
        let mut settings = Settings::default();
        settings
            .learned_strategy_size_multipliers
            .insert(Strategy::Donchian20Btc200.as_str().to_owned(), 0.0);
        let stale_evidence = serde_json::json!({
            "sampleCount": 100,
            "lastSampleTimestampMs": 1,
            "effectiveSampleSize": 100.0,
            "expectancyCiLowPercent": -1.0,
        });

        for evidence in [None, Some(stale_evidence)] {
            let signal = exploration_signal(now_ms, evidence);
            assert_eq!(
                strategy_entry_walk_forward_gate(
                    &settings,
                    &signal,
                    Some(Strategy::Donchian20Btc200),
                    false,
                    false,
                    true,
                    now_ms,
                ),
                Ok(()),
                "selected exploration remains available to collect fresh evidence"
            );
        }

        let strict_signal = exploration_signal(now_ms, None);
        let strict = strategy_entry_walk_forward_gate(
            &settings,
            &strict_signal,
            Some(Strategy::Donchian20Btc200),
            false,
            false,
            false,
            now_ms,
        );
        assert!(strict.is_err());
        assert!(strict.unwrap_err().contains("walk_forward_missing"));
    }

    #[test]
    fn exploratory_notional_stays_risk_capped_and_deep_book_floor_tracks_small_probe() {
        let mut settings = Settings::default();
        settings.max_order_notional = 1_000.0;
        settings.risk_limited_sizing = true;
        settings.exploration_percent = 50.0;
        let exploration_cap = exploratory_notional_cap(&settings, 1.0);
        assert_eq!(exploration_cap, 500.0);
        assert_eq!(risk_bounded_order_cap(&settings, exploration_cap), 250.0);

        settings.exploration_percent = 0.5;
        let small_probe =
            risk_bounded_order_cap(&settings, exploratory_notional_cap(&settings, 1.0));
        assert_eq!(small_probe, 5.0);
        assert_eq!(minimum_book_order_for_entry(true, small_probe), 5.0);
        assert_eq!(minimum_book_order_for_entry(false, small_probe), 10.0);
    }

    #[test]
    fn selected_stop_preserves_pre_overlay_monetary_risk_and_existing_caps() {
        let base_notional = 400.0;
        let reference_stop = 0.50;
        let round_trip_fee_and_cost_bps = 20.0;
        let reference_risk = base_notional * (reference_stop + 0.20) / 100.0;

        let wider_stop_notional = stop_risk_bounded_notional(
            base_notional,
            reference_stop,
            1.00,
            round_trip_fee_and_cost_bps,
        );
        assert!((wider_stop_notional - 400.0 * 0.70 / 1.20).abs() < 1e-9);
        assert!((wider_stop_notional * (1.00 + 0.20) / 100.0 - reference_risk).abs() < 1e-9);

        // A narrower profile may reduce risk, but cannot increase the existing
        // order/capital cap to spend the unused allowance.
        let tighter_notional = stop_risk_bounded_notional(
            base_notional,
            reference_stop,
            0.25,
            round_trip_fee_and_cost_bps,
        );
        assert_eq!(tighter_notional, base_notional);
        assert!(tighter_notional * (0.25 + 0.20) / 100.0 < reference_risk);

        // The actual available-capital reservation remains a separate hard
        // cap before stop sizing, and the helper cannot exceed it.
        let available_capital = 75.0;
        let capital_capped = stop_risk_bounded_notional(
            base_notional.min(available_capital),
            reference_stop,
            1.00,
            round_trip_fee_and_cost_bps,
        );
        assert!(capital_capped <= available_capital);
        assert!(capital_capped < available_capital);

        assert_eq!(
            stop_risk_bounded_notional(base_notional, reference_stop, 0.0, 20.0),
            0.0,
            "a missing or invalid selected protective stop must fail closed"
        );
        assert_eq!(
            stop_risk_bounded_notional(base_notional, reference_stop, 1.0, f64::NAN),
            0.0,
            "invalid cost evidence must fail closed"
        );
    }

    fn signal(strategy: &str, strength: f64, timestamp: u64) -> Signal {
        Signal {
            range_location: None,
            symbol: "ETH-USDT".into(),
            direction: "bullish".into(),
            strength,
            timestamp,
            stale: false,
            score: strength,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: true,
            strategy: Some(strategy.into()),
            strategy_decision: None,
            market_regime: None,
            market_state: None,
            news_attribution: None,
            entry_gate: Some(serde_json::json!({"selection_mode":"exploratory"})),
        }
    }

    fn signal_with_edge(
        strategy: &str,
        strength: f64,
        timestamp: u64,
        lower_bound_percent: Option<serde_json::Value>,
        sample_timestamp_ms: u64,
    ) -> Signal {
        let mut signal = signal(strategy, strength, timestamp);
        let mut decision = serde_json::json!({
            "strategy_parameters": {
                "scout_minimum_walk_forward_samples": 3.0,
                "scout_maximum_evidence_age_seconds": 86400.0
            }
        });
        if let Some(lower_bound) = lower_bound_percent {
            signal.entry_gate = Some(serde_json::json!({"selection_mode":"strict"}));
            decision["strategy_walk_forward"] = serde_json::json!({
                "sampleCount": 3,
                "effectiveSampleSize": 3.0,
                "expectancyCiLowPercent": lower_bound,
                "lastSampleTimestampMs": sample_timestamp_ms
            });
        }
        signal.strategy_decision = Some(decision);
        signal
    }

    #[test]
    fn case_confidence_scales_size_and_respects_score_direction() {
        let weak = signal("live-momentum", 0.2, 1);
        let moderate = signal("live-momentum", 0.6, 1);
        let strong = signal("live-momentum", 1.0, 1);

        let weak_size = signal_confidence_size_multiplier(&weak);
        let moderate_size = signal_confidence_size_multiplier(&moderate);
        let strong_size = signal_confidence_size_multiplier(&strong);
        assert!(weak_size < moderate_size && moderate_size < strong_size);
        assert_eq!(strong_size, 1.0);

        let mut contradictory = strong;
        contradictory.score = -1.0;
        assert_eq!(signal_confidence_size_multiplier(&contradictory), 0.25);
    }

    fn dashboard(best: Vec<Signal>) -> Dashboard {
        Dashboard {
            best,
            missed_opportunity_bands: Arc::new(Vec::new()),
            markets: Arc::new(BTreeMap::new()),
            queued_at: None,
            queue_wait_ms: None,
        }
    }

    #[test]
    fn recent_net_edge_and_learned_size_multiplier_handle_empty_profitable_and_losing_samples() {
        assert_eq!(recent_net_pnl_edge(&LearningStats::default()), None);
        let zero = LearningStats {
            recent_trade_pnls: vec![(current_time_ms(), 0.0)],
            ..LearningStats::default()
        };
        assert_eq!(recent_net_pnl_edge(&zero), Some(0.0));
        let wins = LearningStats {
            samples: 10,
            positive: 8,
            recent_trade_pnls: vec![(current_time_ms(), 5.0), (current_time_ms(), 3.0)],
            ..LearningStats::default()
        };
        let losses = LearningStats {
            samples: 10,
            positive: 2,
            recent_trade_pnls: vec![(current_time_ms(), -8.0), (current_time_ms(), 1.0)],
            ..LearningStats::default()
        };
        assert!(recent_net_pnl_edge(&wins).unwrap() > 0.0);
        assert!(learned_strategy_size_multiplier(&wins) > 1.0);
        assert!(recent_net_pnl_edge(&losses).unwrap() < 0.0);
        assert!(learned_strategy_size_multiplier(&losses) < 1.0);
        assert_eq!(
            learned_strategy_size_multiplier(&LearningStats::default()),
            1.0
        );
        assert_eq!(
            learned_strategy_size_multiplier(&LearningStats {
                samples: 2,
                positive: 3,
                ..LearningStats::default()
            }),
            1.0
        );
    }

    #[test]
    fn signal_weighted_daily_budget_leaves_unproven_variable_share_unused() {
        let mut settings = Settings::default();
        settings.strategies = vec![Strategy::Donchian20Btc200, Strategy::Donchian55Btc200];
        settings.daily_notional_limit = 500.0;
        settings.daily_strategy_notional_limit = 500.0;
        let no_signals = dashboard(Vec::new());
        assert_eq!(
            signal_weighted_strategy_budget(
                &settings,
                &no_signals,
                Strategy::Donchian20Btc200,
                StrategyCategory::Daily,
                100_000
            ),
            0.0
        );
        let signals = dashboard(vec![
            signal("donchian-20-btc200", 1.0, 100_000),
            signal("donchian-20-btc200", 1.0, 100_000),
        ]);
        let favored = signal_weighted_strategy_budget(
            &settings,
            &signals,
            Strategy::Donchian20Btc200,
            StrategyCategory::Daily,
            100_000,
        );
        let other = signal_weighted_strategy_budget(
            &settings,
            &signals,
            Strategy::Donchian55Btc200,
            StrategyCategory::Daily,
            100_000,
        );
        assert!(favored > other);
        assert_eq!(other, 0.0);
        assert_eq!(favored, 250.0);
        assert!(favored + other < 500.0);

        let stale = dashboard(vec![signal("donchian-20-btc200", 1.0, 1)]);
        assert_eq!(
            signal_weighted_strategy_budget(
                &settings,
                &stale,
                Strategy::Donchian20Btc200,
                StrategyCategory::Daily,
                100_000_000
            ),
            0.0
        );
    }

    #[test]
    fn candidates_without_validated_edge_receive_only_the_exploration_floor() {
        let mut settings = Settings::default();
        settings.strategies = vec![Strategy::Donchian20Btc200, Strategy::Donchian55Btc200];
        settings.daily_notional_limit = 500.0;
        settings.daily_strategy_notional_limit = 500.0;
        let mut zero_weight = signal("donchian-55-btc200", 0.0, 100_000);
        zero_weight.direction = "bearish".into();
        let candidates = dashboard(vec![
            signal("donchian-20-btc200", 1.0, 100_000),
            zero_weight,
        ]);
        let positive = signal_weighted_strategy_budget(
            &settings,
            &candidates,
            Strategy::Donchian20Btc200,
            StrategyCategory::Daily,
            100_000,
        );
        let zero = signal_weighted_strategy_budget(
            &settings,
            &candidates,
            Strategy::Donchian55Btc200,
            StrategyCategory::Daily,
            100_000,
        );
        assert_eq!(zero, 125.0);
        assert_eq!(positive, 125.0);
        assert_eq!(positive + zero, 250.0);
    }

    #[test]
    fn validated_positive_edge_and_directional_confidence_weight_variable_budget() {
        let now_ms = 100_000;
        let first = Strategy::Donchian20Btc200;
        let second = Strategy::Donchian55Btc200;
        let mut settings = Settings::default();
        settings.strategies = vec![first, second];
        settings.daily_notional_limit = 600.0;
        settings.daily_strategy_notional_limit = 600.0;
        settings
            .learned_strategy_size_multipliers
            .insert(first.as_str().into(), 1.2);

        let long = signal_with_edge(
            first.as_str(),
            1.0,
            now_ms,
            Some(serde_json::json!(1.0)),
            now_ms - 1_000,
        );
        let mut short = signal_with_edge(
            second.as_str(),
            0.25,
            now_ms,
            Some(serde_json::json!(2.0)),
            now_ms - 1_000,
        );
        short.direction = "bearish".into();
        short.score = -0.25;

        let candidates = dashboard(vec![long.clone(), short.clone()]);
        let first_budget = signal_weighted_strategy_budget(
            &settings,
            &candidates,
            first,
            StrategyCategory::Daily,
            now_ms,
        );
        let second_budget = signal_weighted_strategy_budget(
            &settings,
            &candidates,
            second,
            StrategyCategory::Daily,
            now_ms,
        );
        assert!(first_budget > second_budget);
        assert!(first_budget > 150.0 && second_budget > 150.0);
        assert!((first_budget + second_budget - 600.0).abs() < 1e-9);

        let mut contradictory = short;
        contradictory.score = 0.25;
        let mixed = dashboard(vec![long, contradictory]);
        let first_with_mixed = signal_weighted_strategy_budget(
            &settings,
            &mixed,
            first,
            StrategyCategory::Daily,
            now_ms,
        );
        let second_with_mixed = signal_weighted_strategy_budget(
            &settings,
            &mixed,
            second,
            StrategyCategory::Daily,
            now_ms,
        );
        assert_eq!(second_with_mixed, 150.0);
        assert_eq!(first_with_mixed, 450.0);
    }

    #[test]
    fn variable_share_is_monotone_in_edge_and_directional_confidence() {
        let now_ms = 100_000;
        let first = Strategy::Donchian20Btc200;
        let second = Strategy::Donchian55Btc200;
        let mut settings = Settings::default();
        settings.strategies = vec![first, second];
        settings.daily_notional_limit = 600.0;
        settings.daily_strategy_notional_limit = 600.0;

        let lower_edge = signal_with_edge(
            first.as_str(),
            0.5,
            now_ms,
            Some(serde_json::json!(1.0)),
            now_ms - 1_000,
        );
        let higher_edge = signal_with_edge(
            second.as_str(),
            0.5,
            now_ms,
            Some(serde_json::json!(2.0)),
            now_ms - 1_000,
        );
        let edge_dashboard = dashboard(vec![lower_edge, higher_edge]);
        let lower_edge_budget = signal_weighted_strategy_budget(
            &settings,
            &edge_dashboard,
            first,
            StrategyCategory::Daily,
            now_ms,
        );
        let higher_edge_budget = signal_weighted_strategy_budget(
            &settings,
            &edge_dashboard,
            second,
            StrategyCategory::Daily,
            now_ms,
        );
        assert!(higher_edge_budget > lower_edge_budget);

        let lower_confidence = signal_with_edge(
            first.as_str(),
            0.25,
            now_ms,
            Some(serde_json::json!(1.0)),
            now_ms - 1_000,
        );
        let higher_confidence = signal_with_edge(
            second.as_str(),
            1.0,
            now_ms,
            Some(serde_json::json!(1.0)),
            now_ms - 1_000,
        );
        let confidence_dashboard = dashboard(vec![lower_confidence, higher_confidence]);
        let lower_confidence_budget = signal_weighted_strategy_budget(
            &settings,
            &confidence_dashboard,
            first,
            StrategyCategory::Daily,
            now_ms,
        );
        let higher_confidence_budget = signal_weighted_strategy_budget(
            &settings,
            &confidence_dashboard,
            second,
            StrategyCategory::Daily,
            now_ms,
        );
        assert!(higher_confidence_budget > lower_confidence_budget);
    }

    #[test]
    fn missing_stale_future_negative_and_malformed_edges_never_get_variable_share() {
        let now_ms = 200_000_000;
        let first = Strategy::Donchian20Btc200;
        let second = Strategy::Donchian55Btc200;
        let mut settings = Settings::default();
        settings.strategies = vec![first, second];
        settings.daily_notional_limit = 600.0;
        settings.daily_strategy_notional_limit = 600.0;
        let proven = signal_with_edge(
            first.as_str(),
            1.0,
            now_ms,
            Some(serde_json::json!(1.0)),
            now_ms - 1_000,
        );
        let unproven = [
            signal_with_edge(second.as_str(), 1.0, now_ms, None, now_ms - 1_000),
            signal_with_edge(
                second.as_str(),
                1.0,
                now_ms,
                Some(serde_json::json!(1.0)),
                now_ms - 86_401_000,
            ),
            signal_with_edge(
                second.as_str(),
                1.0,
                now_ms,
                Some(serde_json::json!(-0.1)),
                now_ms - 1_000,
            ),
            signal_with_edge(
                second.as_str(),
                1.0,
                now_ms,
                Some(serde_json::json!("not-a-number")),
                now_ms - 1_000,
            ),
            signal_with_edge(
                second.as_str(),
                1.0,
                now_ms,
                Some(serde_json::json!(1.0)),
                now_ms + 1,
            ),
        ];
        for mut candidate in unproven {
            // These candidates have no validated strict edge. They receive
            // only the exploration floor when explicitly selected as probes.
            candidate.entry_gate = Some(serde_json::json!({"selection_mode":"exploratory"}));
            let candidates = dashboard(vec![proven.clone(), candidate]);
            let first_budget = signal_weighted_strategy_budget(
                &settings,
                &candidates,
                first,
                StrategyCategory::Daily,
                now_ms,
            );
            let second_budget = signal_weighted_strategy_budget(
                &settings,
                &candidates,
                second,
                StrategyCategory::Daily,
                now_ms,
            );
            assert_eq!(second_budget, 150.0);
            assert_eq!(first_budget, 450.0);
            assert!((first_budget + second_budget - 600.0).abs() < 1e-9);
        }

        let mut future_signal = signal_with_edge(
            second.as_str(),
            1.0,
            now_ms + 1,
            Some(serde_json::json!(2.0)),
            now_ms - 1_000,
        );
        future_signal.score = 1.0;
        let future_candidates = dashboard(vec![proven.clone(), future_signal]);
        assert_eq!(
            signal_weighted_strategy_budget(
                &settings,
                &future_candidates,
                second,
                StrategyCategory::Daily,
                now_ms,
            ),
            0.0
        );

        let no_edge = dashboard(vec![
            signal_with_edge(first.as_str(), 1.0, now_ms, None, now_ms - 1_000),
            signal_with_edge(second.as_str(), 1.0, now_ms, None, now_ms - 1_000),
        ]);
        let no_edge_total = [first, second]
            .into_iter()
            .map(|kind| {
                signal_weighted_strategy_budget(
                    &settings,
                    &no_edge,
                    kind,
                    StrategyCategory::Daily,
                    now_ms,
                )
            })
            .sum::<f64>();
        assert_eq!(no_edge_total, 300.0);
    }

    #[test]
    fn same_strategy_multiple_signals_do_not_multiply_its_edge_weight() {
        let now_ms = 100_000;
        let first = Strategy::Donchian20Btc200;
        let second = Strategy::Donchian55Btc200;
        let mut settings = Settings::default();
        settings.strategies = vec![first, second];
        settings.daily_notional_limit = 600.0;
        settings.daily_strategy_notional_limit = 600.0;
        let candidate = signal_with_edge(
            first.as_str(),
            0.8,
            now_ms,
            Some(serde_json::json!(1.0)),
            now_ms - 1_000,
        );
        let other = signal_with_edge(
            second.as_str(),
            0.5,
            now_ms,
            Some(serde_json::json!(1.0)),
            now_ms - 1_000,
        );
        let one_signal = dashboard(vec![candidate.clone(), other.clone()]);
        let multiple_signals = dashboard(vec![candidate.clone(), candidate, other]);
        assert_eq!(
            signal_weighted_strategy_budget(
                &settings,
                &one_signal,
                first,
                StrategyCategory::Daily,
                now_ms,
            ),
            signal_weighted_strategy_budget(
                &settings,
                &multiple_signals,
                first,
                StrategyCategory::Daily,
                now_ms,
            )
        );
    }

    #[test]
    fn stale_wrong_direction_and_non_candidate_signals_do_not_activate_shares() {
        let mut settings = Settings::default();
        settings.strategies = vec![Strategy::Donchian20Btc200, Strategy::Donchian55Btc200];
        settings.daily_notional_limit = 500.0;
        settings.daily_strategy_notional_limit = 500.0;
        let mut wrong_direction = signal("donchian-55-btc200", 1.0, 100_000);
        wrong_direction.direction = "flat".into();
        let mut non_candidate = signal("donchian-55-btc200", 1.0, 100_000);
        non_candidate.strategy_candidate = false;
        let candidates = dashboard(vec![
            signal("donchian-20-btc200", 1.0, 1),
            wrong_direction,
            non_candidate,
        ]);
        assert_eq!(
            signal_weighted_strategy_budget(
                &settings,
                &candidates,
                Strategy::Donchian20Btc200,
                StrategyCategory::Daily,
                100_000_000,
            ),
            0.0
        );
        assert_eq!(
            signal_weighted_strategy_budget(
                &settings,
                &candidates,
                Strategy::Donchian55Btc200,
                StrategyCategory::Daily,
                100_000_000,
            ),
            0.0
        );
    }

    #[test]
    fn legacy_budget_maps_do_not_override_the_shared_candidate_split() {
        let mut settings = Settings::default();
        settings.strategies = vec![Strategy::Donchian20Btc200, Strategy::Donchian55Btc200];
        settings
            .daily_strategy_budgets
            .insert("donchian-20-btc200".into(), 0.0);
        settings
            .daily_strategy_budgets
            .insert("donchian-55-btc200".into(), 900.0);
        settings.daily_strategy_notional_limit = 0.0;
        let signals = dashboard(vec![signal("donchian-55-btc200", 1.0, 100_000)]);
        assert_eq!(
            signal_weighted_strategy_budget(
                &settings,
                &signals,
                Strategy::Donchian20Btc200,
                StrategyCategory::Daily,
                100_000
            ),
            0.0
        );
        assert_eq!(daily_shared_pool_ceiling(&settings, "live-momentum"), 0.0);
        assert_eq!(
            intraday_shared_pool_ceiling(&settings, "donchian-20-btc200", 0),
            0.0
        );

        let strategy = Some(Strategy::RiskAdjustedMonthlyMomentum);
        assert_eq!(
            signal_strategy_order_size_multiplier(
                strategy,
                Some(&serde_json::json!({"strategy_order_size_multiplier":0.5}))
            )
            .unwrap(),
            0.5
        );
        assert!(
            signal_strategy_order_size_multiplier(
                strategy,
                Some(&serde_json::json!({"strategy_order_size_multiplier":0.0}))
            )
            .is_err()
        );
        assert_eq!(
            signal_strategy_order_size_multiplier(None, None).unwrap(),
            1.0
        );
        assert_eq!(
            signal_agreement_size_multiplier(Some(Strategy::Donchian20Btc200), None).unwrap(),
            1.0
        );
        assert_eq!(
            signal_agreement_size_multiplier(
                Some(Strategy::Donchian20Btc200),
                Some(&serde_json::json!({"strategy_agreement_size_multiplier":1.1}))
            )
            .unwrap(),
            1.1
        );
        assert!(
            signal_agreement_size_multiplier(
                Some(Strategy::Donchian20Btc200),
                Some(&serde_json::json!({"strategy_agreement_size_multiplier":1.3}))
            )
            .is_err()
        );
    }

    #[test]
    fn legacy_allocations_do_not_partition_global_pool_across_symbols_and_horizons() {
        let now_ms = 100_000;
        let daily_a = Strategy::Donchian20Btc200;
        let daily_b = Strategy::Donchian55Btc200;
        let intraday = Strategy::LiveMomentum;
        let mut settings = Settings::default();
        settings.strategies = vec![daily_a, daily_b, intraday];
        settings.daily_notional_limit = 600.0;
        settings.daily_strategy_notional_limit = 0.0;
        settings.intraday_notional_limit = 0.0;
        let mut first = signal_with_edge(
            daily_a.as_str(),
            1.0,
            now_ms,
            Some(serde_json::json!(0.5)),
            now_ms - 1_000,
        );
        first.entry_gate = Some(serde_json::json!({"selection_mode":"strict"}));
        let mut second = signal(daily_b.as_str(), 0.5, now_ms);
        second.symbol = "BTC-USDT".to_owned();
        let mut third = signal(intraday.as_str(), 0.5, now_ms);
        third.symbol = "SOL-USDT".to_owned();
        let dashboard = dashboard(vec![first, second, third]);
        let baseline = shared_candidate_allocations(&settings, &dashboard, now_ms, None);
        let baseline_total = baseline
            .values()
            .map(|allocation| allocation.effective_allocation)
            .sum::<f64>();
        assert_eq!(baseline.len(), 3);
        assert!((baseline_total - 600.0).abs() < 1e-9);
        assert!(baseline.contains_key(&(
            daily_a.as_str().to_owned(),
            "ETH-USDT".to_owned(),
            "daily".to_owned()
        )));
        assert!(baseline.contains_key(&(
            intraday.as_str().to_owned(),
            "SOL-USDT".to_owned(),
            "intraday".to_owned()
        )));

        settings
            .daily_strategy_budgets
            .insert(daily_a.as_str().to_owned(), 0.0);
        settings
            .daily_strategy_budgets
            .insert(daily_b.as_str().to_owned(), 900.0);
        settings
            .intraday_strategy_budgets
            .insert(intraday.as_str().to_owned(), 0.0);
        settings.daily_strategy_notional_limit = 1.0;
        settings.intraday_notional_limit = 1.0;
        let with_legacy = shared_candidate_allocations(&settings, &dashboard, now_ms, None);
        assert_eq!(
            with_legacy
                .values()
                .map(|allocation| allocation.effective_allocation)
                .sum::<f64>(),
            baseline_total
        );
        assert!(
            with_legacy
                .values()
                .all(|allocation| allocation.effective_allocation > 0.0)
        );
    }

    #[test]
    fn strict_candidate_without_validated_edge_requires_explicit_exploration_authorization() {
        let now_ms = 100_000;
        let strategy = Strategy::Donchian20Btc200;
        let mut settings = Settings::default();
        settings.strategies = vec![strategy];
        settings.daily_notional_limit = 500.0;
        let mut candidate = signal(strategy.as_str(), 0.8, now_ms);
        candidate.entry_gate = Some(serde_json::json!({"selection_mode":"strict"}));
        let dashboard = dashboard(vec![candidate.clone()]);
        assert!(shared_candidate_allocations(&settings, &dashboard, now_ms, None).is_empty());
        assert!(
            shared_candidate_allocations(&settings, &dashboard, now_ms, Some((&candidate, false)),)
                .is_empty()
        );

        let authorized =
            shared_candidate_allocations(&settings, &dashboard, now_ms, Some((&candidate, true)));
        let allocation = authorized
            .get(&(
                strategy.as_str().to_owned(),
                candidate.symbol.clone(),
                "daily".to_owned(),
            ))
            .expect("explicitly exploratory selection receives a bounded pool floor");
        assert_eq!(allocation.evidence_eligible, Some(false));
        assert_eq!(allocation.state, "exploratory");
        assert_eq!(allocation.effective_allocation, 250.0);
    }

    #[test]
    fn paused_strategy_has_no_entry_budget_and_can_reduce_a_portfolio_position() {
        let mut settings = Settings::default();
        let strategy = Strategy::RiskAdjustedTrendPortfolio;
        settings.strategies = vec![strategy, Strategy::Donchian20Btc200];
        settings.daily_strategy_notional_limit = 500.0;
        settings
            .learned_strategy_size_multipliers
            .insert(strategy.as_str().into(), 0.0);
        let signals = dashboard(vec![signal(strategy.as_str(), 1.0, 100_000)]);
        assert_eq!(
            signal_weighted_strategy_budget(
                &settings,
                &signals,
                strategy,
                StrategyCategory::Daily,
                100_000
            ),
            0.0
        );
        assert_eq!(
            portfolio_reduction_quantity(&settings, "ETH-USDT", strategy.as_str()),
            0.0
        );
        settings
            .strategy_positions
            .entry("ETH-USDT".into())
            .or_default()
            .insert(
                strategy.as_str().into(),
                TraderPosition {
                    quantity: 2.0,
                    ..TraderPosition::default()
                },
            );
        assert_eq!(
            portfolio_reduction_quantity(&settings, "ETH-USDT", strategy.as_str()),
            2.0
        );
        assert!(
            learned_pause_block_reason(
                &settings,
                Some(strategy),
                &signal(strategy.as_str(), 1.0, 0),
                true,
                "bullish",
                0
            )
            .is_some()
        );
        assert!(
            learned_pause_block_reason(
                &settings,
                Some(strategy),
                &signal(strategy.as_str(), 1.0, 0),
                false,
                "bearish",
                0
            )
            .is_some()
        );
        assert!(
            learned_pause_block_reason(
                &settings,
                Some(strategy),
                &signal(strategy.as_str(), 1.0, 0),
                true,
                "bearish",
                0
            )
            .is_none()
        );
        settings
            .daily_strategy_budgets
            .insert(strategy.as_str().into(), 200.0);
        assert_eq!(
            signal_weighted_strategy_budget(
                &settings,
                &signals,
                strategy,
                StrategyCategory::Daily,
                100_000
            ),
            0.0
        );
    }

    #[test]
    fn live_strategy_ignores_legacy_zero_outside_recovery_window() {
        let strategy = Strategy::LiveTradeFlow;
        let mut settings = Settings::default();
        settings
            .learned_strategy_size_multipliers
            .insert(strategy.as_str().to_owned(), 0.0);
        settings.paper_learning_exploration_until_ms = 10_000;

        assert_eq!(
            intraday_strategy_size_multiplier(&settings, strategy, 9_999),
            0.20
        );
        assert_eq!(
            intraday_strategy_size_multiplier(&settings, strategy, 10_000),
            1.0
        );
        assert_eq!(
            intraday_strategy_size_multiplier(&settings, strategy, 10_001),
            1.0
        );
        assert!(!learned_strategy_entry_paused(&settings, strategy, 10_000));
        assert!(!learned_strategy_entry_paused(&settings, strategy, 9_999));
    }

    #[test]
    fn live_entry_does_not_use_strategy_wide_pause() {
        let strategy = Strategy::LiveTradeFlow;
        let mut settings = Settings::default();
        settings
            .learned_strategy_size_multipliers
            .insert(strategy.as_str().to_owned(), 0.0);
        settings.paper_learning_exploration_until_ms = 10_000;
        let candidate = signal(strategy.as_str(), 0.8, 100_000);

        assert!(
            learned_pause_block_reason(
                &settings,
                Some(strategy),
                &candidate,
                false,
                "bullish",
                10_000,
            )
            .is_none()
        );
        assert!(
            learned_pause_block_reason(
                &settings,
                Some(strategy),
                &candidate,
                false,
                "bullish",
                9_999,
            )
            .is_none()
        );
    }

    #[test]
    fn entry_requires_positive_expectancy_lower_bound_from_prior_profile_evidence() {
        let strategy = Strategy::LiveTradeFlow;
        let now_ms = current_time_ms();
        let mut candidate = signal(strategy.as_str(), 0.8, now_ms);
        let mut decision = serde_json::json!({
            "strategy_parameters": {
                "scout_minimum_walk_forward_samples": 3.0,
                "scout_maximum_evidence_age_seconds": 86400.0
            }
        });
        candidate.strategy_decision = Some(decision.clone());
        assert!(validated_strategy_entry_edge_at(&candidate, strategy, now_ms).is_err());
        decision["strategy_walk_forward"] = serde_json::json!({
            "sampleCount": 3, "effectiveSampleSize": 3.0,
            "positiveNetRate": 0.99, "averageNetReturnPercent": 0.5,
            "expectancyCiLowPercent": -0.01,
            "lastSampleTimestampMs": now_ms - 1_000
        });
        candidate.strategy_decision = Some(decision.clone());
        assert!(validated_strategy_entry_edge_at(&candidate, strategy, now_ms).is_err());
        decision["strategy_walk_forward"]["expectancyCiLowPercent"] = serde_json::json!(0.1);
        candidate.strategy_decision = Some(decision.clone());
        assert_eq!(
            validated_strategy_entry_edge_at(&candidate, strategy, now_ms).unwrap(),
            0.1,
        );
        decision["strategy_walk_forward"]["lastSampleTimestampMs"] = serde_json::json!(now_ms);
        candidate.strategy_decision = Some(decision);
        assert!(validated_strategy_entry_edge_at(&candidate, strategy, now_ms).is_err());
        candidate.timestamp = now_ms + 1;
        assert!(validated_strategy_entry_edge_at(&candidate, strategy, now_ms).is_err());
    }

    #[test]
    fn loss_cluster_is_scoped_to_profile_and_market_context_and_expires() {
        let mut settings = Settings::default();
        let mut candidate = signal("live-trade-flow", 0.8, 100_000);
        candidate.strategy_decision =
            Some(serde_json::json!({"strategy_parameters":{"minimum_flow_imbalance":0.2}}));
        let profile = strategy::profile_id(
            &candidate.symbol,
            "live-trade-flow",
            &signal_strategy_parameters(&candidate),
        );
        let key = format!("{profile}|{}", market_context(&candidate));
        settings.recent_profile_context_returns.insert(
            key.clone(),
            vec![(99_999, -0.1), (99_998, -0.2), (99_997, -0.1)],
        );
        assert!(has_recent_profile_loss_cluster(
            &candidate, &settings, 100_000
        ));
        assert!(!has_recent_profile_loss_cluster(
            &candidate,
            &settings,
            100_000 + LOSS_CLUSTER_WINDOW_MS
        ));
        candidate.direction = "bearish".into();
        assert!(!has_recent_profile_loss_cluster(
            &candidate, &settings, 100_000
        ));
        candidate.direction = "bullish".into();
        candidate.strategy_decision.as_mut().unwrap()["strategy_parameters"]["minimum_flow_imbalance"] =
            serde_json::json!(0.3);
        assert!(!has_recent_profile_loss_cluster(
            &candidate, &settings, 100_000
        ));
        settings
            .recent_profile_context_returns
            .get_mut(&key)
            .unwrap()[1]
            .1 = 0.1;
        candidate.strategy_decision.as_mut().unwrap()["strategy_parameters"]["minimum_flow_imbalance"] =
            serde_json::json!(0.2);
        assert!(!has_recent_profile_loss_cluster(
            &candidate, &settings, 100_000
        ));
    }

    #[test]
    fn named_strategy_stop_uses_its_profile_without_unvalidated_widening() {
        assert_eq!(
            effective_entry_stop(Some(Strategy::LiveTradeFlow), 0.5, Some(0.5)),
            0.5
        );
        assert!(effective_entry_stop(None, 0.5, Some(0.5)) > 0.5);
    }
}
