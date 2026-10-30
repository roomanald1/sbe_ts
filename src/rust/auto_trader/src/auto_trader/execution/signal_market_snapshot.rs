struct SignalMarketSnapshot {
    market: serde_json::Value,
    bid: f64,
    ask: f64,
    mid: f64,
    market_cache_lookup_ms: f64,
    market_http_fetch_ms: f64,
}
