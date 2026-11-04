fn publish_scout_strategy_status(
    state: &AutoTraderState,
    active_strategy_symbols: Vec<(String, String)>,
    active_parameter_profiles: Vec<(String, String, String, String)>,
    enabled_strategies: Vec<String>,
) {
    let base = std::env::var("INSIGHTS_URL").unwrap_or_else(|_| "http://127.0.0.1:3004".into());
    let client = state.client.clone();
    tokio::spawn(async move {
        if let Err(error) = client
            .post(format!("{base}/internal/signals/scout-strategy-status"))
            .timeout(std::time::Duration::from_secs(2))
            .json(&serde_json::json!({
                "active_strategy_symbols": active_strategy_symbols,
                "active_parameter_profiles": active_parameter_profiles,
                "enabled_strategies": enabled_strategies,
            }))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            crate::service_log!("WARN", "Could not publish daily Scout exit status: {error}");
        }
    });
}

fn valid_scout_parameter_probe(signal: &Signal, strategy: Strategy) -> bool {
    let Some(decision) = signal.strategy_decision.as_ref() else { return false; };
    if decision.get("scout_parameter_probe").and_then(serde_json::Value::as_bool) != Some(true) {
        return true;
    }
    let Some(key) = decision.get("scout_parameter_key").and_then(serde_json::Value::as_str) else { return false; };
    let Some(value) = decision.get("scout_parameter_value").and_then(serde_json::Value::as_f64) else { return false; };
    let expected_profile_symbol = if matches!(strategy, Strategy::RiskAdjustedTrendPortfolio | Strategy::RiskAdjustedMonthlyMomentum) {
        "BTC-USDT"
    } else {
        signal.symbol.as_str()
    };
    if decision.get("scout_parameter_profile_symbol").and_then(serde_json::Value::as_str)
        != Some(expected_profile_symbol)
    {
        return false;
    }
    if key.starts_with("combo:") {
        let Some(combination) = parse_scout_combination(key, strategy) else { return false; };
        return value == 0.0 && combination.iter().all(|(parameter, expected)| {
            decision.get("strategy_parameters").and_then(|parameters| parameters.get(parameter))
                .and_then(serde_json::Value::as_f64).is_some_and(|actual| (actual - expected).abs() <= 1e-7)
        });
    }
    let Some(spec) = all_strategy_parameter_specs(strategy).into_iter().find(|spec| spec.key.as_str() == key) else { return false; };
    if spec.key == StrategyParameterKey::LearningRate || !value.is_finite() || value < spec.min || value > spec.max { return false; }
    let position = (value - spec.min) / spec.step;
    (position - position.round()).abs() <= 1e-6
        && decision.get("strategy_parameters").and_then(|parameters| parameters.get(key))
            .and_then(serde_json::Value::as_f64) == Some(value)
}

fn daily_strategy_signal_key(signal: &Signal) -> Option<(String, String, u64)> {
    let strategy = signal.strategy.as_deref()?;
    let decision = signal.strategy_decision.as_ref()?;
    let candle_or_anchor = if strategy == "risk-adjusted-monthly-momentum" {
        decision.get("monthly_anchor_close_time_ms")
    } else {
        decision.get("bar_close_timestamp")
    }?
    .as_u64()?;
    Some((strategy.to_owned(), signal.symbol.clone(), candle_or_anchor))
}

#[derive(Deserialize)]
struct DailyStrategySignalBatch {
    snapshot: DailyStrategySnapshot,
    diagnostics: Vec<StrategyDiagnostic>,
}

