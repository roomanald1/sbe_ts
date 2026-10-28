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
