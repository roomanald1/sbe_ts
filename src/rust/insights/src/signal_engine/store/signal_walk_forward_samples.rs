fn non_overlapping_samples(
    mut samples: Vec<SignalOutcome>,
    horizon_seconds: u64,
) -> Vec<SignalOutcome> {
    samples.sort_by_key(|sample| sample.signal_timestamp);
    let mut accepted = Vec::new();
    let mut next_timestamp = 0;
    for sample in samples {
        if sample.signal_timestamp < next_timestamp { continue; }
        next_timestamp = sample.evaluated_timestamp.max(
            sample.signal_timestamp.saturating_add(horizon_seconds.saturating_mul(1000)),
        );
        let has_directional_return = sample.net_return_percent.is_some()
            && matches!(sample.direction.as_str(), "bullish" | "bearish");
        if has_directional_return { accepted.push(sample); }
    }
    accepted
}

#[cfg(test)]
mod walk_forward_sample_tests {
    use super::*;

    fn sample(timestamp: u64, evaluated: u64, direction: &str, net: Option<f64>) -> SignalOutcome {
        SignalOutcome {
            record_type: "outcome".into(), symbol: "BTC-USDT".into(),
            signal_timestamp: timestamp, evaluated_timestamp: evaluated,
            horizon_seconds: 60, return_percent: 1.0, direction_match: true,
            direction: direction.into(), strength: 0.5, net_return_percent: net,
            execution_cost_percent: None, trend_5m_volatility_units: None,
            trend_5m_direction_match: None, trend_5m_net_return_percent: None,
            features: None,
        }
    }

    #[test]
    fn non_overlapping_samples_sort_filter_direction_and_advance_the_time_window() {
        let selected = non_overlapping_samples(vec![
            sample(200, 250, "bullish", Some(0.1)),
            sample(100, 250, "neutral", Some(0.2)),
            sample(300, 350, "bearish", None),
            sample(400, 450, "bearish", Some(-0.1)),
        ], 0);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].signal_timestamp, 400);
    }
}
