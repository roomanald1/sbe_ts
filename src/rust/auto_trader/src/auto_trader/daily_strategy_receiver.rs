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
    if batch.snapshot.generated_signals.iter().any(|signal| {
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
    *state.latest_daily_strategy_snapshot.lock().await = Some(batch.snapshot.clone());
    if let Ok(mut current) = state.strategy_diagnostic.lock() {
        *current = batch.diagnostics;
    }
    let selected = {
        let settings = state.settings.lock().await;
        if !state.enabled.load(Ordering::Relaxed) {
            return Ok(StatusCode::ACCEPTED);
        }
        settings.strategies.clone()
    };
    let mut by_symbol = BTreeMap::<String, Vec<Signal>>::new();
    for signal in batch.snapshot.generated_signals {
        let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
            continue;
        };
        if selected.contains(&strategy) {
            by_symbol
                .entry(signal.symbol.clone())
                .or_default()
                .push(signal);
        }
    }
    if !by_symbol.is_empty() {
        let mut queue = state.pending_signals.lock().await;
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
                },
            );
        }
        drop(queue);
        state.signal_notify.notify_one();
    }
    Ok(StatusCode::ACCEPTED)
}
