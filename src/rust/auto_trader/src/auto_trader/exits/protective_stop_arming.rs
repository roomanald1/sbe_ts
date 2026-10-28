fn build_protective_stop_request(
    symbol: &str,
    side: &str,
    quantity: f64,
    trigger_price: f64,
    trigger_direction: &str,
    client_order_id: String,
) -> serde_json::Value {
    serde_json::json!({
        "symbol": symbol,
        "side": side,
        "order_type": "stop-market",
        "quantity": quantity,
        "trigger_price": trigger_price,
        "trigger_direction": trigger_direction,
        "source": "auto",
        "client_order_id": client_order_id,
    })
}

async fn arm_protective_stop(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    symbol: &str,
    strategy: &str,
    position_side: &str,
    quantity: f64,
    entry_price: f64,
    entry_fees: f64,
    stop_loss_override: Option<f64>,
) -> Result<(), (StatusCode, String)> {
    if quantity <= 0.0 || entry_price <= 0.0 {
        return Ok(());
    }
    let (stop_loss_percent, is_enabled) = {
        let settings = state.settings.lock().await;
        (
            stop_loss_override.unwrap_or(settings.stop_loss_percent),
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
    let client_order_id = format!(
        "auto-stop-{symbol}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let request = build_protective_stop_request(
        symbol,
        side,
        quantity,
        trigger_price,
        trigger_direction,
        client_order_id,
    );
    let response = client
        .post(format!("{base}/positions/orders"))
        .json(&request)
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
    let stop = ProtectiveStop {
        barrier_id: id.clone(),
        strategy: strategy.to_owned(),
        order_timestamp: timestamp,
        quantity,
        processed_quantity: 0.0,
        processed_notional: 0.0,
        processed_fee: 0.0,
    };
    let aggregate_active = settings.positions.get(symbol).is_some_and(|position| {
        position.quantity.abs() > 1e-12
            && position.quantity.signum() == if is_long { 1.0 } else { -1.0 }
    });
    let lot_active = settings.strategy_positions.get(symbol).and_then(|lots| lots.get(strategy)).is_some_and(|position| {
        position.quantity.abs() > 1e-12
            && position.quantity.signum() == if is_long { 1.0 } else { -1.0 }
    });
    if aggregate_active && lot_active {
        if let Some(position) = settings.positions.get_mut(symbol) { position.protective_stops.push(stop.clone()); }
        if let Some(position) = settings.strategy_positions.get_mut(symbol).and_then(|lots| lots.get_mut(strategy)) { position.protective_stops.push(stop); }
        settings.orders.push(new_protective_stop_order(
            timestamp,
            symbol,
            side,
            strategy.to_owned(),
            quantity,
            trigger_price,
        ));
        if settings.orders.len() > MAX_RETAINED_AUTO_TRADER_ORDERS {
            let excess = settings.orders.len() - MAX_RETAINED_AUTO_TRADER_ORDERS;
            settings.orders.drain(..excess);
        }
        request_settings_persist(state)?;
    } else {
        drop(settings);
        let _ = client
            .delete(format!("{base}/positions/orders/{id}"))
            .send()
            .await;
    }
    Ok(())
}


fn new_protective_stop_order(
    timestamp: u64,
    symbol: &str,
    side: &str,
    strategy: String,
    quantity: f64,
    trigger_price: f64,
) -> TraderOrder {
    TraderOrder {
        timestamp,
        triggered_at: None,
        symbol: symbol.to_owned(),
        side: side.to_owned(),
        order_type: "StopMarket".into(),
        purpose: "stop-loss".into(),
        strategy: Some(strategy),
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
    }
}
