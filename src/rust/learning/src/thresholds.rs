use super::*;

const MIN_TRAIN_SAMPLES: usize = 20;
const MIN_VALIDATION_SAMPLES: usize = 10;
const MAX_VALIDATION_SAMPLES: usize = 64;
const MIN_VALIDATION_GAIN_PERCENT: f64 = 0.05;

pub(super) fn walk_forward_threshold(
    spec: &ParameterSpec,
    base: f64,
    strategy: &str,
    history: &[ThresholdTrainingSample],
) -> Option<f64> {
    if history.len() < MIN_TRAIN_SAMPLES + MIN_VALIDATION_SAMPLES {
        return None;
    }
    let mut samples = history
        .iter()
        .filter(|sample| {
            sample.strategy == strategy
                && sample.parameter_key == spec.key
                && sample.threshold.is_finite()
                && sample.feature_value.is_finite()
                && sample.net_return_percent.is_finite()
        })
        .cloned()
        .collect::<Vec<_>>();
    samples.sort_by_key(|sample| sample.observed_at_ms);
    if samples.len() < MIN_TRAIN_SAMPLES + MIN_VALIDATION_SAMPLES {
        return None;
    }
    if samples.len() > MIN_VALIDATION_SAMPLES + MAX_VALIDATION_SAMPLES {
        samples.drain(..samples.len() - MIN_VALIDATION_SAMPLES - MAX_VALIDATION_SAMPLES);
    }
    let split_timestamp = samples[samples.len() * 2 / 3].observed_at_ms;
    let split = samples.partition_point(|sample| sample.observed_at_ms < split_timestamp);
    let (train, validation) = samples.split_at(split);
    if train.len() < MIN_TRAIN_SAMPLES || validation.len() < MIN_VALIDATION_SAMPLES {
        return None;
    }

    // Historical observations only support stricter thresholds than the
    // thresholds that generated those samples. Relaxation is assessed through
    // paired parameter probes so missed entries are not inferred from trades.
    let supported_boundary = if spec.key == "rsi_entry_max" {
        train
            .iter()
            .map(|sample| sample.threshold)
            .fold(base, f64::min)
    } else {
        train
            .iter()
            .map(|sample| sample.threshold)
            .fold(base, f64::max)
    };
    let step_count = ((spec.max - spec.min) / spec.step).round() as usize;
    let mut best: Option<(f64, f64)> = None;
    for index in 0..=step_count {
        let candidate = (spec.min + index as f64 * spec.step).clamp(spec.min, spec.max);
        let tighter_than_observed = if spec.key == "rsi_entry_max" {
            candidate <= supported_boundary
        } else {
            candidate >= supported_boundary
        };
        if !tighter_than_observed {
            continue;
        }
        let Some((count, _, lower_bound)) = threshold_return_stats(train, &spec.key, candidate)
        else {
            continue;
        };
        if count >= MIN_TRAIN_SAMPLES && best.is_none_or(|(_, best_bound)| lower_bound > best_bound)
        {
            best = Some((candidate, lower_bound));
        }
    }
    let (candidate, _) = best?;
    let (candidate_count, candidate_mean, candidate_bound) =
        threshold_return_stats(validation, &spec.key, candidate)?;
    let (baseline_count, _, baseline_bound) = threshold_return_stats(validation, &spec.key, base)?;
    (candidate_count >= MIN_VALIDATION_SAMPLES
        && baseline_count >= MIN_VALIDATION_SAMPLES
        && candidate_mean > 0.0
        && candidate_bound >= baseline_bound + MIN_VALIDATION_GAIN_PERCENT)
        .then_some(candidate)
}

fn threshold_return_stats(
    samples: &[ThresholdTrainingSample],
    parameter_key: &str,
    threshold: f64,
) -> Option<(usize, f64, f64)> {
    let outcomes = samples
        .iter()
        .filter_map(|sample| {
            let tighter_than_sample = if parameter_key == "rsi_entry_max" {
                threshold <= sample.threshold
            } else {
                threshold >= sample.threshold
            };
            let feature_passes = if parameter_key == "rsi_entry_max" {
                sample.feature_value <= threshold
            } else if parameter_key == "minimum_momentum" {
                sample.feature_value > threshold
            } else {
                sample.feature_value >= threshold
            };
            (tighter_than_sample && feature_passes).then_some(sample.net_return_percent)
        })
        .collect::<Vec<_>>();
    if outcomes.is_empty() {
        return None;
    }
    let count = outcomes.len();
    let mean = outcomes.iter().sum::<f64>() / count as f64;
    if count < 2 {
        return Some((count, mean, f64::NEG_INFINITY));
    }
    let variance = outcomes
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (count - 1) as f64;
    Some((count, mean, mean - 1.96 * (variance / count as f64).sqrt()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_forward_promotes_a_stricter_holdout_validated_threshold() {
        let spec = ParameterSpec {
            key: "minimum_flow_imbalance".into(),
            label: "Minimum flow".into(),
            default: 0.02,
            min: 0.02,
            max: 0.8,
            step: 0.01,
            unit: "ratio".into(),
            learning_direction: -1.0,
        };
        let samples = (0..70_u64)
            .map(|index| {
                let (feature, return_percent) = if index % 2 == 0 {
                    (0.025, -0.2)
                } else {
                    (0.075, 1.0)
                };
                ThresholdTrainingSample {
                    symbol: "BTC-USDT".into(),
                    strategy: "live-trade-flow".into(),
                    parameter_key: spec.key.clone(),
                    observed_at_ms: index,
                    threshold: 0.02,
                    feature_value: feature,
                    net_return_percent: return_percent,
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            walk_forward_threshold(&spec, 0.02, "live-trade-flow", &samples),
            Some(0.03)
        );
    }
}
