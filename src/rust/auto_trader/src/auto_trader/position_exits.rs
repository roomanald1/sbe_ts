async fn maybe_exit_auto_position(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    // Reconciliation performs position-manager HTTP calls and may persist
    // settings. Keep it outside the shared cycle lock so this periodic
    // maintenance work does not inflate cycle-lock hold time. Each in-memory
    // update is serialized by the settings mutex, and the exit path below
    // still takes the cycle lock before mutating orders or positions.
    sync_protective_stops(client, base, state).await?;
    ensure_protective_stops(client, base, state).await?;
    let positions = {
        let settings = state.settings.lock().await;
        settings
            .positions
            .iter()
            .filter(|(_, position)| position.quantity.abs() > 1e-12)
            .map(|(symbol, position)| (symbol.clone(), position.clone()))
            .collect::<Vec<_>>()
    };
    // These reads do not depend on each other. Fetch them concurrently, then
    // retain the existing deterministic position processing and serialized
    // stop/order handling below.
    let mut input_tasks = JoinSet::new();
    for (symbol, _) in &positions {
        let client = client.clone();
        let base = base.to_owned();
        let symbol = symbol.clone();
        input_tasks.spawn(async move {
            let inputs: serde_json::Value = client
                .get(format!("{base}/internal/insights-inputs?symbol={symbol}"))
                .send()
                .await
                .map_err(upstream_error)?
                .error_for_status()
                .map_err(upstream_error)?
                .json()
                .await
                .map_err(upstream_error)?;
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
    // Market input requests do not mutate trader state. Keep them outside the
    // shared cycle lock, then serialize exit decisions and order mutations.
    let _cycle = acquire_cycle_lock(state, "position_maintenance").await;
    for (symbol, position) in positions {
        if let Some(result) =
            evaluate_position_exit(client, base, state, symbol, position, &mut inputs_by_symbol)
                .await?
        {
            return Ok(Some(result));
        }
    }
    Ok(None)
}

async fn evaluate_position_exit(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    symbol: String,
    position: TraderPosition,
    inputs_by_symbol: &mut BTreeMap<String, serde_json::Value>,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let position_is_current = {
        let settings = state.settings.lock().await;
        settings.positions.get(&symbol).is_some_and(|current| {
            (current.quantity - position.quantity).abs() <= 1e-12
                && (current.average_entry_price - position.average_entry_price).abs() <= 1e-12
        })
    };
    if !position_is_current {
        return Ok(None);
    }
    let Some(inputs) = inputs_by_symbol.remove(&symbol) else {
        return Ok(None);
    };
    let market = &inputs["markets"][&symbol];
    let bid = market["bids"][0]["price"].as_f64();
    let ask = market["asks"][0]["price"].as_f64();
    let Some((bid, ask)) = bid
        .zip(ask)
        .filter(|(bid, ask)| bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid)
    else {
        return Ok(None);
    };
    let is_long = position.quantity > 0.0;
    let exit_price = if is_long { bid } else { ask };
    let entry_notional = position.quantity.abs() * position.average_entry_price;
    let net_open_pnl = (exit_price - position.average_entry_price) * position.quantity
        - position.entry_fees
        - position.quantity.abs() * exit_price * configured_taker_fee_rate();
    let return_fraction = if entry_notional > 0.0 {
        net_open_pnl / entry_notional
    } else {
        0.0
    };
    let (stop_loss_percent, take_profit_percent) = {
        let settings = state.settings.lock().await;
        (settings.stop_loss_percent, settings.take_profit_percent)
    };
    let take_profit_percent = position
        .horizon_take_profit_percent
        .map(|target| target.max(take_profit_percent))
        .unwrap_or(take_profit_percent);
    let reason = if return_fraction <= -stop_loss_percent / 100.0 {
        Some("stop-loss")
    } else if return_fraction >= take_profit_percent / 100.0 {
        Some("take-profit")
    } else {
        None
    };
    let Some(reason) = reason else {
        return Ok(None);
    };
    if reason == "take-profit" {
        let probability = profitable_close_probability_5m(market, &position);
        if probability.is_some_and(|value| value >= TAKE_PROFIT_HOLD_PROBABILITY_FLOOR) {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "System clock is before Unix epoch".into(),
                    )
                })?
                .as_millis() as u64;
            let mut settings = state.settings.lock().await;
            if let Some(current) = settings.positions.get_mut(&symbol) {
                let was_deferred = current.take_profit_deferred_at_ms.is_some();
                let deferred_at = *current.take_profit_deferred_at_ms.get_or_insert(now_ms);
                if now_ms.saturating_sub(deferred_at) < MAX_TAKE_PROFIT_HOLD_MS {
                    if !was_deferred {
                        persist_settings(&state.settings_path, &settings)?;
                        crate::service_log!(
                            "INFO",
                            "Deferring take-profit for {symbol}: five-minute profitable-close probability is {:.1}%",
                            probability.unwrap_or_default() * 100.0
                        );
                    }
                    return Ok(None);
                }
            }
        }
    }
    submit_position_exit_order(
        client,
        base,
        state,
        symbol,
        position,
        bid,
        ask,
        reason,
        return_fraction,
    )
    .await
}

