pub(crate) fn deserialize_record(kind: i64, payload: &[u8]) -> Result<StoredSignalRecord> {
    if kind == 0 {
        return deserialize_snapshot_record(payload);
    }
    if kind == 6 {
        return deserialize_quote_path_record(payload);
    }
    Ok(bincode::deserialize(payload)?)
}

fn deserialize_snapshot_record(payload: &[u8]) -> Result<StoredSignalRecord> {
    let mut failures = Vec::new();
    match deserialize_exact::<StoredSignalRecord>(payload) {
        Ok(record @ StoredSignalRecord::Snapshot(_)) => return Ok(record),
        Ok(_) => failures.push("current schema decoded a different record kind".to_owned()),
        Err(error) => failures.push(format!("current schema: {error}")),
    }
    match deserialize_exact::<ExtendedSnapshotRecord>(payload) {
        Ok(ExtendedSnapshotRecord::Snapshot(snapshot)) => {
            return Ok(StoredSignalRecord::Snapshot(snapshot_from_extended(snapshot)));
        }
        Err(error) => failures.push(format!("extended component schema: {error}")),
    }
    match deserialize_exact::<LegacySnapshotWithoutVectorsRecord>(payload) {
        Ok(LegacySnapshotWithoutVectorsRecord::Snapshot(snapshot)) => {
            return Ok(StoredSignalRecord::Snapshot(snapshot_from_scalar_legacy(snapshot)));
        }
        Err(error) => failures.push(format!("scalar-only component schema: {error}")),
    }
    match deserialize_exact::<LegacySnapshotWithOptionalRangesRecord>(payload) {
        Ok(LegacySnapshotWithOptionalRangesRecord::Snapshot(snapshot)) => {
            return Ok(StoredSignalRecord::Snapshot(
                snapshot_from_scalar_legacy_with_optional_ranges(snapshot),
            ));
        }
        Err(error) => failures.push(format!("optional range schema: {error}")),
    }
    match deserialize_exact::<LegacySnapshotWithoutUpperRangeRecord>(payload) {
        Ok(LegacySnapshotWithoutUpperRangeRecord::Snapshot(snapshot)) => {
            return Ok(StoredSignalRecord::Snapshot(
                snapshot_from_scalar_legacy_without_upper_range(snapshot),
            ));
        }
        Err(error) => failures.push(format!("snapshot without upper range schema: {error}")),
    }
    match deserialize_exact::<LegacySnapshotWithoutNewsRecord>(payload) {
        Ok(LegacySnapshotWithoutNewsRecord::Snapshot(snapshot)) => {
            return Ok(StoredSignalRecord::Snapshot(snapshot_from_legacy(snapshot)));
        }
        Err(error) => failures.push(format!("snapshot without news schema: {error}")),
    }
    match deserialize_exact::<LegacyShiftedSnapshotRecord>(payload) {
        Ok(LegacyShiftedSnapshotRecord::Snapshot(snapshot)) => {
            return Ok(StoredSignalRecord::Snapshot(snapshot_from_extended(snapshot)));
        }
        Ok(_) => failures.push("shifted schema decoded a different record kind".to_owned()),
        Err(error) => failures.push(format!("shifted enum schema: {error}")),
    }
    Err(anyhow::anyhow!(
        "unsupported kind 0 snapshot schema ({})",
        failures.join("; ")
    ))
}

