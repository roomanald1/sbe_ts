use super::*;

pub(super) async fn get_profiles(
    State(service): State<ServiceState>,
) -> Json<Vec<StrategyProfile>> {
    let state = service.state.lock().await;
    Json(state.active_profiles.values().cloned().collect())
}

pub(super) async fn get_decision_snapshot(
    State(service): State<ServiceState>,
) -> Json<LearnedProfileSnapshot> {
    let state = service.state.lock().await;
    let profiles = state.active_profiles.values().cloned().collect::<Vec<_>>();
    let mut totals = BTreeMap::<String, (usize, usize)>::new();
    for profile in &profiles {
        let entry = totals.entry(profile.strategy.clone()).or_default();
        entry.0 = entry.0.saturating_add(profile.learning_samples);
        entry.1 = entry.1.saturating_add(profile.positive_samples);
    }
    let strategy_size_multipliers = totals
        .into_iter()
        .map(|(strategy, (samples, positive))| {
            let multiplier = if samples == 0 || positive > samples {
                1.0
            } else {
                let n = samples as f64;
                let posterior = (positive as f64 + 1.0) / (n + 2.0);
                let confidence = n / (n + 2.0);
                let edge = ((posterior - 0.5) * 2.0 * confidence).clamp(-1.0, 1.0);
                let target = if edge < 0.0 {
                    1.0 + edge * 0.30
                } else {
                    1.0 + edge * 1.50
                };
                target.clamp(0.70, 2.50)
            };
            (strategy, multiplier)
        })
        .collect();
    Json(LearnedProfileSnapshot {
        profiles,
        strategy_size_multipliers,
        context_learning: state.context_learning.clone(),
        execution_learning: state.execution_learning.clone(),
        scout_parameter_variants: scout_parameter_variant_snapshots(&state),
    })
}