async fn submit_position_exit_order(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    symbol: String,
    position: TraderPosition,
    bid: f64,
    ask: f64,
    reason: &str,
    return_fraction: f64,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let is_long = position.quantity > 0.0;
    let entry_notional = position.quantity.abs() * position.average_entry_price;
    let entry_decision_id = position.entry_decision_id.clone();
    let protected_quantity = position
        .protective_stops
        .iter()
        .map(|stop| (stop.quantity - stop.processed_quantity).max(0.0))
        .sum::<f64>();
    if reason == "stop-loss" && protected_quantity + 1e-12 >= position.quantity.abs() {
        // The server-side barrier is already watching this price; don't race it with a second IOC.
        return Ok(None);
    }
    if !cancel_protective_stops(client, base, state, &symbol).await? {
        // A stop is already triggering or could not be cancelled. Let its server-side execution settle.
        return Ok(None);
    }
    let side = if is_long { "sell" } else { "buy" };
    let quantity = position.quantity.abs();
    let notional = quantity * (bid + ask) / 2.0;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    {
        let mut settings = state.settings.lock().await;
        reset_daily_budget(&mut settings);
        settings.used_notional_today = allocated_notional(&settings);
        settings.orders.push(TraderOrder {
            timestamp,
            triggered_at: None,
            symbol: symbol.clone(),
            side: side.into(),
            order_type: "Ioc".into(),
            purpose: reason.into(),
            decision_id: None,
            quantity,
            filled_quantity: 0.0,
            fill_rate: 0.0,
            average_price: None,
            fees_paid: 0.0,
            reserved_notional: notional,
            execution_order_id: None,
            processed_fill_trade_ids: Vec::new(),
            learning_context: None,
            book_bid_depth_10bps: None,
            book_ask_depth_10bps: None,
            book_depth_imbalance_10bps: None,
            estimated_sweep_bps: None,
            estimated_ioc_all_in_bps: None,
            decision_bid: None,
            decision_ask: None,
            decision_mid: None,
            submitted_limit_price: None,
            target_price: None,
            horizon_take_profit_percent: None,
            limit_to_mid_bps: None,
            limit_to_crossing_quote_bps: None,
            expires_at: None,
            failure_reason: None,
            stage_timings_ms: BTreeMap::new(),
            status: "submitting".into(),
            realized_pnl: 0.0,
        });
    }
    let submit_started = Instant::now();
    let response = match submit_configured_order(
        client,
        base,
        &symbol,
        side,
        quantity,
        notional,
        bid,
        ask,
        AutoOrderType::Ioc,
        ExecutionParameters::default(),
        timestamp.to_string(),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            let mut settings = state.settings.lock().await;
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == timestamp)
            {
                order.status = "failed".into();
            }
            persist_settings(&state.settings_path, &settings)?;
            return Err(error);
        }
    };
    let submit_response_ms = submit_started.elapsed().as_secs_f64() * 1000.0;
    let mut settings = state.settings.lock().await;
    let filled = response["filled_quantity"].as_f64().unwrap_or(0.0);
    let average_price = response["average_execution_price"].as_f64();
    let fee = response["estimated_fee"].as_f64().unwrap_or(0.0).abs();
    let (realized_pnl, learned_key) = if filled > 0.0 {
        average_price
            .map(|price| {
                apply_auto_fill(
                    &mut settings.positions,
                    &symbol,
                    side,
                    filled,
                    price,
                    fee,
                    "Ioc",
                    reason,
                    None,
                )
            })
            .unwrap_or_default()
    } else {
        (0.0, None)
    };
    let realized_pnl_usd = realized_usd(&settings, &symbol, realized_pnl);
    settings.realized_pnl += realized_pnl_usd;
    settings.daily_realized_pnl += realized_pnl_usd;
    if filled + 1e-12 >= quantity {
        let closed_quantity = filled.min(quantity);
        let entry_notional = closed_quantity * position.average_entry_price;
        if entry_notional > 0.0 {
            let fees_usd = realized_usd(&settings, &symbol, fee);
            record_closed_trade_outcome(
                &mut settings,
                realized_pnl / entry_notional * 100.0,
                realized_pnl_usd,
                fees_usd,
            );
            adapt_exit_policy(&mut settings);
        }
    }
    if let Some(key) = learned_key {
        let stats = settings.learning.entry(key).or_default();
        record_learning_outcome(stats, realized_pnl_usd);
    }
    if let Some(order) = settings
        .orders
        .iter_mut()
        .find(|order| order.timestamp == timestamp)
    {
        order.filled_quantity = filled;
        order.average_price = average_price;
        order.realized_pnl = realized_pnl;
        order.status = if filled > 0.0 { "filled" } else { "accepted" }.into();
    }
    persist_settings(&state.settings_path, &settings)?;
    drop(settings);
    if let Some(decision_id) = entry_decision_id {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let payload = serde_json::json!({
            "decision_id": decision_id, "stage": format!("exit:{timestamp}"), "timestamp": now, "symbol": symbol,
            "status": if filled > 0.0 { "filled" } else { "accepted" }, "reason": reason,
            "exit_order_type": "Ioc", "exit_order_quantity": quantity, "exit_filled_quantity": filled,
            "exit_average_price": average_price, "exit_fees": fee, "realized_pnl": realized_pnl,
            "net_return_percent": if entry_notional > 0.0 { Some(realized_pnl / entry_notional * 100.0) } else { None },
            "side": side,
            "decision_mid_price": (bid + ask) / 2.0,
            "submitted_at": timestamp,
            "response_received_at": now,
            "submit_response_ms": submit_response_ms,
            "fill_rate": if quantity > 0.0 { (filled / quantity).clamp(0.0, 1.0) } else { 0.0 },
            "implementation_shortfall_bps": average_price.map(|price| if side == "buy" { (price / ((bid + ask) / 2.0) - 1.0) * 10_000.0 } else { (1.0 - price / ((bid + ask) / 2.0)) * 10_000.0 }),
            "fee_bps": if filled > 0.0 { Some(fee / (filled * ((bid + ask) / 2.0)) * 10_000.0) } else { None },
            "exit_bid": bid, "exit_ask": ask
        });
        store_auto_trader_audit(client, &payload).await;
    }
    Ok(Some(CycleResult {
        status: "order_submitted",
        reason: Some(format!(
            "Auto-trader {reason} at {:.4}%",
            return_fraction * 100.0
        )),
        symbol: Some(symbol),
        side: Some(side.into()),
        quantity: Some(quantity),
        order_type: Some("Ioc exit".into()),
        order_response: Some(response),
    }))
}

