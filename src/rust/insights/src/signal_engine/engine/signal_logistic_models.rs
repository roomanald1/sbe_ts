fn feature_vector(components: &SignalComponents) -> Option<[f64; 4]> {
    let values = [
        components
            .trend_1m_percent
            .map(|v| (v / 0.25).clamp(-4.0, 4.0)),
        components
            .momentum_10s_percent
            .map(|v| (v / 0.12).clamp(-4.0, 4.0)),
        components.trade_flow_imbalance,
        components.book_imbalance,
    ];
    values
        .iter()
        .all(Option::is_some)
        .then(|| values.map(Option::unwrap))
}

fn train_logistic(samples: &[SignalOutcome]) -> Option<[f64; 5]> {
    let training = samples
        .iter()
        .filter_map(|sample| Some((sample.features?, (sample.return_percent > 0.0) as u8 as f64)))
        .collect::<Vec<_>>();
    if training.len() < WALK_FORWARD_MIN_TRAINING_SAMPLES {
        return None;
    }
    let mut weights = [0.0; 5];
    for _ in 0..250 {
        let mut gradient = [0.0; 5];
        for (features, target) in &training {
            let z = weights[0]
                + features
                    .iter()
                    .enumerate()
                    .map(|(index, value)| weights[index + 1] * value)
                    .sum::<f64>();
            let probability = 1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp());
            let error = probability - target;
            gradient[0] += error;
            for index in 0..4 {
                gradient[index + 1] += error * features[index];
            }
        }
        let scale = 0.04 / training.len() as f64;
        for index in 0..5 {
            weights[index] -= scale * gradient[index];
        }
    }
    Some(weights)
}

fn logistic_up_probability(weights: &[f64; 5], features: &[f64; 4]) -> f64 {
    let z = weights[0]
        + features
            .iter()
            .enumerate()
            .map(|(index, value)| weights[index + 1] * value)
            .sum::<f64>();
    1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp())
}

fn train_trend_logistic(samples: &[SignalOutcome]) -> Option<[f64; 6]> {
    let training = samples
        .iter()
        .filter_map(|sample| {
            Some((
                sample.features?,
                sample.trend_5m_volatility_units?.clamp(-4.0, 4.0),
                (sample.return_percent > 0.0) as u8 as f64,
            ))
        })
        .collect::<Vec<_>>();
    if training.len() < WALK_FORWARD_MIN_TRAINING_SAMPLES {
        return None;
    }
    let mut weights = [0.0; 6];
    for _ in 0..250 {
        let mut gradient = [0.0; 6];
        for (features, trend, target) in &training {
            let z = weights[0]
                + features
                    .iter()
                    .enumerate()
                    .map(|(index, value)| weights[index + 1] * value)
                    .sum::<f64>()
                + weights[5] * trend;
            let probability = 1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp());
            let error = probability - target;
            gradient[0] += error;
            for index in 0..4 {
                gradient[index + 1] += error * features[index];
            }
            gradient[5] += error * trend;
        }
        let scale = 0.04 / training.len() as f64;
        for index in 0..6 {
            weights[index] -= scale * gradient[index];
        }
    }
    Some(weights)
}

fn trend_logistic_up_probability(weights: &[f64; 6], features: &[f64; 4], trend: f64) -> f64 {
    let z = weights[0]
        + features
            .iter()
            .enumerate()
            .map(|(index, value)| weights[index + 1] * value)
            .sum::<f64>()
        + weights[5] * trend.clamp(-4.0, 4.0);
    1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp())
}

fn research_feature_vector(features: &SignalResearchFeatures) -> Option<[f64; 8]> {
    let values = [
        features
            .spread_bps
            .map(|value| (value / 10.0).clamp(0.0, 4.0)),
        features
            .microprice_offset_bps
            .map(|value| (value / 10.0).clamp(-4.0, 4.0)),
        features
            .depth_imbalance_10bps
            .map(|value| value.clamp(-1.0, 1.0)),
        features.trade_flow_5s.map(|value| value.clamp(-1.0, 1.0)),
        features.trade_flow_15s.map(|value| value.clamp(-1.0, 1.0)),
        features
            .trade_flow_acceleration
            .map(|value| value.clamp(-1.0, 1.0)),
        features
            .btc_return_1m_percent
            .map(|value| (value / 0.25).clamp(-4.0, 4.0)),
        features
            .eth_return_1m_percent
            .map(|value| (value / 0.25).clamp(-4.0, 4.0)),
    ];
    values
        .iter()
        .all(Option::is_some)
        .then(|| values.map(Option::unwrap))
}

