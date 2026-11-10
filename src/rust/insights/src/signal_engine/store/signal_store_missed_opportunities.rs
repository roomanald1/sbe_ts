impl SignalStore {
    pub fn cached_walk_forward(&self) -> Vec<SignalWalkForward> {
        self.walk_forward_cache
            .try_lock()
            .ok()
            .and_then(|cache| cache.as_ref().map(|(_, value)| value.clone()))
            .unwrap_or_default()
    }

    pub fn cached_missed_opportunity_bands(&self) -> Vec<MissedOpportunityGateBand> {
        self.missed_opportunity_cache
            .try_lock()
            .ok()
            .and_then(|cache| cache.as_ref().map(|(_, value)| value.clone()))
            .unwrap_or_default()
    }

    pub async fn missed_opportunity_bands(&self) -> Result<Vec<MissedOpportunityGateBand>> {
        let cache = self.missed_opportunity_cache.lock().await;
        if let Some((created, value)) = cache.as_ref() {
            // This analysis scans a large audit/evaluation history. Keep the
            // last complete result around longer and refresh it off the caller's
            // path; signal ingestion must never wait for this database work.
            if created.elapsed() < Duration::from_secs(15 * 60) {
                return Ok(value.clone());
            }
            let stale = value.clone();
            if self
                .missed_opportunity_refreshing
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let store = self.clone();
                tokio::spawn(async move {
                    match store.compute_missed_opportunity_bands().await {
                        Ok(value) => {
                            *store.missed_opportunity_cache.lock().await =
                                Some((Instant::now(), value));
                        }
                        Err(error) => {
                            eprintln!("Missed opportunity band refresh failed: {error:#}")
                        }
                    }
                    store
                        .missed_opportunity_refreshing
                        .store(false, Ordering::Release);
                });
            }
            return Ok(stale);
        }
        drop(cache);
        if self
            .missed_opportunity_refreshing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let store = self.clone();
            tokio::spawn(async move {
                match store.compute_missed_opportunity_bands().await {
                    Ok(value) => {
                        *store.missed_opportunity_cache.lock().await =
                            Some((Instant::now(), value));
                    }
                    Err(error) => eprintln!("Missed opportunity band refresh failed: {error:#}"),
                }
                store
                    .missed_opportunity_refreshing
                    .store(false, Ordering::Release);
            });
        }
        Ok(Vec::new())
    }

    async fn compute_missed_opportunity_bands(&self) -> Result<Vec<MissedOpportunityGateBand>> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        let since = now_ms.saturating_sub(30 * 24 * 60 * 60 * 1_000);
        let decisions = self.load_auto_trader_decision_audit_window(since).await?;
        let evaluation_since = decisions
            .iter()
            .filter(|decision| is_skipped_candidate_decision(decision))
            .filter_map(|decision| {
                decision
                    .get("signal_timestamp")
                    .and_then(serde_json::Value::as_u64)
            })
            .min()
            .or(Some(since));
        let quote_paths = self
            .read_records_with_corrupt_rows(6, evaluation_since, None, true)
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::CounterfactualQuotePath(path) => Some((
                    (
                        path.symbol.clone(),
                        path.signal_timestamp,
                        path.horizon_seconds,
                    ),
                    path.samples,
                )),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        let evaluations = self
            .read_records_with_corrupt_rows(2, evaluation_since, None, true)
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Evaluation(mut evaluation)
                    if evaluation.evaluated_timestamp >= since
                        && evaluation.evaluated_timestamp <= now_ms =>
                {
                    evaluation.counterfactual_quote_path = quote_paths
                        .get(&(
                            evaluation.symbol.clone(),
                            evaluation.signal_timestamp,
                            evaluation.horizon_seconds,
                        ))
                        .cloned()
                        .unwrap_or_default();
                    Some(evaluation)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let evaluation_by_signal = evaluations
            .into_iter()
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
        let strategy_directions = index_strategy_directions(&decisions);
        let mut resolved_skips = Vec::new();
        for decision in decisions {
            if !is_skipped_candidate_decision(&decision) {
                continue;
            }
            let Some(symbol) = decision.get("symbol").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(timestamp) = decision
                .get("signal_timestamp")
                .and_then(serde_json::Value::as_u64)
            else {
                continue;
            };
            let Some(direction) = decision
                .get("direction")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let Some(score) = decision
                .get("score")
                .and_then(serde_json::Value::as_f64)
                .filter(|v| v.is_finite())
            else {
                continue;
            };
            let Some(strength) = decision
                .get("strength")
                .and_then(serde_json::Value::as_f64)
                .filter(|v| v.is_finite())
            else {
                continue;
            };
            let strategy = decision
                .get("strategy")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            let market_context = decision
                .get("market_regime")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    decision
                        .get("entry_gate")
                        .and_then(|gate| gate.get("market_context"))
                        .and_then(serde_json::Value::as_str)
                })
                .or_else(|| {
                    decision
                        .get("market_context")
                        .and_then(serde_json::Value::as_str)
                })
                .map(|context| context.split(':').nth(1).unwrap_or(context).to_owned())
                .or_else(|| {
                    decision
                        .get("ranging_probability")
                        .and_then(serde_json::Value::as_f64)
                        .map(|probability| {
                            if probability >= 0.60 {
                                "range".to_owned()
                            } else {
                                "mixed".to_owned()
                            }
                        })
                });
            let stop_loss = decision
                .get("stop_loss_percent")
                .and_then(serde_json::Value::as_f64);
            let take_profit = decision
                .get("take_profit_percent")
                .and_then(serde_json::Value::as_f64);
            let mut samples = Vec::new();
            for horizon in [60_u64, 300, 900, 3_600] {
                let Some(evaluation) =
                    evaluation_by_signal.get(&(symbol.to_owned(), timestamp, horizon))
                else {
                    continue;
                };
                if evaluation.evaluated_timestamp > now_ms {
                    continue;
                }
                let (terminal_net, time_to_profit) = match direction {
                    "bullish" => (
                        evaluation.long_net_percent,
                        evaluation.long_time_to_positive_net_seconds,
                    ),
                    "bearish" => (
                        evaluation.short_net_percent,
                        evaluation.short_time_to_positive_net_seconds,
                    ),
                    _ => (None, None),
                };
                let terminal_net = terminal_net.filter(|value| value.is_finite());
                let stop_target_net = stop_loss.zip(take_profit).and_then(|(stop, target)| {
                    stop_target_counterfactual_return(evaluation, direction, stop, target)
                });
                let strategy_exit_net = strategy
                    .as_deref()
                    .and_then(|strategy_name| {
                        next_strategy_direction_change(
                            &strategy_directions,
                            symbol,
                            strategy_name,
                            direction,
                            timestamp,
                        )
                    })
                    .filter(|invalidation| *invalidation <= evaluation.evaluated_timestamp)
                    .and_then(|invalidation| {
                        strategy_invalidation_counterfactual_return(
                            evaluation,
                            direction,
                            invalidation,
                        )
                    });
                if terminal_net.is_some()
                    || stop_target_net.is_some()
                    || strategy_exit_net.is_some()
                {
                    samples.push((
                        horizon,
                        terminal_net,
                        time_to_profit,
                        stop_target_net,
                        strategy_exit_net,
                    ));
                }
            }
            if samples.iter().any(|sample| sample.0 == 60) {
                resolved_skips.push((
                    symbol.to_owned(),
                    direction.to_owned(),
                    timestamp,
                    score.abs(),
                    strength,
                    strategy,
                    market_context,
                    samples,
                ));
            }
        }
        resolved_skips.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(left.1.cmp(&right.1))
                .then(left.2.cmp(&right.2))
        });
        let mut next_allowed =
            HashMap::<(String, String, Option<String>, Option<String>), u64>::new();
        let mut aggregates = HashMap::<
            (
                String,
                usize,
                u64,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
            Vec<(Option<f64>, Option<u64>, Option<f64>, Option<f64>)>,
        >::new();
        for (symbol, direction, timestamp, score, strength, strategy, market_context, samples) in
            resolved_skips
        {
            let key = (
                symbol.clone(),
                direction.clone(),
                strategy.clone(),
                market_context.clone(),
            );
            if timestamp < *next_allowed.get(&key).unwrap_or(&0) {
                continue;
            }
            next_allowed.insert(key, timestamp.saturating_add(60_000));
            for (horizon, net, time_to_profit, stop_target_net, strategy_exit_net) in samples {
                for (index, threshold) in MISSED_OPPORTUNITY_SCORE_THRESHOLDS.iter().enumerate() {
                    if score >= *threshold {
                        aggregates
                            .entry(("score".to_owned(), index, horizon, None, None, None, None))
                            .or_default()
                            .push((net, time_to_profit, stop_target_net, strategy_exit_net));
                        aggregates
                            .entry((
                                "score".to_owned(),
                                index,
                                horizon,
                                Some(symbol.clone()),
                                Some(direction.clone()),
                                strategy.clone(),
                                market_context.clone(),
                            ))
                            .or_default()
                            .push((net, time_to_profit, stop_target_net, strategy_exit_net));
                    }
                }
                for (index, threshold) in MISSED_OPPORTUNITY_STRENGTH_THRESHOLDS.iter().enumerate()
                {
                    if strength >= *threshold {
                        aggregates
                            .entry((
                                "strength".to_owned(),
                                index,
                                horizon,
                                None,
                                None,
                                None,
                                None,
                            ))
                            .or_default()
                            .push((net, time_to_profit, stop_target_net, strategy_exit_net));
                        aggregates
                            .entry((
                                "strength".to_owned(),
                                index,
                                horizon,
                                Some(symbol.clone()),
                                Some(direction.clone()),
                                strategy.clone(),
                                market_context.clone(),
                            ))
                            .or_default()
                            .push((net, time_to_profit, stop_target_net, strategy_exit_net));
                    }
                }
            }
        }
        let mut bands = Vec::new();
        for (gate, thresholds) in [
            ("score", MISSED_OPPORTUNITY_SCORE_THRESHOLDS.as_slice()),
            (
                "strength",
                MISSED_OPPORTUNITY_STRENGTH_THRESHOLDS.as_slice(),
            ),
        ] {
            for (index, threshold) in thresholds.iter().enumerate() {
                for horizon in [60_u64, 300, 900, 3_600] {
                    let samples = aggregates
                        .get(&(gate.to_owned(), index, horizon, None, None, None, None))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    bands.push(missed_opportunity_band(
                        gate, *threshold, None, None, None, None, horizon, samples,
                    ));
                    for ((_, _, _, symbol, direction, strategy, market_context), samples) in
                        aggregates.iter().filter(
                            |((sample_gate, sample_index, sample_horizon, symbol, _, _, _), _)| {
                                sample_gate == gate
                                    && *sample_index == index
                                    && *sample_horizon == horizon
                                    && symbol.is_some()
                            },
                        )
                    {
                        bands.push(missed_opportunity_band(
                            gate,
                            *threshold,
                            symbol.clone(),
                            direction.clone(),
                            strategy.clone(),
                            market_context.clone(),
                            horizon,
                            samples,
                        ));
                    }
                }
            }
        }
        Ok(bands)
    }
}

