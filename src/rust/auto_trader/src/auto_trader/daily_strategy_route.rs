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

use strategies::daily::daily_strategy_signal_key;

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
    if batch.refresh_only {
        let mut latest = state.latest_daily_strategy_snapshot.lock().await;
        if let Some(current) = latest.as_mut() {
            current.as_of_ms = batch.snapshot.as_of_ms;
        } else {
            let mut initial = batch.snapshot;
            initial.diagnostics = batch.diagnostics.clone();
            for diagnostic in &batch.diagnostics {
                for item in &diagnostic.symbols {
                    initial.evaluated_strategy_symbols
                        .insert((diagnostic.strategy.clone(), item.symbol.clone()));
                }
            }
            *latest = Some(initial);
            if let Ok(mut current) = state.strategy_diagnostic.lock() {
                *current = batch.diagnostics;
            }
        }
        return Ok(StatusCode::ACCEPTED);
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
                .is_some_and(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
    }) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Daily strategy batch has an invalid signal".into(),
        ));
    }
    let trading_strategies = state.settings.lock().await.strategies.clone();
    snapshot.active_strategy_symbols = snapshot
        .generated_signals
        .iter()
        .filter_map(|signal| {
            let strategy = signal.strategy.as_deref().and_then(Strategy::from_name)?;
            if !trading_strategies.contains(&strategy)
                || signal.strategy_decision.as_ref().and_then(|value| value.get("scout_parameter_probe"))
                    .and_then(serde_json::Value::as_bool) == Some(true)
            {
                return None;
            }
            let parameters = signal_strategy_parameters(signal);
            let input = StrategySignal::entry(signal, &parameters);
            let entered = matches!(strategy.evaluate(&input), Ok(StrategyDecision::Enter));
            if !entered {
                return None;
            }
            Some((strategy.as_str().to_owned(), signal.symbol.clone()))
        })
        .collect();
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
            snapshot
                .active_strategy_symbols
                .extend(previous.active_strategy_symbols.iter().cloned());
            snapshot
                .evaluated_strategy_symbols
                .extend(previous.evaluated_strategy_symbols.iter().cloned());
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
    snapshot.active_strategy_symbols = snapshot
        .generated_signals
        .iter()
        .filter_map(|signal| {
            let strategy = signal.strategy.as_deref().and_then(Strategy::from_name)?;
            if !settings.strategies.contains(&strategy)
                || signal.strategy_decision.as_ref().and_then(|value| value.get("scout_parameter_probe"))
                    .and_then(serde_json::Value::as_bool) == Some(true)
            {
                return None;
            }
            let parameters = signal_strategy_parameters(signal);
            let entered = matches!(
                strategy.evaluate(&StrategySignal::entry(signal, &parameters)),
                Ok(StrategyDecision::Enter)
            );
            if !entered {
                return None;
            }
            Some((strategy.as_str().to_owned(), signal.symbol.clone()))
        })
        .collect();
    let active_symbols = snapshot.active_strategy_symbols.clone();
    for diagnostic in &mut diagnostics {
        for item in &mut diagnostic.symbols {
            item.signal_active =
                active_symbols.contains(&(diagnostic.strategy.clone(), item.symbol.clone()));
        }
    }
    snapshot.diagnostics = diagnostics.clone();
    let enabled_strategies = settings
        .strategies
        .iter()
        .filter(|strategy| !strategy.is_dated_carry())
        .map(|strategy| strategy.as_str().to_owned())
        .collect::<Vec<_>>();
    let active_strategy_symbols = snapshot
        .active_strategy_symbols
        .iter()
        .filter(|(strategy, _)| enabled_strategies.iter().any(|enabled| enabled == strategy))
        .cloned()
        .collect::<Vec<_>>();
    // Daily parameter probes are evaluated as paired scouts by Insights. Keep
    // their exact tested profiles active so the signal engine continues each
    // pending probe until its outcome window completes.
    let active_parameter_profiles = snapshot.generated_signals.iter().filter_map(|signal| {
        let decision = signal.strategy_decision.as_ref()?;
        (decision.get("scout_parameter_probe")?.as_bool()? ).then_some((
            signal.strategy.as_deref()?.to_owned(),
            signal.symbol.clone(),
            decision.get("scout_parameter_key")?.as_str()?.to_owned(),
            format!("{:.8}", decision.get("scout_parameter_value")?.as_f64()?),
        ))
    }).collect::<Vec<_>>();
    let mut by_symbol = BTreeMap::<String, Vec<Signal>>::new();
    let mut cursors_changed = false;
    for signal in snapshot.generated_signals.iter().cloned() {
        let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
            continue;
        };
        if !settings.strategies.contains(&strategy) {
            continue;
        }
        if signal.strategy_decision.as_ref()
            .and_then(|decision| decision.get("scout_parameter_probe"))
            .and_then(serde_json::Value::as_bool) == Some(true)
        {
            continue;
        }
        if let Some((strategy_name, symbol, candle_or_anchor)) = daily_strategy_signal_key(&signal)
        {
            let cursor_key = format!("{strategy_name}|{symbol}");
            let already_processed = settings
                .daily_strategy_signal_cursors
                .get(&cursor_key)
                .is_some_and(|previous| *previous >= candle_or_anchor);
            if already_processed {
                continue;
            } else {
                settings
                    .daily_strategy_signal_cursors
                    .insert(cursor_key, candle_or_anchor);
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
            snapshot
                .active_strategy_symbols
                .extend(previous.active_strategy_symbols.iter().cloned());
            snapshot
                .evaluated_strategy_symbols
                .extend(previous.evaluated_strategy_symbols.iter().cloned());
        }
        *latest = Some(snapshot.clone());
    }
    if let Ok(mut current) = state.strategy_diagnostic.lock() {
        *current = diagnostics.clone();
    }
    let sampled_strategies = enabled_strategies;
    publish_scout_strategy_status(
        &state,
        active_strategy_symbols,
        active_parameter_profiles,
        sampled_strategies,
    );
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
