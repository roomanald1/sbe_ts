async fn ensure_protective_stops(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let barriers: Vec<serde_json::Value> = client
        .get(format!("{base}/positions/barriers"))
        .send().await.map_err(upstream_error)?
        .error_for_status().map_err(upstream_error)?
        .json().await.map_err(upstream_error)?;
    reconcile_local_stop_order_statuses(state, &barriers).await?;
    let (lots, stop_strategies) = {
        let settings = state.settings.lock().await;
        let lots = if settings.strategy_positions.is_empty() {
            settings.positions.iter().filter(|(_, position)| position.quantity.abs() > 1e-12)
                .map(|(symbol, position)| (symbol.clone(), position.strategy.clone().unwrap_or_else(|| "live-insights".into()), position.clone()))
                .collect::<Vec<_>>()
        } else {
            settings.strategy_positions.iter().flat_map(|(symbol, strategies)| strategies.iter()
                .filter(|(_, position)| position.quantity.abs() > 1e-12)
                .map(|(strategy, position)| (symbol.clone(), strategy.clone(), position.clone())))
                .collect::<Vec<_>>()
        };
        let stop_strategies = if settings.strategy_positions.is_empty() {
            settings.positions.iter().flat_map(|(symbol, position)| {
                position.protective_stops.iter().map(move |stop| (
                    (symbol.clone(), stop.barrier_id.clone()),
                    position.strategy.clone().unwrap_or_else(|| "live-insights".into()),
                ))
            }).collect::<BTreeMap<_, _>>()
        } else {
            settings.strategy_positions.iter().flat_map(|(symbol, strategies)| {
                strategies.iter()
                    .filter(|(_, position)| position.quantity.abs() > 1e-12)
                    .flat_map(move |(strategy, position)| position.protective_stops.iter()
                        .map(move |stop| ((symbol.clone(), stop.barrier_id.clone()), strategy.clone())))
            }).collect::<BTreeMap<_, _>>()
        };
        (lots, stop_strategies)
    };

    // Position Manager survives AutoTrader state resets. Barriers from old
    // strategy lots must be removed before they can act on a later position
    // in the same symbol. A triggering barrier is left alone and blocks new
    // coverage for that symbol until its fill can be reconciled.
    let mut blocked_symbols = BTreeSet::new();
    for barrier in barriers.iter().filter(|barrier| {
        barrier["source"].as_str() == Some("auto")
            && barrier["parent_order_id"].as_str().is_some_and(|id| id.starts_with("auto-stop-"))
            && matches!(barrier["status"].as_str(), Some("armed" | "triggering"))
    }) {
        let symbol = barrier["symbol"].as_str().unwrap_or_default();
        let barrier_id = barrier["id"].as_str().unwrap_or_default();
        if stop_strategies.contains_key(&(symbol.to_owned(), barrier_id.to_owned())) {
            continue;
        }
        if barrier["status"].as_str() == Some("triggering") {
            blocked_symbols.insert(symbol.to_owned());
            continue;
        }
        let created_at = barrier["created_at"].as_u64().unwrap_or_default();
        if !cancel_protective_barrier(client, base, state, symbol, barrier_id, created_at, None).await? {
            blocked_symbols.insert(symbol.to_owned());
            let mut settings = state.settings.lock().await;
            settings.last_risk_note = format!(
                "Orphan auto-stop {barrier_id} for {symbol} could not be canceled; stop coverage is deferred"
            );
        }
    }
    for (symbol, strategy, position) in lots {
        if blocked_symbols.contains(&symbol) {
            continue;
        }
        let strategy_kind = Strategy::from_name(&strategy).unwrap_or(Strategy::LiveInsights);
        if !strategy_kind.uses_protective_stops() {
            for stop in &position.protective_stops {
                if !cancel_protective_barrier(
                    client, base, state, &symbol, &stop.barrier_id,
                    stop.order_timestamp, Some(&strategy),
                ).await? {
                    let mut settings = state.settings.lock().await;
                    settings.last_risk_note = format!(
                        "Could not cancel obsolete stop for {symbol} ({strategy}); retrying"
                    );
                }
            }
            continue;
        }
        let desired_quantity = position.quantity.abs();
        let active = barriers.iter().filter(|barrier| {
            if barrier["symbol"].as_str() != Some(symbol.as_str())
                || barrier["source"].as_str() != Some("auto")
                || !barrier["parent_order_id"].as_str().is_some_and(|id| id.starts_with("auto-stop-"))
                || !matches!(barrier["status"].as_str(), Some("armed" | "triggering")) {
                return false;
            }
            let barrier_id = barrier["id"].as_str().unwrap_or_default();
            stop_strategies.get(&(symbol.clone(), barrier_id.to_owned())).is_some_and(|owner| owner == &strategy)
        }).collect::<Vec<_>>();
        if active.iter().any(|barrier| barrier["status"].as_str() == Some("triggering")) {
            continue;
        }
        let expected_side = if position.quantity > 0.0 { "sell" } else { "buy" };
        let covered_quantity = active.iter()
            .filter(|barrier| barrier["side"].as_str() == Some(expected_side))
            .map(|barrier| barrier["remaining_quantity"].as_f64().unwrap_or(0.0).max(0.0))
            .sum::<f64>();
        let wrong_side = active.iter().any(|barrier| barrier["side"].as_str() != Some(expected_side));
        let tolerance = (desired_quantity * 1e-6).max(1e-12);
        let coverage_mismatch = wrong_side || (covered_quantity - desired_quantity).abs() > tolerance;
        if !wrong_side && desired_quantity > covered_quantity + tolerance && !active.is_empty() {
            let uncovered_quantity = desired_quantity - covered_quantity;
            let uncovered_fees = position.entry_fees * (uncovered_quantity / desired_quantity);
            if let Err((_, error)) = arm_protective_stop(
                client, base, state, &symbol, &strategy,
                if position.quantity > 0.0 { "buy" } else { "sell" },
                uncovered_quantity, position.average_entry_price, uncovered_fees,
                position.stop_loss_percent,
            ).await {
                let mut settings = state.settings.lock().await;
                settings.last_risk_note = format!("Additional protective stop failed for {symbol} ({strategy}): {error}");
            }
            continue;
        }
        if coverage_mismatch && !active.is_empty() {
            let mut cancelled_all = true;
            for barrier in &active {
                let Some(id) = barrier["id"].as_str() else { cancelled_all = false; break; };
                let created_at = barrier["created_at"].as_u64().unwrap_or_default();
                if !cancel_protective_barrier(client, base, state, &symbol, id, created_at, Some(&strategy)).await? {
                    cancelled_all = false;
                    break;
                }
            }
            if !cancelled_all {
                let mut settings = state.settings.lock().await;
                settings.last_risk_note = format!("Could not replace stop protection for {symbol} ({strategy}); no replacement added");
                continue;
            }
        }
        if desired_quantity <= 1e-12 || (!active.is_empty() && !coverage_mismatch) { continue; }
        if let Err((_, error)) = arm_protective_stop(
            client, base, state, &symbol, &strategy,
            if position.quantity > 0.0 { "buy" } else { "sell" },
            desired_quantity, position.average_entry_price, position.entry_fees,
            position.stop_loss_percent,
        ).await {
            let mut settings = state.settings.lock().await;
            settings.last_risk_note = format!("Protective stop failed for {symbol} ({strategy}): {error}; exit monitor remains active");
        }
    }
    Ok(())
}

