async fn run_cycle_inner(
    state: &AutoTraderState,
    mut dashboard: Dashboard,
) -> Result<CycleResult, (StatusCode, String)> {
    let cycle_started = Instant::now();
    let _cycle = acquire_cycle_lock(state, "signal").await;
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
    if let Some(result) = enforce_daily_realized_loss_limit(state, &client, &position_url).await? {
        return Ok(result);
    }
    let dashboard_ms = 0.0;
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
    let has_daily_strategy_signals = dashboard.best.iter().any(|signal| signal.strategy_candidate
        && signal.strategy.as_deref().and_then(Strategy::from_name)
            .is_some_and(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry()));
    if has_daily_strategy_signals {
        dashboard.best.retain(|signal| signal.strategy_candidate
            && signal.strategy.as_deref().and_then(Strategy::from_name)
                .is_some_and(|strategy| !strategy.is_live_insight()
                    && !strategy.is_dated_carry()
                    && strategy_settings.strategies.contains(&strategy)));
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
            reason: Some("No signal or daily strategies are enabled".into()),
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
    let (open_positions, max_entries_per_auto_position) = {
        let settings = state.settings.lock().await;
        (
            settings
                .positions
                .iter()
                .filter(|(_, position)| position.quantity.abs() > 1e-12)
                .map(|(symbol, position)| (symbol.clone(), position.clone()))
                .collect::<BTreeMap<_, _>>(),
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
        max_entries_per_auto_position,
    );
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
    let evidence = dashboard.walk_forward.iter().find(|item| {
        item.symbol == signal.symbol && is_entry_evidence_horizon(item.horizon_seconds)
    });
    let selected_payload = signal_audit_payload(
        signal,
        "decision",
        now.saturating_mul(1000),
        "selected",
        Some(if scout {
            "scout candidate selected; capped paper exploration"
        } else if exploratory {
            "exploratory candidate selected"
        } else {
            "strict walk-forward candidate selected"
        }),
        Some(exploratory),
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        evidence,
    );
    let mut selected_payload = selected_payload;
    selected_payload["scout"] = serde_json::json!(scout);
    queue_auto_trader_audit(state, selected_payload);
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
        market_inputs_ms,
    } = match load_signal_market_snapshot(&client, &position_url, &dashboard, now, signal).await {
        Ok(snapshot) => snapshot,
        Err(result) => return Ok(result),
    };
    submit_selected_signal(SelectedSignalOrderRequest {
        state,
        client: &client,
        position_url: &position_url,
        dashboard: &dashboard,
        signal,
        exploratory,
        scout,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        cycle_started,
        dashboard_ms,
        signal_selection_ms,
        now,
        market,
        bid,
        ask,
        mid,
        market_inputs_ms,
    })
    .await
}