fn train_research_logistic(
    samples: &[SignalOutcome],
    features: &HashMap<(String, u64), [f64; 8]>,
) -> Option<[f64; 9]> {
    let training = samples
        .iter()
        .filter_map(|sample| {
            Some((
                *features.get(&(sample.symbol.clone(), sample.signal_timestamp))?,
                (sample.return_percent > 0.0) as u8 as f64,
            ))
        })
        .collect::<Vec<_>>();
    if training.len() < WALK_FORWARD_MIN_TRAINING_SAMPLES {
        return None;
    }
    let mut weights = [0.0; 9];
    for _ in 0..250 {
        let mut gradient = [0.0; 9];
        for (features, target) in &training {
            let z = weights[0]
                + features
                    .iter()
                    .enumerate()
                    .map(|(index, value)| weights[index + 1] * value)
                    .sum::<f64>();
            let probability = 1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp());
            let error = probability - target;
            gradient[0] += error;
            for index in 0..8 {
                gradient[index + 1] += error * features[index];
            }
        }
        let scale = 0.04 / training.len() as f64;
        for index in 0..9 {
            weights[index] -= scale * gradient[index];
        }
    }
    Some(weights)
}

fn research_logistic_up_probability(weights: &[f64; 9], features: &[f64; 8]) -> f64 {
    let z = weights[0]
        + features
            .iter()
            .enumerate()
            .map(|(index, value)| weights[index + 1] * value)
            .sum::<f64>();
    1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp())
}

#[cfg(test)]
mod logistic_model_tests {
    use super::*;

    fn outcome(index: usize, target_up: bool) -> SignalOutcome {
        SignalOutcome {
            record_type: "outcome".into(),
            symbol: "BTC-USDT".into(),
            signal_timestamp: index as u64,
            evaluated_timestamp: index as u64 + 1,
            horizon_seconds: 60,
            return_percent: if target_up { 1.0 } else { -1.0 },
            direction_match: target_up,
            direction: if target_up { "bullish" } else { "bearish" }.into(),
            strength: 0.5,
            net_return_percent: Some(0.5),
            execution_cost_percent: Some(0.1),
            trend_5m_volatility_units: Some(if target_up { 5.0 } else { -5.0 }),
            trend_5m_direction_match: Some(target_up),
            trend_5m_net_return_percent: Some(0.5),
            features: Some([if target_up { 1.0 } else { -1.0 }, 0.25, 0.5, -0.25]),
        }
    }

    #[test]
    fn feature_vectors_require_complete_inputs_and_apply_bounds() {
        let components = SignalComponents {
            trend_1m_percent: Some(2.0),
            momentum_10s_percent: Some(-1.0),
            trade_flow_imbalance: Some(0.4),
            book_imbalance: Some(-0.2),
            ..Default::default()
        };
        assert_eq!(feature_vector(&components), Some([4.0, -4.0, 0.4, -0.2]));
        assert_eq!(feature_vector(&SignalComponents::default()), None);
        let research = SignalResearchFeatures {
            symbol: "BTC-USDT".into(), timestamp: 1,
            spread_bps: Some(50.0), microprice_offset_bps: Some(-50.0),
            bid_notional_10bps: None, ask_notional_10bps: None,
            depth_imbalance_10bps: Some(2.0), depth_imbalance_change_1s: None,
            depth_imbalance_change_5s: None, trade_flow_5s: Some(-2.0),
            trade_flow_15s: Some(0.5), trade_flow_60s: None,
            trade_flow_acceleration: Some(0.25), signed_trade_notional_5s: None,
            signed_trade_notional_15s: None, trade_count_5s: 0, trade_count_60s: 0,
            average_trade_notional_5s: None, price_response_bps_per_1000usd_5s: None,
            btc_return_1m_percent: Some(2.0), btc_return_5m_percent: None,
            eth_return_1m_percent: Some(-2.0), eth_return_5m_percent: None,
        };
        assert_eq!(research_feature_vector(&research), Some([4.0, -4.0, 1.0, -1.0, 0.5, 0.25, 4.0, -4.0]));
    }

    #[test]
    fn logistic_models_train_only_with_minimum_samples_and_return_probabilities() {
        let samples = (0..WALK_FORWARD_MIN_TRAINING_SAMPLES)
            .map(|index| outcome(index, index % 2 == 0)).collect::<Vec<_>>();
        assert!(train_logistic(&samples[..WALK_FORWARD_MIN_TRAINING_SAMPLES - 1]).is_none());
        let weights = train_logistic(&samples).unwrap();
        assert!((0.0..=1.0).contains(&logistic_up_probability(&weights, &[0.5; 4])));
        let trend_weights = train_trend_logistic(&samples).unwrap();
        assert!((0.0..=1.0).contains(&trend_logistic_up_probability(&trend_weights, &[0.5; 4], 9.0)));
        let research = samples.iter().map(|sample| ((sample.symbol.clone(), sample.signal_timestamp), [0.1; 8])).collect();
        assert!(train_research_logistic(&samples[..WALK_FORWARD_MIN_TRAINING_SAMPLES - 1], &research).is_none());
        let research_weights = train_research_logistic(&samples, &research).unwrap();
        assert!((0.0..=1.0).contains(&research_logistic_up_probability(&research_weights, &[0.1; 8])));
    }
}