fn deserialize_quote_path_record(payload: &[u8]) -> Result<StoredSignalRecord> {
    let mut failures = Vec::new();
    match deserialize_exact::<StoredSignalRecord>(payload) {
        Ok(record @ StoredSignalRecord::CounterfactualQuotePath(_)) => return Ok(record),
        Ok(_) => failures.push("current schema decoded a different record kind".to_owned()),
        Err(error) => failures.push(format!("current schema: {error}")),
    }
    match deserialize_exact::<LegacyStoredSignalRecord>(payload) {
        Ok(LegacyStoredSignalRecord::CounterfactualQuotePath(path)) => {
            return Ok(StoredSignalRecord::CounterfactualQuotePath(
                CounterfactualQuotePath {
                    symbol: path.symbol,
                    signal_timestamp: path.signal_timestamp,
                    horizon_seconds: path.horizon_seconds,
                    samples: path
                        .samples
                        .into_iter()
                        .map(|sample| CounterfactualQuoteSample {
                            timestamp: sample.timestamp,
                            bid_execution_price: depth_adjusted_quote_execution_price(
                                &sample.bid_levels,
                            )
                            .or_else(|| valid_quote_execution_price(sample.bid)),
                            ask_execution_price: depth_adjusted_quote_execution_price(
                                &sample.ask_levels,
                            )
                            .or_else(|| valid_quote_execution_price(sample.ask)),
                        })
                        .collect(),
                },
            ));
        }
        Ok(_) => failures.push("legacy schema decoded a different record kind".to_owned()),
        Err(error) => failures.push(format!("legacy schema: {error}")),
    }
    Err(anyhow::anyhow!(
        "unsupported kind 6 quote-path schema ({})",
        failures.join("; ")
    ))
}

fn deserialize_exact<T: serde::de::DeserializeOwned>(payload: &[u8]) -> Result<T> {
    use bincode::Options;

    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .deserialize(payload)
        .map_err(Into::into)
}

fn snapshot_from_extended(snapshot: ExtendedSignalSnapshot) -> SignalSnapshot {
    SignalSnapshot {
        symbol: snapshot.symbol,
        timestamp: snapshot.timestamp,
        direction: snapshot.direction,
        score: snapshot.score,
        strength: snapshot.strength,
        evidence_coverage: None,
        estimated_accuracy: None,
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
            ..SignalComponents::default()
        },
        reasons: snapshot.reasons,
        news_attribution: None,
        stale: snapshot.stale,
        ranging_probability: snapshot.ranging_probability,
        range_lower_bound: snapshot.range_lower_bound,
        range_upper_bound: snapshot.range_upper_bound,
    }
}

fn snapshot_from_legacy(snapshot: LegacySignalSnapshotWithoutNews) -> SignalSnapshot {
    SignalSnapshot {
        symbol: snapshot.symbol,
        timestamp: snapshot.timestamp,
        direction: snapshot.direction,
        score: snapshot.score,
        strength: snapshot.strength,
        components: snapshot.components,
        reasons: snapshot.reasons,
        news_attribution: None,
        stale: snapshot.stale,
        ranging_probability: snapshot.ranging_probability,
        range_lower_bound: snapshot.range_lower_bound,
        range_upper_bound: snapshot.range_upper_bound,
        evidence_coverage: None,
        estimated_accuracy: None,
    }
}

fn snapshot_from_scalar_legacy(snapshot: LegacySignalSnapshotWithoutVectors) -> SignalSnapshot {
    SignalSnapshot {
        symbol: snapshot.symbol,
        timestamp: snapshot.timestamp,
        direction: snapshot.direction,
        score: snapshot.score,
        strength: snapshot.strength,
        components: components_from_scalar_legacy(snapshot.components),
        reasons: snapshot.reasons,
        news_attribution: snapshot.news_attribution,
        stale: snapshot.stale,
        ranging_probability: Some(snapshot.ranging_probability),
        range_lower_bound: snapshot.range_lower_bound,
        range_upper_bound: snapshot.range_upper_bound,
        evidence_coverage: None,
        estimated_accuracy: None,
    }
}

fn snapshot_from_scalar_legacy_without_upper_range(
    snapshot: LegacySnapshotWithoutUpperRange,
) -> SignalSnapshot {
    SignalSnapshot {
        symbol: snapshot.symbol,
        timestamp: snapshot.timestamp,
        direction: snapshot.direction,
        score: snapshot.score,
        strength: snapshot.strength,
        components: components_from_scalar_legacy(snapshot.components),
        reasons: snapshot.reasons,
        news_attribution: snapshot.news_attribution,
        stale: snapshot.stale,
        ranging_probability: snapshot.ranging_probability,
        range_lower_bound: snapshot.range_lower_bound,
        range_upper_bound: None,
        evidence_coverage: None,
        estimated_accuracy: None,
    }
}