async fn arm_protective_stop(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    symbol: &str,
    position_side: &str,
    quantity: f64,
    entry_price: f64,
    entry_fees: f64,
) -> Result<(), (StatusCode, String)> {
    if quantity <= 0.0 || entry_price <= 0.0 {
        return Ok(());
    }
    let (stop_loss_percent, is_enabled) = {
        let settings = state.settings.lock().await;
        (
            settings.stop_loss_percent,
            state.enabled.load(Ordering::Relaxed),
        )
    };
    if !is_enabled {
        return Ok(());
    }
    let is_long = position_side == "buy";
    let side = if is_long { "sell" } else { "buy" };
    let fee_rate = configured_taker_fee_rate();
    let loss_fraction = stop_loss_percent / 100.0;
    let trigger_price = if is_long {
        (quantity * entry_price * (1.0 - loss_fraction) + entry_fees)
            / (quantity * (1.0 - fee_rate))
    } else {
        (quantity * entry_price * (1.0 + loss_fraction) - entry_fees)
            / (quantity * (1.0 + fee_rate))
    };
    let trigger_direction = if is_long { "below" } else { "above" };
    let response = client
        .post(format!("{base}/positions/orders"))
        .json(&serde_json::json!({
            "symbol": symbol, "side": side, "order_type": "stop-market", "quantity": quantity,
            "trigger_price": trigger_price, "trigger_direction": trigger_direction,
            "source": "auto", "client_order_id": format!("auto-stop-{symbol}-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis())
        }))
        .send()
        .await
        .map_err(upstream_error)?;
    let status = response.status();
    let payload = response
        .json::<serde_json::Value>()
        .await
        .map_err(upstream_error)?;
    if !status.is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("Protective stop rejected ({status}): {payload}"),
        ));
    }
    let barrier = &payload["barrier_order"];
    let id = barrier["id"]
        .as_str()
        .ok_or((
            StatusCode::BAD_GATEWAY,
            "Protective stop response did not include a barrier id".into(),
        ))?
        .to_owned();
    let timestamp = barrier["created_at"].as_u64().unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    });
    let mut settings = state.settings.lock().await;
    if let Some(position) = settings.positions.get_mut(symbol).filter(|position| {
        position.quantity.abs() > 1e-12
            && position.quantity.signum() == if is_long { 1.0 } else { -1.0 }
    }) {
        position.protective_stops.push(ProtectiveStop {
            barrier_id: id,
            order_timestamp: timestamp,
            quantity,
            processed_quantity: 0.0,
            processed_notional: 0.0,
            processed_fee: 0.0,
        });
        settings.orders.push(TraderOrder {
            timestamp,
            triggered_at: None,
            symbol: symbol.to_owned(),
            side: side.to_owned(),
            order_type: "StopMarket".into(),
            purpose: "stop-loss".into(),
            decision_id: None,
            quantity,
            filled_quantity: 0.0,
            fill_rate: 0.0,
            average_price: None,
            fees_paid: 0.0,
            reserved_notional: 0.0,
            execution_order_id: None,
            processed_fill_trade_ids: Vec::new(),
            learning_context: None,
            book_bid_depth_10bps: None,
            book_ask_depth_10bps: None,
            book_depth_imbalance_10bps: None,
            estimated_sweep_bps: None,
            estimated_ioc_all_in_bps: None,
            decision_bid: None,
            decision_ask: None,
            decision_mid: None,
            submitted_limit_price: None,
            target_price: Some(trigger_price),
            horizon_take_profit_percent: None,
            limit_to_mid_bps: None,
            limit_to_crossing_quote_bps: None,
            expires_at: None,
            failure_reason: None,
            stage_timings_ms: BTreeMap::new(),
            status: "armed".into(),
            realized_pnl: 0.0,
        });
        if settings.orders.len() > MAX_RETAINED_AUTO_TRADER_ORDERS {
            let excess = settings.orders.len() - MAX_RETAINED_AUTO_TRADER_ORDERS;
            settings.orders.drain(..excess);
        }
        persist_settings(&state.settings_path, &settings)?;
    } else {
        drop(settings);
        let _ = client
            .delete(format!("{base}/positions/orders/{id}"))
            .send()
            .await;
    }
    Ok(())
}

