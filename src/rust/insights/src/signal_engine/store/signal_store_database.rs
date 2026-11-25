impl SignalStore {
    pub fn from_environment() -> Result<Self> {
        let path = std::env::var_os("SIGNALS_DB_FILE")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("SIGNALS_FILE")
                    .map(|path| PathBuf::from(path).with_extension("sqlite"))
            })
            .unwrap_or(std::env::current_dir()?.join("signals.sqlite"));
        let connection = Self::open_database_at(&path)?;
        let (writer, receiver) = mpsc::channel(4096);
        tokio::task::spawn_blocking(move || run_database_writer(connection, receiver));
        let audit_path = audit_database_path(&path);
        let audit_connection = Self::open_audit_database_at(&audit_path)?;
        migrate_legacy_audit_tables(&path, &audit_path)?;
        let (audit_writer, audit_receiver) = mpsc::channel(4096);
        tokio::task::spawn_blocking(move || run_database_writer(audit_connection, audit_receiver));
        Ok(Self {
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
        })
    }

    fn open_database_at(path: &PathBuf) -> Result<Connection> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA wal_autocheckpoint=1000; CREATE TABLE IF NOT EXISTS signal_records (id INTEGER PRIMARY KEY, timestamp INTEGER NOT NULL, kind INTEGER NOT NULL, payload BLOB NOT NULL); CREATE INDEX IF NOT EXISTS signal_records_timestamp ON signal_records(timestamp); CREATE INDEX IF NOT EXISTS signal_records_kind_timestamp ON signal_records(kind, timestamp); CREATE TABLE IF NOT EXISTS scout_path_registrations (decision_id TEXT PRIMARY KEY, symbol TEXT NOT NULL, signal_timestamp INTEGER NOT NULL, direction TEXT NOT NULL, entry_price REAL NOT NULL, entry_bid REAL, entry_ask REAL, entry_quote_timestamp INTEGER, active INTEGER NOT NULL DEFAULT 1, capture_complete INTEGER NOT NULL DEFAULT 1, truncated INTEGER NOT NULL DEFAULT 0, sample_count INTEGER NOT NULL DEFAULT 0); CREATE INDEX IF NOT EXISTS scout_path_registrations_symbol ON scout_path_registrations(symbol, active); CREATE TABLE IF NOT EXISTS scout_path_samples (decision_id TEXT NOT NULL, path_epoch INTEGER NOT NULL, sequence INTEGER NOT NULL, timestamp INTEGER NOT NULL, bid REAL NOT NULL, ask REAL NOT NULL, freshness_ms INTEGER NOT NULL, gap_ms INTEGER NOT NULL, first_source_timestamp INTEGER, maximum_gap_ms INTEGER NOT NULL, source_timestamps_monotonic INTEGER NOT NULL, maximum_favorable_excursion_percent REAL, maximum_adverse_excursion_percent REAL, sample_count INTEGER NOT NULL, capture_complete INTEGER NOT NULL, PRIMARY KEY(decision_id, path_epoch, sequence)); CREATE INDEX IF NOT EXISTS scout_path_samples_order ON scout_path_samples(decision_id, path_epoch, sequence);")?;
        Ok(connection)
    }

    fn open_audit_database_at(path: &PathBuf) -> Result<Connection> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(30))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint=1000; CREATE TABLE IF NOT EXISTS auto_trader_audit (decision_id TEXT NOT NULL, stage TEXT NOT NULL, timestamp INTEGER NOT NULL, symbol TEXT, payload BLOB NOT NULL, PRIMARY KEY(decision_id, stage)); CREATE INDEX IF NOT EXISTS auto_trader_audit_timestamp ON auto_trader_audit(timestamp); CREATE INDEX IF NOT EXISTS auto_trader_audit_stage_timestamp ON auto_trader_audit(stage, timestamp DESC); CREATE TABLE IF NOT EXISTS auto_trader_audit_daily_summary (day_start_ms INTEGER NOT NULL, stage TEXT NOT NULL, symbol TEXT NOT NULL, record_count INTEGER NOT NULL, PRIMARY KEY(day_start_ms, stage, symbol)); CREATE TABLE IF NOT EXISTS insights_migrations (migration TEXT PRIMARY KEY, completed_at_ms INTEGER NOT NULL);")?;
        Ok(connection)
    }

    async fn send_audit_write(&self, operation: WriteOperation) -> Result<()> {
        let (ack, result) = oneshot::channel();
        self.audit_writer
            .send(WriteRequest { operation, ack })
            .await
            .map_err(|_| anyhow::anyhow!("audit database writer stopped"))?;
        result
            .await
            .map_err(|_| anyhow::anyhow!("audit database writer dropped acknowledgement"))?
            .map_err(|error| anyhow::anyhow!(error))
    }

    fn open_database_path(path: &PathBuf) -> Result<Connection> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA synchronous=NORMAL;")?;
        Ok(connection)
    }

    async fn read_records(
        &self,
        kind: i64,
        since: Option<u64>,
        limit: Option<usize>,
    ) -> Result<Vec<StoredSignalRecord>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::read_records_sync(&path, kind, since, limit, false)
        })
        .await
        .map_err(|error| anyhow::anyhow!("signal read task failed: {error}"))?
    }

    async fn read_records_with_corrupt_rows(
        &self,
        kind: i64,
        since: Option<u64>,
        limit: Option<usize>,
        skip_corrupt_rows: bool,
    ) -> Result<Vec<StoredSignalRecord>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::read_records_sync(&path, kind, since, limit, skip_corrupt_rows)
        })
        .await
        .map_err(|error| anyhow::anyhow!("signal read task failed: {error}"))?
    }

    fn read_records_sync(
        path: &PathBuf,
        kind: i64,
        since: Option<u64>,
        limit: Option<usize>,
        skip_corrupt_rows: bool,
    ) -> Result<Vec<StoredSignalRecord>> {
        let connection = Self::open_database_path(path)?;
        let mut sql = String::from("SELECT payload FROM signal_records WHERE kind = ?");
        let mut values = vec![rusqlite::types::Value::Integer(kind)];
        if let Some(timestamp) = since {
            sql.push_str(" AND timestamp >= ?");
            values.push(rusqlite::types::Value::Integer(
                timestamp.min(i64::MAX as u64) as i64,
            ));
        }
        sql.push_str(" ORDER BY timestamp DESC, id DESC");
        if let Some(limit) = limit {
            sql.push_str(" LIMIT ?");
            values.push(rusqlite::types::Value::Integer(
                limit.min(i64::MAX as usize) as i64,
            ));
        }
        let compressed_records = {
            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(values.iter()), |row| {
                row.get::<_, Vec<u8>>(0)
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut records = Vec::new();
        for compressed in compressed_records {
            let mut decoder = MultiGzDecoder::new(compressed.as_slice());
            let mut payload = Vec::new();
            let decoded = decoder
                .read_to_end(&mut payload)
                .map_err(anyhow::Error::from)
                .and_then(|_| deserialize_record(kind, &payload));
            match decoded {
                Ok(record) => records.push(record),
                Err(_) if skip_corrupt_rows => continue,
                Err(error) => return Err(error),
            }
        }
        if (0..=6).contains(&kind) && limit.is_none_or(|limit| records.len() < limit) {
            let since_day = since.map(|timestamp| (timestamp / 86_400_000) as i64 * 86_400_000);
            let archive_ids = {
                let mut statement = connection.prepare(
                    "SELECT rowid FROM signal_daily_summary WHERE kind = ?1 AND training_payload IS NOT NULL AND (?2 IS NULL OR day_start_ms >= ?2) ORDER BY day_start_ms DESC",
                )?;
                let rows =
                    statement.query_map(params![kind, since_day], |row| row.get::<_, i64>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for archive_id in archive_ids {
                if limit.is_some_and(|limit| records.len() >= limit) {
                    break;
                }
                let compressed: Vec<u8> = connection.query_row(
                    "SELECT training_payload FROM signal_daily_summary WHERE rowid = ?1 AND kind = ?2 AND training_payload IS NOT NULL",
                    params![archive_id, kind],
                    |row| row.get(0),
                )?;
                let remaining = limit.map_or(usize::MAX, |limit| limit - records.len());
                let mut archived = match read_archive_tail(&compressed, remaining) {
                    Ok(archived) => archived,
                    Err(_) if skip_corrupt_rows => continue,
                    Err(error) => return Err(error),
                };
                if let Some(timestamp) = since {
                    archived.retain(|record| record_timestamp(record) >= timestamp);
                }
                records.append(&mut archived);
            }
        }
        drop(connection);
        records.sort_by_key(record_timestamp);
        if let Some(limit) = limit {
            let start = records.len().saturating_sub(limit);
            records.drain(..start);
        }
        Ok(records)
    }

    async fn read_recent_records(
        &self,
        kind: i64,
        limit: usize,
    ) -> Result<Vec<StoredSignalRecord>> {
        self.read_recent_records_with_corrupt_rows(kind, limit, false)
            .await
    }

    async fn read_recent_records_with_corrupt_rows(
        &self,
        kind: i64,
        limit: usize,
        skip_corrupt_rows: bool,
    ) -> Result<Vec<StoredSignalRecord>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::read_recent_records_sync(&path, kind, limit, skip_corrupt_rows)
        })
        .await
        .map_err(|error| anyhow::anyhow!("recent signal read task failed: {error}"))?
    }

    fn read_recent_records_sync(
        path: &PathBuf,
        kind: i64,
        limit: usize,
        skip_corrupt_rows: bool,
    ) -> Result<Vec<StoredSignalRecord>> {
        let connection = Self::open_database_path(path)?;
        let mut records = Vec::new();
        let compressed_records = {
            let mut statement = connection.prepare(
                "SELECT payload FROM signal_records WHERE kind = ?1 ORDER BY timestamp DESC, id DESC LIMIT ?2",
            )?;
            let rows = statement
                .query_map(params![kind, limit.min(i64::MAX as usize) as i64], |row| {
                    row.get::<_, Vec<u8>>(0)
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        // End the SQLite read transaction before decompressing/deserializing.
        // Large requests (up to 100k research records) otherwise pin a WAL
        // snapshot for the full CPU-bound decode loop and can stall checkpoints.
        for compressed in compressed_records {
            let mut payload = Vec::new();
            let decoded = MultiGzDecoder::new(compressed.as_slice())
                .read_to_end(&mut payload)
                .map_err(anyhow::Error::from)
                .and_then(|_| deserialize_record(kind, &payload));
            match decoded {
                Ok(record) => records.push(record),
                Err(_) if skip_corrupt_rows => continue,
                Err(error) => return Err(error),
            }
        }
        if records.len() < limit && (0..=6).contains(&kind) {
            let archive_ids = {
                let mut statement = connection.prepare(
                    "SELECT rowid FROM signal_daily_summary WHERE kind = ?1 AND training_payload IS NOT NULL ORDER BY day_start_ms DESC",
                )?;
                let rows = statement.query_map([kind], |row| row.get::<_, i64>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for archive_id in archive_ids {
                if records.len() >= limit {
                    break;
                }
                let compressed = connection.query_row(
                    "SELECT training_payload FROM signal_daily_summary WHERE rowid = ?1 AND kind = ?2 AND training_payload IS NOT NULL",
                    params![archive_id, kind],
                    |row| row.get::<_, Vec<u8>>(0),
                )?;
                let remaining = limit - records.len();
                let decoded = read_archive_tail(&compressed, remaining);
                match decoded {
                    Ok(mut archived) => records.append(&mut archived),
                    Err(_) if skip_corrupt_rows => continue,
                    Err(error) => return Err(error),
                }
            }
        }
        drop(connection);
        records.sort_by_key(record_timestamp);
        let start = records.len().saturating_sub(limit);
        records.drain(..start);
        Ok(records)
    }

    async fn read_calibration_records(&self) -> Result<Arc<Vec<StoredSignalRecord>>> {
        // Best, recent, and performance views use overlapping records. Share
        // one bounded dashboard-only window across their independently
        // refreshed caches; live signal generation never reads this cache.
        let mut cache = self.calibration_records_cache.lock().await;
        if let Some((created, records)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(15) {
                return Ok(Arc::clone(records));
            }
        }
        let mut records = self
            .read_recent_records(0, CALIBRATION_SNAPSHOT_LIMIT)
            .await?;
        records.extend(
            self.read_recent_records(1, CALIBRATION_OUTCOME_LIMIT)
                .await?,
        );
        records.sort_by_key(record_timestamp);
        let records = Arc::new(records);
        *cache = Some((Instant::now(), Arc::clone(&records)));
        Ok(records)
    }
}

fn audit_database_path(signal_path: &PathBuf) -> PathBuf {
    signal_path.with_file_name("auto_trader_audit.sqlite")
}

fn migrate_legacy_audit_tables(signal_path: &PathBuf, audit_path: &PathBuf) -> Result<()> {
    let legacy = Connection::open(signal_path)?;
    legacy.busy_timeout(Duration::from_secs(30))?;
    let has_audit: bool = legacy.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='auto_trader_audit')",
        [],
        |row| row.get(0),
    )?;
    if !has_audit {
        return Ok(());
    }
    let has_summary: bool = legacy.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='auto_trader_audit_daily_summary')",
        [],
        |row| row.get(0),
    )?;
    let audit = Connection::open(audit_path)?;
    audit.busy_timeout(Duration::from_secs(30))?;
    audit.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS auto_trader_audit (decision_id TEXT NOT NULL, stage TEXT NOT NULL, timestamp INTEGER NOT NULL, symbol TEXT, payload BLOB NOT NULL, PRIMARY KEY(decision_id, stage)); CREATE INDEX IF NOT EXISTS auto_trader_audit_timestamp ON auto_trader_audit(timestamp); CREATE INDEX IF NOT EXISTS auto_trader_audit_stage_timestamp ON auto_trader_audit(stage, timestamp DESC); CREATE TABLE IF NOT EXISTS auto_trader_audit_daily_summary (day_start_ms INTEGER NOT NULL, stage TEXT NOT NULL, symbol TEXT NOT NULL, record_count INTEGER NOT NULL, PRIMARY KEY(day_start_ms, stage, symbol)); CREATE TABLE IF NOT EXISTS insights_migrations (migration TEXT PRIMARY KEY, completed_at_ms INTEGER NOT NULL);")?;
    let migration_complete: bool = audit.query_row(
        "SELECT EXISTS(SELECT 1 FROM insights_migrations WHERE migration = 'split_auto_trader_audit_v1')",
        [],
        |row| row.get(0),
    )?;
    if migration_complete {
        return Ok(());
    }
    let already_populated: bool = audit.query_row(
        "SELECT EXISTS(SELECT 1 FROM auto_trader_audit LIMIT 1) OR EXISTS(SELECT 1 FROM auto_trader_audit_daily_summary LIMIT 1)",
        [],
        |row| row.get(0),
    )?;
    if already_populated {
        audit.execute(
            "INSERT OR IGNORE INTO insights_migrations(migration, completed_at_ms) VALUES ('split_auto_trader_audit_v1', ?1)",
            [std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64],
        )?;
        return Ok(());
    }
    audit.execute(
        "ATTACH DATABASE ?1 AS legacy",
        [signal_path.to_string_lossy().as_ref()],
    )?;
    let transaction = audit.unchecked_transaction()?;
    transaction.execute_batch("INSERT OR IGNORE INTO main.auto_trader_audit(rowid, decision_id, stage, timestamp, symbol, payload) SELECT rowid, decision_id, stage, timestamp, symbol, payload FROM legacy.auto_trader_audit;")?;
    if has_summary {
        transaction.execute_batch("INSERT OR IGNORE INTO main.auto_trader_audit_daily_summary SELECT * FROM legacy.auto_trader_audit_daily_summary;")?;
    }
    transaction.execute(
        "INSERT OR IGNORE INTO insights_migrations(migration, completed_at_ms) VALUES ('split_auto_trader_audit_v1', ?1)",
        [std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64],
    )?;
    transaction.commit()?;
    audit.execute("DETACH DATABASE legacy", [])?;
    Ok(())
}

/// Read only the newest `limit` records from a daily archive. Archives are
/// serialized as a bincode Vec in timestamp order; decoding one record at a
/// time avoids materializing both the full decompressed archive and its full
/// decoded Vec when a caller only needs a recent window.
fn read_archive_tail(compressed: &[u8], limit: usize) -> Result<Vec<StoredSignalRecord>> {
    use std::collections::VecDeque;

    use bincode::Options;

    let decoder = MultiGzDecoder::new(compressed);
    let mut records = bincode::Deserializer::with_reader(
        decoder,
        bincode::DefaultOptions::new().with_fixint_encoding(),
    );
    let count: u64 = serde::Deserialize::deserialize(&mut records)?;
    let mut tail = VecDeque::with_capacity(limit.min(count as usize));
    for _ in 0..count {
        let record = StoredSignalRecord::deserialize(&mut records)?;
        if limit == 0 {
            continue;
        }
        if tail.len() == limit {
            tail.pop_front();
        }
        tail.push_back(record);
    }
    Ok(tail.into_iter().collect())
}

#[cfg(test)]
mod archive_reader_tests {
    use super::*;

    #[test]
    fn dashboard_calibration_cache_has_a_small_retention_bound() {
        assert_eq!(super::CALIBRATION_SNAPSHOT_LIMIT, 1_000);
        assert_eq!(super::CALIBRATION_OUTCOME_LIMIT, 5_000);
    }

    #[test]
    fn archive_reader_keeps_only_the_newest_records() {
        let records = (0..20)
            .map(|timestamp| {
                StoredSignalRecord::ExternalMarket(ExternalMarketFeatures {
                    underlying: "BTC".to_owned(),
                    timestamp,
                    swap_instrument: "BTC-USDT-SWAP".to_owned(),
                    funding_rate: None,
                    next_funding_rate: None,
                    open_interest_contracts: None,
                    open_interest_currency: None,
                    open_interest_change_percent: None,
                    mark_price: None,
                    index_price: None,
                    perp_basis_bps: None,
                    options_atm_iv: None,
                    options_25d_skew: None,
                    options_expiry_timestamp: None,
                })
            })
            .collect::<Vec<_>>();
        let serialized = bincode::serialize(&records).unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&serialized).unwrap();
        let compressed = encoder.finish().unwrap();

        let tail = read_archive_tail(&compressed, 4).unwrap();
        let timestamps = tail
            .iter()
            .map(|record| match record {
                StoredSignalRecord::ExternalMarket(features) => features.timestamp,
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        assert_eq!(timestamps, [16, 17, 18, 19]);
    }
}
