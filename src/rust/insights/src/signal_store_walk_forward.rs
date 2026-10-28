impl SignalStore {
    async fn compute_walk_forward(&self) -> Result<Vec<SignalWalkForward>> {
        let outcomes = self.read_outcomes().await?;
        let evaluations = self
            .read_recent_records_with_corrupt_rows(2, 100_000, true)
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Evaluation(evaluation) => Some(evaluation),
                _ => None,
            })
            .map(|evaluation| {
                (
                    (
                        evaluation.symbol.clone(),
                        evaluation.signal_timestamp,
                        evaluation.horizon_seconds,
                    ),
                    evaluation,
                )
            })
            .collect::<HashMap<_, _>>();
        let snapshots = self.read_snapshots().await?;
        let snapshots_by_key = snapshots
            .into_iter()
            .map(|snapshot| ((snapshot.symbol.clone(), snapshot.timestamp), snapshot))
            .collect::<HashMap<_, _>>();
        let research_features = self.research_features(None, usize::MAX).await?;
        let research_features_by_key = research_features
            .into_iter()
            .filter_map(|features| {
                Some((
                    (features.symbol.clone(), features.timestamp),
                    research_feature_vector(&features)?,
                ))
            })
            .collect::<HashMap<_, _>>();
        let mut grouped: HashMap<(String, u64), Vec<SignalOutcome>> = HashMap::new();
        for outcome in outcomes {
            grouped
                .entry((outcome.symbol.clone(), outcome.horizon_seconds))
                .or_default()
                .push(outcome);
        }
        let mut result = Vec::new();
        for ((symbol, horizon_seconds), samples) in grouped {
            let non_overlapping = non_overlapping_samples(samples, horizon_seconds);
            if non_overlapping.len() <= WALK_FORWARD_MIN_TRAINING_SAMPLES {
                continue;
            }

            let mut metrics = WalkForwardMetrics::new();
            for index in WALK_FORWARD_MIN_TRAINING_SAMPLES..non_overlapping.len() {
                let start = index.saturating_sub(WALK_FORWARD_TRAINING_WINDOW);
                let sample = &non_overlapping[index];
                let sample_net = sample.net_return_percent.unwrap_or_default();
                if sample.direction == "bullish" {
                    metrics.all_signal_long.record(sample.direction_match, sample_net);
                } else if sample.direction == "bearish" {
                    metrics.all_signal_short.record(sample.direction_match, sample_net);
                }
                if sample.horizon_seconds == 60 {
                    if let (Some(snapshot), Some(cost)) = (
                        snapshots_by_key.get(&(sample.symbol.clone(), sample.signal_timestamp)),
                        sample.execution_cost_percent,
                    ) {
                        let can_explore = snapshot.strength >= 0.05
                            && snapshot.ranging_probability.unwrap_or(0.0) < 0.75
                            && matches!(
                                snapshot.direction.as_str(),
                                "bullish" | "bearish" | "neutral"
                            );
                        if can_explore {
                            let up = snapshot.score >= 0.0;
                            let matches = (up && sample.return_percent > 0.0)
                                || (!up && sample.return_percent < 0.0);
                            let net = if up {
                                sample.return_percent
                            } else {
                                -sample.return_percent
                            } - cost;
                            for (band, threshold) in EXPLORATION_SCORE_THRESHOLDS.iter().enumerate()
                            {
                                if snapshot.score.abs() >= *threshold {
                                    metrics.exploration_selected[band] += 1;
                                    if matches {
                                        metrics.exploration_correct[band] += 1;
                                    }
                                    if net > 0.0 {
                                        metrics.exploration_positive[band] += 1;
                                    }
                                    metrics.exploration_net[band] += net;
                                }
                            }
                        }
                    }
                }
                let training_window = &non_overlapping[start..index];
                let training_count = training_window
                    .iter()
                    .filter(|prior| prior.evaluated_timestamp <= sample.signal_timestamp)
                    .count();
                if training_count < WALK_FORWARD_MIN_TRAINING_SAMPLES {
                    continue;
                }
                metrics.max_training_samples = metrics.max_training_samples.max(training_count);
                metrics.evaluated_signals += 1;
                if sample.direction_match {
                    metrics.baseline_correct += 1;
                }
                metrics.baseline_net_total += sample.net_return_percent.unwrap_or_default();
                if (index - WALK_FORWARD_MIN_TRAINING_SAMPLES)
                    % WALK_FORWARD_LOGISTIC_REFRESH_INTERVAL
                    == 0
                {
                    let training = training_window
                        .iter()
                        .filter(|prior| prior.evaluated_timestamp <= sample.signal_timestamp)
                        .cloned()
                        .collect::<Vec<_>>();
                    let logistic_training = training
                        .iter()
                        .filter(|prior| prior.features.is_some())
                        .cloned()
                        .collect::<Vec<_>>();
                    metrics.logistic_weights = train_logistic(&logistic_training);
                    metrics.trend_logistic_weights = train_trend_logistic(&logistic_training);
                    metrics.research_logistic_weights =
                        train_research_logistic(&training, &research_features_by_key);
                }
                if let (Some(weights), Some(features), Some(trend), Some(cost)) = (
                    metrics.trend_logistic_weights.as_ref(),
                    sample.features.as_ref(),
                    sample.trend_5m_volatility_units,
                    sample.execution_cost_percent,
                ) {
                    let probability = trend_logistic_up_probability(weights, features, trend);
                    let up = probability >= 0.5;
                    let matches =
                        (up && sample.return_percent > 0.0) || (!up && sample.return_percent < 0.0);
                    let net = if up {
                        sample.return_percent
                    } else {
                        -sample.return_percent
                    } - cost;
                    metrics.trend_logistic_selected += 1;
                    if matches {
                        metrics.trend_logistic_correct += 1;
                    }
                    if net > 0.0 {
                        metrics.trend_logistic_positive += 1;
                    }
                    metrics.trend_logistic_net_total += net;
                    let contrarian_up = probability < 0.5;
                    let contrarian_matches = (contrarian_up && sample.return_percent > 0.0)
                        || (!contrarian_up && sample.return_percent < 0.0);
                    let contrarian_net = if contrarian_up {
                        sample.return_percent
                    } else {
                        -sample.return_percent
                    } - cost;
                    metrics.contrarian_selected += 1;
                    if contrarian_matches {
                        metrics.contrarian_correct += 1;
                    }
                    if contrarian_net > 0.0 {
                        metrics.contrarian_positive += 1;
                    }
                    metrics.contrarian_net_total += contrarian_net;
                    let confidence = probability.max(1.0 - probability);
                    for (band, threshold) in TREND_LOGISTIC_CONFIDENCE_THRESHOLDS.iter().enumerate()
                    {
                        if confidence >= *threshold {
                            metrics.trend_logistic_confident_selected[band] += 1;
                            if matches {
                                metrics.trend_logistic_confident_correct[band] += 1;
                            }
                            if net > 0.0 {
                                metrics.trend_logistic_confident_positive[band] += 1;
                            }
                            metrics.trend_logistic_confident_net[band] += net;
                            metrics.contrarian_confident_selected[band] += 1;
                            if contrarian_matches {
                                metrics.contrarian_confident_correct[band] += 1;
                            }
                            if contrarian_net > 0.0 {
                                metrics.contrarian_confident_positive[band] += 1;
                            }
                            metrics.contrarian_confident_net[band] += contrarian_net;
                        }
                    }
                }
                if let (Some(weights), Some(features), Some(cost)) = (
                    metrics.research_logistic_weights.as_ref(),
                    research_features_by_key.get(&(sample.symbol.clone(), sample.signal_timestamp)),
                    sample.execution_cost_percent,
                ) {
                    let up = research_logistic_up_probability(weights, features) >= 0.5;
                    let matches =
                        (up && sample.return_percent > 0.0) || (!up && sample.return_percent < 0.0);
                    let net = if up {
                        sample.return_percent
                    } else {
                        -sample.return_percent
                    } - cost;
                    if up {
                        metrics.research_logistic_long.record(matches, net);
                    } else {
                        metrics.research_logistic_short.record(matches, net);
                    }
                }
                if let (Some(weights), Some(features), Some(cost)) = (
                    metrics.logistic_weights.as_ref(),
                    sample.features.as_ref(),
                    sample.execution_cost_percent,
                ) {
                    let up = logistic_up_probability(&weights, features) >= 0.5;
                    let matches =
                        (up && sample.return_percent > 0.0) || (!up && sample.return_percent < 0.0);
                    let net = if up {
                        sample.return_percent
                    } else {
                        -sample.return_percent
                    } - cost;
                    metrics.logistic_selected += 1;
                    if matches {
                        metrics.logistic_correct += 1;
                    }
                    if net > 0.0 {
                        metrics.logistic_positive += 1;
                    }
                    metrics.logistic_net_total += net;
                    if up {
                        metrics.logistic_long.record(matches, net);
                    } else {
                        metrics.logistic_short.record(matches, net);
                    }
                }
                if let (Some(matches), Some(net_return)) = (
                    sample.trend_5m_direction_match,
                    sample.trend_5m_net_return_percent,
                ) {
                    metrics.five_minute_trend_samples += 1;
                    if matches {
                        metrics.five_minute_trend_correct += 1;
                    }
                    metrics.five_minute_trend_net_total += net_return;
                }
                let Some(threshold) =
                    select_strength_threshold(training_window, sample.signal_timestamp)
                else {
                    continue;
                };
                metrics.last_threshold = Some(threshold);
                if sample.strength < threshold {
                    continue;
                }
                metrics.selected_signals += 1;
                if sample.direction_match {
                    metrics.correct += 1;
                }
                let net = sample.net_return_percent.unwrap_or_default();
                metrics.net_total += net;
                if net > 0.0 {
                    metrics.positive_net += 1;
                }
                if sample.direction == "bullish" {
                    metrics.signal_long.record(sample.direction_match, net);
                } else if sample.direction == "bearish" {
                    metrics.signal_short.record(sample.direction_match, net);
                }
            }
            if metrics.evaluated_signals == 0 {
                continue;
            }
            result.push(build_walk_forward_result(
                symbol,
                horizon_seconds,
                metrics,
                &non_overlapping,
                &evaluations,
            ));
        }
        result.sort_by(|a, b| {
            a.symbol
                .cmp(&b.symbol)
                .then(a.horizon_seconds.cmp(&b.horizon_seconds))
        });
        Ok(result)
    }
}