fn snapshot_from_scalar_legacy_with_optional_ranges(
    snapshot: LegacySignalSnapshotWithOptionalRanges,
) -> SignalSnapshot {
    SignalSnapshot {
        symbol: snapshot.symbol,
        timestamp: snapshot.timestamp,
        direction: snapshot.direction,
        score: snapshot.score,
        strength: snapshot.strength,
        components: components_from_scalar_legacy(snapshot.components),
        reasons: snapshot.reasons,
        news_attribution: snapshot.news_attribution,
        stale: snapshot.stale,
        ranging_probability: snapshot.ranging_probability,
        range_lower_bound: snapshot.range_lower_bound,
        range_upper_bound: snapshot.range_upper_bound,
        evidence_coverage: None,
        estimated_accuracy: None,
    }
}

fn components_from_scalar_legacy(
    components: LegacySignalComponentsWithoutVectors,
) -> SignalComponents {
    SignalComponents {
        trend_1m_percent: components.trend_1m_percent,
        momentum_10s_percent: components.momentum_10s_percent,
        trade_flow_imbalance: components.trade_flow_imbalance,
        book_imbalance: components.book_imbalance,
        volatility_1m_percent: components.volatility_1m_percent,
        trend_5m_percent: components.trend_5m_percent,
        trend_5m_volatility_units: components.trend_5m_volatility_units,
        mispricing_vs_vwap_bps: components.mispricing_vs_vwap_bps,
        pressure_without_price_move: components.pressure_without_price_move,
        aggressor_cost_bps: components.aggressor_cost_bps,
        ..SignalComponents::default()
    }
}

#[derive(Deserialize, Serialize)]
enum LegacySnapshotWithoutVectorsRecord {
    Snapshot(LegacySignalSnapshotWithoutVectors),
}

#[derive(Deserialize, Serialize)]
struct LegacySignalSnapshotWithoutVectors {
    symbol: String,
    timestamp: u64,
    direction: String,
    score: f64,
    strength: f64,
    components: LegacySignalComponentsWithoutVectors,
    reasons: Vec<String>,
    news_attribution: Option<NewsAttribution>,
    stale: bool,
    ranging_probability: f64,
    range_lower_bound: Option<f64>,
    range_upper_bound: Option<f64>,
}

#[derive(Deserialize, Serialize)]
enum LegacySnapshotWithOptionalRangesRecord {
    Snapshot(LegacySignalSnapshotWithOptionalRanges),
}

#[derive(Deserialize, Serialize)]
struct LegacySignalSnapshotWithOptionalRanges {
    symbol: String,
    timestamp: u64,
    direction: String,
    score: f64,
    strength: f64,
    components: LegacySignalComponentsWithoutVectors,
    reasons: Vec<String>,
    news_attribution: Option<NewsAttribution>,
    stale: bool,
    ranging_probability: Option<f64>,
    range_lower_bound: Option<f64>,
    range_upper_bound: Option<f64>,
}

#[derive(Deserialize, Serialize)]
enum LegacySnapshotWithoutUpperRangeRecord {
    Snapshot(LegacySnapshotWithoutUpperRange),
}

#[derive(Deserialize, Serialize)]
struct LegacySnapshotWithoutUpperRange {
    symbol: String,
    timestamp: u64,
    direction: String,
    score: f64,
    strength: f64,
    components: LegacySignalComponentsWithoutVectors,
    reasons: Vec<String>,
    news_attribution: Option<NewsAttribution>,
    stale: bool,
    ranging_probability: Option<f64>,
    range_lower_bound: Option<f64>,
}

