use crate::{Signal, SignalComponents, Strategy, StrategySignal, all_strategy_parameter_specs};
use serde_json::json;
use std::collections::BTreeMap;

pub(crate) fn benchmark_signal(strategy: &str, candidate: bool) -> Signal {
    Signal {
        symbol: "BTC-USDT".into(),
        direction: "bullish".into(),
        strength: 1.0,
        timestamp: 1,
        stale: false,
        score: 1.0,
        ranging_probability: None,
        components: SignalComponents::default(),
        strategy_candidate: candidate,
        strategy: Some(strategy.into()),
        strategy_decision: Some(json!({
            "observed_close": 120.0,
            "primary_level": 100.0,
            "secondary_level": 100.0,
            "btc_gate_required": true,
            "btc_close": 120.0,
            "btc_sma": 100.0,
            "primary_metric_value": 10.0,
            "strategy_order_size_multiplier": 0.5,
            "monthly_anchor_close_time_ms": 1,
            "cross_sectional_metric_value": 10.0,
            "cross_sectional_threshold": 5.0
        })),
        market_regime: None,
        news_attribution: None,
        entry_gate: None,
    }
}

/// Measure registered strategy entry and position evaluation paths.
pub fn benchmark_strategy_evaluation(iterations: usize) -> Vec<(String, f64, f64)> {
    use std::{hint::black_box, time::Instant};
    assert!(iterations > 0, "benchmark iterations must be positive");
    Strategy::all()
        .map(|strategy| {
            let parameters = all_strategy_parameter_specs(strategy)
                .into_iter()
                .map(|spec| (spec.key.as_str().to_owned(), spec.default))
                .collect::<BTreeMap<_, _>>();
            let signal = benchmark_signal(strategy.as_str(), true);
            let entry = StrategySignal::entry(&signal, &parameters);
            let started = Instant::now();
            for _ in 0..iterations {
                black_box(strategy.evaluate(black_box(&entry)).unwrap());
            }
            let entry_ns = started.elapsed().as_nanos() as f64 / iterations as f64;
            let position = StrategySignal::position(&parameters, true, Some(true), false);
            let started = Instant::now();
            for _ in 0..iterations {
                black_box(strategy.evaluate(black_box(&position)).unwrap());
            }
            let position_ns = started.elapsed().as_nanos() as f64 / iterations as f64;
            (strategy.as_str().to_owned(), entry_ns, position_ns)
        })
        .collect()
}
