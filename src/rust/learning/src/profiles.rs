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
    let cutoff = now_ms().saturating_sub(7 * 24 * 60 * 60 * 1_000);
    let mut totals = BTreeMap::<String, (usize, usize, Vec<f64>)>::new();
    let mut recent_profile_context_returns = BTreeMap::<String, Vec<(u64, f64)>>::new();
    for profile in &profiles {
        let entry = totals.entry(profile.strategy.clone()).or_default();
        entry.0 = entry.0.saturating_add(profile.learning_samples);
        entry.1 = entry.1.saturating_add(profile.positive_samples);
    }
    for variant in state.variants.values() {
        let returns = &mut totals
            .entry(variant.profile.strategy.clone())
            .or_default()
            .2;
        returns.extend(variant.outcomes.iter().filter_map(|outcome| {
            (outcome.source == LearningOutcomeSource::Trade
                && outcome.observed_at_ms >= cutoff
                && outcome.net_return_percent.is_finite())
            .then_some(outcome.net_return_percent)
        }));
        for outcome in &variant.outcomes {
            if outcome.source != LearningOutcomeSource::Trade
                || outcome.observed_at_ms < cutoff
                || !outcome.net_return_percent.is_finite()
            {
                continue;
            }
            let Some(context) = outcome.learning_context.as_deref() else {
                continue;
            };
            recent_profile_context_returns
                .entry(format!("{}|{}", variant.profile.profile_id, context))
                .or_default()
                .push((outcome.observed_at_ms, outcome.net_return_percent));
        }
    }
    for returns in recent_profile_context_returns.values_mut() {
        returns.sort_unstable_by_key(|sample| std::cmp::Reverse(sample.0));
        returns.truncate(5);
    }
    let contextual_stop_loss_percents = contextual_stop_loss_percents(&state, cutoff);
    let strategy_size_multipliers = totals
        .into_iter()
        .map(|(strategy, (samples, positive, returns))| {
            let multiplier = strategy_size_multiplier(samples, positive, &returns);
            (strategy, multiplier)
        })
        .collect();
    Json(LearnedProfileSnapshot {
        profiles,
        strategy_size_multipliers,
        recent_profile_context_returns,
        contextual_stop_loss_percents,
        context_learning: state.context_learning.clone(),
        execution_learning: state.execution_learning.clone(),
        scout_parameter_variants: scout_parameter_variant_snapshots(&state),
    })
}

const MIN_CONTEXTUAL_STOP_REPLAYS: usize = 5;

