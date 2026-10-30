async fn sync_protective_stops(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let tracked = {
        let settings = state.settings.lock().await;
        settings.strategy_positions.iter().flat_map(|(symbol, lots)| {
            lots.iter().flat_map(|(strategy, position)| position.protective_stops.iter().cloned()
                .map({ let symbol = symbol.clone(); let strategy = strategy.clone(); move |stop| (symbol.clone(), strategy.clone(), stop) }))
        }).collect::<Vec<_>>()
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
    for (symbol, strategy, stop) in tracked {
        let barrier = barriers
            .iter()
            .find(|barrier| barrier["id"].as_str() == Some(&stop.barrier_id));
        let Some(barrier) = barrier else {
            let mut settings = state.settings.lock().await;
            if let Some(position) = settings.positions.get_mut(&symbol) {
                position.protective_stops.retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(position) = settings.strategy_positions.get_mut(&symbol).and_then(|lots| lots.get_mut(&strategy)) {
                position.protective_stops.retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == stop.order_timestamp)
            {
                order.status = "cancelled".into();
            }
            request_settings_persist(state)?;
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
            let entry_decision_id = settings.strategy_positions.get(&symbol)
                .and_then(|lots| lots.get(&strategy))
                .and_then(|position| position.entry_decision_id.clone());
            let entry_price_before = settings.strategy_positions.get(&symbol)
                .and_then(|lots| lots.get(&strategy))
                .map(|position| position.average_entry_price)
                .filter(|price| *price > 0.0)
                .unwrap_or(fill_price);
            let (price_path, effective_stop_loss_percent, effective_take_profit_percent, entry_fees, entry_quantity, position_entry_timestamp_ms, prior_realized_pnl, initial_entry_notional, prior_total_fees, total_entry_fees_paid) = settings
                .strategy_positions.get(&symbol)
                .and_then(|lots| lots.get(&strategy))
                .map(|position| (
                    position.price_path.clone(),
                    position.stop_loss_percent.unwrap_or(settings.stop_loss_percent),
                    position.take_profit_percent.or(position.horizon_take_profit_percent).unwrap_or(settings.take_profit_percent),
                    position.entry_fees,
                    position.quantity.abs(),
                    position.entry_timestamp_ms,
                    position.realized_pnl,
                    position.initial_entry_notional,
                    position.total_fees_paid,
                    position.total_entry_fees_paid,
                ))
                .unwrap_or_default();
            let (realized_delta, learned_key) = apply_auto_fill(
                &mut settings.positions, &symbol, side, delta_quantity, fill_price,
                delta_fee, "StopMarket", "stop-loss", false, None,
            );
            let Settings { strategy_positions, positions, .. } = &mut *settings;
            let (strategy_realized_delta, _) = apply_strategy_position_fill(
                strategy_positions, positions, &symbol,
                &strategy, side, delta_quantity, fill_price, delta_fee,
                "StopMarket", "stop-loss", false, None, None, None, None, None,
            );
            let realized_delta_usd = realized_usd(&settings, &symbol, realized_delta);
            settings.realized_pnl += realized_delta_usd;
            settings.daily_realized_pnl += realized_delta_usd;
            settings.used_notional_today = allocated_notional(&settings);
            let fully_closed = settings.strategy_positions.get(&symbol)
                .and_then(|lots| lots.get(&strategy))
                .is_none_or(|position| position.quantity.abs() <= 1e-12);
            let entry_notional = if initial_entry_notional > 0.0 {
                initial_entry_notional
            } else {
                delta_quantity * entry_price_before
            };
            if fully_closed && entry_notional > 0.0 {
                let total_trade_pnl = prior_realized_pnl + strategy_realized_delta;
                let total_fees = if prior_total_fees > 0.0 {
                    prior_total_fees + delta_fee
                } else {
                    entry_fees + delta_fee
                };
                let fees_usd = realized_usd(&settings, &symbol, total_fees);
                let strategy_realized_usd = realized_usd(&settings, &symbol, total_trade_pnl);
                record_closed_trade_outcome(
                    &mut settings,
                    total_trade_pnl / entry_notional * 100.0,
                    strategy_realized_usd,
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
                order.fill_rate = if order.quantity > 0.0 {
                    (total_filled / order.quantity).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                order.average_price = barrier["average_execution_price"].as_f64();
                order.fees_paid = total_fee;
                order.realized_pnl += realized_delta;
                record_strategy_order_realized_pnl(order, strategy_realized_delta);
                order.status = status.to_owned();
            }
            let Settings { strategy_positions, positions, .. } = &mut *settings;
            for position in [
                positions.get_mut(&symbol),
                strategy_positions.get_mut(&symbol).and_then(|lots| lots.get_mut(&strategy)),
            ].into_iter().flatten() {
                if let Some(tracked) = position.protective_stops.iter_mut().find(|tracked| tracked.barrier_id == stop.barrier_id) {
                    tracked.processed_quantity = total_filled;
                    tracked.processed_notional = total_notional;
                    tracked.processed_fee = total_fee;
                }
            }
            request_settings_persist(state)?;
            let cumulative_strategy_pnl = settings
                .orders
                .iter()
                .find(|order| order.timestamp == stop.order_timestamp)
                .and_then(|order| order.strategy_realized_pnl)
                .unwrap_or(strategy_realized_delta);
            drop(settings);
            if let Some(decision_id) = entry_decision_id {
                let update_timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let payload = protective_stop_audit_payload(
                    &decision_id,
                    &stop,
                    update_timestamp,
                    &symbol,
                    &strategy,
                    status,
                    barrier,
                    total_filled,
                    total_fee,
                    cumulative_strategy_pnl,
                    entry_price_before,
                    entry_fees,
                    entry_quantity,
                    initial_entry_notional,
                    total_entry_fees_paid,
                    prior_total_fees + delta_fee,
                    position_entry_timestamp_ms,
                    effective_stop_loss_percent,
                    effective_take_profit_percent,
                    &price_path,
                );
                queue_auto_trader_audit(state, payload);
            }
        }
        if !matches!(status, "armed" | "triggering") {
            let mut settings = state.settings.lock().await;
            if let Some(position) = settings.positions.get_mut(&symbol) {
                position.protective_stops.retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(position) = settings.strategy_positions.get_mut(&symbol).and_then(|lots| lots.get_mut(&strategy)) {
                position.protective_stops.retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == stop.order_timestamp)
            {
                order.status = status.to_owned();
            }
            request_settings_persist(state)?;
        }
    }
    Ok(())
}



fn protective_stop_audit_payload(
    decision_id: &str,
    stop: &ProtectiveStop,
    update_timestamp: u64,
    symbol: &str,
    strategy: &str,
    status: &str,
    barrier: &serde_json::Value,
    total_filled: f64,
    total_fee: f64,
    cumulative_pnl: f64,
    entry_price_before: f64,
    _entry_fees: f64,
    entry_quantity: f64,
    initial_entry_notional: f64,
    total_entry_fees_paid: f64,
    total_fees_paid: f64,
    position_entry_timestamp_ms: u64,
    effective_stop_loss_percent: f64,
    effective_take_profit_percent: f64,
    price_path: &[TradePriceSample],
) -> serde_json::Value {
    serde_json::json!({
        "record_schema_version": 1,
        "decision_id": decision_id,
        "strategy": strategy,
        "stage": format!("exit:stop:{}", stop.order_timestamp),
        "timestamp": update_timestamp,
        "observed_at": update_timestamp,
        "symbol": symbol,
        "side": barrier["side"],
        "status": status,
        "reason": "stop-loss",
        "exit_order_type": "StopMarket",
        "submitted_at": barrier["triggered_at"],
        "barrier_order_id": stop.barrier_id,
        "exit_order_quantity": stop.quantity,
        "exit_filled_quantity": total_filled,
        "exit_average_price": barrier["average_execution_price"],
        "exit_fees": total_fee,
        "realized_pnl": cumulative_pnl,
        "entry_average_price": entry_price_before,
        "entry_fees_paid": total_entry_fees_paid,
        "entry_timestamp_ms": position_entry_timestamp_ms,
        "entry_context": {
            "average_price": entry_price_before,
            "fees": total_entry_fees_paid,
            "initial_entry_notional": initial_entry_notional,
            "total_entry_fees_paid": total_entry_fees_paid,
            "total_fees_paid": total_fees_paid,
            "timestamp_ms": position_entry_timestamp_ms,
            "quantity": if entry_price_before > 0.0 && initial_entry_notional > 0.0 {
                initial_entry_notional / entry_price_before
            } else {
                entry_quantity
            },
        },
        "duration_ms": (position_entry_timestamp_ms > 0)
            .then_some(update_timestamp.saturating_sub(position_entry_timestamp_ms)),
        "barrier_trigger_price": barrier["trigger_price"],
        "barrier_trigger_direction": barrier["trigger_direction"],
        "effective_stop_loss_percent": effective_stop_loss_percent,
        "effective_take_profit_percent": effective_take_profit_percent,
        "price_path": price_path.iter().map(|sample| serde_json::json!({
            "timestamp": sample.timestamp,
            "bid": sample.bid,
            "ask": sample.ask,
        })).collect::<Vec<_>>(),
        "net_return_percent": (entry_price_before > 0.0).then_some(
            cumulative_pnl / initial_entry_notional.max(1e-12) * 100.0
        ),
    })
}


#[cfg(test)]
mod protective_stop_audit_tests {
    use super::*;

    #[test]
    fn audit_payload_records_barrier_fill_and_duration() {
        let stop = ProtectiveStop { barrier_id: "barrier-1".into(), order_timestamp: 1_000, quantity: 2.0, ..ProtectiveStop::default() };
        let path = [TradePriceSample { timestamp: 1_500, bid: 99.0, ask: 101.0 }];
        let barrier = serde_json::json!({"side":"sell","triggered_at":1200,"average_execution_price":98.0,"trigger_price":99.0,"trigger_direction":"below"});
        let payload = protective_stop_audit_payload("decision-3", &stop, 2_000, "BTC-USDT", "strategy-a", "filled", &barrier, 2.0, 0.1, -4.1, 100.0, 0.2, 2.0, 200.0, 0.2, 0.3, 1_000, 5.0, 10.0, &path);

        assert_eq!(payload["stage"], "exit:stop:1000");
        assert_eq!(payload["barrier_order_id"], "barrier-1");
        assert_eq!(payload["duration_ms"], 1_000);
        assert_eq!(payload["entry_context"]["average_price"], 100.0);
        assert_eq!(payload["entry_context"]["quantity"], 2.0);
        assert_eq!(payload["price_path"][0]["bid"], 99.0);
        assert_eq!(payload["net_return_percent"], -2.05);
    }
}
