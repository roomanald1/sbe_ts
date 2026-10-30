fn external_close_audit_payload(
    symbol: &str,
    prior_quantity: f64,
    closed_quantity: f64,
    prior_average_entry_price: f64,
    position: &TraderPosition,
) -> serde_json::Value {
    serde_json::json!({
        "record_schema_version": 1,
        "decision_id": position.entry_decision_id,
        "trade_id": position.entry_decision_id,
        "strategy": position.strategy,
        "symbol": symbol,
        "entry_average_price": prior_average_entry_price,
        "entry_fees_paid": position.entry_fees,
        "entry_timestamp_ms": position.entry_timestamp_ms,
        "effective_stop_loss_percent": position.stop_loss_percent,
        "effective_take_profit_percent": position.take_profit_percent.or(position.horizon_take_profit_percent),
        "price_path": position.price_path.iter().map(|sample| serde_json::json!({
            "timestamp": sample.timestamp, "bid": sample.bid, "ask": sample.ask
        })).collect::<Vec<_>>(),
        "entry_quantity": prior_quantity.abs(),
        "close_quantity": closed_quantity,
        "entry_position_side": if prior_quantity > 0.0 { "long" } else { "short" },
        "entry_context": {
            "average_price": prior_average_entry_price,
            "fees": position.total_entry_fees_paid,
            "initial_entry_notional": position.initial_entry_notional,
            "total_entry_fees_paid": position.total_entry_fees_paid,
            "total_fees_paid": position.total_fees_paid,
            "timestamp_ms": position.entry_timestamp_ms,
            "quantity": if prior_average_entry_price > 0.0 && position.initial_entry_notional > 0.0 {
                position.initial_entry_notional / prior_average_entry_price
            } else {
                prior_quantity.abs()
            },
        },
    })
}

fn external_close_quantity(prior_quantity: f64, current_quantity: f64) -> f64 {
    if prior_quantity.signum() == current_quantity.signum() {
        (prior_quantity.abs() - current_quantity.abs()).max(0.0)
    } else {
        prior_quantity.abs()
    }
}

fn external_close_fills(
    target_quantity: f64,
    close_side: &str,
    trades: &[serde_json::Value],
    through_ms: u64,
    known_exit_ids: &BTreeSet<String>,
) -> Vec<serde_json::Value> {
    let mut candidates = trades.iter()
        .filter(|trade| trade["side"].as_str() == Some(close_side))
        .filter(|trade| trade["timestamp"].as_u64().is_some_and(|timestamp| timestamp <= through_ms))
        .filter_map(|trade| {
            let quantity = trade["quantity"].as_f64()?;
            let price = trade["execution_price"].as_f64()?;
            if !quantity.is_finite() || quantity <= 0.0 || !price.is_finite() || price <= 0.0 {
                return None;
            }
            let parent_order_id = trade["parent_order_id"].as_str().map(str::to_owned);
            if parent_order_id.as_ref().is_some_and(|id| known_exit_ids.contains(id)) {
                return None;
            }
            Some((trade["timestamp"].as_u64().unwrap_or_default(), quantity, price,
                trade["fee"].as_f64().unwrap_or_default().abs(),
                parent_order_id.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null)))
        }).collect::<Vec<_>>();
    candidates.sort_by_key(|fill| fill.0);

    let mut remaining = target_quantity.max(0.0);
    let mut selected = Vec::new();
    for (timestamp, quantity, price, fee, parent_order_id) in candidates.into_iter().rev() {
        if remaining <= 1e-12 { break; }
        let allocated = quantity.min(remaining);
        selected.push(serde_json::json!({
            "timestamp": timestamp,
            "quantity": allocated,
            "price": price,
            "fee": fee * allocated / quantity,
            "parent_order_id": parent_order_id,
        }));
        remaining -= allocated;
    }
    if remaining > 1e-8 { return Vec::new(); }
    selected.reverse();
    selected
}

