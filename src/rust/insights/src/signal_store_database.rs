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
        Ok(Self {
            path,
            writer,
            recent_cache: Arc::new(Mutex::new(None)),
            best_cache: Arc::new(Mutex::new(None)),
            performance_cache: Arc::new(Mutex::new(None)),
            walk_forward_cache: Arc::new(Mutex::new(None)),
            missed_opportunity_cache: Arc::new(Mutex::new(None)),
            walk_forward_refreshing: Arc::new(AtomicBool::new(false)),
        })
    }

    fn open_database_at(path: &PathBuf) -> Result<Connection> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; CREATE TABLE IF NOT EXISTS signal_records (id INTEGER PRIMARY KEY, timestamp INTEGER NOT NULL, kind INTEGER NOT NULL, payload BLOB NOT NULL); CREATE INDEX IF NOT EXISTS signal_records_timestamp ON signal_records(timestamp); CREATE INDEX IF NOT EXISTS signal_records_kind_timestamp ON signal_records(kind, timestamp); CREATE TABLE IF NOT EXISTS auto_trader_audit (decision_id TEXT NOT NULL, stage TEXT NOT NULL, timestamp INTEGER NOT NULL, symbol TEXT, payload BLOB NOT NULL, PRIMARY KEY(decision_id, stage)); CREATE INDEX IF NOT EXISTS auto_trader_audit_timestamp ON auto_trader_audit(timestamp);")?;
        Ok(connection)
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
        tokio::task::spawn_blocking(move || Self::read_records_sync(&path, kind, since, limit))
            .await
            .map_err(|error| anyhow::anyhow!("signal read task failed: {error}"))?
    }

    fn read_records_sync(
        path: &PathBuf,
        kind: i64,
        since: Option<u64>,
        limit: Option<usize>,
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
        let compressed_archives = if (0..=5).contains(&kind) {
            let since_day = since.map(|timestamp| (timestamp / 86_400_000) as i64 * 86_400_000);
            let mut statement = connection.prepare(
                "SELECT training_payload FROM signal_daily_summary WHERE kind = ?1 AND training_payload IS NOT NULL AND (?2 IS NULL OR day_start_ms >= ?2)",
            )?;
            statement
                .query_map(params![kind, since_day], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        drop(connection);
        let mut records = Vec::new();
        for compressed in compressed_records {
            let mut decoder = MultiGzDecoder::new(compressed.as_slice());
            let mut payload = Vec::new();
            decoder.read_to_end(&mut payload)?;
            records.push(deserialize_record(kind, &payload)?);
        }
        for compressed in compressed_archives {
            let mut decoder = MultiGzDecoder::new(compressed.as_slice());
            let mut payload = Vec::new();
            decoder.read_to_end(&mut payload)?;
            let mut archived: Vec<StoredSignalRecord> = bincode::deserialize(&payload)?;
            if let Some(timestamp) = since {
                archived.retain(|record| record_timestamp(record) >= timestamp);
            }
            records.extend(archived);
        }
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
        let mut statement = connection.prepare(
            "SELECT payload FROM signal_records WHERE kind = ?1 ORDER BY timestamp DESC, id DESC LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![kind, limit.min(i64::MAX as usize) as i64], |row| {
                row.get::<_, Vec<u8>>(0)
            })?;
        let compressed_records = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let mut records = Vec::new();
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
        if records.len() < limit && (0..=5).contains(&kind) {
            let mut statement = connection.prepare(
                "SELECT training_payload FROM signal_daily_summary WHERE kind = ?1 AND training_payload IS NOT NULL ORDER BY day_start_ms DESC",
            )?;
            let archives = statement.query_map([kind], |row| row.get::<_, Vec<u8>>(0))?;
            let archives = archives.collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            for compressed in archives {
                if records.len() >= limit {
                    break;
                }
                let mut payload = Vec::new();
                let decoded = MultiGzDecoder::new(compressed.as_slice())
                    .read_to_end(&mut payload)
                    .map_err(anyhow::Error::from)
                    .and_then(|_| {
                        bincode::deserialize::<Vec<StoredSignalRecord>>(&payload)
                            .map_err(anyhow::Error::from)
                    });
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

    async fn read_calibration_records(&self) -> Result<Vec<StoredSignalRecord>> {
        // The display needs recent signal history, not a decompression pass over
        // every archived outcome each time its cache expires.
        let mut records = self.read_recent_records(0, 10_000).await?;
        records.extend(self.read_recent_records(1, 100_000).await?);
        records.sort_by_key(record_timestamp);
        Ok(records)
    }
}
