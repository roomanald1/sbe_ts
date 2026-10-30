async fn submit_cycle_signal(
    state: &AutoTraderState,
    client: &reqwest::Client,
    position_url: &str,
    dashboard: &Dashboard,
    signal: &Signal,
    exploratory: bool,
    scout: bool,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
    signal_selection_ms: f64,
    now: u64,
) -> Result<CycleResult, (StatusCode, String)> {
    let evidence = dashboard.walk_forward.iter().find(|item| {
        item.symbol == signal.symbol && is_entry_evidence_horizon(item.horizon_seconds)
    });
    let shadow_only = scout || exploratory;
    if shadow_only {
        let sampled_at = now.saturating_mul(1_000);
        let sample_key = format!(
            "{}|{}|{}",
            signal.strategy.as_deref().unwrap_or("live-insights"),
            signal.symbol,
            signal.direction
        );
        let mut settings = state.settings.lock().await;
        settings
            .shadow_sample_at_ms
            .retain(|_, timestamp| sampled_at.saturating_sub(*timestamp) < 86_400_000);
        if settings.shadow_sample_at_ms.get(&sample_key).is_some_and(|previous| {
            sampled_at.saturating_sub(*previous) < 300_000
        }) {
            return Ok(skipped_for_signal(
                signal,
                "A shadow learning sample for this strategy, symbol, and direction is already active",
            ));
        }
        let active_shadow_samples = settings
            .shadow_sample_at_ms
            .values()
            .filter(|timestamp| sampled_at.saturating_sub(**timestamp) < 300_000)
            .count();
        if active_shadow_samples >= settings.max_concurrent_shadow_samples {
            return Ok(skipped_for_signal(
                signal,
                "Maximum number of active fee-free shadow samples is already in use",
            ));
        }
        settings.shadow_sample_at_ms.insert(sample_key, sampled_at);
        drop(settings);
        request_settings_persist(state)?;
    }
    let selected_payload = signal_audit_payload(
        signal,
        "decision",
        now.saturating_mul(1000),
        if shadow_only { "shadowed" } else { "selected" },
        Some(if scout {
            "scout retained for fee-free shadow evaluation"
        } else if exploratory {
            "exploratory candidate retained for fee-free shadow evaluation"
        } else {
            "strategy candidate selected"
        }),
        Some(exploratory),
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        evidence,
    );
    let mut selected_payload = selected_payload;
    selected_payload["scout"] = serde_json::json!(scout);
    if shadow_only {
        let context = market_context(signal);
        let learning_context = if scout {
            format!("scout:{context}:P0:depth:shadow")
        } else {
            format!("{context}:P0:depth:shadow")
        };
        selected_payload["shadow_learning_context"] = serde_json::json!(learning_context);
        if scout {
            let settings = state.settings.lock().await;
            selected_payload["shadow_notional_usd"] = serde_json::json!(
                settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION
            );
        }
        if let Some(book) = dashboard.markets.get(&signal.symbol) {
            selected_payload["shadow_entry_bid"] = book["bids"][0]["price"].clone();
            selected_payload["shadow_entry_ask"] = book["asks"][0]["price"].clone();
            selected_payload["shadow_entry_quote_timestamp"] =
                book["updated_at_ms"].clone();
        }
        selected_payload["shadow_taker_fee_rate"] =
            serde_json::json!(configured_taker_fee_rate());
    }
    queue_auto_trader_audit(state, selected_payload);
    if shadow_only {
        return Ok(skipped_for_signal(
            signal,
            "Low-confidence candidate recorded for shadow evaluation; no fee-bearing order submitted",
        ));
    }
    if !signal
        .symbol
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Ok(skipped_for_signal(
            signal,
            "Selected insight has an invalid instrument symbol",
        ));
    }
    let SignalMarketSnapshot {
        market,
        bid,
        ask,
        mid,
        market_cache_lookup_ms,
        market_http_fetch_ms,
    } = match load_signal_market_snapshot(client, position_url, dashboard, now, signal).await {
        Ok(snapshot) => snapshot,
        Err(result) => return Ok(result),
    };
    // Market snapshot retrieval above is read-only and can be slow. The selected
    // order path locks through the in-memory reservation, then releases before HTTP.
    submit_selected_signal(SelectedSignalOrderRequest {
        state,
        client,
        position_url,
        dashboard,
        signal,
        exploratory,
        scout,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        signal_selection_ms,
        now,
        market,
        bid,
        ask,
        mid,
        market_cache_lookup_ms,
        market_http_fetch_ms,
    })
    .await
}