fn is_skipped_candidate_decision(decision: &serde_json::Value) -> bool {
    decision.get("stage").and_then(serde_json::Value::as_str) == Some("decision")
        && matches!(
            decision.get("status").and_then(serde_json::Value::as_str),
            Some("skipped" | "rejected")
        )
}

fn index_strategy_directions(
    decisions: &[serde_json::Value],
) -> HashMap<(String, String), Vec<(u64, String)>> {
    let mut index = HashMap::<(String, String), Vec<(u64, String)>>::new();
    for decision in decisions {
        if decision.get("stage").and_then(serde_json::Value::as_str) != Some("decision") {
            continue;
        }
        let (Some(symbol), Some(strategy), Some(direction), Some(timestamp)) = (
            decision
                .get("symbol")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty()),
            decision
                .get("strategy")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty()),
            decision
                .get("direction")
                .and_then(serde_json::Value::as_str)
                .filter(|value| matches!(*value, "bullish" | "bearish")),
            decision
                .get("signal_timestamp")
                .and_then(serde_json::Value::as_u64),
        ) else {
            continue;
        };
        index
            .entry((symbol.to_owned(), strategy.to_owned()))
            .or_default()
            .push((timestamp, direction.to_owned()));
    }
    for history in index.values_mut() {
        history.sort_by_key(|(timestamp, _)| *timestamp);
    }
    index
}

