async fn maybe_exit_auto_position(
    state: &AutoTraderState,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    // Reconciliation performs Position Management RSocket calls and may persist
    // settings. Keep it outside the shared cycle lock so this periodic
    // maintenance work does not inflate cycle-lock hold time. Each in-memory
    // update is serialized by the settings mutex, and the exit path below
    // still takes the cycle lock before mutating orders or positions.
    sync_protective_stops(state)
        .await
        .map_err(|(status, error)| (status, format!("Protective-stop sync: {error}")))?;
    reconcile_auto_positions_with_ledger(state)
        .await
        .map_err(|(status, error)| (status, format!("Position-ledger reconciliation: {error}")))?;
    ensure_protective_stops(state)
        .await
        .map_err(|(status, error)| (status, format!("Protective-stop maintenance: {error}")))?;
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
            let inputs = crate::service_rpc::execution_get(&format!(
                "/internal/market-mark?symbol={symbol}"
            ))
            .await
            .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()));
            (symbol, inputs)
        });
    }
    let mut inputs_by_symbol = BTreeMap::new();
    while let Some(result) = input_tasks.join_next().await {
        let (symbol, inputs) = result.map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Market input task failed: {error}"),
            )
        })?;
        match inputs {
            Ok(inputs) => {
                inputs_by_symbol.insert(symbol, inputs);
            }
            Err((status, error)) => crate::service_log!(
                "WARN",
                "Skipping {symbol} exit evaluation because its market mark is unavailable ({status}): {error}"
            ),
        }
    }
    // Evaluate positions without the shared cycle lock. Exit submission takes
    // that lock only after an exit is actually required, so routine path sampling
    // and settings persistence do not block incoming signal cycles.
    for (symbol, strategy, position) in positions {
        let evaluation_label = format!("{symbol} ({strategy})");
        if let Some(result) =
            evaluate_position_exit(state, symbol, strategy, position, &mut inputs_by_symbol)
                .await
                .map_err(|(status, error)| (status, format!("{evaluation_label}: {error}")))?
        {
            return Ok(Some(result));
        }
    }
    Ok(None)
}