fn scout_parameter_variant_snapshots(
    state: &LearningState,
) -> Vec<strategies::ScoutParameterVariant> {
    type CandidateKey = (String, String, String, String);
    let mut outcomes = BTreeMap::<CandidateKey, Vec<f64>>::new();
    let mut baseline_directions = BTreeMap::<CandidateKey, [usize; 4]>::new();
    let mut baseline_deltas = BTreeMap::<CandidateKey, (f64, usize)>::new();
    for variant in state.variants.values() {
        for outcome in &variant.outcomes {
            // Multi-parameter comparisons are promoted as a tested group, so
            // don't present their returns as isolated single-parameter tests.
            if outcome.tested_parameter_keys.len() != 1 {
                continue;
            }
            let parameter = &outcome.tested_parameter_keys[0];
            let Some(value) = variant.profile.parameters.get(parameter) else {
                continue;
            };
            let key = (
                variant.profile.symbol.clone(),
                variant.profile.strategy.clone(),
                parameter.clone(),
                format!("{value:.8}"),
            );
            outcomes
                .entry(key.clone())
                .or_default()
                .push(outcome.net_return_percent);
            let direction_counts = baseline_directions.entry(key.clone()).or_default();
            let baseline = outcome.baseline_parameters.get(parameter).copied();
            let step = variant
                .specs
                .iter()
                .find(|spec| spec.key == *parameter)
                .map_or(1e-8, |spec| spec.step * 1e-6);
            match baseline {
                Some(baseline) if *value > baseline + step => direction_counts[0] += 1,
                Some(baseline) if *value < baseline - step => direction_counts[1] += 1,
                Some(_) => direction_counts[2] += 1,
                None => direction_counts[3] += 1,
            }
            if let Some(baseline) = baseline {
                let delta = baseline_deltas.entry(key.clone()).or_default();
                delta.0 += *value - baseline;
                delta.1 += 1;
            }
        }
    }

    let mut snapshots = Vec::new();
    for (key, returns) in outcomes {
        let (symbol, strategy, parameter, value_string) = key.clone();
        let Ok(candidate_value) = value_string.parse::<f64>() else {
            continue;
        };
        if returns.is_empty() {
            continue;
        }
        let sample_count = returns.len();
        let direction_counts = baseline_directions.get(&key).copied().unwrap_or_default();
        let (paired_delta_sum, paired_delta_samples) =
            baseline_deltas.get(&key).copied().unwrap_or_default();
        let paired_delta_mean =
            (paired_delta_samples > 0).then_some(paired_delta_sum / paired_delta_samples as f64);
        let minimum_samples = state.minimum_live_scout_parameter_samples;
        let mean = returns.iter().sum::<f64>() / sample_count as f64;
        let (lower_bound, upper_bound) = if sample_count >= minimum_samples && sample_count > 1 {
            let variance = returns
                .iter()
                .map(|value| (value - mean).powi(2))
                .sum::<f64>()
                / (sample_count - 1) as f64;
            let interval = one_sided_90_t_critical(sample_count).map(|critical| {
                let margin = critical * (variance / sample_count as f64).sqrt();
                (mean - margin, mean + margin)
            });
            interval.map_or((None, None), |(lower, upper)| (Some(lower), Some(upper)))
        } else {
            (None, None)
        };
        let positive_evidence = lower_bound.is_some_and(|bound| bound > 0.0);
        let negative_evidence = upper_bound.is_some_and(|bound| bound < 0.0);
        let active_value = state
            .active_profiles
            .get(&format!("{symbol}:{strategy}"))
            .and_then(|profile| profile.parameters.get(&parameter))
            .copied();
        snapshots.push(strategies::ScoutParameterVariant {
            symbol,
            strategy,
            parameter,
            candidate_value,
            sample_count,
            minimum_samples,
            average_net_improvement_percent: mean,
            lower_90_one_sided_improvement_percent: lower_bound,
            upper_90_one_sided_improvement_percent: upper_bound,
            paired_baseline_increase_samples: direction_counts[0],
            paired_baseline_decrease_samples: direction_counts[1],
            paired_baseline_same_samples: direction_counts[2],
            paired_baseline_unknown_samples: direction_counts[3],
            paired_baseline_delta_mean: paired_delta_mean,
            paired_baseline_delta_samples: paired_delta_samples,
            promoted: positive_evidence
                && active_value.is_some_and(|value| (value - candidate_value).abs() <= 1e-8),
            rejected: negative_evidence,
        });
    }
    snapshots.sort_by(|left, right| {
        (&left.symbol, &left.strategy, &left.parameter)
            .cmp(&(&right.symbol, &right.strategy, &right.parameter))
            .then_with(|| left.candidate_value.total_cmp(&right.candidate_value))
    });
    snapshots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scout_variant(symbol: &str, returns: &[f64]) -> RegisteredProfile {
        RegisteredProfile {
            profile: StrategyProfile {
                profile_id: format!("{symbol}-profile"),
                version: 0,
                symbol: symbol.into(),
                strategy: "example-strategy".into(),
                parameters: BTreeMap::from([("threshold".into(), 0.7)]),
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            },
            specs: vec![ParameterSpec {
                key: "threshold".into(),
                label: "Threshold".into(),
                default: 0.5,
                min: 0.0,
                max: 1.0,
                step: 0.1,
                unit: "".into(),
                learning_direction: 1.0,
            }],
            samples: returns.len(),
            positive: 0,
            realized_pnl: 0.0,
            outcomes: returns
                .iter()
                .enumerate()
                .map(|(index, value)| LearningOutcome {
                    observed_at_ms: index as u64,
                    net_return_percent: *value,
                    tested_parameter_keys: vec!["threshold".into()],
                    baseline_parameters: BTreeMap::from([("threshold".into(), 0.5)]),
                })
                .collect(),
        }
    }

    #[test]
    fn negative_upper_bound_marks_a_well_sampled_candidate_to_avoid() {
        let returns = vec![-0.2; 9];
        let sample_count = returns.len();
        let mean = returns.iter().sum::<f64>() / sample_count as f64;
        let variance = returns
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (sample_count - 1) as f64;
        let margin = one_sided_90_t_critical(sample_count).unwrap()
            * (variance / sample_count as f64).sqrt();
        assert!(mean + margin < 0.0);
    }

    #[test]
    fn scout_parameter_evidence_is_aggregated_per_symbol() {
        let mut state = LearningState::default();
        state.minimum_live_scout_parameter_samples = 2;
        state
            .variants
            .insert("a".into(), scout_variant("A", &[0.2, 0.4]));
        state
            .variants
            .insert("b".into(), scout_variant("B", &[-0.3]));

        let snapshots = scout_parameter_variant_snapshots(&state);
        let a = snapshots.iter().find(|row| row.symbol == "A").unwrap();
        let b = snapshots.iter().find(|row| row.symbol == "B").unwrap();

        assert_eq!(a.sample_count, 2);
        assert!((a.average_net_improvement_percent - 0.3).abs() < 1e-10);
        assert_eq!(a.paired_baseline_increase_samples, 2);
        assert_eq!(b.sample_count, 1);
        assert_eq!(b.average_net_improvement_percent, -0.3);
        assert_eq!(b.paired_baseline_increase_samples, 1);
        assert_eq!(b.lower_90_one_sided_improvement_percent, None);
    }
}

