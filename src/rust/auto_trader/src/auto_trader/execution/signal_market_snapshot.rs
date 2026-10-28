struct SignalMarketSnapshot {
    market: serde_json::Value,
    bid: f64,
    ask: f64,
    mid: f64,
    market_inputs_ms: f64,
}

