const MAX_SCOUT_PATH_SAMPLES_PER_WRITE: usize = 32_768;

fn valid_scout_path_sample(sample: &ScoutPathSample) -> bool {
    let sqlite_safe = |value: u64| value <= i64::MAX as u64;
    !sample.decision_id.trim().is_empty()
        && sample.decision_id.len() <= 240
        && !sample.symbol.trim().is_empty()
        && sample.symbol.len() <= 32
        && sample.path_epoch > 0
        && sqlite_safe(sample.path_epoch)
        && sample.sequence > 0
        && sqlite_safe(sample.sequence)
        && sample.timestamp > 0
        && sqlite_safe(sample.timestamp)
        && sample.freshness_ms <= i64::MAX as u64
        && sample.gap_ms <= i64::MAX as u64
        && sample.first_source_timestamp.is_none_or(sqlite_safe)
        && sample
            .first_source_timestamp
            .is_none_or(|first| first <= sample.timestamp)
        && sample.maximum_gap_ms >= sample.gap_ms
        && sample.maximum_gap_ms <= i64::MAX as u64
        && sample.sample_count >= sample.sequence
        && sample.sample_count <= i64::MAX as u64
        && sample.bid.is_finite()
        && sample.ask.is_finite()
        && sample.bid > 0.0
        && sample.ask >= sample.bid
        && sample
            .maximum_favorable_excursion_percent
            .is_none_or(f64::is_finite)
        && sample
            .maximum_adverse_excursion_percent
            .is_none_or(f64::is_finite)
}

fn recovered_scout_path_is_complete(
    decision_id: &str,
    symbol: &str,
    registered_sample_count: u64,
    truncated: bool,
    capture_complete: bool,
    samples: &[ScoutPathSample],
) -> bool {
    if !capture_complete
        || samples.iter().any(|sample| {
            !valid_scout_path_sample(sample)
                || sample.decision_id != decision_id
                || sample.symbol != symbol
                || !sample.capture_complete
        })
    {
        return false;
    }
    let Some(first) = samples.first() else {
        return registered_sample_count == 0 || truncated;
    };
    if !truncated && (first.sequence != 1 || first.sample_count != 1) {
        return false;
    }
    for pair in samples.windows(2) {
        let (previous, current) = (&pair[0], &pair[1]);
        if current.path_epoch < previous.path_epoch
            || current.sequence != previous.sequence.saturating_add(1)
            || current.sample_count != previous.sample_count.saturating_add(1)
            || current.timestamp < previous.timestamp
            || current.gap_ms != current.timestamp.saturating_sub(previous.timestamp)
        {
            return false;
        }
    }
    samples.last().is_some_and(|last| {
        last.sample_count == registered_sample_count
            && (truncated || samples.len() as u64 == registered_sample_count)
    })
}

impl SignalStore {
    pub async fn register_scout_path(
        &self,
        decision_id: String,
        symbol: String,
        signal_timestamp: u64,
        direction: String,
        entry_price: f64,
        entry_quote: Option<(f64, f64, u64)>,
    ) -> Result<(
        Vec<ScoutPathSample>,
        bool,
        bool,
        Option<u64>,
        u64,
        Option<(f64, f64, u64)>,
    )> {
        if decision_id.trim().is_empty()
            || decision_id.len() > 240
            || symbol.trim().is_empty()
            || symbol.len() > 32
            || !matches!(direction.as_str(), "bullish" | "bearish")
            || signal_timestamp == 0
            || signal_timestamp > i64::MAX as u64
            || !entry_price.is_finite()
            || entry_price <= 0.0
            || entry_quote.is_some_and(|(bid, ask, timestamp)| {
                !bid.is_finite()
                    || !ask.is_finite()
                    || bid <= 0.0
                    || ask < bid
                    || timestamp == 0
                    || timestamp > signal_timestamp
                    || timestamp > i64::MAX as u64
            })
        {
            anyhow::bail!("invalid Scout path registration identity or entry quote");
        }
        self.send_write(WriteOperation::ScoutRegistration {
            decision_id: decision_id.clone(),
            symbol,
            signal_timestamp,
            direction,
            entry_price,
            entry_bid: entry_quote.map(|quote| quote.0),
            entry_ask: entry_quote.map(|quote| quote.1),
            entry_quote_timestamp: entry_quote.map(|quote| quote.2),
        })
        .await?;
        let restored = self.load_scout_path(&decision_id).await?;
        if !restored.2 {
            self.mark_scout_path_capture_failed(vec![decision_id])
                .await?;
        }
        Ok(restored)
    }