//TODO comment this fucntion
fn contextual_stop_loss_percents(state: &LearningState, cutoff: u64) -> BTreeMap<String, f64> {
    let mut candidates = BTreeMap::<String, BTreeMap<String, Vec<f64>>>::new();
    for observation in state.exit_timing_observations.values() {
        if observation["reason"].as_str() != Some("stop-loss")
            || observation["observed_at_ms"].as_u64().unwrap_or_default() < cutoff
        {
            continue;
        }
        let entry = &observation["entry_context"];
        let Some(profile_id) = entry["strategy_decision"]["learning_profile_id"].as_str() else {
            continue;
        };
        let Some(context) =
            market_context_from_learning_context(observation["learning_context"].as_str())
        else {
            continue;
        };
        let Some(current_stop) = observation["effective_stop_loss_percent"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0)
        else {
            continue;
        };
        let Some(entry_price) = entry["average_price"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0)
        else {
            continue;
        };
        let Some(current_return) = observation["exit_return_percent"]
            .as_f64()
            .filter(|v| v.is_finite())
        else {
            continue;
        };
        let Some(strategy) = observation["strategy"]
            .as_str()
            .and_then(strategies::Strategy::from_name)
        else {
            continue;
        };
        let specs = strategy.parameter_specs();
        let Some(spec) = specs
            .iter()
            .find(|spec| spec.key.as_str() == "stop_loss_percent")
        else {
            continue;
        };
        let candidate_stop = (current_stop + spec.step).min(spec.max);
        if candidate_stop <= current_stop + 1e-9 {
            continue;
        }
        let long = observation["side"].as_str() == Some("sell");
        let mut path = observation["pre_exit_price_path"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        path.extend(
            observation["price_path"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        );
        path.sort_unstable_by_key(|sample| sample["timestamp"].as_u64().unwrap_or_default());
        if path.len() < 3 {
            continue;
        }
        let threshold = if long {
            entry_price * (1.0 - candidate_stop / 100.0)
        } else {
            entry_price * (1.0 + candidate_stop / 100.0)
        };
        let exit = path
            .iter()
            .find_map(|sample| {
                let executable = if long {
                    sample["bid"].as_f64()?
                } else {
                    sample["ask"].as_f64()?
                };
                ((long && executable <= threshold) || (!long && executable >= threshold))
                    .then_some(executable)
            })
            .or_else(|| {
                path.last().and_then(|sample| {
                    if long {
                        sample["bid"].as_f64()
                    } else {
                        sample["ask"].as_f64()
                    }
                })
            });
        let Some(exit) = exit.filter(|v| v.is_finite() && *v > 0.0) else {
            continue;
        };
        let gross = if long {
            exit / entry_price - 1.0
        } else {
            entry_price / exit - 1.0
        } * 100.0;
        let quantity = entry["quantity"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(0.0);
        let fees = entry["fees"].as_f64().unwrap_or(0.0).abs()
            + observation["exit_fees"].as_f64().unwrap_or(0.0).abs();
        let replay_return = gross
            - if quantity > 0.0 {
                fees / (entry_price * quantity) * 100.0
            } else {
                0.0
            };
        if !replay_return.is_finite() {
            continue;
        }
        candidates
            .entry(format!("{profile_id}|{context}"))
            .or_default()
            .entry(format!("{candidate_stop:.8}"))
            .or_default()
            .push(replay_return - current_return);
    }
    candidates
        .into_iter()
        .filter_map(|(key, values)| {
            values
                .into_iter()
                .filter_map(|(candidate, improvements)| {
                    if improvements.len() < MIN_CONTEXTUAL_STOP_REPLAYS {
                        return None;
                    }
                    candidate.parse::<f64>().ok().map(|candidate| {
                        let mean = improvements.iter().sum::<f64>() / improvements.len() as f64;
                        let variance = improvements
                            .iter()
                            .map(|value| (value - mean).powi(2))
                            .sum::<f64>()
                            / improvements.len() as f64;
                        let lower =
                            mean - 1.645 * (variance.sqrt() / (improvements.len() as f64).sqrt());
                        (candidate, lower)
                    })
                })
                .filter(|(_, lower)| *lower > 0.0)
                .max_by(|(_, left), (_, right)| left.total_cmp(right))
                .map(|(candidate, _)| (key, candidate))
        })
        .collect()
}

fn market_context_from_learning_context(value: Option<&str>) -> Option<String> {
    let value = value?;
    let context = value.split(":P").next().unwrap_or(value);
    let mut parts = context.split(':');
    let (Some(side), Some(regime), Some(symbol), Some(strategy)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    (matches!(side, "long" | "short")
        && matches!(regime, "range" | "trend" | "volatile" | "mixed")
        && !symbol.is_empty()
        && !strategy.is_empty()
        && parts.next().is_none())
    .then(|| context.to_owned())
}

const MIN_TRADES_FOR_NEGATIVE_EDGE_PAUSE: usize = 20;

fn strategy_size_multiplier(samples: usize, positive: usize, net_returns: &[f64]) -> f64 {
    if net_returns.len() >= MIN_TRADES_FOR_NEGATIVE_EDGE_PAUSE
        && net_returns.iter().sum::<f64>() / net_returns.len() as f64 <= 0.0
    {
        return 0.0;
    }
    if samples == 0 || positive > samples {
        return 1.0;
    }
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
}

pub(super) fn rebuild_trade_profile_metrics(state: &mut LearningState, now: u64) {
    let cutoff = now.saturating_sub(7 * 24 * 60 * 60 * 1_000);
    let mut totals = BTreeMap::<String, (usize, usize, f64)>::new();
    for variant in state.variants.values_mut() {
        let mut samples = 0;
        let mut positive = 0;
        let mut realized_pnl = 0.0;
        for outcome in variant.outcomes.iter().filter(|outcome| {
            outcome.source == LearningOutcomeSource::Trade && outcome.observed_at_ms >= cutoff
        }) {
            samples += 1;
            positive += usize::from(outcome.net_return_percent > 0.0);
            realized_pnl += outcome.realized_pnl_usd.unwrap_or_default();
        }
        variant.samples = samples;
        variant.positive = positive;
        variant.realized_pnl = realized_pnl;
        let total = totals
            .entry(format!(
                "{}:{}",
                variant.profile.symbol, variant.profile.strategy
            ))
            .or_default();
        total.0 += samples;
        total.1 += positive;
        total.2 += realized_pnl;
    }
    for (key, profile) in &mut state.active_profiles {
        let (samples, positive, realized_pnl) = totals.get(key).copied().unwrap_or_default();
        if profile.learning_samples != samples
            || profile.positive_samples != positive
            || (profile.realized_pnl - realized_pnl).abs() > 1e-9
        {
            profile.learning_samples = samples;
            profile.positive_samples = positive;
            profile.realized_pnl = realized_pnl;
            profile.version = profile.version.saturating_add(1);
        }
    }
}

fn scout_parameter_variant_snapshots(
    state: &LearningState,
) -> Vec<strategies::ScoutParameterVariant> {
    type CandidateKey = (String, String, String, String);
    let mut outcomes = BTreeMap::<CandidateKey, BTreeMap<String, Vec<f64>>>::new();
    let mut baseline_directions = BTreeMap::<CandidateKey, [usize; 4]>::new();
    let mut baseline_deltas = BTreeMap::<CandidateKey, (f64, usize)>::new();
    let mut direction_groups = BTreeSet::<(CandidateKey, String)>::new();
    let mut delta_groups = BTreeSet::<(CandidateKey, String)>::new();
    for (profile_id, variant) in &state.variants {
        for (outcome_index, outcome) in variant.outcomes.iter().enumerate() {
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
                .entry(
                    outcome
                        .evidence_group_id
                        .clone()
                        .unwrap_or_else(|| format!("{profile_id}:{outcome_index}")),
                )
                .or_default()
                .push(outcome.net_return_percent);
            let group_id = outcome
                .evidence_group_id
                .clone()
                .unwrap_or_else(|| format!("{profile_id}:{outcome_index}"));
            let direction_counts = baseline_directions.entry(key.clone()).or_default();
            let baseline = outcome.baseline_parameters.get(parameter).copied();
            let step = variant
                .specs
                .iter()
                .find(|spec| spec.key == *parameter)
                .map_or(1e-8, |spec| spec.step * 1e-6);
            if direction_groups.insert((key.clone(), group_id.clone())) {
                match baseline {
                    Some(baseline) if *value > baseline + step => direction_counts[0] += 1,
                    Some(baseline) if *value < baseline - step => direction_counts[1] += 1,
                    Some(_) => direction_counts[2] += 1,
                    None => direction_counts[3] += 1,
                }
            }
            if let Some(baseline) =
                baseline.filter(|_| delta_groups.insert((key.clone(), group_id)))
            {
                let delta = baseline_deltas.entry(key.clone()).or_default();
                delta.0 += *value - baseline;
                delta.1 += 1;
            }
        }
    }

    let mut snapshots = Vec::new();
    for (key, grouped_observations) in outcomes {
        let (symbol, strategy, parameter, value_string) = key.clone();
        let Ok(candidate_value) = value_string.parse::<f64>() else {
            continue;
        };
        if grouped_observations.is_empty() {
            continue;
        }
        let returns = grouped_observations
            .values()
            .map(|observations| observations.iter().sum::<f64>() / observations.len() as f64)
            .collect::<Vec<_>>();
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

    #[test]
    fn sufficiently_sampled_negative_net_trades_pause_strategy_sizing() {
        let losing = vec![-0.01; MIN_TRADES_FOR_NEGATIVE_EDGE_PAUSE];
        assert_eq!(strategy_size_multiplier(200, 20, &losing), 0.0);
        assert!(strategy_size_multiplier(19, 2, &vec![-0.01; 19]) > 0.0);
        assert!(strategy_size_multiplier(200, 20, &vec![0.01; 20]) > 0.0);
        // A high win rate cannot hide negative average net returns.
        assert_eq!(strategy_size_multiplier(200, 180, &[-0.02; 20]), 0.0);
    }

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
                    evidence_group_id: None,
                    learning_context: None,
                    source: LearningOutcomeSource::Scout,
                    realized_pnl_usd: None,
                    paired_comparison: false,
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

    #[test]
    fn horizon_observations_share_one_parameter_evidence_sample() {
        let mut state = LearningState::default();
        let mut variant = scout_variant("A", &[0.2, 0.4, 0.6, 0.8]);
        for outcome in &mut variant.outcomes {
            outcome.evidence_group_id = Some("paired-scout:comparison".into());
        }
        state.variants.insert("candidate".into(), variant);

        let snapshots = scout_parameter_variant_snapshots(&state);
        let evidence = snapshots.iter().find(|row| row.symbol == "A").unwrap();

        assert_eq!(evidence.sample_count, 1);
        assert!((evidence.average_net_improvement_percent - 0.5).abs() < 1e-10);
        assert_eq!(evidence.paired_baseline_increase_samples, 1);
    }

    #[tokio::test]
    async fn strategy_summary_separates_trade_returns_from_scout_evidence() {
        let mut state = LearningState::default();
        let mut trade = scout_variant("BTC-USDT", &[-0.02]);
        trade.profile.strategy = "example-strategy".into();
        trade.outcomes[0].source = LearningOutcomeSource::Trade;
        trade.outcomes[0].realized_pnl_usd = Some(-1.0);
        trade.outcomes[0].baseline_parameters.clear();
        trade.outcomes[0].tested_parameter_keys.clear();
        trade.outcomes[0].observed_at_ms = now_ms();
        let active_profile = trade.profile.clone();
        state.variants.insert("trade".into(), trade);
        state
            .active_profiles
            .insert("BTC-USDT:example-strategy".into(), active_profile);

        let mut previous_trade = scout_variant("BTC-USDT", &[-0.4]);
        previous_trade.profile.strategy = "example-strategy".into();
        previous_trade
            .profile
            .parameters
            .insert("threshold".into(), 0.6);
        previous_trade.outcomes[0].source = LearningOutcomeSource::Trade;
        previous_trade.outcomes[0].observed_at_ms = now_ms();
        state
            .variants
            .insert("previous-trade".into(), previous_trade);

        let mut scout = scout_variant("ETH-USDT", &[0.4]);
        scout.profile.strategy = "example-strategy".into();
        scout.outcomes[0].baseline_parameters.clear();
        scout.outcomes[0].observed_at_ms = now_ms();
        state.variants.insert("scout".into(), scout);

        let mut paired = scout_variant("SOL-USDT", &[0.1]);
        paired.profile.strategy = "example-strategy".into();
        paired.outcomes[0].observed_at_ms = now_ms();
        paired.outcomes[0].paired_comparison = true;
        state.variants.insert("paired".into(), paired);

        let mut paper_scout = scout_variant("DOGE-USDT", &[0.05]);
        paper_scout.profile.strategy = "example-strategy".into();
        paper_scout.outcomes[0].source = LearningOutcomeSource::PaperScoutTrade;
        paper_scout.outcomes[0].realized_pnl_usd = Some(0.25);
        paper_scout.outcomes[0].baseline_parameters.clear();
        paper_scout.outcomes[0].tested_parameter_keys.clear();
        paper_scout.outcomes[0].observed_at_ms = now_ms();
        state.variants.insert("paper-scout".into(), paper_scout);

        let mut legacy = scout_variant("XRP-USDT", &[99.0]);
        legacy.profile.strategy = "example-strategy".into();
        legacy.outcomes[0].source = LearningOutcomeSource::Legacy;
        legacy.outcomes[0].observed_at_ms = now_ms();
        state.variants.insert("legacy".into(), legacy);

        let service = ServiceState {
            state: Arc::new(tokio::sync::Mutex::new(state)),
            path: PathBuf::new(),
        };
        let Json(summaries) = get_summary(State(service)).await;
        let summary = &summaries[0];

        assert_eq!(summary.learning_samples, 2);
        assert_eq!(summary.positive_rate, Some(0.0));
        assert!((summary.average_net_return_percent.unwrap() + 0.21).abs() < 1e-10);
        assert_eq!(summary.active_profile_trade_samples, 1);
        assert_eq!(
            summary.active_profile_average_net_return_percent,
            Some(-0.02)
        );
        assert_eq!(summary.other_profile_trade_samples, 1);
        assert_eq!(summary.other_profile_average_net_return_percent, Some(-0.4));
        assert_eq!(summary.scout_outcome_samples, 2);
        assert_eq!(summary.scout_average_net_return_percent, Some(0.25));
        assert_eq!(summary.paper_scout_trade_samples, 1);
        assert_eq!(summary.paper_scout_average_net_return_percent, Some(0.05));
        assert_eq!(summary.paired_scout_samples, 1);
        assert_eq!(summary.average_paired_scout_lift_percent, Some(0.1));
        assert_eq!(summary.unclassified_samples, 1);
    }

    #[tokio::test]
    async fn decision_snapshot_keeps_recent_real_returns_scoped_to_profile_and_context() {
        let mut state = LearningState::default();
        let mut variant = scout_variant("BTC-USDT", &[-0.1, -0.2, -0.3, 99.0]);
        let context = "long:trend:BTC-USDT:example-strategy";
        for (index, outcome) in variant.outcomes.iter_mut().enumerate() {
            outcome.observed_at_ms = now_ms() + index as u64;
            outcome.learning_context = Some(context.into());
            if index < 3 {
                outcome.source = LearningOutcomeSource::Trade;
            }
        }
        let profile_id = variant.profile.profile_id.clone();
        state.variants.insert("variant".into(), variant);
        let service = ServiceState {
            state: Arc::new(tokio::sync::Mutex::new(state)),
            path: PathBuf::new(),
        };
        let Json(snapshot) = get_decision_snapshot(State(service)).await;
        let samples = &snapshot.recent_profile_context_returns[&format!("{profile_id}|{context}")];
        assert_eq!(
            samples.iter().map(|sample| sample.1).collect::<Vec<_>>(),
            vec![-0.3, -0.2, -0.1]
        );
    }

    #[test]
    fn startup_rebuilds_profile_stats_from_recent_closed_trades_only() {
        let mut state = LearningState::default();
        let mut trade = scout_variant("BTC-USDT", &[0.03]);
        trade.profile.strategy = "example-strategy".into();
        trade.outcomes[0].source = LearningOutcomeSource::Trade;
        trade.outcomes[0].realized_pnl_usd = Some(12.0);
        trade.outcomes[0].observed_at_ms = now_ms();
        trade.outcomes[0].baseline_parameters.clear();
        trade.outcomes[0].tested_parameter_keys.clear();
        state.variants.insert("trade".into(), trade);

        let mut scout = scout_variant("BTC-USDT", &[50.0]);
        scout.profile.strategy = "example-strategy".into();
        scout.outcomes[0].observed_at_ms = now_ms();
        state.variants.insert("scout".into(), scout);

        state.active_profiles.insert(
            "BTC-USDT:example-strategy".into(),
            StrategyProfile {
                profile_id: "active".into(),
                version: 4,
                symbol: "BTC-USDT".into(),
                strategy: "example-strategy".into(),
                parameters: BTreeMap::new(),
                learning_samples: 100,
                positive_samples: 80,
                realized_pnl: 500.0,
            },
        );

        rebuild_trade_profile_metrics(&mut state, now_ms());
        let active = &state.active_profiles["BTC-USDT:example-strategy"];
        assert_eq!(active.learning_samples, 1);
        assert_eq!(active.positive_samples, 1);
        assert_eq!(active.realized_pnl, 12.0);
        assert_eq!(active.version, 5);
    }
}

pub(super) async fn get_summary(State(service): State<ServiceState>) -> Json<Vec<StrategySummary>> {
    let state = service.state.lock().await;
    let cutoff = now_ms().saturating_sub(7 * 24 * 60 * 60 * 1000);
    #[derive(Default)]
    struct Returns {
        trades: Vec<f64>,
        active_profile_trades: Vec<f64>,
        other_profile_trades: Vec<f64>,
        paper_scout_trades: Vec<f64>,
        scouts: BTreeMap<String, Vec<f64>>,
        paired_scouts: BTreeMap<String, Vec<f64>>,
        unclassified: usize,
    }
    let mut summaries: BTreeMap<String, Returns> = BTreeMap::new();
    for (profile_id, profile) in &state.variants {
        let is_active_profile = state
            .active_profiles
            .get(&format!(
                "{}:{}",
                profile.profile.symbol, profile.profile.strategy
            ))
            .is_some_and(|active| active.parameters == profile.profile.parameters);
        let summary = summaries
            .entry(profile.profile.strategy.clone())
            .or_default();
        for (index, outcome) in profile.outcomes.iter().enumerate().filter(|(_, outcome)| {
            outcome.observed_at_ms >= cutoff && outcome.net_return_percent.is_finite()
        }) {
            match outcome.source {
                LearningOutcomeSource::Trade => {
                    summary.trades.push(outcome.net_return_percent);
                    if is_active_profile {
                        summary
                            .active_profile_trades
                            .push(outcome.net_return_percent);
                    } else {
                        summary
                            .other_profile_trades
                            .push(outcome.net_return_percent);
                    }
                }
                LearningOutcomeSource::PaperScoutTrade => {
                    summary.paper_scout_trades.push(outcome.net_return_percent)
                }
                LearningOutcomeSource::Scout => {
                    // Scout net is the overall Scout return, including paired
                    // comparisons. Keep a second bucket so the paired lift
                    // column can still be shown independently.
                    let group = outcome
                        .evidence_group_id
                        .clone()
                        .unwrap_or_else(|| format!("{profile_id}:{index}"));
                    summary
                        .scouts
                        .entry(group.clone())
                        .or_default()
                        .push(outcome.net_return_percent);
                    if outcome.paired_comparison {
                        summary
                            .paired_scouts
                            .entry(group)
                            .or_default()
                            .push(outcome.net_return_percent);
                    }
                }
                LearningOutcomeSource::Legacy => summary.unclassified += 1,
            }
        }
    }
    Json(
        summaries
            .into_iter()
            .map(|(strategy, returns)| {
                let summarize = |values: &[f64]| {
                    let count = values.len();
                    let positive = values.iter().filter(|value| **value > 0.0).count();
                    (
                        count,
                        (count > 0).then_some(positive as f64 / count as f64),
                        (count > 0).then_some(values.iter().sum::<f64>() / count as f64),
                    )
                };
                let (trade_count, trade_positive_rate, trade_average) = summarize(&returns.trades);
                let (active_trade_count, active_trade_positive_rate, active_trade_average) =
                    summarize(&returns.active_profile_trades);
                let (other_trade_count, other_trade_positive_rate, other_trade_average) =
                    summarize(&returns.other_profile_trades);
                let (paper_scout_count, paper_scout_positive_rate, paper_scout_average) =
                    summarize(&returns.paper_scout_trades);
                let grouped_returns = |groups: &BTreeMap<String, Vec<f64>>| {
                    groups
                        .values()
                        .map(|values| values.iter().sum::<f64>() / values.len() as f64)
                        .collect::<Vec<_>>()
                };
                let scouts = grouped_returns(&returns.scouts);
                let paired_scouts = grouped_returns(&returns.paired_scouts);
                let (scout_count, scout_positive_rate, scout_average) = summarize(&scouts);
                let (paired_count, paired_positive_rate, paired_average) =
                    summarize(&paired_scouts);
                StrategySummary {
                    strategy,
                    learning_samples: trade_count,
                    positive_rate: trade_positive_rate,
                    average_net_return_percent: trade_average,
                    scout_outcome_samples: scout_count,
                    scout_positive_rate,
                    scout_average_net_return_percent: scout_average,
                    active_profile_trade_samples: active_trade_count,
                    active_profile_trade_positive_rate: active_trade_positive_rate,
                    active_profile_average_net_return_percent: active_trade_average,
                    other_profile_trade_samples: other_trade_count,
                    other_profile_trade_positive_rate: other_trade_positive_rate,
                    other_profile_average_net_return_percent: other_trade_average,
                    paper_scout_trade_samples: paper_scout_count,
                    paper_scout_positive_rate,
                    paper_scout_average_net_return_percent: paper_scout_average,
                    paired_scout_samples: paired_count,
                    paired_scout_positive_rate: paired_positive_rate,
                    average_paired_scout_lift_percent: paired_average,
                    unclassified_samples: returns.unclassified,
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
