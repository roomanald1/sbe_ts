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
                .is_some_and(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
    }) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Daily strategy batch has an invalid signal".into(),
        ));
    }
    let parameters = BTreeMap::new();
    snapshot.active_strategy_symbols = snapshot.generated_signals.iter().filter_map(|signal| {
        let strategy = signal.strategy.as_deref().and_then(Strategy::from_name)?;
        let input = StrategySignal::entry(signal, &parameters);
        matches!(strategy.evaluate(&input), Ok(StrategyDecision::Enter))
            .then(|| (strategy.as_str().to_owned(), signal.symbol.clone()))
    }).collect();
    let active_symbols = snapshot.active_strategy_symbols.clone();
    *state.latest_daily_strategy_snapshot.lock().await = Some(snapshot.clone());
    if let Ok(mut current) = state.strategy_diagnostic.lock() {
        let mut diagnostics = batch.diagnostics;
        for diagnostic in &mut diagnostics {
            for item in &mut diagnostic.symbols {
                item.signal_active = active_symbols.contains(&(diagnostic.strategy.clone(), item.symbol.clone()));
            }
        }
        *current = diagnostics;
    }
    if !state.enabled.load(Ordering::Relaxed) {
        return Ok(StatusCode::ACCEPTED);
    }
    let mut settings = state.settings.lock().await;
    let mut by_symbol = BTreeMap::<String, Vec<Signal>>::new();
    let mut cursors_changed = false;
    for signal in snapshot.generated_signals {
        let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
            continue;
        };
        if !settings.strategies.contains(&strategy) {
            continue;
        }
        if let Some((strategy_name, symbol, candle_or_anchor)) = daily_strategy_signal_key(&signal) {
            let cursor_key = format!("{strategy_name}|{symbol}");
            if settings.daily_strategy_signal_cursors.get(&cursor_key).is_some_and(|previous| *previous >= candle_or_anchor)
            {
                continue;
            }
            settings.daily_strategy_signal_cursors.insert(cursor_key, candle_or_anchor);
            cursors_changed = true;
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