    pub async fn persist_scout_path_samples(&self, samples: Vec<ScoutPathSample>) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        if samples.len() > MAX_SCOUT_PATH_SAMPLES_PER_WRITE
            || samples
                .iter()
                .any(|sample| !valid_scout_path_sample(sample))
        {
            let decision_ids = samples
                .iter()
                .filter(|sample| {
                    !sample.decision_id.trim().is_empty() && sample.decision_id.len() <= 240
                })
                .map(|sample| sample.decision_id.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .take(MAX_SCOUT_PATH_SAMPLES_PER_WRITE)
                .collect::<Vec<_>>();
            self.mark_scout_path_capture_failed(decision_ids).await?;
            anyhow::bail!("invalid or oversized Scout path sample batch");
        }
        self.send_write(WriteOperation::ScoutPathSamples(samples))
            .await
    }

    pub async fn complete_scout_path(&self, decision_id: String) -> Result<()> {
        if decision_id.trim().is_empty() || decision_id.len() > 240 {
            anyhow::bail!("invalid Scout path completion identity");
        }
        self.send_write(WriteOperation::ScoutPathComplete(decision_id))
            .await
    }

    pub async fn mark_scout_path_capture_failed(&self, decision_ids: Vec<String>) -> Result<()> {
        if decision_ids.is_empty() {
            return Ok(());
        }
        if decision_ids.len() > MAX_SCOUT_PATH_SAMPLES_PER_WRITE
            || decision_ids
                .iter()
                .any(|id| id.trim().is_empty() || id.len() > 240)
        {
            anyhow::bail!("invalid or oversized Scout path failure marker batch");
        }
        self.send_write(WriteOperation::ScoutPathFailure(decision_ids))
            .await
    }

    async fn load_scout_path(
        &self,
        decision_id: &str,
    ) -> Result<(
        Vec<ScoutPathSample>,
        bool,
        bool,
        Option<u64>,
        u64,
        Option<(f64, f64, u64)>,
    )> {
        let path = self.path.clone();
        let decision_id = decision_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let connection = Self::open_database_path(&path)?;
            let (symbol, signal_timestamp, capture_complete, truncated, sample_count, entry_bid, entry_ask, quote_timestamp): (String, i64, i64, i64, i64, Option<f64>, Option<f64>, Option<i64>) = connection.query_row(
                "SELECT symbol, signal_timestamp, capture_complete, truncated, sample_count, entry_bid, entry_ask, entry_quote_timestamp FROM scout_path_registrations WHERE decision_id=?1",
                [&decision_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )?;
            let mut statement = connection.prepare(
                "SELECT registrations.symbol, samples.path_epoch, samples.sequence, samples.timestamp, samples.bid, samples.ask, samples.freshness_ms, samples.gap_ms, samples.first_source_timestamp, samples.maximum_gap_ms, samples.source_timestamps_monotonic, samples.maximum_favorable_excursion_percent, samples.maximum_adverse_excursion_percent, samples.sample_count, samples.capture_complete FROM scout_path_samples AS samples JOIN scout_path_registrations AS registrations ON registrations.decision_id=samples.decision_id WHERE samples.decision_id=?1 ORDER BY samples.path_epoch, samples.sequence LIMIT 4096",
            )?;
            let rows = statement.query_map([&decision_id], |row| {
                Ok(ScoutPathSample {
                    decision_id: decision_id.clone(),
                    symbol: row.get(0)?,
                    path_epoch: row.get::<_, i64>(1)? as u64,
                    sequence: row.get::<_, i64>(2)? as u64,
                    timestamp: row.get::<_, i64>(3)? as u64,
                    bid: row.get(4)?,
                    ask: row.get(5)?,
                    freshness_ms: row.get::<_, i64>(6)? as u64,
                    gap_ms: row.get::<_, i64>(7)? as u64,
                    first_source_timestamp: row.get::<_, Option<i64>>(8)?.map(|value| value as u64),
                    maximum_gap_ms: row.get::<_, i64>(9)? as u64,
                    source_timestamps_monotonic: row.get(10)?,
                    maximum_favorable_excursion_percent: row.get(11)?,
                    maximum_adverse_excursion_percent: row.get(12)?,
                    sample_count: row.get::<_, i64>(13)? as u64,
                    capture_complete: row.get::<_, i64>(14)? == 1,
                })
            })?;
            let samples = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            let freshness = samples.last().map(|sample| sample.freshness_ms);
            let entry_quote = entry_bid.zip(entry_ask).zip(quote_timestamp.map(|time| time as u64)).map(|((bid, ask), time)| (bid, ask, time));
            let truncated = truncated != 0;
            let entry_quote_valid = match (entry_bid, entry_ask, quote_timestamp) {
                (None, None, None) => true,
                (Some(bid), Some(ask), Some(timestamp)) => {
                    bid.is_finite()
                        && ask.is_finite()
                        && bid > 0.0
                        && ask >= bid
                        && timestamp > 0
                        && timestamp <= signal_timestamp
                }
                _ => false,
            };
            let capture_complete = sample_count >= 0
                && signal_timestamp > 0
                && entry_quote_valid
                && recovered_scout_path_is_complete(
                    &decision_id,
                    &symbol,
                    sample_count.max(0) as u64,
                    truncated,
                capture_complete == 1,
                    &samples,
                );
            Ok((
                samples,
                truncated,
                capture_complete,
                freshness,
                sample_count.max(0) as u64,
                entry_quote,
            ))
        })
        .await
        .map_err(|error| anyhow::anyhow!("Scout path read task failed: {error}"))?
        .map(|(samples, truncated, complete, freshness, sample_count, entry_quote)| {
            // Recovery always crosses a process boundary; do not claim a path
            // was fully captured, even if every queued write was acknowledged.
            (samples, truncated, complete, freshness, sample_count, entry_quote)
        })
    }

    pub async fn append(&self, snapshot: &SignalSnapshot) -> Result<()> {
        self.append_record(StoredSignalRecord::Snapshot(snapshot.clone()))
            .await
    }

    pub async fn append_outcome(&self, outcome: &SignalOutcome) -> Result<()> {
        self.append_record(StoredSignalRecord::Outcome(outcome.clone()))
            .await
    }

    pub async fn append_evaluation(&self, evaluation: &SignalEvaluation) -> Result<()> {
        self.append_records(evaluation_records(evaluation)).await
    }

    async fn append_record(&self, record: StoredSignalRecord) -> Result<()> {
        self.append_records(vec![record]).await
    }

    pub async fn append_records(&self, records: Vec<StoredSignalRecord>) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        self.send_write(WriteOperation::Records(records)).await
    }

    async fn send_write(&self, operation: WriteOperation) -> Result<()> {
        let (ack, result) = oneshot::channel();
        self.writer
            .send(WriteRequest { operation, ack })
            .await
            .map_err(|_| anyhow::anyhow!("signal database writer stopped"))?;
        result
            .await
            .map_err(|_| anyhow::anyhow!("signal database writer dropped acknowledgement"))?
            .map_err(|error| anyhow::anyhow!(error))
    }
}