async fn sync_protective_stops(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let tracked = {
        let settings = state.settings.lock().await;
        settings
            .positions
            .iter()
            .flat_map(|(symbol, position)| {
                position
                    .protective_stops
                    .iter()
                    .cloned()
                    .map(move |stop| (symbol.clone(), stop))
            })
            .collect::<Vec<_>>()
    };
    if tracked.is_empty() {
        return Ok(());
    }
    let barriers: Vec<serde_json::Value> = client
        .get(format!("{base}/positions/barriers"))
        .send()
        .await
        .map_err(upstream_error)?
        .error_for_status()
        .map_err(upstream_error)?
        .json()
        .await
        .map_err(upstream_error)?;
    for (symbol, stop) in tracked {
        let barrier = barriers
            .iter()
            .find(|barrier| barrier["id"].as_str() == Some(&stop.barrier_id));
        let Some(barrier) = barrier else {
            let mut settings = state.settings.lock().await;
            if let Some(position) = settings.positions.get_mut(&symbol) {
                position
                    .protective_stops
                    .retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == stop.order_timestamp)
            {
                order.status = "cancelled".into();
            }
            persist_settings(&state.settings_path, &settings)?;
            continue;
        };
        let total_filled = barrier["filled_quantity"].as_f64().unwrap_or(0.0).max(0.0);
        let total_notional =
            barrier["average_execution_price"].as_f64().unwrap_or(0.0) * total_filled;
        let total_fee = barrier["fee_paid"].as_f64().unwrap_or(0.0).abs();
        let delta_quantity = (total_filled - stop.processed_quantity).max(0.0);
        let delta_notional = (total_notional - stop.processed_notional).max(0.0);
        let delta_fee = (total_fee - stop.processed_fee).max(0.0);
        let status = barrier["status"].as_str().unwrap_or("unknown");
        if delta_quantity > 1e-12 && delta_notional > 0.0 {
            let side = if barrier["side"].as_str() == Some("buy") {
                "buy"
            } else {
                "sell"
            };
            let fill_price = delta_notional / delta_quantity;
            let mut settings = state.settings.lock().await;
            let entry_decision_id = settings
                .positions
                .get(&symbol)
                .and_then(|position| position.entry_decision_id.clone());
            let entry_price_before = settings
                .positions
                .get(&symbol)
                .map(|position| position.average_entry_price)
                .filter(|price| *price > 0.0)
                .unwrap_or(fill_price);
            let (realized_delta, learned_key) = apply_auto_fill(
                &mut settings.positions,
                &symbol,
                side,
                delta_quantity,
                fill_price,
                delta_fee,
                "StopMarket",
                "stop-loss",
                None,
            );
            let realized_delta_usd = realized_usd(&settings, &symbol, realized_delta);
            settings.realized_pnl += realized_delta_usd;
            settings.daily_realized_pnl += realized_delta_usd;
            settings.used_notional_today = allocated_notional(&settings);
            let entry_notional = delta_quantity * entry_price_before;
            let fully_closed = settings
                .positions
                .get(&symbol)
                .is_none_or(|position| position.quantity.abs() <= 1e-12);
            if fully_closed && entry_notional > 0.0 {
                let fees_usd = realized_usd(&settings, &symbol, delta_fee);
                record_closed_trade_outcome(
                    &mut settings,
                    realized_delta / entry_notional * 100.0,
                    realized_delta_usd,
                    fees_usd,
                );
                adapt_exit_policy(&mut settings);
            }
            if let Some(key) = learned_key {
                let stats = settings.learning.entry(key).or_default();
                record_learning_outcome(stats, realized_delta_usd);
            }
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == stop.order_timestamp)
            {
                order.triggered_at = barrier["triggered_at"].as_u64();
                order.filled_quantity = total_filled;
                order.average_price = barrier["average_execution_price"].as_f64();
                order.realized_pnl += realized_delta;
                order.status = status.to_owned();
            }
            if let Some(position) = settings.positions.get_mut(&symbol) {
                if let Some(tracked) = position
                    .protective_stops
                    .iter_mut()
                    .find(|tracked| tracked.barrier_id == stop.barrier_id)
                {
                    tracked.processed_quantity = total_filled;
                    tracked.processed_notional = total_notional;
                    tracked.processed_fee = total_fee;
                }
            }
            persist_settings(&state.settings_path, &settings)?;
            let cumulative_pnl = settings
                .orders
                .iter()
                .find(|order| order.timestamp == stop.order_timestamp)
                .map(|order| order.realized_pnl)
                .unwrap_or(realized_delta);
            drop(settings);
            if let Some(decision_id) = entry_decision_id {
                let update_timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let payload = serde_json::json!({
                    "decision_id": decision_id, "stage": format!("exit:stop:{}", stop.order_timestamp), "timestamp": update_timestamp,
                    "observed_at": update_timestamp, "symbol": symbol, "side": barrier["side"],
                    "status": status, "reason": "stop-loss", "exit_order_type": "StopMarket",
                    "submitted_at": barrier["triggered_at"],
                    "exit_order_quantity": stop.quantity, "exit_filled_quantity": total_filled,
                    "exit_average_price": barrier["average_execution_price"],
                    "exit_fees": total_fee, "realized_pnl": cumulative_pnl,
                    "net_return_percent": if entry_price_before > 0.0 { Some(cumulative_pnl / (total_filled * entry_price_before).max(1e-12) * 100.0) } else { None }
                });
                store_auto_trader_audit(client, &payload).await;
            }
        }
        if !matches!(status, "armed" | "triggering") {
            let mut settings = state.settings.lock().await;
            if let Some(position) = settings.positions.get_mut(&symbol) {
                position
                    .protective_stops
                    .retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == stop.order_timestamp)
            {
                order.status = status.to_owned();
            }
            persist_settings(&state.settings_path, &settings)?;
        }
    }
    Ok(())
}