fn daily_candidate_priority(settings: &Settings, signal: &Signal) -> (f64, f64) {
    let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
        return (0.0, 0.0);
    };
    let outcomes = strategy_learning_stats(settings, &signal.symbol, strategy);
    let confidence = confident_win_rate_edge(outcomes.positive, outcomes.samples);
    let mean_realized_pnl = if outcomes.samples > 0 {
        outcomes.realized_pnl / outcomes.samples as f64
    } else {
        0.0
    };
    (confidence, mean_realized_pnl)
}

fn daily_strategy_candidates<'a>(signals: &[&'a Signal], settings: &Settings) -> Vec<&'a Signal> {
    let mut candidates = signals
        .iter()
        .copied()
        .filter(|signal| daily_signal_candidate(signal))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        let left_priority = daily_candidate_priority(settings, left);
        let right_priority = daily_candidate_priority(settings, right);
        right_priority
            .0
            .total_cmp(&left_priority.0)
            .then_with(|| right_priority.1.total_cmp(&left_priority.1))
            .then_with(|| left.strategy.cmp(&right.strategy))
    });
    candidates
}

fn daily_agreement_size_multiplier(signal: &Signal, candidates: &[&Signal]) -> f64 {
    let agreeing_strategies = candidates
        .iter()
        .filter(|candidate| {
            candidate.symbol == signal.symbol && candidate.direction == signal.direction
        })
        .filter_map(|candidate| candidate.strategy.as_deref())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    (1.0 + agreeing_strategies.saturating_sub(1).min(2) as f64 * 0.1).min(1.2)
}

fn daily_strategy_batch_result(
    last_order: Option<CycleResult>,
    evaluated: usize,
    submitted: usize,
    details: &[String],
) -> CycleResult {
    let summary = if details.is_empty() {
        format!("Daily strategy batch: {submitted}/{evaluated} orders queued; shared limits respected")
    } else {
        format!(
            "Daily strategy batch: {submitted}/{evaluated} orders queued; {}",
            details.join("; ")
        )
    };
    if let Some(mut result) = last_order {
        result.reason = Some(summary);
        return result;
    }
    CycleResult {
        status: "skipped",
        reason: Some(summary),
        symbol: None,
        side: None,
        quantity: None,
        order_type: None,
        order_response: None,
    }
}

