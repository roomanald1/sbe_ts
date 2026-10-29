fn position_exit_audit_payload(
    decision_id: &str,
    observed_at: u64,
    submitted_at: u64,
    symbol: &str,
    reason: &str,
    side: &str,
    response: &serde_json::Value,
    position: &TraderPosition,
    quantity: f64,
    filled: f64,
    average_price: Option<f64>,
    fee: f64,
    realized_pnl: f64,
    entry_notional: f64,
    return_fraction: f64,
    bid: f64,
    ask: f64,
    submit_response_ms: f64,
    stop_loss_percent: f64,
    take_profit_percent: f64,
) -> serde_json::Value {
    let mid = (bid + ask) / 2.0;
    let mut payload = serde_json::json!({
        "record_schema_version": 2,
        "decision_id": decision_id,
        "trade_id": decision_id,
        "stage": format!("exit:{submitted_at}"),
        "timestamp": observed_at,
        "symbol": symbol,
        "status": exit_fill_status(filled, quantity),
        "reason": reason,
        "exit_order_type": "Ioc",
        "exit_order_quantity": quantity,
        "exit_filled_quantity": filled,
        "exit_average_price": average_price,
        "exit_fees": fee,
        "realized_pnl": realized_pnl,
        "execution_response": response,
        "net_return_percent": (entry_notional > 0.0).then_some(realized_pnl / entry_notional * 100.0),
        "side": side,
        "decision_mid_price": mid,
        "submitted_at": submitted_at,
        "response_received_at": observed_at,
        "submit_response_ms": submit_response_ms,
        "fill_rate": (quantity > 0.0).then_some((filled / quantity).clamp(0.0, 1.0)),
        "implementation_shortfall_bps": average_price.map(|price| if side == "buy" { (price / mid - 1.0) * 10_000.0 } else { (1.0 - price / mid) * 10_000.0 }),
        "fee_bps": (filled > 0.0).then_some(fee / (filled * mid) * 10_000.0),
        "exit_bid": bid,
        "exit_ask": ask,
        "effective_stop_loss_percent": stop_loss_percent,
        "effective_take_profit_percent": take_profit_percent,
    });
    payload["price_path"] = serde_json::json!(position.price_path.iter().map(|sample| {
        serde_json::json!({ "timestamp": sample.timestamp, "bid": sample.bid, "ask": sample.ask })
    }).collect::<Vec<_>>());
    payload["entry_context"] = serde_json::json!({
        "strategy_decision": position.strategy_decision,
        "average_price": position.average_entry_price,
        "fees": position.entry_fees,
        "timestamp_ms": position.entry_timestamp_ms,
        "duration_ms": (position.entry_timestamp_ms > 0).then_some(observed_at.saturating_sub(position.entry_timestamp_ms)),
        "quantity": position.quantity,
    });
    payload["exit_rule_trace"] = serde_json::json!({
        "reason": reason,
        "net_open_pnl": return_fraction * entry_notional,
        "net_return_fraction": return_fraction,
        "stop_loss_percent": stop_loss_percent,
        "take_profit_percent": take_profit_percent,
        "entry_notional": entry_notional,
        "entry_average_price": position.average_entry_price,
        "entry_fees": position.entry_fees,
        "observed_bid": bid,
        "observed_ask": ask,
    });
    let protection = profit_protection_parameters(position);
    payload["exit_rule_trace"]["profit_protection"] = serde_json::json!({
        "activation_percent": protection.activation_percent,
        "giveback_percent": protection.giveback_percent,
        "minimum_lock_percent": protection.minimum_lock_percent,
        "peak_return_percent": position.profit_protection_peak_return_percent,
        "triggered": reason == "profit-protection",
    });
    payload

}

fn exit_fill_status(filled: f64, quantity: f64) -> &'static str {
    if filled + 1e-12 >= quantity {
        "filled"
    } else if filled > 0.0 {
        "partially_filled_closed"
    } else {
        "expired_or_cancelled"
    }
}


fn new_position_exit_order(
    timestamp: u64,
    symbol: &str,
    side: &str,
    reason: &str,
    position: &TraderPosition,
    quantity: f64,
    notional: f64,
) -> TraderOrder {
    TraderOrder {
        timestamp,
        triggered_at: None,
        symbol: symbol.to_owned(),
        side: side.to_owned(),
        order_type: "Ioc".into(),
        purpose: reason.into(),
        strategy: Some(
            position
                .strategy
                .clone()
                .unwrap_or_else(|| "live-insights".into()),
        ),
        // Link the protective exit to the decision that opened this position.
        decision_id: position.entry_decision_id.clone(),
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
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_exit_audit_includes_execution_and_entry_context() {
        let position = TraderPosition {
            quantity: 2.0,
            average_entry_price: 100.0,
            entry_fees: 0.2,
            entry_timestamp_ms: 1_000,
            strategy_decision: Some(serde_json::json!({"strategy": "test"})),
            price_path: vec![TradePriceSample { timestamp: 1_500, bid: 101.0, ask: 102.0 }],
            ..TraderPosition::default()
        };
        let payload = position_exit_audit_payload(
            "decision-1", 2_000, 1_800, "BTC-USDT", "take-profit", "sell",
            &serde_json::json!({"ok": true}), &position, 2.0, 2.0, Some(110.0),
            0.1, 19.7, 200.0, 0.0985, 109.0, 111.0, 2.0, 5.0, 10.0,
        );

        assert_eq!(payload["stage"], "exit:1800");
        assert_eq!(payload["status"], "filled");
        assert_eq!(payload["net_return_percent"], 9.85);
        assert_eq!(payload["entry_context"]["duration_ms"], 1_000);
        assert_eq!(payload["price_path"][0]["timestamp"], 1_500);
        assert_eq!(payload["exit_rule_trace"]["reason"], "take-profit");
    }

    #[test]
    fn position_exit_audit_handles_zero_quantity_metrics() {
        let payload = position_exit_audit_payload(
            "decision-2", 2, 1, "ETH-USDT", "stop-loss", "buy",
            &serde_json::Value::Null, &TraderPosition::default(), 0.0, 0.0,
            None, 0.0, 0.0, 0.0, 0.0, 99.0, 101.0, 0.0, 5.0, 10.0,
        );
        assert_eq!(payload["status"], "filled");
        assert_eq!(payload["fill_rate"], serde_json::Value::Null);
        assert_eq!(payload["net_return_percent"], serde_json::Value::Null);
        assert_eq!(payload["fee_bps"], serde_json::Value::Null);
    }
}
