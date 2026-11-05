fn benchmark_signal(strategy: &str, candidate: bool) -> Signal {
    Signal {
        symbol: "BTC-USDT".into(), direction: "bullish".into(), strength: 1.0,
        timestamp: 1, stale: false, score: 1.0, ranging_probability: None,
        components: SignalComponents::default(), strategy_candidate: candidate,
        strategy: Some(strategy.into()), strategy_decision: Some(serde_json::json!({
            "observed_close": 120.0, "primary_level": 100.0, "secondary_level": 100.0,
            "btc_gate_required": true, "btc_close": 120.0, "btc_sma": 100.0,
            "primary_metric_value": 10.0, "strategy_order_size_multiplier": 0.5,
            "monthly_anchor_close_time_ms": 1, "cross_sectional_metric_value": 10.0,
            "cross_sectional_threshold": 5.0
        })), market_regime: None, news_attribution: None, entry_gate: None,
    }
}

/// Benchmark AutoTrader-only paths that are outside the reusable strategy crate.
pub fn benchmark_autotrader_paths(iterations: usize) -> Vec<(String, f64)> {
    use std::{hint::black_box, time::Instant};
    assert!(iterations > 0, "benchmark iterations must be positive");
    let signal = benchmark_signal("donchian-55-btc200", true);
    let parameters = ExecutionParameters::default();
    let signal_payload = serde_json::json!({ "symbol": signal.symbol, "direction": signal.direction,
        "strength": signal.strength, "timestamp": signal.timestamp, "strategy": signal.strategy,
        "strategy_decision": signal.strategy_decision });
    let market = serde_json::json!({
        "bids": (0..20).map(|i| serde_json::json!({"price": 100.0 - i as f64 * 0.01, "size": 1.0 + i as f64})).collect::<Vec<_>>(),
        "asks": (0..20).map(|i| serde_json::json!({"price": 100.02 + i as f64 * 0.01, "size": 1.5 + i as f64})).collect::<Vec<_>>()
    });
    let mut results = Vec::new();
    let started = Instant::now();
    for _ in 0..iterations { black_box(serde_json::to_vec(black_box(&signal_payload)).unwrap()); }
    results.push(("signal serialization".into(), started.elapsed().as_nanos() as f64 / iterations as f64));
    let started = Instant::now();
    for _ in 0..iterations { black_box(assess_deep_book(black_box(&market), "BTC-USDT", true, 2_000.0, 10.0)); }
    results.push(("market depth assessment".into(), started.elapsed().as_nanos() as f64 / iterations as f64));
    let started = Instant::now();
    for index in 0..iterations { black_box(build_order_request("BTC-USDT", "buy", 0.25, 100.0, 100.02, AutoOrderType::Limit, parameters, "bench-order", index as u64)); }
    results.push(("limit order request construction".into(), started.elapsed().as_nanos() as f64 / iterations as f64));
    let overrides = BTreeMap::new();
    let started = Instant::now();
    for _ in 0..iterations { black_box(validate_strategy_parameter_overrides(black_box(&overrides)).unwrap()); }
    results.push(("strategy parameter validation".into(), started.elapsed().as_nanos() as f64 / iterations as f64));
    results
}