async fn run_cycle_inner(
    state: &AutoTraderState,
    mut dashboard: Dashboard,
) -> Result<CycleResult, (StatusCode, String)> {
    {
        let mut settings = state.settings.lock().await;
        settings.last_risk_note.clear();
        settings.risk_limited_sizing = false;
    }
    *state.latest_missed_opportunity_bands.lock().await =
        dashboard.missed_opportunity_bands.clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "System clock is before Unix epoch".into(),
            )
        })?
        .as_secs();
    let position_url =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let client = state.client.clone();
    if let Some(result) = enforce_daily_realized_loss_limit(state).await? {
        return Ok(result);
    }
    let (live_strategies, strategies_enabled, strategy_settings) = {
        let settings = state.settings.lock().await;
        let legacy_composite = settings
            .strategies
            .iter()
            .any(|strategy| strategy.is_legacy_composite());
        let selected = settings
            .strategies
            .iter()
            .copied()
            .filter(|strategy| strategy.is_live_insight() && !strategy.is_legacy_composite())
            .collect::<Vec<_>>();
        let live =
            if legacy_composite || (settings.signal_strategies_enabled && selected.is_empty()) {
                settings
                    .strategies
                    .iter()
                    .find(|strategy| strategy.is_legacy_composite())
                    .map(|strategy| strategy.expanded_strategies().to_vec())
                    .unwrap_or_default()
            } else {
                selected
            };
        (
            live,
            settings
                .strategies
                .iter()
                .any(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry()),
            settings.clone(),
        )
    };
    let has_live_signal_strategies = !live_strategies.is_empty() && !dashboard.best.is_empty();
    let has_daily_strategy_signals = dashboard.best.iter().any(|signal| {
        signal.strategy_candidate
            && signal
                .strategy
                .as_deref()
                .and_then(Strategy::from_name)
                .is_some_and(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
    });
    if has_daily_strategy_signals {
        dashboard.best.retain(|signal| {
            signal.strategy_candidate
                && signal
                    .strategy
                    .as_deref()
                    .and_then(Strategy::from_name)
                    .is_some_and(|strategy| {
                        !strategy.is_live_insight()
                            && !strategy.is_dated_carry()
                            && strategy_settings.strategies.contains(&strategy)
                    })
        });
    } else if has_live_signal_strategies {
        dashboard.best = dashboard
            .best
            .iter()
            .flat_map(|source_signal| {
                let strategy_settings = &strategy_settings;
                live_strategies.iter().copied().filter_map(move |strategy| {
                    let parameters = strategy_parameter_values(
                        &strategy_settings,
                        &source_signal.symbol,
                        strategy,
                    );
                    let insight = StrategyInsight::LiveSignal {
                        signal: source_signal,
                        parameters: &parameters,
                    };
                    if !matches!(
                        strategy.evaluate_insight(&insight),
                        Ok(Some(StrategyEvaluation::LiveSignal))
                    ) {
                        return None;
                    }
                    let mut signal = source_signal.clone();
                    signal.strategy = Some(strategy.as_str().to_owned());
                    signal.strategy_decision = Some(serde_json::json!({
                        "strategy": strategy.as_str(),
                        "contract": strategy.contract(),
                        "strategy_parameters": parameters,
                        "observed_insight": signal.components,
                        "direction": signal.direction,
                        "strength": signal.strength,
                        "score": signal.score,
                    }));
                    Some(signal)
                })
            })
            .collect();
    } else if strategies_enabled {
        return Ok(CycleResult {
            status: "skipped",
            reason: Some("Waiting for Insights daily strategy signals".into()),
            symbol: None,
            side: None,
            quantity: None,
            order_type: None,
            order_response: None,
        });
    }
    if dashboard.best.is_empty() {
        return Ok(CycleResult {
            status: "skipped",
            reason: Some("No eligible signal is currently available from enabled live or daily strategies".into()),
            symbol: None,
            side: None,
            quantity: None,
            order_type: None,
            order_response: None,
        });
    }
    let selection_started = Instant::now();
    let (
        learning,
        risk_appetite,
        active_scout_count,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    ) = {
        let settings = state.settings.lock().await;
        (
            {
                let mut learning = settings.learning.clone();
                decay_learning_map(&mut learning, current_time_ms());
                learning
            },
            settings.risk_appetite,
            settings
                .positions
                .values()
                .filter(|position| position.quantity.abs() > 1e-12 && position.scout_entry)
                .count(),
            settings.stop_loss_percent,
            settings.take_profit_percent,
        )
    };
    let PreparedSignals {
        signals,
        inferred_direction_count,
        neutral_direction_floor,
    } = prepare_directional_signal_candidates(
        state,
        &dashboard,
        now,
        risk_appetite,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    );
    let (open_positions, strategy_positions, max_entries_per_auto_position) = {
        let settings = state.settings.lock().await;
        (
            settings
                .positions
                .iter()
                .filter(|(_, position)| position.quantity.abs() > 1e-12)
                .map(|(symbol, position)| (symbol.clone(), position.clone()))
                .collect::<BTreeMap<_, _>>(),
            settings.strategy_positions.clone(),
            settings.max_entries_per_auto_position,
        )
    };
    let FreshSignalCandidates {
        fresh_directional,
        neutral_scout_candidates,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols,
        non_directional_count,
        position_blocked_count,
    } = collect_signal_candidates(
        &signals,
        now,
        risk_appetite,
        &open_positions,
        &strategy_positions,
        max_entries_per_auto_position,
    );
    let signal_selection_ms = selection_started.elapsed().as_secs_f64() * 1000.0;
    if has_daily_strategy_signals {
        let candidates = daily_strategy_candidates(&fresh_directional, &strategy_settings);
        let evaluated = candidates.len();
        if candidates.is_empty() {
            let reason = format!(
                "No daily strategy candidate passed entry checks; {} blocked by existing positions; {} stale",
                position_blocked_count, stale_signal_count,
            );
            return Ok(skipped(&reason));
        }
        let mut submitted = 0;
        let mut last_order = None;
        let mut details = Vec::new();
        for signal in &candidates {
            let strategy_name = signal.strategy.as_deref().unwrap_or("unknown-strategy");
            let mut sized_signal = (**signal).clone();
            let decision = sized_signal
                .strategy_decision
                .get_or_insert_with(|| serde_json::json!({}));
            if let Some(decision) = decision.as_object_mut() {
                decision.insert(
                    "strategy_agreement_size_multiplier".into(),
                    serde_json::json!(daily_agreement_size_multiplier(signal, &candidates)),
                );
            }
            sized_signal.entry_gate = Some(serde_json::json!({
                "selection_mode": "daily_strategy",
                "strategy": strategy_name,
                "strategy_decision": sized_signal.strategy_decision.clone(),
            }));
            let result = match submit_cycle_signal(
                state,
                &client,
                &position_url,
                &dashboard,
                &sized_signal,
                false,
                false,
                counterfactual_stop_loss_percent,
                counterfactual_take_profit_percent,
                signal_selection_ms,
                now,
            )
            .await
            {
                Ok(result) => result,
                Err((status, error)) => {
                    details.push(format!("{strategy_name}: request error {status}: {error}"));
                    continue;
                }
            };
            if result.status == "order_submitting" {
                submitted += 1;
                last_order = Some(result);
            } else {
                let reason = result.reason.as_deref().unwrap_or(result.status);
                details.push(format!("{strategy_name}: {reason}"));
            }
        }
        return Ok(daily_strategy_batch_result(
            last_order, evaluated, submitted, &details,
        ));
    }
    let (signal, exploratory, scout) = match select_cycle_signal(SignalSelectionInput {
        state,
        dashboard: &dashboard,
        signals: &signals,
        fresh_directional: &fresh_directional,
        neutral_scout_candidates: &neutral_scout_candidates,
        now,
        inferred_direction_count,
        neutral_direction_floor,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols: &stale_symbols,
        non_directional_count,
        position_blocked_count,
        learning: &learning,
        risk_appetite,
        active_scout_count,
        open_positions: &open_positions,
        max_entries_per_auto_position,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    }) {
        Ok(selected) => selected,
        Err(result) => return Ok(result),
    };
    let signal_selection_ms = selection_started.elapsed().as_secs_f64() * 1000.0;
    let mut audited_signal = signal.clone();
    audited_signal.entry_gate = Some(candidate_entry_audit_value(
        signal,
        &dashboard,
        &learning,
        risk_appetite,
        active_scout_count,
        if scout { "scout" } else if exploratory { "exploratory" } else { "strict" },
    ));
    submit_cycle_signal(
        state,
        &client,
        &position_url,
        &dashboard,
        &audited_signal,
        exploratory,
        scout,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        signal_selection_ms,
        now,
    )
    .await
}

#[cfg(test)]
mod parallel_daily_cycle_tests {
    use super::*;

    fn signal(strategy: &str) -> Signal {
        Signal {
            symbol: "ETH-USDT".into(),
            direction: "bullish".into(),
            strength: 1.0,
            timestamp: 10,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: true,
            strategy: Some(strategy.into()),
            strategy_decision: None,
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    #[test]
    fn batch_keeps_every_strategy_candidate_for_same_symbol() {
        let signals = vec![
            signal("trend-50-200"),
            signal("donchian-55-btc200"),
            signal("live-momentum"),
        ];
        let candidates =
            daily_strategy_candidates(&signals.iter().collect::<Vec<_>>(), &Settings::default());
        let names = candidates
            .iter()
            .map(|item| item.strategy.as_deref().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["donchian-55-btc200", "trend-50-200"]);
    }
}