fn evaluation_records(evaluation: &SignalEvaluation) -> Vec<StoredSignalRecord> {
    vec![StoredSignalRecord::Evaluation(evaluation.clone())]
}

#[cfg(test)]
mod evaluation_write_tests {
    use super::*;

    #[test]
    fn appending_evaluation_does_not_generate_a_quote_path_record() {
        let evaluation = SignalEvaluation {
            symbol: "BTC-USDT".into(),
            signal_timestamp: 10_000,
            evaluated_timestamp: 11_000,
            horizon_seconds: 1,
            signal_direction: "bullish".into(),
            signal_score: 0.5,
            ranging_probability: None,
            reference_entry_price: 100.0,
            reference_exit_price: 101.0,
            entry_bid: Some(99.9),
            entry_ask: Some(100.1),
            entry_quote_timestamp: Some(10_000),
            exit_bid: Some(100.9),
            exit_ask: Some(101.1),
            exit_quote_timestamp: Some(11_000),
            taker_fee_rate: 0.001,
            long_gross_percent: 1.0,
            short_gross_percent: -1.0,
            long_net_percent: Some(0.8),
            short_net_percent: Some(-1.2),
            path_high: 101.0,
            path_high_timestamp: 11_000,
            path_low: 100.0,
            path_low_timestamp: 10_000,
            long_time_to_positive_net_seconds: None,
            short_time_to_positive_net_seconds: None,
            scout_path_evidence: None,
        };

        let records = evaluation_records(&evaluation);
        assert_eq!(records.len(), 1);
        assert!(matches!(
            records.as_slice(),
            [StoredSignalRecord::Evaluation(_)]
        ));
    }
}

#[cfg(test)]
mod scout_path_persistence_tests {
    use super::*;

