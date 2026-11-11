async fn maybe_exit_auto_position(
    state: &AutoTraderState,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    // Reconciliation performs Position Management RSocket calls and may persist
    // settings. Keep it outside the shared cycle lock so this periodic
    // maintenance work does not inflate cycle-lock hold time. Each in-memory
    // update is serialized by the settings mutex, and the exit path below
    // still takes the cycle lock before mutating orders or positions.
    sync_protective_stops(state).await?;
    reconcile_auto_positions_with_ledger(state).await?;
    ensure_protective_stops(state).await?;
    let positions = {
        let settings = state.settings.lock().await;
        settings
            .strategy_positions
            .iter()
            .flat_map(|(symbol, lots)| {
                lots.iter()
                    .filter(|(_, position)| position.quantity.abs() > 1e-12)
                    .map(|(strategy, position)| {
                        (symbol.clone(), strategy.clone(), position.clone())
                    })
            })
            .collect::<Vec<_>>()
    };
    // These reads do not depend on each other. Fetch them concurrently, then
    // retain the existing deterministic position processing and serialized
    // stop/order handling below.
    let mut input_tasks = JoinSet::new();
    for (symbol, _, _) in &positions {
        let symbol = symbol.clone();
        input_tasks.spawn(async move {
            let inputs = crate::service_rpc::position_get(&format!(
                "/internal/insights-inputs?symbol={symbol}"
            )).await.map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
            Ok::<_, (StatusCode, String)>((symbol, inputs))
        });
    }
    let mut inputs_by_symbol = BTreeMap::new();
    while let Some(result) = input_tasks.join_next().await {
        let (symbol, inputs) = result.map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Market input task failed: {error}"),
            )
        })??;
        inputs_by_symbol.insert(symbol, inputs);
    }
    let regime_now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let research_regime = state
        .latest_daily_strategy_snapshot
        .lock()
        .await
        .clone()
        .filter(|snapshot| regime_now_ms.saturating_sub(snapshot.as_of_ms) <= 10 * 60_000);
    if research_regime.is_none() {
        crate::service_log!(
            "WARN",
            "Insights daily strategy state unavailable or stale for trend exits"
        );
    }
    // Evaluate positions without the shared cycle lock. Exit submission takes
    // that lock only after an exit is actually required, so routine path sampling
    // and settings persistence do not block incoming signal cycles.
    for (symbol, strategy, position) in positions {
        if let Some(result) = evaluate_position_exit(
            state,
            symbol,
            strategy,
            position,
            &mut inputs_by_symbol,
            research_regime.as_ref(),
        )
        .await?
        {
            return Ok(Some(result));
        }
    }
    Ok(None)
}
