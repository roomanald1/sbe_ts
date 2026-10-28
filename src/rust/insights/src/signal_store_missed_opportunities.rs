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
                .walk_forward_refreshing
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let store = self.clone();
                tokio::spawn(async move {
                    if let Ok(value) = store.compute_missed_opportunity_bands().await {
                        *store.missed_opportunity_cache.lock().await =
                            Some((Instant::now(), value));
                    }
                    store
                        .walk_forward_refreshing
                        .store(false, Ordering::Release);
                });
            }
            return Ok(stale);
        }
        drop(cache);
        if self
            .walk_forward_refreshing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let store = self.clone();
            tokio::spawn(async move {
                if let Ok(value) = store.compute_missed_opportunity_bands().await {
                    *store.missed_opportunity_cache.lock().await =
                        Some((Instant::now(), value));
                }
                store
                    .walk_forward_refreshing
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
        let decisions = self
            .load_auto_trader_audit_with_corrupt_rows(Some(since), 50_000, true)
            .await?;
        let evaluations = self
            .read_recent_records_with_corrupt_rows(2, 100_000, true)
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Evaluation(evaluation)
                    if evaluation.evaluated_timestamp >= since
                        && evaluation.evaluated_timestamp <= now_ms =>
                {
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
        let mut resolved_skips = Vec::new();
        for decision in decisions {
            if decision.get("stage").and_then(serde_json::Value::as_str) != Some("decision")
                || decision.get("status").and_then(serde_json::Value::as_str) != Some("skipped")
                || !decision
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|reason| reason.starts_with("No tradeable bias:"))
            {
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
                if terminal_net.is_some() || stop_target_net.is_some() {
                    samples.push((horizon, terminal_net, time_to_profit, stop_target_net));
                }
            }
            if samples.iter().any(|sample| sample.0 == 60) {
                resolved_skips.push((
                    symbol.to_owned(),
                    direction.to_owned(),
                    timestamp,
                    score.abs(),
                    strength,
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
        let mut next_allowed = HashMap::<(String, String), u64>::new();
        let mut aggregates =
            HashMap::<(String, usize, u64), Vec<(Option<f64>, Option<u64>, Option<f64>)>>::new();
        for (symbol, direction, timestamp, score, strength, samples) in resolved_skips {
            let key = (symbol, direction);
            if timestamp < *next_allowed.get(&key).unwrap_or(&0) {
                continue;
            }
            next_allowed.insert(key, timestamp.saturating_add(60_000));
            for (horizon, net, time_to_profit, stop_target_net) in samples {
                for (index, threshold) in MISSED_OPPORTUNITY_SCORE_THRESHOLDS.iter().enumerate() {
                    if score >= *threshold {
                        aggregates
                            .entry(("score".to_owned(), index, horizon))
                            .or_default()
                            .push((net, time_to_profit, stop_target_net));
                    }
                }
                for (index, threshold) in MISSED_OPPORTUNITY_STRENGTH_THRESHOLDS.iter().enumerate()
                {
                    if strength >= *threshold {
                        aggregates
                            .entry(("strength".to_owned(), index, horizon))
                            .or_default()
                            .push((net, time_to_profit, stop_target_net));
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
                        .get(&(gate.to_owned(), index, horizon))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    let evaluation_count = samples.len();
                    let net_samples = samples
                        .iter()
                        .filter_map(|(net, _, _)| *net)
                        .collect::<Vec<_>>();
                    let count = net_samples.len();
                    let positive_count = net_samples.iter().filter(|net| **net > 0.0).count();
                    let barrier_samples = samples
                        .iter()
                        .filter_map(|(_, _, net)| *net)
                        .collect::<Vec<_>>();
                    let barrier_count = barrier_samples.len();
                    let barrier_positive_count =
                        barrier_samples.iter().filter(|net| **net > 0.0).count();
                    let mut times = samples
                        .iter()
                        .filter_map(|(_, time, _)| *time)
                        .collect::<Vec<_>>();
                    times.sort_unstable();
                    let time_count = times.len();
                    let average_time = (time_count > 0)
                        .then(|| times.iter().sum::<u64>() as f64 / time_count as f64);
                    let average_barrier_net = (barrier_count > 0)
                        .then(|| barrier_samples.iter().sum::<f64>() / barrier_count as f64);
                    let median_time = (time_count > 0).then(|| {
                        if time_count % 2 == 0 {
                            (times[time_count / 2 - 1] + times[time_count / 2]) as f64 / 2.0
                        } else {
                            times[time_count / 2] as f64
                        }
                    });
                    bands.push(MissedOpportunityGateBand {
                        gate: gate.to_owned(),
                        threshold: *threshold,
                        horizon_seconds: horizon,
                        sample_count: count,
                        average_net_return_percent: (count > 0)
                            .then(|| net_samples.iter().sum::<f64>() / count as f64),
                        positive_net_rate: (count > 0)
                            .then_some(positive_count as f64 / count as f64),
                        time_to_profit_sample_count: time_count,
                        time_to_profit_rate: (evaluation_count > 0)
                            .then_some(time_count as f64 / evaluation_count as f64),
                        average_time_to_profit_seconds: average_time,
                        median_time_to_profit_seconds: median_time,
                        stop_target_sample_count: barrier_count,
                        stop_target_average_net_return_percent: average_barrier_net,
                        stop_target_positive_net_rate: (barrier_count > 0)
                            .then_some(barrier_positive_count as f64 / barrier_count as f64),
                    });
                }
            }
        }
        Ok(bands)
    }
}
