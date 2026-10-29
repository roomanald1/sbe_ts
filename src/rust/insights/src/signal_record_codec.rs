fn deserialize_record(kind: i64, payload: &[u8]) -> Result<StoredSignalRecord> {
    if let Ok(record) = bincode::deserialize(payload) {
        return Ok(record);
    }
    if kind == 0 {
        let ExtendedSnapshotRecord::Snapshot(snapshot) = bincode::deserialize(payload)?;
        return Ok(StoredSignalRecord::Snapshot(SignalSnapshot {
            symbol: snapshot.symbol,
            timestamp: snapshot.timestamp,
            direction: snapshot.direction,
            score: snapshot.score,
            strength: snapshot.strength,
            components: SignalComponents {
                trend_1m_percent: snapshot.components.trend_1m_percent,
                momentum_10s_percent: snapshot.components.momentum_10s_percent,
                trade_flow_imbalance: snapshot.components.trade_flow_imbalance,
                book_imbalance: snapshot.components.book_imbalance,
                volatility_1m_percent: snapshot.components.volatility_1m_percent,
                trend_5m_percent: snapshot.components.trend_5m_percent,
                trend_5m_volatility_units: snapshot.components.trend_5m_volatility_units,
                mispricing_vs_vwap_bps: snapshot.components.mispricing_vs_vwap_bps,
                pressure_without_price_move: snapshot
                    .components
                    .pressure_without_price_move
                    .or(snapshot.components.absorption_imbalance),
                aggressor_cost_bps: snapshot.components.aggressor_cost_bps,
            },
            reasons: snapshot.reasons,
            news_attribution: None,
            stale: snapshot.stale,
            ranging_probability: snapshot.ranging_probability,
            range_lower_bound: snapshot.range_lower_bound,
            range_upper_bound: snapshot.range_upper_bound,
        }));
    }
    Ok(bincode::deserialize(payload)?)
}

fn compress_record(record: &StoredSignalRecord) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&bincode::serialize(record)?)?;
    Ok(encoder.finish()?)
}

fn record_kind(record: &StoredSignalRecord) -> i64 {
    match record {
        StoredSignalRecord::Snapshot(_) => 0,
        StoredSignalRecord::Outcome(_) => 1,
        StoredSignalRecord::Evaluation(_) => 2,
        StoredSignalRecord::BookSnapshot(_) => 3,
        StoredSignalRecord::ResearchFeatures(_) => 4,
        StoredSignalRecord::ExternalMarket(_) => 5,
    }
}

fn record_timestamp(record: &StoredSignalRecord) -> u64 {
    match record {
        StoredSignalRecord::Snapshot(snapshot) => snapshot.timestamp,
        StoredSignalRecord::Outcome(outcome) => outcome.signal_timestamp,
        StoredSignalRecord::Evaluation(evaluation) => evaluation.signal_timestamp,
        StoredSignalRecord::BookSnapshot(snapshot) => snapshot.timestamp,
        StoredSignalRecord::ResearchFeatures(features) => features.timestamp,
        StoredSignalRecord::ExternalMarket(features) => features.timestamp,
    }
}

fn accuracy_bucket(snapshot: &SignalSnapshot) -> u8 {
    let value = if snapshot.direction == "ranging" {
        snapshot.ranging_probability.unwrap_or_default()
    } else {
        snapshot.strength
    };
    (value.clamp(0.0, 0.999) * 10.0) as u8
}

