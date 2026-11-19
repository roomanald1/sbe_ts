use super::*;

const MIN_TRAIN_SAMPLES: usize = 20;
const MIN_VALIDATION_SAMPLES: usize = 10;
const MIN_FOLDS: usize = 2;
const MAX_FOLDS: usize = 4;
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
    // Reserve the newest observations for a final confirmation holdout. Candidate
    // selection and rolling validation never inspect this tail.
    let holdout_len = (samples.len() / 5).max(MIN_VALIDATION_SAMPLES);
    let holdout_start = samples.len().saturating_sub(holdout_len);
    let (development, holdout) = samples.split_at(holdout_start);
    if development.len() < MIN_TRAIN_SAMPLES + MIN_VALIDATION_SAMPLES
        || holdout.len() < MIN_VALIDATION_SAMPLES
    {
        return None;
    }

    // Build expanding train windows followed by disjoint validation windows.
    // The final development window ends exactly where the untouched holdout begins.
    let available_folds =
        development.len().saturating_sub(2 * MIN_TRAIN_SAMPLES) / MIN_VALIDATION_SAMPLES;
    let fold_count = available_folds.min(MAX_FOLDS);
    if fold_count < MIN_FOLDS {
        return None;
    }
    let validation_len = (development.len().saturating_sub(2 * MIN_TRAIN_SAMPLES) / fold_count)
        .max(MIN_VALIDATION_SAMPLES);
    let first_validation = development
        .len()
        .saturating_sub(fold_count * validation_len);
    let folds = (0..fold_count)
        .map(|fold| {
            let start = first_validation + fold * validation_len;
            let end = (start + validation_len).min(development.len());
            (&development[..start], &development[start..end])
        })
        .collect::<Vec<_>>();
    if folds.iter().any(|(train, validation)| {
        train.len() < MIN_TRAIN_SAMPLES || validation.len() < MIN_VALIDATION_SAMPLES
    }) {
        return None;
    }

    // Historical observations only support stricter thresholds than the
    // thresholds that generated those samples. Relaxation is assessed through
    // paired parameter probes so missed entries are not inferred from trades.
    let training_samples = folds
        .iter()
        .flat_map(|(train, _)| train.iter())
        .collect::<Vec<_>>();
    let supported_boundary = if spec.key == "rsi_entry_max" {
        training_samples
            .iter()
            .map(|sample| sample.threshold)
            .fold(base, f64::min)
    } else {
        training_samples
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
        let training_scores = folds
            .iter()
            .filter_map(|(train, _)| threshold_return_stats(train, &spec.key, candidate))
            .filter(|(count, _, _)| *count >= MIN_TRAIN_SAMPLES)
            .map(|(_, mean, _)| mean)
            .collect::<Vec<_>>();
        if training_scores.len() == folds.len() {
            let score = training_scores.iter().sum::<f64>() / training_scores.len() as f64;
            if best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((candidate, score));
            }
        }
    }
    let (candidate, _) = best?;
    // Score each candidate only after it has been fixed by training. Every
    // rolling window and the later holdout must beat the active baseline.
    let validations_pass = folds
        .iter()
        .map(|(_, validation)| *validation)
        .chain(std::iter::once(holdout))
        .all(|validation| {
            let Some((candidate_count, candidate_mean, candidate_bound)) =
                threshold_return_stats(validation, &spec.key, candidate)
            else {
                return false;
            };
            let Some((baseline_count, _, baseline_bound)) =
                threshold_return_stats(validation, &spec.key, base)
            else {
                return false;
            };
            candidate_count >= MIN_VALIDATION_SAMPLES
                && baseline_count >= MIN_VALIDATION_SAMPLES
                && candidate_mean > 0.0
                && candidate_bound >= baseline_bound + MIN_VALIDATION_GAIN_PERCENT
        });
    validations_pass.then_some(candidate)
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
        let samples = (0..200_u64)
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

    #[test]
    fn walk_forward_rejects_a_candidate_that_fails_the_final_holdout() {
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
        let samples = (0..200_u64)
            .map(|index| {
                // The final 20% is a newer, untouched holdout with only losing
                // outcomes for the selected stricter threshold.
                let (feature_value, net_return_percent) = if index >= 160 {
                    (0.025, -0.2)
                } else if index % 2 == 0 {
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
                    feature_value,
                    net_return_percent,
                }
            })
            .collect::<Vec<_>>();

        assert_eq!(
            walk_forward_threshold(&spec, 0.02, "live-trade-flow", &samples),
            None
        );
    }
}