async fn receive_daily_strategy_signals(
    State(state): State<AutoTraderState>,
    Json(batch): Json<DailyStrategySignalBatch>,
) -> Result<StatusCode, (StatusCode, String)> {
    let now_ms = current_time_ms();
    if batch.snapshot.as_of_ms > now_ms + 5_000
        || now_ms.saturating_sub(batch.snapshot.as_of_ms) > 10 * 60_000
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Daily strategy signal batch is stale".into(),
        ));
    }
    let mut snapshot = batch.snapshot;
    if snapshot.generated_signals.iter().any(|signal| {
        !signal.strategy_candidate
            || signal.stale
            || signal.timestamp > now_ms + 5_000
            || now_ms.saturating_sub(signal.timestamp) > 10 * 60_000
            || !signal
                .strategy
                .as_deref()
                .and_then(Strategy::from_name)
                .is_some_and(|strategy| {
                    !strategy.is_live_insight() && !strategy.is_dated_carry()
                        && valid_scout_parameter_probe(signal, strategy)
                })
    }) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Daily strategy batch has an invalid signal".into(),
        ));
    }
    let mut _active_parameter_profiles = BTreeSet::new();
    snapshot.active_strategy_symbols = snapshot.generated_signals.iter().filter_map(|signal| {
        let strategy = signal.strategy.as_deref().and_then(Strategy::from_name)?;
        let parameters = signal_strategy_parameters(signal);
        let input = StrategySignal::entry(signal, &parameters);
        let probe = signal.strategy_decision.as_ref().and_then(|decision| {
            Some((
                decision.get("scout_parameter_key")?.as_str()?,
                decision.get("scout_parameter_value")?.as_f64()?,
            ))
        });
        let entered = if let Some(("rsi_exit_level", value)) = probe {
            signal.strategy_decision.as_ref().is_some_and(|decision| {
                decision.get("primary_metric_value").and_then(serde_json::Value::as_f64)
                    .is_some_and(|rsi| rsi < value)
                    && decision.get("secondary_passed").and_then(serde_json::Value::as_bool) == Some(true)
            })
        } else {
            matches!(strategy.evaluate(&input), Ok(StrategyDecision::Enter))
        };
        if !entered {
            return None;
        }
        if let Some((key, value)) = probe {
            _active_parameter_profiles.insert((
                strategy.as_str().to_owned(), signal.symbol.clone(), key.to_owned(), format!("{value:.8}"),
            ));
            None
        } else {
            Some((strategy.as_str().to_owned(), signal.symbol.clone()))
        }
    }).collect();
    let active_symbols = snapshot.active_strategy_symbols.clone();
    let mut diagnostics = batch.diagnostics;
    snapshot.evaluated_strategy_symbols.clear();
    for diagnostic in &mut diagnostics {
        for item in &mut diagnostic.symbols {
            item.evaluated = true;
            let key = (diagnostic.strategy.clone(), item.symbol.clone());
            snapshot.evaluated_strategy_symbols.insert(key.clone());
            item.signal_active = active_symbols.contains(&key);
        }
    }
    snapshot.diagnostics = diagnostics.clone();
    {
        let mut latest = state.latest_daily_strategy_snapshot.lock().await;
        if let Some(previous) = latest.as_ref().filter(|previous| {
            !snapshot.sweep_id.is_empty() && previous.sweep_id == snapshot.sweep_id
        }) {
            snapshot.active_strategy_symbols.extend(previous.active_strategy_symbols.iter().cloned());
            snapshot.evaluated_strategy_symbols.extend(previous.evaluated_strategy_symbols.iter().cloned());
        }
        *latest = Some(snapshot.clone());
    }
    if let Ok(mut current) = state.strategy_diagnostic.lock() {
        *current = diagnostics.clone();
    }
    if !state.enabled.load(Ordering::Relaxed) {
        publish_scout_strategy_status(&state, Vec::new(), Vec::new(), Vec::new());
        return Ok(StatusCode::ACCEPTED);
    }
    let mut settings = state.settings.lock().await;
    for signal in &mut snapshot.generated_signals {
        inject_learned_parameters(&settings, signal);
    }
    let mut active_profiles = BTreeSet::new();
    snapshot.active_strategy_symbols = snapshot.generated_signals.iter().filter_map(|signal| {
        let strategy = signal.strategy.as_deref().and_then(Strategy::from_name)?;
        let parameters = signal_strategy_parameters(signal);
        let probe = signal.strategy_decision.as_ref().and_then(|decision| Some((
            decision.get("scout_parameter_key")?.as_str()?,
            decision.get("scout_parameter_value")?.as_f64()?,
        )));
        let entered = if let Some(("rsi_exit_level", value)) = probe {
            signal.strategy_decision.as_ref().is_some_and(|decision| {
                decision.get("primary_metric_value").and_then(serde_json::Value::as_f64)
                    .is_some_and(|rsi| rsi < value)
                    && decision.get("secondary_passed").and_then(serde_json::Value::as_bool) == Some(true)
            })
        } else {
            matches!(strategy.evaluate(&StrategySignal::entry(signal, &parameters)), Ok(StrategyDecision::Enter))
        };
        if !entered { return None; }
        if let Some((key, value)) = probe {
            active_profiles.insert((strategy.as_str().to_owned(), signal.symbol.clone(), key.to_owned(), format!("{value:.8}")));
            None
        } else {
            Some((strategy.as_str().to_owned(), signal.symbol.clone()))
        }
    }).collect();
    let active_symbols = snapshot.active_strategy_symbols.clone();
    for diagnostic in &mut diagnostics {
        for item in &mut diagnostic.symbols {
            item.signal_active = active_symbols.contains(&(diagnostic.strategy.clone(), item.symbol.clone()));
        }
    }
    snapshot.diagnostics = diagnostics.clone();
    let enabled_strategies = settings.strategies.iter()
        .filter(|strategy| !strategy.is_dated_carry())
        .map(|strategy| strategy.as_str().to_owned())
        .collect::<Vec<_>>();
    let scout_strategies = settings.scout_strategies.iter()
        .filter(|strategy| !strategy.is_dated_carry())
        .map(|strategy| strategy.as_str().to_owned())
        .collect::<Vec<_>>();
    let active_strategy_symbols = snapshot.active_strategy_symbols.iter()
        .filter(|(strategy, _)| enabled_strategies.iter().any(|enabled| enabled == strategy))
        .cloned()
        .collect::<Vec<_>>();
    let active_parameter_profiles = active_profiles.into_iter()
        .filter(|(strategy, _, _, _)| enabled_strategies.iter().any(|enabled| enabled == strategy)
            || scout_strategies.iter().any(|scout| scout == strategy))
        .collect::<Vec<_>>();
    let mut by_symbol = BTreeMap::<String, Vec<Signal>>::new();
    let mut cursors_changed = false;
    for signal in snapshot.generated_signals.iter().cloned() {
        let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
            continue;
        };
        if !settings.strategies.contains(&strategy) && !scout_strategies.iter().any(|name| name == strategy.as_str()) {
            continue;
        }
        let scout_only = !settings.strategies.contains(&strategy);
        if scout_only {
            let parameters = signal_strategy_parameters(&signal);
            let input = StrategySignal::entry(&signal, &parameters);
            let is_probe = signal.strategy_decision.as_ref().and_then(|decision| decision.get("scout_parameter_probe")).and_then(serde_json::Value::as_bool) == Some(true);
            if !is_probe && !matches!(strategy.evaluate(&input), Ok(StrategyDecision::Scout)) { continue; }
        }
        if let Some((strategy_name, symbol, candle_or_anchor)) = daily_strategy_signal_key(&signal) {
            let cursor_key = format!("{strategy_name}|{symbol}");
            let already_processed = settings.daily_strategy_signal_cursors
                .get(&cursor_key)
                .is_some_and(|previous| *previous >= candle_or_anchor);
            if already_processed {
                // Parameter probes are always shadow-only and may share the
                // same candle cursor as their baseline signal.
                let parameter_probe = signal.strategy_decision.as_ref()
                    .and_then(|decision| decision.get("scout_parameter_probe"))
                    .and_then(serde_json::Value::as_bool) == Some(true);
                if !parameter_probe {
                    // Revisit ordinary Scouts after a restart; per-level cooldowns
                    // prevent duplicate samples.
                    let parameters = signal_strategy_parameters(&signal);
                    let input = StrategySignal::entry(&signal, &parameters);
                    if !matches!(strategy.evaluate(&input), Ok(StrategyDecision::Scout)) {
                        continue;
                    }
                }
            } else {
                settings.daily_strategy_signal_cursors.insert(cursor_key, candle_or_anchor);
                cursors_changed = true;
            }
        }
        by_symbol
            .entry(signal.symbol.clone())
            .or_default()
            .push(signal);
    }
    if cursors_changed {
        request_settings_persist(&state)?;
    }
    drop(settings);
    {
        let mut latest = state.latest_daily_strategy_snapshot.lock().await;
        if let Some(previous) = latest.as_ref().filter(|previous| {
            !snapshot.sweep_id.is_empty() && previous.sweep_id == snapshot.sweep_id
        }) {
            snapshot.active_strategy_symbols.extend(previous.active_strategy_symbols.iter().cloned());
            snapshot.evaluated_strategy_symbols.extend(previous.evaluated_strategy_symbols.iter().cloned());
        }
        *latest = Some(snapshot.clone());
    }
    if let Ok(mut current) = state.strategy_diagnostic.lock() { *current = diagnostics.clone(); }
    let mut sampled_strategies = enabled_strategies;
    sampled_strategies.extend(scout_strategies);
    sampled_strategies.sort();
    sampled_strategies.dedup();
    publish_scout_strategy_status(&state, active_strategy_symbols, active_parameter_profiles, sampled_strategies);
    if !by_symbol.is_empty() {
        let mut queue = state.pending_daily_signals.lock().await;
        for (symbol, best) in by_symbol {
            queue_pending_signal(
                &mut queue,
                PendingSignalKind::Daily,
                symbol,
                Dashboard {
                    best,
                    walk_forward: Arc::new(Vec::new()),
                    missed_opportunity_bands: Vec::new(),
                    markets: Arc::new(BTreeMap::new()),
                    queued_at: None,
                    queue_wait_ms: None,
                },
            );
        }
        drop(queue);
        state.daily_signal_notify.notify_one();
    }
    Ok(StatusCode::ACCEPTED)
}