async fn reconcile_local_stop_order_statuses(
    state: &AutoTraderState,
    barriers: &[serde_json::Value],
) -> Result<(), (StatusCode, String)> {
    let mut settings = state.settings.lock().await;
    let mut changed = false;
    for order in settings.orders.iter_mut().filter(|order| order.purpose == "stop-loss") {
        let barrier = barriers.iter().find(|barrier| {
            barrier["symbol"].as_str() == Some(order.symbol.as_str())
                && barrier["created_at"].as_u64() == Some(order.timestamp)
                && barrier["source"].as_str() == Some("auto")
                && barrier["parent_order_id"].as_str().is_some_and(|id| id.starts_with("auto-stop-"))
        });
        let next_status = if let Some(barrier) = barrier {
            let status = barrier["status"].as_str().unwrap_or("cancelled");
            if status == "partially_filled" { "partially_filled_closed" } else { status }
        } else if matches!(order.status.as_str(), "armed" | "triggering" | "partially_filled") {
            "cancelled"
        } else {
            continue;
        };
        if order.status != next_status {
            order.status = next_status.to_owned();
            changed = true;
        }
        if let Some(barrier) = barrier {
            let filled = barrier["filled_quantity"].as_f64().unwrap_or(order.filled_quantity);
            let average_price = barrier["average_execution_price"].as_f64().or(order.average_price);
            let fee = barrier["fee_paid"].as_f64().unwrap_or(order.fees_paid).abs();
            if (order.filled_quantity - filled).abs() > 1e-12
                || order.average_price != average_price
                || (order.fees_paid - fee).abs() > 1e-12
                || order.triggered_at != barrier["triggered_at"].as_u64()
            {
                order.filled_quantity = filled;
                order.fill_rate = if order.quantity > 0.0 { (filled / order.quantity).clamp(0.0, 1.0) } else { 0.0 };
                order.average_price = average_price;
                order.fees_paid = fee;
                order.triggered_at = barrier["triggered_at"].as_u64();
                changed = true;
            }
        }
    }
    if changed { request_settings_persist(state)?; }
    Ok(())
}