#[derive(Deserialize, Serialize)]
struct LegacySignalComponentsWithoutVectors {
    trend_1m_percent: Option<f64>,
    momentum_10s_percent: Option<f64>,
    trade_flow_imbalance: Option<f64>,
    book_imbalance: Option<f64>,
    volatility_1m_percent: Option<f64>,
    trend_5m_percent: Option<f64>,
    trend_5m_volatility_units: Option<f64>,
    mispricing_vs_vwap_bps: Option<f64>,
    pressure_without_price_move: Option<f64>,
    aggressor_cost_bps: Option<f64>,
}

#[derive(Deserialize, Serialize)]
enum LegacySnapshotWithoutNewsRecord {
    Snapshot(LegacySignalSnapshotWithoutNews),
}

#[derive(Deserialize, Serialize)]
struct LegacySignalSnapshotWithoutNews {
    symbol: String,
    timestamp: u64,
    direction: String,
    score: f64,
    strength: f64,
    components: SignalComponents,
    reasons: Vec<String>,
    stale: bool,
    ranging_probability: Option<f64>,
    range_lower_bound: Option<f64>,
    range_upper_bound: Option<f64>,
}

#[derive(Deserialize)]
enum LegacyStoredSignalRecord {
    Snapshot,
    Outcome,
    Evaluation,
    BookSnapshot,
    ResearchFeatures,
    ExternalMarket,
    CounterfactualQuotePath(LegacyCounterfactualQuotePath),
}

#[derive(Deserialize)]
struct LegacyCounterfactualQuotePath {
    symbol: String,
    signal_timestamp: u64,
    horizon_seconds: u64,
    samples: Vec<LegacyCounterfactualQuoteSample>,
}