pub(super) async fn get_summary(State(service): State<ServiceState>) -> Json<Vec<StrategySummary>> {
    let state = service.state.lock().await;
    let cutoff = now_ms().saturating_sub(7 * 24 * 60 * 60 * 1000);
    let mut summaries: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for profile in state.variants.values() {
        summaries
            .entry(profile.profile.strategy.clone())
            .or_default()
            .extend(
                profile
                    .outcomes
                    .iter()
                    .filter(|outcome| outcome.observed_at_ms >= cutoff)
                    .map(|outcome| outcome.net_return_percent),
            );
    }
    Json(
        summaries
            .into_iter()
            .map(|(strategy, returns)| {
                let positive = returns.iter().filter(|value| **value > 0.0).count();
                let count = returns.len();
                StrategySummary {
                    strategy,
                    learning_samples: count,
                    positive_rate: (count > 0).then_some(positive as f64 / count as f64),
                    average_net_return_percent: (count > 0)
                        .then_some(returns.iter().sum::<f64>() / count as f64),
                }
            })
            .collect(),
    )
}

pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub(super) fn event_observed_at(event: &FeedbackEvent) -> u64 {
    match event {
        FeedbackEvent::ScoutOutcome { observed_at_ms, .. }
        | FeedbackEvent::TradeOutcome { observed_at_ms, .. } => *observed_at_ms,
    }
}

pub(super) async fn register_profile(
    State(service): State<ServiceState>,
    Json(request): Json<StrategyProfileRegistration>,
) -> Result<StatusCode, (StatusCode, String)> {
    if request.profile.profile_id.is_empty()
        || request
            .profile
            .parameters
            .values()
            .any(|value| !value.is_finite())
        || request.specs.iter().any(|spec| !valid_spec(spec))
        || request
            .specs
            .iter()
            .any(|spec| !request.profile.parameters.contains_key(&spec.key))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid strategy profile registration".into(),
        ));
    }
    let profile_key = key(&request.profile.symbol, &request.profile.strategy);
    let mut state = service.state.lock().await;
    let id = variant_key(&request.profile);
    state.variants.entry(id).or_insert(RegisteredProfile {
        profile: request.profile.clone(),
        specs: request.specs,
        samples: request.profile.learning_samples,
        positive: request.profile.positive_samples,
        realized_pnl: request.profile.realized_pnl,
        outcomes: Vec::new(),
    });
    let current = state
        .active_profiles
        .entry(profile_key)
        .or_insert(request.profile.clone());
    if request.profile.version > current.version {
        *current = request.profile;
    }
    persist(&service.path, &state)?;
    Ok(StatusCode::ACCEPTED)
}