fn external_strategy_close_audits(
    symbol: &str,
    target_quantity: f64,
    close_side: &str,
    strategy_lots: &[(String, TraderPosition)],
    trades: &[serde_json::Value],
    observed_at: u64,
    known_exit_ids: &BTreeSet<String>,
) -> Vec<serde_json::Value> {
    let close_fills = external_close_fills(target_quantity, close_side, trades, observed_at, known_exit_ids);
    if close_fills.is_empty() { return Vec::new(); }

    let mut lots = strategy_lots.to_vec();
    lots.sort_by_key(|(_, lot)| lot.entry_timestamp_ms);
    let originals = lots.iter().map(|(_, lot)| lot.clone()).collect::<Vec<_>>();
    let mut pnl_by_lot = vec![0.0; lots.len()];
    let mut fills_by_lot = vec![Vec::<serde_json::Value>::new(); lots.len()];
    let mut closed_by_lot = vec![0.0; lots.len()];
    let position_sign = if close_side == "sell" { 1.0 } else { -1.0 };

    for fill in &close_fills {
        let Some(mut fill_remaining) = fill["quantity"].as_f64() else { continue; };
        let fill_quantity = fill_remaining;
        let price = fill["price"].as_f64().unwrap_or_default();
        let total_fee = fill["fee"].as_f64().unwrap_or_default().abs();
        for index in 0..lots.len() {
            if fill_remaining <= 1e-12 { break; }
            let (strategy, position) = &mut lots[index];
            if position.quantity.abs() <= 1e-12 || position.quantity.signum() != position_sign {
                continue;
            }
            let allocated = fill_remaining.min(position.quantity.abs());
            let fee = total_fee * allocated / fill_quantity;
            let mut one_position = BTreeMap::new();
            one_position.insert(strategy.clone(), position.clone());
            let (realized, _) = apply_auto_fill(
                &mut one_position,
                strategy,
                close_side,
                allocated,
                price,
                fee,
                "ExternalLedgerClose",
                "external-or-manual-close",
                false,
                None,
            );
            *position = one_position.remove(strategy).unwrap_or_default();
            pnl_by_lot[index] += realized;
            closed_by_lot[index] += allocated;
            fills_by_lot[index].push(serde_json::json!({
                "timestamp": fill["timestamp"],
                "quantity": allocated,
                "price": price,
                "fee": fee,
                "parent_order_id": fill["parent_order_id"],
            }));
            fill_remaining -= allocated;
        }
    }

    lots.into_iter().enumerate().filter_map(|(index, (strategy, _))| {
        let position = &originals[index];
        let closed_quantity = closed_by_lot[index];
        let decision_id = position.entry_decision_id.as_deref()?;
        // Manual closes can reduce a strategy lot without closing it entirely.
        // Emit an audit for the realized portion so it is included in the
        // strategy P&L breakdown instead of appearing as an unattributed ledger
        // adjustment. The remaining quantity stays open in the strategy lot.
        if closed_quantity <= 1e-12 { return None; }
        let fills = &fills_by_lot[index];
        let exit_notional = fills.iter().filter_map(|fill| {
            Some(fill["quantity"].as_f64()? * fill["price"].as_f64()?)
        }).sum::<f64>();
        let exit_fees = fills.iter().filter_map(|fill| fill["fee"].as_f64()).sum::<f64>();
        let entry_notional = position.initial_entry_notional;
        let entry_quantity = if entry_notional > 0.0 && position.average_entry_price > 0.0 {
            entry_notional / position.average_entry_price
        } else {
            position.quantity.abs()
        };
        let entry_fees = if position.total_entry_fees_paid > 0.0 {
            position.total_entry_fees_paid
        } else {
            position.entry_fees.abs()
        };
        Some(serde_json::json!({
            "record_schema_version": 2,
            "decision_id": decision_id,
            "trade_id": decision_id,
            "strategy": strategy,
            "stage": format!("exit:external:{symbol}:{}:{observed_at}", position.entry_timestamp_ms),
            "timestamp": observed_at,
            "symbol": symbol,
            "side": close_side,
            "status": "filled",
            "reason": "external-or-manual-close",
            "exit_order_type": "external-ledger-fill",
            "exit_order_quantity": closed_quantity,
            "exit_filled_quantity": closed_quantity,
            "exit_average_price": (closed_quantity > 0.0).then_some(exit_notional / closed_quantity),
            "exit_fees": exit_fees,
            "exit_fills": fills,
            "realized_pnl": pnl_by_lot[index],
            "entry_average_price": position.average_entry_price,
            "entry_fees_paid": entry_fees,
            "entry_timestamp_ms": position.entry_timestamp_ms,
            "entry_context": {
                "average_price": position.average_entry_price,
                "fees": entry_fees,
                "initial_entry_notional": entry_notional,
                "total_entry_fees_paid": entry_fees,
                "total_fees_paid": position.total_fees_paid + exit_fees,
                "timestamp_ms": position.entry_timestamp_ms,
                "quantity": entry_quantity,
            },
            "duration_ms": (position.entry_timestamp_ms > 0).then_some(observed_at.saturating_sub(position.entry_timestamp_ms)),
            "pnl_attribution_method": "fifo_strategy_position_lots",
        }))
    }).collect()
}