#[derive(Deserialize)]
struct LegacyCounterfactualQuoteSample {
    timestamp: u64,
    bid: f64,
    ask: f64,
    bid_levels: Vec<(f64, f64)>,
    ask_levels: Vec<(f64, f64)>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
enum LegacyShiftedSnapshotRecord {
    Outcome,
    Evaluation,
    BookSnapshot,
    ResearchFeatures,
    ExternalMarket,
    CounterfactualQuotePath,
    Snapshot(ExtendedSignalSnapshot),
}

fn valid_quote_execution_price(price: f64) -> Option<f64> {
    (price.is_finite() && price > 0.0).then_some(price)
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
        StoredSignalRecord::CounterfactualQuotePath(_) => 6,
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
        StoredSignalRecord::CounterfactualQuotePath(path) => path.signal_timestamp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_keeps_raw_strength_separate_from_estimated_accuracy() {
        let mut snapshot = SignalSnapshot {
            symbol: "BTC-USDT".into(),
            timestamp: 10_000,
            direction: "bullish".into(),
            score: 0.75,
            strength: 0.75 * 4.0 / 9.0,
            evidence_coverage: Some(4.0 / 9.0),
            estimated_accuracy: None,
            components: SignalComponents::default(),
            reasons: Vec::new(),
            news_attribution: None,
            stale: false,
            ranging_probability: None,
            range_lower_bound: None,
            range_upper_bound: None,
        };
        let records = vec![StoredSignalRecord::Snapshot(snapshot.clone())];

        calibrate_signal_scores(std::slice::from_mut(&mut snapshot), &records);

        assert!((snapshot.strength - 0.75 * 4.0 / 9.0).abs() < 1e-12);
        assert_eq!(snapshot.estimated_accuracy, Some(0.5));
    }

    #[test]
    fn legacy_bincode_snapshots_default_new_grading_fields() {
        let payload =
            bincode::serialize(&ExtendedSnapshotRecord::Snapshot(ExtendedSignalSnapshot {
                symbol: "BTC-USDT".into(),
                timestamp: 10_000,
                direction: "bullish".into(),
                score: 0.75,
                strength: 0.4,
                components: ExtendedSignalComponents {
                    trend_1m_percent: None,
                    momentum_10s_percent: None,
                    trade_flow_imbalance: None,
                    book_imbalance: None,
                    volatility_1m_percent: None,
                    trend_5m_percent: None,
                    trend_5m_volatility_units: None,
                    mispricing_vs_vwap_bps: None,
                    pressure_without_price_move: None,
                    aggressor_cost_bps: None,
                    absorption_imbalance: None,
                },
                reasons: Vec::new(),
                stale: false,
                ranging_probability: None,
                range_lower_bound: None,
                range_upper_bound: None,
            }))
            .unwrap();

        let StoredSignalRecord::Snapshot(snapshot) = deserialize_record(0, &payload).unwrap()
        else {
            panic!("legacy payload should decode as a signal snapshot");
        };

        assert_eq!(snapshot.strength, 0.4);
        assert_eq!(snapshot.evidence_coverage, None);
        assert_eq!(snapshot.estimated_accuracy, None);
    }

    #[test]
    fn historical_snapshot_without_news_decodes_current_component_vectors() {
        let legacy = LegacySnapshotWithoutNewsRecord::Snapshot(LegacySignalSnapshotWithoutNews {
            symbol: "BTC-USDT".into(),
            timestamp: 10_000,
            direction: "bullish".into(),
            score: 0.75,
            strength: 0.4,
            components: SignalComponents {
                momentum_by_window: (0..6)
                    .map(|window_seconds| WindowedSignalValue {
                        window_seconds,
                        value: Some(0.1),
                        sample_count: 2,
                        span_ms: 1_000,
                    })
                    .collect(),
                ..SignalComponents::default()
            },
            reasons: vec!["historical signal".into()],
            stale: false,
            ranging_probability: Some(0.2),
            range_lower_bound: Some(99.0),
            range_upper_bound: Some(101.0),
        });
        let payload = bincode::serialize(&legacy).unwrap();

        let StoredSignalRecord::Snapshot(snapshot) = deserialize_record(0, &payload).unwrap()
        else {
            panic!("historical payload should decode as a signal snapshot");
        };

        assert_eq!(snapshot.symbol, "BTC-USDT");
        assert_eq!(snapshot.components.momentum_by_window.len(), 6);
        assert_eq!(snapshot.reasons, vec!["historical signal"]);
        assert!(snapshot.news_attribution.is_none());
        assert_eq!(snapshot.evidence_coverage, None);
        assert_eq!(snapshot.estimated_accuracy, None);
    }

    #[test]
    fn historical_scalar_snapshot_decodes_without_windowed_features() {
        let legacy = LegacySnapshotWithoutVectorsRecord::Snapshot(
            LegacySignalSnapshotWithoutVectors {
                symbol: "BTC-USDT".into(),
                timestamp: 10_000,
                direction: "neutral".into(),
                score: 0.4,
                strength: 0.3,
                components: LegacySignalComponentsWithoutVectors {
                    trend_1m_percent: Some(0.1),
                    momentum_10s_percent: Some(0.2),
                    trade_flow_imbalance: None,
                    book_imbalance: Some(-0.2),
                    volatility_1m_percent: Some(0.3),
                    trend_5m_percent: Some(0.4),
                    trend_5m_volatility_units: Some(0.5),
                    mispricing_vs_vwap_bps: Some(0.6),
                    pressure_without_price_move: Some(0.7),
                    aggressor_cost_bps: Some(0.8),
                },
                reasons: vec!["historical reason".into()],
                news_attribution: None,
                stale: true,
                ranging_probability: 0.35,
                range_lower_bound: Some(99.0),
                range_upper_bound: Some(101.0),
            },
        );
        let payload = bincode::serialize(&legacy).unwrap();

        let StoredSignalRecord::Snapshot(snapshot) = deserialize_record(0, &payload).unwrap()
        else {
            panic!("historical payload should decode as a signal snapshot");
        };

        assert_eq!(snapshot.symbol, "BTC-USDT");
        assert_eq!(
            snapshot.components.pressure_without_price_move,
            Some(0.7)
        );
        assert!(snapshot.components.momentum_by_window.is_empty());
        assert!(snapshot.news_attribution.is_none());
        assert_eq!(snapshot.ranging_probability, Some(0.35));
        assert_eq!(snapshot.range_lower_bound, Some(99.0));
        assert_eq!(snapshot.range_upper_bound, Some(101.0));
        assert!(snapshot.evidence_coverage.is_none());
        assert!(snapshot.estimated_accuracy.is_none());
    }

    #[test]
    fn historical_snapshot_without_upper_range_defaults_that_field() {
        let legacy = LegacySnapshotWithoutUpperRangeRecord::Snapshot(
            LegacySnapshotWithoutUpperRange {
                symbol: "ETH-USDT".into(),
                timestamp: 20_000,
                direction: "neutral".into(),
                score: 0.2,
                strength: 0.1,
                components: LegacySignalComponentsWithoutVectors {
                    trend_1m_percent: Some(0.1),
                    momentum_10s_percent: None,
                    trade_flow_imbalance: None,
                    book_imbalance: None,
                    volatility_1m_percent: Some(0.2),
                    trend_5m_percent: None,
                    trend_5m_volatility_units: None,
                    mispricing_vs_vwap_bps: None,
                    pressure_without_price_move: None,
                    aggressor_cost_bps: None,
                },
                reasons: Vec::new(),
                news_attribution: None,
                stale: false,
                ranging_probability: None,
                range_lower_bound: None,
            },
        );
        let payload = bincode::serialize(&legacy).unwrap();

        let StoredSignalRecord::Snapshot(snapshot) = deserialize_record(0, &payload).unwrap()
        else {
            panic!("historical payload should decode as a signal snapshot");
        };

        assert_eq!(snapshot.symbol, "ETH-USDT");
        assert_eq!(snapshot.components.trend_1m_percent, Some(0.1));
        assert!(snapshot.components.momentum_by_window.is_empty());
        assert!(snapshot.ranging_probability.is_none());
        assert!(snapshot.range_lower_bound.is_none());
        assert!(snapshot.range_upper_bound.is_none());
    }

    #[test]
    fn historical_optional_range_snapshot_defaults_new_grading_fields() {
        let legacy = LegacySnapshotWithOptionalRangesRecord::Snapshot(
            LegacySignalSnapshotWithOptionalRanges {
                symbol: "ETH-USDT".into(),
                timestamp: 30_000,
                direction: "neutral".into(),
                score: 0.2,
                strength: 0.1,
                components: LegacySignalComponentsWithoutVectors {
                    trend_1m_percent: None,
                    momentum_10s_percent: Some(0.1),
                    trade_flow_imbalance: None,
                    book_imbalance: None,
                    volatility_1m_percent: None,
                    trend_5m_percent: None,
                    trend_5m_volatility_units: None,
                    mispricing_vs_vwap_bps: None,
                    pressure_without_price_move: None,
                    aggressor_cost_bps: None,
                },
                reasons: Vec::new(),
                news_attribution: None,
                stale: false,
                ranging_probability: None,
                range_lower_bound: None,
                range_upper_bound: None,
            },
        );
        let payload = bincode::serialize(&legacy).unwrap();

        let StoredSignalRecord::Snapshot(snapshot) = deserialize_record(0, &payload).unwrap()
        else {
            panic!("historical payload should decode as a signal snapshot");
        };

        assert_eq!(snapshot.symbol, "ETH-USDT");
        assert_eq!(snapshot.components.momentum_10s_percent, Some(0.1));
        assert!(snapshot.ranging_probability.is_none());
        assert!(snapshot.range_lower_bound.is_none());
        assert!(snapshot.range_upper_bound.is_none());
        assert!(snapshot.evidence_coverage.is_none());
        assert!(snapshot.estimated_accuracy.is_none());
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
            | StoredSignalRecord::ExternalMarket(_)
            | StoredSignalRecord::CounterfactualQuotePath(_) => {}
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
        snapshot.estimated_accuracy = Some(if snapshot.direction == "ranging" {
            estimate(&accuracies, snapshot)
        } else {
            estimate_point_in_time(&outcomes_by_key, snapshot)
        });

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
        snapshot.refresh_raw_strength();
    }
}

#[cfg(test)]
mod counterfactual_quote_path_record_tests {
    use super::*;