    fn test_store(path: PathBuf) -> SignalStore {
        let connection = SignalStore::open_database_at(&path).unwrap();
        let (writer, receiver) = mpsc::channel(64);
        tokio::task::spawn_blocking(move || run_database_writer(connection, receiver));
        let audit_path = audit_database_path(&path);
        let audit_connection = SignalStore::open_audit_database_at(&audit_path).unwrap();
        let (audit_writer, audit_receiver) = mpsc::channel(8);
        tokio::task::spawn_blocking(move || run_database_writer(audit_connection, audit_receiver));
        SignalStore {
            path,
            audit_path,
            writer,
            audit_writer,
            recent_cache: Arc::new(Mutex::new(None)),
            best_cache: Arc::new(Mutex::new(None)),
            performance_cache: Arc::new(Mutex::new(None)),
            execution_quality_cache: Arc::new(Mutex::new(None)),
            decision_quality_cache: Arc::new(Mutex::new(None)),
            calibration_records_cache: Arc::new(Mutex::new(None)),
        }
    }

    #[tokio::test]
    async fn scout_path_recovery_is_idempotent_and_bounded() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path_timestamp = (unique / 1_000_000) as u64;
        let path = std::env::temp_dir().join(format!("insights-scout-path-{unique}.sqlite"));
        let store = test_store(path.clone());
        let entry_quote = Some((99.9, 100.1, path_timestamp));
        store
            .register_scout_path(
                "scout-1".into(),
                "BTC-USDT".into(),
                path_timestamp,
                "bullish".into(),
                100.0,
                entry_quote,
            )
            .await
            .unwrap();
        let samples = (1..=4_100)
            .map(|sequence| ScoutPathSample {
                decision_id: "scout-1".into(),
                symbol: "BTC-USDT".into(),
                path_epoch: 100,
                sequence,
                timestamp: path_timestamp + sequence,
                bid: 99.9 + sequence as f64 / 10_000.0,
                ask: 100.1 + sequence as f64 / 10_000.0,
                freshness_ms: 2,
                gap_ms: 1,
                first_source_timestamp: Some(path_timestamp + 1),
                maximum_gap_ms: 1,
                source_timestamps_monotonic: true,
                maximum_favorable_excursion_percent: Some(0.01),
                maximum_adverse_excursion_percent: Some(0.0),
                sample_count: sequence,
                capture_complete: true,
            })
            .collect::<Vec<_>>();
        store
            .persist_scout_path_samples(samples.clone())
            .await
            .unwrap();
        store
            .persist_scout_path_samples(vec![samples[4_099].clone()])
            .await
            .unwrap();
        drop(store);
        tokio::task::yield_now().await;
        let restarted_store = test_store(path.clone());
        let (restored, truncated, complete, freshness, sample_count, restored_entry_quote) =
            restarted_store
            .register_scout_path(
                "scout-1".into(),
                "BTC-USDT".into(),
                path_timestamp,
                "bullish".into(),
                100.0,
                None,
            )
            .await
            .unwrap();
        assert_eq!(restored.len(), 4_096);
        assert!(truncated);
        assert!(complete);
        assert_eq!(sample_count, 4_100);
        assert_eq!(freshness, Some(2));
        assert_eq!(restored_entry_quote, entry_quote);
        assert_eq!(restored.first().unwrap().sequence, 5);
        assert_eq!(restored.last().unwrap().sequence, 4_100);
        assert!(restored.iter().all(|sample| sample.symbol == "BTC-USDT"));
        restarted_store
            .mark_scout_path_capture_failed(vec!["scout-1".into()])
            .await
            .unwrap();
        let (_, _, complete_after_failure, _, _, _) = restarted_store
            .register_scout_path(
                "scout-1".into(),
                "BTC-USDT".into(),
                10_000,
                "bullish".into(),
                100.0,
                None,
            )
            .await
            .unwrap();
        assert!(!complete_after_failure);
    }

    #[tokio::test]
    async fn malformed_registered_symbol_marks_partial_path_incomplete_without_storing_sample() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let timestamp = (unique / 1_000_000) as u64;
        let path =
            std::env::temp_dir().join(format!("insights-scout-path-identity-{unique}.sqlite"));
        let store = test_store(path);
        let entry_quote = Some((99.9, 100.1, timestamp));
        store
            .register_scout_path(
                "identity-check".into(),
                "BTC-USDT".into(),
                timestamp,
                "bullish".into(),
                100.0,
                entry_quote,
            )
            .await
            .unwrap();
        let malformed = ScoutPathSample {
            decision_id: "identity-check".into(),
            symbol: "ETH-USDT".into(),
            path_epoch: 1,
            sequence: 1,
            timestamp: timestamp + 1,
            bid: 99.9,
            ask: 100.1,
            freshness_ms: 1,
            gap_ms: 0,
            first_source_timestamp: Some(timestamp + 1),
            maximum_gap_ms: 0,
            source_timestamps_monotonic: true,
            maximum_favorable_excursion_percent: Some(0.0),
            maximum_adverse_excursion_percent: Some(0.0),
            sample_count: 1,
            capture_complete: true,
        };

        assert!(
            store
                .persist_scout_path_samples(vec![malformed])
                .await
                .is_err()
        );
        let (samples, _, capture_complete, _, _, _) = store
            .register_scout_path(
                "identity-check".into(),
                "BTC-USDT".into(),
                timestamp,
                "bullish".into(),
                100.0,
                None,
            )
            .await
            .unwrap();
        assert!(samples.is_empty());
        assert!(!capture_complete);
    }

    #[tokio::test]
    async fn conflicting_duplicate_sequence_fails_closed() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let timestamp = (unique / 1_000_000) as u64;
        let store = test_store(
            std::env::temp_dir().join(format!("insights-scout-path-conflict-{unique}.sqlite")),
        );
        store
            .register_scout_path(
                "duplicate-check".into(),
                "BTC-USDT".into(),
                timestamp,
                "bullish".into(),
                100.0,
                Some((99.9, 100.1, timestamp)),
            )
            .await
            .unwrap();
        let sample = ScoutPathSample {
            decision_id: "duplicate-check".into(),
            symbol: "BTC-USDT".into(),
            path_epoch: 42,
            sequence: 1,
            timestamp: timestamp + 1,
            bid: 99.9,
            ask: 100.1,
            freshness_ms: 1,
            gap_ms: 0,
            first_source_timestamp: Some(timestamp + 1),
            maximum_gap_ms: 0,
            source_timestamps_monotonic: true,
            maximum_favorable_excursion_percent: Some(0.0),
            maximum_adverse_excursion_percent: Some(0.0),
            sample_count: 1,
            capture_complete: true,
        };
        store
            .persist_scout_path_samples(vec![sample.clone()])
            .await
            .unwrap();
        let mut conflicting = sample;
        conflicting.bid = 99.8;
        assert!(
            store
                .persist_scout_path_samples(vec![conflicting])
                .await
                .is_err()
        );
        let (_, _, capture_complete, _, _, _) = store
            .register_scout_path(
                "duplicate-check".into(),
                "BTC-USDT".into(),
                timestamp,
                "bullish".into(),
                100.0,
                None,
            )
            .await
            .unwrap();
        assert!(!capture_complete);
    }

    #[tokio::test]
    async fn malformed_durable_epoch_is_not_restored_as_a_complete_path() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let timestamp = (unique / 1_000_000) as u64;
        let path =
            std::env::temp_dir().join(format!("insights-scout-path-corrupt-{unique}.sqlite"));
        let store = test_store(path.clone());
        store
            .register_scout_path(
                "corrupt-check".into(),
                "BTC-USDT".into(),
                timestamp,
                "bullish".into(),
                100.0,
                Some((99.9, 100.1, timestamp)),
            )
            .await
            .unwrap();
        let sample = ScoutPathSample {
            decision_id: "corrupt-check".into(),
            symbol: "BTC-USDT".into(),
            path_epoch: 11,
            sequence: 1,
            timestamp: timestamp + 1,
            bid: 99.9,
            ask: 100.1,
            freshness_ms: 1,
            gap_ms: 0,
            first_source_timestamp: Some(timestamp + 1),
            maximum_gap_ms: 0,
            source_timestamps_monotonic: true,
            maximum_favorable_excursion_percent: Some(0.0),
            maximum_adverse_excursion_percent: Some(0.0),
            sample_count: 1,
            capture_complete: true,
        };
        store
            .persist_scout_path_samples(vec![sample])
            .await
            .unwrap();
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE scout_path_samples SET path_epoch=0 WHERE decision_id='corrupt-check'",
                [],
            )
            .unwrap();

        let (samples, _, capture_complete, _, _, _) = store
            .register_scout_path(
                "corrupt-check".into(),
                "BTC-USDT".into(),
                timestamp,
                "bullish".into(),
                100.0,
                None,
            )
            .await
            .unwrap();
        assert_eq!(samples.len(), 1);
        assert!(!capture_complete);
    }
}