fn next_strategy_direction_change(
    index: &HashMap<(String, String), Vec<(u64, String)>>,
    symbol: &str,
    strategy: &str,
    direction: &str,
    timestamp: u64,
) -> Option<u64> {
    index
        .get(&(symbol.to_owned(), strategy.to_owned()))?
        .iter()
        .find(|(observed_at, observed_direction)| {
            *observed_at > timestamp && observed_direction != direction
        })
        .map(|(observed_at, _)| *observed_at)
}

fn missed_opportunity_band(
    gate: &str,
    threshold: f64,
    symbol: Option<String>,
    direction: Option<String>,
    strategy: Option<String>,
    market_context: Option<String>,
    horizon_seconds: u64,
    samples: &[(Option<f64>, Option<u64>, Option<f64>, Option<f64>)],
) -> MissedOpportunityGateBand {
    let evaluation_count = samples.len();
    let net_samples = samples
        .iter()
        .filter_map(|(net, _, _, _)| *net)
        .collect::<Vec<_>>();
    let count = net_samples.len();
    let positive_count = net_samples.iter().filter(|net| **net > 0.0).count();
    let barrier_samples = samples
        .iter()
        .filter_map(|(_, _, net, _)| *net)
        .collect::<Vec<_>>();
    let barrier_count = barrier_samples.len();
    let barrier_positive_count = barrier_samples.iter().filter(|net| **net > 0.0).count();
    let mut times = samples
        .iter()
        .filter_map(|(_, time, _, _)| *time)
        .collect::<Vec<_>>();
    times.sort_unstable();
    let time_count = times.len();
    let average_time =
        (time_count > 0).then(|| times.iter().sum::<u64>() as f64 / time_count as f64);
    let average_barrier_net =
        (barrier_count > 0).then(|| barrier_samples.iter().sum::<f64>() / barrier_count as f64);
    let strategy_exit_samples = samples
        .iter()
        .filter_map(|(_, _, _, net)| *net)
        .collect::<Vec<_>>();
    let strategy_exit_count = strategy_exit_samples.len();
    let strategy_exit_positive_count = strategy_exit_samples
        .iter()
        .filter(|net| **net > 0.0)
        .count();
    let median_time = (time_count > 0).then(|| {
        if time_count % 2 == 0 {
            (times[time_count / 2 - 1] + times[time_count / 2]) as f64 / 2.0
        } else {
            times[time_count / 2] as f64
        }
    });
    MissedOpportunityGateBand {
        gate: gate.to_owned(),
        threshold,
        symbol,
        direction,
        strategy,
        market_context,
        horizon_seconds,
        sample_count: count,
        average_net_return_percent: (count > 0)
            .then(|| net_samples.iter().sum::<f64>() / count as f64),
        positive_net_rate: (count > 0).then_some(positive_count as f64 / count as f64),
        time_to_profit_sample_count: time_count,
        time_to_profit_rate: (evaluation_count > 0)
            .then_some(time_count as f64 / evaluation_count as f64),
        average_time_to_profit_seconds: average_time,
        median_time_to_profit_seconds: median_time,
        stop_target_sample_count: barrier_count,
        stop_target_average_net_return_percent: average_barrier_net,
        stop_target_positive_net_rate: (barrier_count > 0)
            .then_some(barrier_positive_count as f64 / barrier_count as f64),
        strategy_exit_sample_count: strategy_exit_count,
        strategy_exit_average_net_return_percent: (strategy_exit_count > 0)
            .then(|| strategy_exit_samples.iter().sum::<f64>() / strategy_exit_count as f64),
        strategy_exit_positive_net_rate: (strategy_exit_count > 0)
            .then_some(strategy_exit_positive_count as f64 / strategy_exit_count as f64),
    }
}

