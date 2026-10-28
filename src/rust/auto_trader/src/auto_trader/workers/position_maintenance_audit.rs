fn external_close_audit_payload(
    symbol: &str,
    prior_quantity: f64,
    prior_average_entry_price: f64,
    position: &TraderPosition,
) -> serde_json::Value {
    serde_json::json!({
        "record_schema_version": 1,
        "decision_id": position.entry_decision_id,
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
        "entry_position_side": if prior_quantity > 0.0 { "long" } else { "short" },
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
        let payload = external_close_audit_payload("ETH-USDT", -3.0, 12.0, &position);
        assert_eq!(payload["decision_id"], "decision-4");
        assert_eq!(payload["entry_position_side"], "short");
        assert_eq!(payload["entry_quantity"], 3.0);
        assert_eq!(payload["effective_take_profit_percent"], 8.0);
        assert_eq!(payload["price_path"][0]["ask"], 11.0);
    }
}