    #[allow(dead_code)]
    #[derive(Serialize)]
    enum LegacyStoredSignalRecordForTest {
        Snapshot,
        Outcome,
        Evaluation,
        BookSnapshot,
        ResearchFeatures,
        ExternalMarket,
        CounterfactualQuotePath(LegacyCounterfactualQuotePathForTest),
    }

    #[derive(Serialize)]
    struct LegacyCounterfactualQuotePathForTest {
        symbol: String,
        signal_timestamp: u64,
        horizon_seconds: u64,
        samples: Vec<LegacyCounterfactualQuoteSampleForTest>,
    }

    #[derive(Serialize)]
    struct LegacyCounterfactualQuoteSampleForTest {
        timestamp: u64,
        bid: f64,
        ask: f64,
        bid_levels: Vec<(f64, f64)>,
        ask_levels: Vec<(f64, f64)>,
    }

    #[allow(dead_code)]
    #[derive(Serialize)]
    enum LegacyShiftedSnapshotRecordForTest {
        Outcome,
        Evaluation,
        BookSnapshot,
        ResearchFeatures,
        ExternalMarket,
        CounterfactualQuotePath,
        Snapshot(ExtendedSignalSnapshot),
    }

    #[test]
    fn legacy_quote_paths_decode_with_depth_adjusted_execution_prices() {
        let legacy = LegacyStoredSignalRecordForTest::CounterfactualQuotePath(
            LegacyCounterfactualQuotePathForTest {
                symbol: "BTC-USDT".into(),
                signal_timestamp: 10_000,
                horizon_seconds: 300,
                samples: vec![LegacyCounterfactualQuoteSampleForTest {
                    timestamp: 10_000,
                    bid: 99.9,
                    ask: 100.1,
                    bid_levels: vec![(100.0, 0.2), (99.5, 9.8)],
                    ask_levels: vec![(101.0, 0.2), (101.5, 9.8)],
                }],
            },
        );
        let payload = bincode::serialize(&legacy).unwrap();
        let StoredSignalRecord::CounterfactualQuotePath(path) =
            deserialize_record(6, &payload).unwrap()
        else {
            panic!("expected counterfactual quote path");
        };
        assert!((path.samples[0].bid_execution_price.unwrap() - 99.6).abs() < 1e-10);
        assert!((path.samples[0].ask_execution_price.unwrap() - 101.4).abs() < 1e-10);
    }

