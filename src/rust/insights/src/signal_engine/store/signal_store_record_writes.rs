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
        self.load_scout_path(&decision_id).await
    }

    pub async fn persist_scout_path_samples(&self, samples: Vec<ScoutPathSample>) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        self.send_write(WriteOperation::ScoutPathSamples(samples))
            .await
    }

    pub async fn complete_scout_path(&self, decision_id: String) -> Result<()> {
        self.send_write(WriteOperation::ScoutPathComplete(decision_id))
            .await
    }

    pub async fn mark_scout_path_capture_failed(&self, decision_ids: Vec<String>) -> Result<()> {
        if decision_ids.is_empty() {
            return Ok(());
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
            let (capture_complete, truncated, sample_count, entry_bid, entry_ask, quote_timestamp): (i64, i64, i64, Option<f64>, Option<f64>, Option<i64>) = connection.query_row(
                "SELECT capture_complete, truncated, sample_count, entry_bid, entry_ask, entry_quote_timestamp FROM scout_path_registrations WHERE decision_id=?1",
                [&decision_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
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
                    capture_complete: row.get(14)?,
                })
            })?;
            let samples = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            let freshness = samples.last().map(|sample| sample.freshness_ms);
            let entry_quote = entry_bid.zip(entry_ask).zip(quote_timestamp.map(|time| time as u64)).map(|((bid, ask), time)| (bid, ask, time));
            Ok((samples, truncated != 0, capture_complete != 0, freshness, sample_count as u64, entry_quote))
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
        let store = test_store(path);
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
        let (restored, truncated, complete, freshness, sample_count, restored_entry_quote) = store
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
        store
            .mark_scout_path_capture_failed(vec!["scout-1".into()])
            .await
            .unwrap();
        let (_, _, complete_after_failure, _, _, _) = store
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
}