fn external_close_order(payload: &serde_json::Value) -> Option<TraderOrder> {
    let quantity = payload["exit_filled_quantity"].as_f64()?;
    let average_price = payload["exit_average_price"].as_f64()?;
    let realized_pnl = payload["realized_pnl"].as_f64()?;
    let strategy = payload["strategy"].as_str().unwrap_or("manual").to_owned();
    let decision_id = payload["decision_id"].as_str().map(str::to_owned);
    let timestamp = payload["timestamp"].as_u64()?;
    Some(TraderOrder {
        timestamp,
        triggered_at: None,
        symbol: payload["symbol"].as_str()?.to_owned(),
        side: payload["side"].as_str()?.to_owned(),
        order_type: "ExternalLedgerClose".into(),
        purpose: "external-close".into(),
        strategy: Some(strategy),
        decision_id,
        quantity,
        filled_quantity: quantity,
        fill_rate: 1.0,
        average_price: Some(average_price),
        fees_paid: payload["exit_fees"].as_f64().unwrap_or_default(),
        reserved_notional: quantity * average_price,
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
        status: "filled".into(),
        realized_pnl,
        strategy_realized_pnl: Some(realized_pnl),
        strategy_realized_pnl_today: Some(realized_pnl),
        strategy_realized_pnl_day: Some(utc_day()),
    })
}

#[cfg(test)]
mod external_close_audit_tests {
    use super::*;

    #[test]
    fn external_close_payload_captures_entry_and_path_context() {
        let position = TraderPosition {
            entry_decision_id: Some("decision-4".into()),
            entry_fees: 0.25,
            entry_timestamp_ms: 1_000,
            stop_loss_percent: Some(4.0),
            take_profit_percent: None,
            horizon_take_profit_percent: Some(8.0),
            price_path: vec![TradePriceSample { timestamp: 1_500, bid: 10.0, ask: 11.0 }],
            ..TraderPosition::default()
        };
        let payload = external_close_audit_payload("ETH-USDT", -3.0, 1.25, 12.0, &position);
        assert_eq!(payload["decision_id"], "decision-4");
        assert_eq!(payload["entry_position_side"], "short");
        assert_eq!(payload["entry_quantity"], 3.0);
        assert_eq!(payload["close_quantity"], 1.25);
        assert_eq!(payload["effective_take_profit_percent"], 8.0);
        assert_eq!(payload["price_path"][0]["ask"], 11.0);
    }

    #[test]
    fn partial_manual_close_is_allocated_to_the_strategy_and_persisted_as_an_order() {
        let mut lot = TraderPosition {
            quantity: 2.0,
            average_entry_price: 100.0,
            entry_fees: 0.0,
            initial_entry_notional: 200.0,
            total_entry_fees_paid: 0.0,
            entry_decision_id: Some("decision-5".into()),
            entry_timestamp_ms: 1_000,
            strategy: Some("test-strategy".into()),
            ..TraderPosition::default()
        };
        let fills = vec![serde_json::json!({
            "side": "sell",
            "timestamp": 2_000,
            "quantity": 0.5,
            "execution_price": 110.0,
            "fee": 0.0,
            "parent_order_id": "manual-order-1"
        })];
        let audits = external_strategy_close_audits(
            "ETH-USDT",
            0.5,
            "sell",
            &[("test-strategy".into(), lot.clone())],
            &fills,
            2_000,
            &BTreeSet::new(),
        );

        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0]["exit_filled_quantity"], 0.5);
        assert_eq!(audits[0]["strategy"], "test-strategy");
        assert_eq!(audits[0]["realized_pnl"], 5.0);
        let order = external_close_order(&audits[0]).expect("partial close order");
        assert_eq!(order.purpose, "external-close");
        assert_eq!(order.strategy_realized_pnl, Some(5.0));

        let mut positions = BTreeMap::new();
        positions.insert("test-strategy".to_owned(), lot.clone());
        let (pnl, _) = apply_auto_fill(&mut positions, "test-strategy", "sell", 0.5, 110.0, 0.0,
            "ExternalLedgerClose", "external-or-manual-close", false, None);
        lot = positions.remove("test-strategy").expect("remaining strategy lot");
        assert_eq!(pnl, order.realized_pnl);
        assert!((lot.quantity - 1.5).abs() < 1e-12);
    }

    #[test]
    fn close_quantity_detects_partial_full_and_reversing_closes_without_counting_adds() {
        assert_eq!(external_close_quantity(3.0, 2.0), 1.0);
        assert_eq!(external_close_quantity(-3.0, -1.0), 2.0);
        assert_eq!(external_close_quantity(3.0, 0.0), 3.0);
        assert_eq!(external_close_quantity(3.0, -1.0), 3.0);
        assert_eq!(external_close_quantity(3.0, 4.0), 0.0);
    }

    #[test]
    fn legacy_manual_close_without_strategy_metadata_still_gets_an_audit_order() {
        let order = external_close_order(&serde_json::json!({
            "timestamp": 2_000,
            "symbol": "ETH-USDT",
            "side": "sell",
            "exit_filled_quantity": 0.5,
            "exit_average_price": 110.0,
            "realized_pnl": 5.0,
        })).expect("legacy manual close order");
        assert_eq!(order.strategy.as_deref(), Some("manual"));
        assert_eq!(order.decision_id, None);
        assert_eq!(order.strategy_realized_pnl, Some(5.0));
    }
}