    #[test]
    fn legacy_kind_zero_snapshot_with_shifted_enum_tag_decodes() {
        let legacy = LegacyShiftedSnapshotRecordForTest::Snapshot(ExtendedSignalSnapshot {
            symbol: "BTC-USDT".into(),
            timestamp: 10_000,
            direction: "bullish".into(),
            score: 0.75,
            strength: 0.4,
            components: ExtendedSignalComponents {
                trend_1m_percent: None,
                momentum_10s_percent: None,
                trade_flow_imbalance: None,
                book_imbalance: None,
                volatility_1m_percent: None,
                trend_5m_percent: None,
                trend_5m_volatility_units: None,
                mispricing_vs_vwap_bps: None,
                pressure_without_price_move: None,
                aggressor_cost_bps: None,
                absorption_imbalance: None,
            },
            reasons: Vec::new(),
            stale: false,
            ranging_probability: None,
            range_lower_bound: None,
            range_upper_bound: None,
        });
        let payload = bincode::serialize(&legacy).unwrap();
        let StoredSignalRecord::Snapshot(snapshot) = deserialize_record(0, &payload).unwrap()
        else {
            panic!("expected signal snapshot");
        };
        assert_eq!(snapshot.symbol, "BTC-USDT");
        assert_eq!(snapshot.evidence_coverage, None);
        assert_eq!(snapshot.estimated_accuracy, None);
    }
}