fn calibrate_signal_scores(snapshots: &mut [SignalSnapshot], records: &[StoredSignalRecord]) {
    const PRIOR_SAMPLES: f64 = 10.0;
    const EDGE_PRIOR_SAMPLES: f64 = 20.0;
    const EDGE_SCORE_WEIGHT: f64 = 0.35;
    const EDGE_SCALE_PERCENT: f64 = 0.5;
    let mut history: HashMap<String, Vec<&SignalSnapshot>> = HashMap::new();
    let mut accuracies: HashMap<(String, String, u8), (usize, usize)> = HashMap::new();
    let mut outcomes_by_key: HashMap<(String, String, u8), Vec<(u64, bool)>> = HashMap::new();
    let mut net_edge_history: HashMap<(String, String, u64), Vec<(u64, f64)>> = HashMap::new();
    let mut outcomes = Vec::new();

    for record in records {
        match record {
            StoredSignalRecord::Snapshot(snapshot) => history
                .entry(snapshot.symbol.clone())
                .or_default()
                .push(snapshot),
            StoredSignalRecord::Outcome(outcome) => outcomes.push(outcome),
            StoredSignalRecord::Evaluation(_)
            | StoredSignalRecord::BookSnapshot(_)
            | StoredSignalRecord::ResearchFeatures(_)
            | StoredSignalRecord::ExternalMarket(_) => {}
        }
    }
    for items in history.values_mut() {
        items.sort_by_key(|snapshot| snapshot.timestamp);
    }

    for (symbol, items) in &history {
        for prediction in items.iter().filter(|item| item.direction == "ranging") {
            let (Some(lower), Some(upper)) =
                (prediction.range_lower_bound, prediction.range_upper_bound)
            else {
                continue;
            };
            let target = prediction.timestamp.saturating_add(300_000);
            let index = items.partition_point(|item| item.timestamp < target);
            let Some(observed) = items
                .get(index)
                .filter(|item| item.timestamp <= target.saturating_add(15_000))
            else {
                continue;
            };
            let (Some(future_low), Some(future_high)) =
                (observed.range_lower_bound, observed.range_upper_bound)
            else {
                continue;
            };
            let key = (
                symbol.clone(),
                "ranging".to_owned(),
                accuracy_bucket(prediction),
            );
            let entry = accuracies.entry(key).or_default();
            entry.0 += 1;
            if future_low >= lower && future_high <= upper {
                entry.1 += 1;
            }
        }
    }
    for outcome in outcomes.iter().filter(|outcome| {
        outcome.horizon_seconds == 60
            && outcome.evaluated_timestamp > 0
            && matches!(outcome.direction.as_str(), "bullish" | "bearish")
    }) {
        let bucket = (outcome.strength.clamp(0.0, 0.999) * 10.0) as u8;
        outcomes_by_key
            .entry((outcome.symbol.clone(), outcome.direction.clone(), bucket))
            .or_default()
            .push((outcome.evaluated_timestamp, outcome.direction_match));
    }
    for samples in outcomes_by_key.values_mut() {
        samples.sort_by_key(|(timestamp, _)| *timestamp);
    }

    // Build point-in-time cumulative net returns so each signal only sees
    // outcomes that had completed before it was emitted.
    for outcome in outcomes.iter().filter(|outcome| {
        outcome.evaluated_timestamp > 0
            && matches!(outcome.direction.as_str(), "bullish" | "bearish")
            && outcome.net_return_percent.is_some()
    }) {
        net_edge_history
            .entry((
                outcome.symbol.clone(),
                outcome.direction.clone(),
                outcome.horizon_seconds,
            ))
            .or_default()
            .push((
                outcome.evaluated_timestamp,
                outcome.net_return_percent.unwrap_or_default(),
            ));
    }
    for samples in net_edge_history.values_mut() {
        samples.sort_by_key(|(timestamp, _)| *timestamp);
        let mut cumulative = 0.0;
        for (_, net_return) in samples {
            cumulative += *net_return;
            *net_return = cumulative;
        }
    }

    // Combine adjacent score buckets so each estimate uses more historical examples.
    fn estimate(
        map: &HashMap<(String, String, u8), (usize, usize)>,
        snapshot: &SignalSnapshot,
    ) -> f64 {
        let bucket = accuracy_bucket(snapshot);
        let mut total = (0usize, 0usize);
        for adjacent in bucket.saturating_sub(1)..=bucket.saturating_add(1).min(9) {
            if let Some((samples, hits)) = map.get(&(
                snapshot.symbol.clone(),
                snapshot.direction.clone(),
                adjacent,
            )) {
                total.0 += samples;
                total.1 += hits;
            }
        }
        (total.1 as f64 + PRIOR_SAMPLES * 0.5) / (total.0 as f64 + PRIOR_SAMPLES)
    }

    fn estimate_point_in_time(
        map: &HashMap<(String, String, u8), Vec<(u64, bool)>>,
        snapshot: &SignalSnapshot,
    ) -> f64 {
        let bucket = accuracy_bucket(snapshot);
        let mut total = (0usize, 0usize);
        for adjacent in bucket.saturating_sub(1)..=bucket.saturating_add(1).min(9) {
            if let Some(samples) = map.get(&(
                snapshot.symbol.clone(),
                snapshot.direction.clone(),
                adjacent,
            )) {
                let completed =
                    samples.partition_point(|(timestamp, _)| *timestamp <= snapshot.timestamp);
                total.0 += completed;
                total.1 += samples[..completed]
                    .iter()
                    .filter(|(_, matched)| *matched)
                    .count();
            }
        }
        (total.1 as f64 + PRIOR_SAMPLES * 0.5) / (total.0 as f64 + PRIOR_SAMPLES)
    }

    // The beta prior keeps small samples near 50% until enough resolved forecasts exist.
    for snapshot in snapshots {
        snapshot.strength = if snapshot.direction == "ranging" {
            estimate(&accuracies, snapshot)
        } else {
            estimate_point_in_time(&outcomes_by_key, snapshot)
        };

        let edge_direction = match snapshot.direction.as_str() {
            "bullish" => "bullish",
            "bearish" => "bearish",
            // For neutral forecasts, let prior performance reinforce the side
            // indicated by the raw score so it can help clear the live direction floor.
            "neutral"
                if snapshot.ranging_probability.unwrap_or(0.0) < 0.65 && snapshot.score > 0.0 =>
            {
                "bullish"
            }
            "neutral"
                if snapshot.ranging_probability.unwrap_or(0.0) < 0.65 && snapshot.score < 0.0 =>
            {
                "bearish"
            }
            _ => continue,
        };
        // Prefer 15-minute evidence; fall back to shorter horizons only when
        // the longer horizon has fewer than eight completed costed outcomes.
        let edge = [900, 300, 60].into_iter().find_map(|horizon| {
            let samples = net_edge_history.get(&(
                snapshot.symbol.clone(),
                edge_direction.to_owned(),
                horizon,
            ))?;
            let completed =
                samples.partition_point(|(timestamp, _)| *timestamp <= snapshot.timestamp);
            if completed < 8 {
                return None;
            }
            Some((completed, samples[completed - 1].1 / completed as f64))
        });
        if let Some((sample_count, mean_net_return)) = edge {
            let direction_sign = if edge_direction == "bullish" {
                1.0
            } else {
                -1.0
            };
            let reliability = sample_count as f64 / (sample_count as f64 + EDGE_PRIOR_SAMPLES);
            let edge_signal = (mean_net_return / EDGE_SCALE_PERCENT).tanh();
            let adjusted_score =
                snapshot.score + direction_sign * EDGE_SCORE_WEIGHT * reliability * edge_signal;
            snapshot.score = if direction_sign > 0.0 {
                adjusted_score.clamp(0.0, 1.0)
            } else {
                adjusted_score.clamp(-1.0, 0.0)
            };
            if snapshot.direction == "neutral" && snapshot.score.abs() > 0.2 {
                snapshot.direction = edge_direction.to_owned();
            } else if snapshot.direction == edge_direction && snapshot.score.abs() <= 0.2 {
                snapshot.direction = "neutral".to_owned();
            }
        }
    }
}
