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
        let stop_strategies = settings.strategy_positions.iter().flat_map(|(symbol, strategies)| {
            strategies.iter().flat_map(move |(strategy, position)| position.protective_stops.iter()
                .map(move |stop| ((symbol.clone(), stop.barrier_id.clone()), strategy.clone())))
        }).collect::<BTreeMap<_, _>>();
        (lots, stop_strategies)
    };
    for (symbol, strategy, position) in lots {
        let strategy_kind = Strategy::from_name(&strategy).unwrap_or(Strategy::LiveInsights);
        if !strategy_kind.uses_protective_stops() {
            for stop in &position.protective_stops {
                if !cancel_protective_barrier(
                    client, base, state, &symbol, &stop.barrier_id,
                    stop.order_timestamp, &strategy,
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
                if !cancel_protective_barrier(client, base, state, &symbol, id, created_at, &strategy).await? {
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