#[cfg(test)]
mod missed_opportunity_audit_tests {
    use super::*;

    #[test]
    fn includes_skipped_and_rejected_decision_records_only() {
        for status in ["skipped", "rejected"] {
            let decision = serde_json::json!({ "stage": "decision", "status": status });
            assert!(is_skipped_candidate_decision(&decision));
        }
        assert!(!is_skipped_candidate_decision(
            &serde_json::json!({ "stage": "entry", "status": "skipped" })
        ));
        assert!(!is_skipped_candidate_decision(
            &serde_json::json!({ "stage": "decision", "status": "selected" })
        ));
    }

    #[test]
    fn preserves_opportunity_dimensions_in_band_rows() {
        let band = missed_opportunity_band(
            "score",
            0.4,
            Some("BTC-USDT".into()),
            Some("bullish".into()),
            Some("trend".into()),
            Some("trending".into()),
            300,
            &[(Some(0.25), Some(120), Some(0.3), Some(0.2))],
        );
        assert_eq!(band.symbol.as_deref(), Some("BTC-USDT"));
        assert_eq!(band.direction.as_deref(), Some("bullish"));
        assert_eq!(band.strategy.as_deref(), Some("trend"));
        assert_eq!(band.market_context.as_deref(), Some("trending"));
        assert_eq!(band.sample_count, 1);
        assert_eq!(band.strategy_exit_sample_count, 1);
        assert_eq!(band.strategy_exit_average_net_return_percent, Some(0.2));
    }

    #[test]
    fn strategy_invalidation_uses_the_next_matching_opposite_signal() {
        let decisions = vec![
            serde_json::json!({
                "stage": "decision", "symbol": "BTC-USDT", "strategy": "trend",
                "direction": "bullish", "signal_timestamp": 1_000
            }),
            serde_json::json!({
                "stage": "decision", "symbol": "BTC-USDT", "strategy": "trend",
                "direction": "bullish", "signal_timestamp": 2_000
            }),
            serde_json::json!({
                "stage": "decision", "symbol": "BTC-USDT", "strategy": "trend",
                "direction": "bearish", "signal_timestamp": 3_000
            }),
            serde_json::json!({
                "stage": "decision", "symbol": "ETH-USDT", "strategy": "trend",
                "direction": "bearish", "signal_timestamp": 2_500
            }),
        ];
        let index = index_strategy_directions(&decisions);
        assert_eq!(
            next_strategy_direction_change(&index, "BTC-USDT", "trend", "bullish", 1_000),
            Some(3_000)
        );
        assert_eq!(
            next_strategy_direction_change(&index, "BTC-USDT", "trend", "bullish", 3_000),
            None
        );
        assert_eq!(
            next_strategy_direction_change(&index, "ETH-USDT", "trend", "bullish", 1_000),
            Some(2_500)
        );
    }
}
