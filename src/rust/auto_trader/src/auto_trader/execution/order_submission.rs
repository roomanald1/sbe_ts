async fn submit_and_record_selected_order(
    input: SelectedOrderExecution<'_>,
) -> Result<CycleResult, (StatusCode, String)> {
    let SelectedOrderExecution {
        state,
        client,
        position_url,
        signal,
        exploratory,
        scout,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        effective_stop_loss_percent,
        effective_take_profit_percent,
        evidence,
        side,
        quantity,
        notional,
        bid,
        ask,
        mid,
        order_type,
        execution_params,
        timestamp,
        decision_id,
        learning_context,
        horizon_take_profit_percent,
        mut stage_timings_ms,
        deep_book,
        estimated_ioc_all_in_bps,
        submitted_limit_price,
        limit_to_mid_bps,
        limit_to_crossing_quote_bps,
        expires_at,
    } = input;
    if !state.enabled.load(Ordering::Relaxed) {
        let mut settings = state.settings.lock().await;
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == timestamp)
        {
            order.status = "cancelled_before_submit".into();
        }
        settings.used_notional_today = allocated_notional(&settings);
        request_settings_persist(state)?;
        return Ok(skipped_for_signal(
            signal,
            "Auto Trader was disabled before order submission",
        ));
    }
    let submit_started = Instant::now();
    let response = match submit_configured_order(
        &client,
        &position_url,
        &signal.symbol,
        side,
        quantity,
        notional,
        bid,
        ask,
        order_type,
        execution_params,
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
                order.failure_reason = Some(error.1.clone());
            }
            request_settings_persist(state)?;
            return Err(error);
        }
    };
    let submit_response_ms = submit_started.elapsed().as_secs_f64() * 1000.0;
    record_execution_stage_timings(&response, submit_response_ms, &mut stage_timings_ms);
    let cycle_guard = acquire_cycle_lock(state, "signal").await;
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    let filled = response
        .get("filled_quantity")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let average_price = response
        .get("average_execution_price")
        .and_then(serde_json::Value::as_f64);
    let fee = response
        .get("estimated_fee")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
        .abs();
    let (realized_delta, learned_type) = if filled > 0.0 {
        let prior_quantity = settings
            .positions
            .get(&signal.symbol)
            .map_or(0.0, |position| position.quantity);
        let incoming_sign = if side == "buy" { 1.0_f64 } else { -1.0_f64 };
        let is_new_position_direction =
            prior_quantity == 0.0 || incoming_sign != prior_quantity.signum();
        let result = average_price
            .map(|price| {
                apply_auto_fill(
                    &mut settings.positions,
                    &signal.symbol,
                    side,
                    filled,
                    price,
                    fee,
                    &format!("{order_type:?}"),
                    &learning_context,
                    true,
                    horizon_take_profit_percent,
                )
            })
            .unwrap_or_default();
        if let Some(price) = average_price {
            let Settings {
                strategy_positions,
                positions,
                ..
            } = &mut *settings;
            apply_strategy_position_fill(
                strategy_positions,
                positions,
                &signal.symbol,
                signal.strategy.as_deref().unwrap_or("live-insights"),
                side,
                filled,
                price,
                fee,
                &format!("{order_type:?}"),
                &learning_context,
                true,
                horizon_take_profit_percent,
                Some(decision_id),
                signal.strategy_decision.as_ref(),
                Some(effective_stop_loss_percent),
                Some(effective_take_profit_percent),
            );
        }
        if let Some(position) = settings.positions.get_mut(&signal.symbol) {
            position.entry_decision_id = Some(decision_id.to_owned());
            if signal.strategy.is_some() {
                position.strategy = signal.strategy.clone();
                position.strategy_decision = signal.strategy_decision.clone();
            }
            if position.quantity.abs() > 1e-12 && is_new_position_direction {
                position.stop_loss_percent = Some(effective_stop_loss_percent);
                position.take_profit_percent = Some(effective_take_profit_percent);
            }
        }
        result
    } else {
        (0.0, None)
    };
    let realized_delta_usd = realized_usd(&settings, &signal.symbol, realized_delta);
    settings.realized_pnl += realized_delta_usd;
    settings.daily_realized_pnl += realized_delta_usd;
    if let Some(kind) = learned_type {
        let stats = settings.learning.entry(kind).or_default();
        record_learning_outcome(stats, realized_delta_usd);
    }
    let status = if response
        .get("barrier_order")
        .is_some_and(|value| !value.is_null())
    {
        "armed"
    } else if response.get("order").is_some_and(|value| !value.is_null()) {
        "resting"
    } else if filled > 0.0 {
        "filled"
    } else {
        "accepted"
    };
    if let Some(order) = settings
        .orders
        .iter_mut()
        .find(|order| order.timestamp == timestamp)
    {
        order.filled_quantity = filled;
        order.fill_rate = (filled / quantity).clamp(0.0, 1.0);
        order.average_price = average_price;
        order.fees_paid = fee;
        order.execution_order_id = response
            .get("order")
            .and_then(|order| order.get("id"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                (matches!(&order_type, AutoOrderType::Twap | AutoOrderType::Vwap))
                    .then(|| response.get("id").and_then(serde_json::Value::as_str))
                    .flatten()
            })
            .map(str::to_owned);
        order.status = status.to_owned();
        order.realized_pnl = realized_delta;
        order.stage_timings_ms = stage_timings_ms;
        order.failure_reason = None;
    }
    settings.used_notional_today = allocated_notional(&settings);
    request_settings_persist(state)?;
    drop(settings);
    drop(cycle_guard);
    let mut audit_payload = signal_audit_payload(
        signal,
        "entry",
        timestamp,
        status,
        None,
        Some(exploratory),
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        evidence,
    );
    audit_payload["scout"] = serde_json::json!(scout);
    audit_payload["effective_stop_loss_percent"] = serde_json::json!(effective_stop_loss_percent);
    audit_payload["effective_take_profit_percent"] =
        serde_json::json!(effective_take_profit_percent);
    audit_payload["order_type"] = serde_json::json!(format!("{order_type:?}"));
    audit_payload["order_quantity"] = serde_json::json!(quantity);
    audit_payload["filled_quantity"] = serde_json::json!(filled);
    audit_payload["average_price"] = serde_json::json!(average_price);
    audit_payload["fees"] = serde_json::json!(fee);
    audit_payload["realized_pnl"] = serde_json::json!(realized_delta);
    let response_received_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let implementation_shortfall_bps = average_price.map(|price| {
        if side == "buy" {
            (price / mid - 1.0) * 10_000.0
        } else {
            (1.0 - price / mid) * 10_000.0
        }
    });
    let fee_bps = (filled > 0.0 && mid > 0.0).then(|| fee / (filled * mid) * 10_000.0);
    audit_payload["entry_fill_timestamp_ms"] = serde_json::json!(response_received_at);
    audit_payload["side"] = serde_json::json!(side);
    audit_payload["decision_mid_price"] = serde_json::json!(mid);
    audit_payload["submitted_at"] = serde_json::json!(timestamp);
    audit_payload["response_received_at"] = serde_json::json!(response_received_at);
    audit_payload["submit_response_ms"] = serde_json::json!(submit_response_ms);
    audit_payload["fill_rate"] = serde_json::json!((filled / quantity).clamp(0.0, 1.0));
    audit_payload["implementation_shortfall_bps"] = serde_json::json!(implementation_shortfall_bps);
    audit_payload["fee_bps"] = serde_json::json!(fee_bps);
    audit_payload["all_in_cost_bps"] = serde_json::json!(implementation_shortfall_bps
        .zip(fee_bps)
        .map(|(shortfall, fees)| shortfall + fees));
    audit_payload["volatility_size_scale"] = serde_json::json!(volatility_size_scale(
        counterfactual_stop_loss_percent,
        effective_stop_loss_percent,
    ));
    audit_payload["submitted_notional"] = serde_json::json!(notional);
    audit_payload["volatility_1m_percent"] =
        serde_json::json!(signal.components.volatility_1m_percent);
    audit_payload["bid"] = serde_json::json!(bid);
    audit_payload["ask"] = serde_json::json!(ask);
    audit_payload["spread_bps"] = serde_json::json!((ask - bid) / mid * 10_000.0);
    audit_payload["book_bid_depth_10bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.bid_depth_10bps));
    audit_payload["book_ask_depth_10bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.ask_depth_10bps));
    audit_payload["book_depth_imbalance_10bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.depth_imbalance_10bps));
    audit_payload["estimated_sweep_bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.impact_bps));
    audit_payload["estimated_ioc_all_in_bps"] = serde_json::json!(estimated_ioc_all_in_bps);
    audit_payload["submitted_limit_price"] = serde_json::json!(submitted_limit_price);
    audit_payload["limit_to_mid_bps"] = serde_json::json!(limit_to_mid_bps);
    audit_payload["limit_to_crossing_quote_bps"] = serde_json::json!(limit_to_crossing_quote_bps);
    audit_payload["expires_at"] = serde_json::json!(expires_at);
    audit_payload["execution_response"] = response.clone();
    queue_auto_trader_audit(state, audit_payload);
    let stop_armed = if filled > 0.0 {
        // Reconcile aggregate exposure without cancelling correctly sided stops;
        // newly filled quantity gets its own barrier while existing protection stays armed.
        match ensure_protective_stops(&client, &position_url, state).await {
            Ok(()) => {
                let settings = state.settings.lock().await;
                settings
                    .positions
                    .get(&signal.symbol)
                    .is_some_and(|position| !position.protective_stops.is_empty())
            }
            Err((_, error)) => {
                let mut settings = state.settings.lock().await;
                settings.last_risk_note = format!(
                    "Protective stop could not be refreshed: {error}; five-second exit monitor remains as fallback"
                );
                let _ = request_settings_persist(state);
                false
            }
        }
    } else {
        false
    };
    Ok(CycleResult {
        status: "order_submitted",
        reason: stop_armed.then(|| "Protective stop-market armed".into()),
        symbol: Some(signal.symbol.clone()),
        side: Some(side.into()),
        quantity: Some(quantity),
        order_type: Some(format!("{order_type:?}")),
        order_response: Some(response),
    })
}