async fn ensure_protective_stops(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let positions = {
        let settings = state.settings.lock().await;
        settings
            .positions
            .iter()
            .filter(|(_, position)| position.quantity.abs() > 1e-12)
            .map(|(symbol, position)| (symbol.clone(), position.clone()))
            .collect::<Vec<_>>()
    };
    for (symbol, position) in positions {
        let covered = position
            .protective_stops
            .iter()
            .map(|stop| (stop.quantity - stop.processed_quantity).max(0.0))
            .sum::<f64>();
        let uncovered = (position.quantity.abs() - covered).max(0.0);
        if uncovered > 1e-12 {
            if let Err((_, error)) = arm_protective_stop(
                client,
                base,
                state,
                &symbol,
                if position.quantity > 0.0 {
                    "buy"
                } else {
                    "sell"
                },
                uncovered,
                position.average_entry_price,
                position.entry_fees * uncovered / position.quantity.abs(),
            )
            .await
            {
                let mut settings = state.settings.lock().await;
                settings.last_risk_note = format!(
                    "Protective stop could not be armed for {symbol}: {error}; five-second exit monitor remains as fallback"
                );
            }
        }
    }
    Ok(())
}

async fn cancel_protective_stops(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    symbol: &str,
) -> Result<bool, (StatusCode, String)> {
    let stops = {
        let settings = state.settings.lock().await;
        settings
            .positions
            .get(symbol)
            .map(|position| position.protective_stops.clone())
            .unwrap_or_default()
    };
    for stop in stops {
        let response = client
            .delete(format!("{base}/positions/orders/{}", stop.barrier_id))
            .send()
            .await
            .map_err(upstream_error)?;
        if !response.status().is_success() {
            return Ok(false);
        }
        let mut settings = state.settings.lock().await;
        if let Some(position) = settings.positions.get_mut(symbol) {
            position
                .protective_stops
                .retain(|tracked| tracked.barrier_id != stop.barrier_id);
        }
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == stop.order_timestamp)
        {
            order.status = "cancelled".into();
        }
        persist_settings(&state.settings_path, &settings)?;
    }
    Ok(true)
}
