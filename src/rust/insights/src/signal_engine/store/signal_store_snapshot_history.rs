const SNAPSHOT_HISTORY_MAX_PAGE: usize = 512;
const SNAPSHOT_HISTORY_MAX_SCAN: usize = 4_096;
const SNAPSHOT_HISTORY_MAX_COMPRESSED_BYTES: usize = 8 * 1024 * 1024;
const SNAPSHOT_HISTORY_MAX_RECORD_BYTES: usize = 128 * 1024;
const SNAPSHOT_HISTORY_MAX_RESPONSE_BYTES: usize = 512 * 1024;
const SNAPSHOT_HISTORY_MAX_BOOK_SCAN: usize = 4_096;
const SNAPSHOT_HISTORY_MAX_BOOK_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategySnapshotHistoryQuery {
    pub symbol: String,
    pub from_ms: u64,
    pub through_ms: u64,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategySnapshotHistoryRecord {
    pub snapshot: SignalSnapshot,
    pub top_bid: Option<f64>,
    pub top_ask: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategySnapshotHistoryPage {
    pub symbol: String,
    pub requested_from_ms: u64,
    pub requested_through_ms: u64,
    pub records: Vec<StrategySnapshotHistoryRecord>,
    pub next_cursor: Option<String>,
    pub scanned_records: usize,
    pub coverage_start_ms: Option<u64>,
    pub coverage_through_ms: Option<u64>,
    pub more_available: bool,
    pub archived_days_unscanned: usize,
    pub archived_days_unavailable: usize,
    pub history_complete: bool,
    pub range_exhausted: bool,
    pub corrupt_records: usize,
    pub book_mark_records: usize,
    pub book_mark_coverage_complete: bool,
}

impl SignalStore {
    pub async fn strategy_snapshot_history(
        &self,
        query: StrategySnapshotHistoryQuery,
    ) -> Result<StrategySnapshotHistoryPage> {
        anyhow::ensure!(!query.symbol.is_empty() && query.symbol.len() <= 32, "symbol must contain 1 to 32 bytes");
        anyhow::ensure!(query.from_ms <= query.through_ms, "fromMs must not exceed throughMs");
        anyhow::ensure!(query.from_ms <= i64::MAX as u64 && query.through_ms <= i64::MAX as u64, "time bounds exceed storage range");
        let cursor = query.cursor.as_deref().map(parse_research_history_cursor).transpose()?;
        let limit = query.limit.unwrap_or(256);
        anyhow::ensure!((1..=SNAPSHOT_HISTORY_MAX_PAGE).contains(&limit), "limit must be between 1 and {SNAPSHOT_HISTORY_MAX_PAGE}");
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::strategy_snapshot_history_sync(&path, query, cursor, limit)
        }).await.map_err(|error| anyhow::anyhow!("strategy snapshot history task failed: {error}"))?
    }

    fn strategy_snapshot_history_sync(
        path: &PathBuf,
        query: StrategySnapshotHistoryQuery,
        cursor: Option<(u64, i64)>,
        limit: usize,
    ) -> Result<StrategySnapshotHistoryPage> {
        let connection = Self::open_database_path(&path.to_path_buf())?;
        let mut statement = connection.prepare(
            "SELECT id, timestamp, length(payload) FROM signal_records WHERE kind = 0 AND timestamp >= ?1 AND timestamp <= ?2 AND (?3 IS NULL OR timestamp > ?3 OR (timestamp = ?3 AND id > ?4)) ORDER BY timestamp ASC, id ASC LIMIT ?5",
        )?;
        let rows = statement.query_map(
            params![query.from_ms as i64, query.through_ms as i64, cursor.map(|value| value.0 as i64), cursor.map(|value| value.1).unwrap_or(0), (SNAPSHOT_HISTORY_MAX_SCAN + 1) as i64],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
        )?;
        let (archived_days_unscanned, archived_days_unavailable) = Self::snapshot_archive_coverage(
            &connection, query.from_ms, query.through_ms,
        )?;
        let mut page = StrategySnapshotHistoryPage {
            symbol: query.symbol.clone(), requested_from_ms: query.from_ms,
            requested_through_ms: query.through_ms, records: Vec::with_capacity(limit.min(64)),
            next_cursor: cursor.map(|(timestamp, id)| format!("{timestamp}:{id}")),
            scanned_records: 0, coverage_start_ms: None, coverage_through_ms: None,
            more_available: false, archived_days_unscanned, archived_days_unavailable,
            history_complete: false, book_mark_records: 0, book_mark_coverage_complete: false,
            range_exhausted: false, corrupt_records: 0,
        };
        let mut compressed_bytes = 0usize;
        let mut corrupt_records = 0usize;
        let mut bounded = false;
        for row in rows {
            let (id, timestamp, compressed_len) = row?;
            if page.scanned_records >= SNAPSHOT_HISTORY_MAX_SCAN {
                page.more_available = true;
                bounded = true;
                break;
            }
            let compressed_len = compressed_len.max(0) as usize;
            if compressed_len > SNAPSHOT_HISTORY_MAX_COMPRESSED_BYTES.saturating_sub(compressed_bytes) {
                page.more_available = true;
                bounded = true;
                break;
            }
            let compressed: Vec<u8> = connection.query_row(
                "SELECT payload FROM signal_records WHERE id = ?1 AND kind = 0", [id], |row| row.get(0),
            )?;
            compressed_bytes += compressed_len;
            page.scanned_records += 1;
            let timestamp = timestamp.max(0) as u64;
            let previous_cursor = page.next_cursor.clone();
            page.next_cursor = Some(format!("{timestamp}:{id}"));
            page.coverage_start_ms.get_or_insert(timestamp);
            page.coverage_through_ms = Some(timestamp);
            let decoded = decode_strategy_snapshot_history_record(&compressed);
            let Ok(snapshot) = decoded else { corrupt_records += 1; continue; };
            if snapshot.timestamp < query.from_ms || snapshot.timestamp > query.through_ms || snapshot.symbol != query.symbol {
                continue;
            }
            if page.records.len() >= limit {
                page.next_cursor = previous_cursor;
                page.more_available = true;
                break;
            }
            page.records.push(StrategySnapshotHistoryRecord { snapshot, top_bid: None, top_ask: None });
            if serde_json::to_vec(&page)?.len() > SNAPSHOT_HISTORY_MAX_RESPONSE_BYTES {
                page.records.pop();
                page.next_cursor = previous_cursor;
                page.more_available = true;
                bounded = true;
                break;
            }
        }
        let exhausted = !page.more_available && !bounded;
        page.range_exhausted = exhausted;
        page.corrupt_records = corrupt_records;
        page.history_complete = cursor.is_none() && exhausted && corrupt_records == 0
            && page.archived_days_unscanned == 0 && page.archived_days_unavailable == 0;
        Self::attach_snapshot_history_book_marks(&connection, &mut page)?;
        anyhow::ensure!(serde_json::to_vec(&page)?.len() <= SNAPSHOT_HISTORY_MAX_RESPONSE_BYTES, "snapshot history response exceeds payload limit");
        Ok(page)
    }

    fn snapshot_archive_coverage(connection: &Connection, from_ms: u64, through_ms: u64) -> Result<(usize, usize)> {
        let unscanned = connection.query_row(
            "SELECT COUNT(*) FROM signal_daily_summary WHERE kind = 0 AND training_payload IS NOT NULL AND first_timestamp <= ?1 AND last_timestamp >= ?2",
            params![through_ms as i64, from_ms as i64], |row| row.get::<_, i64>(0),
        )?;
        let unavailable = connection.query_row(
            "SELECT COUNT(*) FROM signal_daily_summary AS archived WHERE kind = 0 AND training_payload IS NULL AND first_timestamp <= ?1 AND last_timestamp >= ?2 AND NOT EXISTS (SELECT 1 FROM signal_records AS raw WHERE raw.kind = 0 AND raw.timestamp >= archived.first_timestamp AND raw.timestamp <= archived.last_timestamp)",
            params![through_ms as i64, from_ms as i64], |row| row.get::<_, i64>(0),
        )?;
        Ok((unscanned.max(0) as usize, unavailable.max(0) as usize))
    }

    fn attach_snapshot_history_book_marks(connection: &Connection, page: &mut StrategySnapshotHistoryPage) -> Result<()> {
        let Some(first) = page.records.first().map(|record| record.snapshot.timestamp) else { return Ok(()); };
        let Some(last) = page.records.last().map(|record| record.snapshot.timestamp) else { return Ok(()); };
        let mut statement = connection.prepare(
            "SELECT id, timestamp, length(payload) FROM signal_records WHERE kind = 3 AND timestamp >= ?1 AND timestamp <= ?2 ORDER BY timestamp DESC, id DESC LIMIT ?3",
        )?;
        let mut rows = statement.query(params![first.saturating_sub(10_000) as i64, last as i64, (SNAPSHOT_HISTORY_MAX_BOOK_SCAN + 1) as i64])?;
        let mut books = Vec::new();
        let mut compressed_bytes = 0usize;
        let mut scanned_records = 0usize;
        let mut book_scan_complete = true;
        while let Some(row) = rows.next()? {
            if scanned_records >= SNAPSHOT_HISTORY_MAX_BOOK_SCAN { book_scan_complete = false; break; }
            scanned_records += 1;
            let id = row.get::<_, i64>(0)?;
            let timestamp = row.get::<_, i64>(1)?.max(0) as u64;
            let compressed_len = row.get::<_, i64>(2)?.max(0) as usize;
            if compressed_len > SNAPSHOT_HISTORY_MAX_BOOK_BYTES.saturating_sub(compressed_bytes) {
                book_scan_complete = false;
                break;
            }
            let compressed: Vec<u8> = connection.query_row("SELECT payload FROM signal_records WHERE id = ?1 AND kind = 3", [id], |row| row.get(0))?;
            compressed_bytes += compressed_len;
            let decoder = MultiGzDecoder::new(compressed.as_slice());
            let mut payload = Vec::new();
            if decoder.take((SNAPSHOT_HISTORY_MAX_RECORD_BYTES + 1) as u64).read_to_end(&mut payload).is_err()
                || payload.len() > SNAPSHOT_HISTORY_MAX_RECORD_BYTES
            {
                book_scan_complete = false;
                continue;
            }
            let Ok(StoredSignalRecord::BookSnapshot(book)) = deserialize_record(3, &payload) else {
                book_scan_complete = false;
                continue;
            };
            if book.timestamp == timestamp { books.push(book); }
        }
        let mut books = books;
        books.sort_by(|left, right| left.timestamp.cmp(&right.timestamp).then(left.symbol.cmp(&right.symbol)));
        let mut available = 0usize;
        for record in &mut page.records {
            let time = record.snapshot.timestamp;
            let book = books.iter().rev().find(|book| book.symbol == record.snapshot.symbol && book.timestamp <= time && time.saturating_sub(book.timestamp) <= 10_000);
            if let Some(book) = book {
                record.top_bid = book.bid_levels.first().map(|level| level.0).filter(|price| price.is_finite() && *price > 0.0);
                record.top_ask = book.ask_levels.first().map(|level| level.0).filter(|price| price.is_finite() && *price > 0.0);
                if record.top_bid.zip(record.top_ask).is_some_and(|(bid, ask)| ask >= bid) { available += 1; }
            }
        }
        page.book_mark_records = available;
        page.book_mark_coverage_complete = book_scan_complete && !page.records.is_empty() && available == page.records.len();
        Ok(())
    }
}

fn decode_strategy_snapshot_history_record(compressed: &[u8]) -> Result<SignalSnapshot> {
    let decoder = MultiGzDecoder::new(compressed);
    let mut payload = Vec::new();
    decoder.take((SNAPSHOT_HISTORY_MAX_RECORD_BYTES + 1) as u64).read_to_end(&mut payload)?;
    anyhow::ensure!(payload.len() <= SNAPSHOT_HISTORY_MAX_RECORD_BYTES, "snapshot record exceeds decoded size limit");
    match deserialize_record(0, &payload)? {
        StoredSignalRecord::Snapshot(snapshot) => Ok(snapshot),
        _ => anyhow::bail!("unexpected record in strategy snapshot history"),
    }
}

#[cfg(test)]
mod snapshot_history_tests {
    use super::*;
    use rusqlite::params;

    struct TempDatabase { directory: PathBuf, path: PathBuf }
    impl Drop for TempDatabase {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.directory); }
    }

    fn snapshot(symbol: &str, timestamp: u64) -> SignalSnapshot {
        SignalSnapshot {
            symbol: symbol.into(), timestamp, direction: "bullish".into(), score: 1.0,
            strength: 0.5, components: SignalComponents::default(), reasons: vec![],
            news_attribution: None, stale: false, ranging_probability: None,
            range_lower_bound: None, range_upper_bound: None, evidence_coverage: Some(1.0),
            estimated_accuracy: None,
        }
    }

    fn database(records: &[(&str, u64)]) -> TempDatabase {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!("insights-snapshot-history-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("signals.sqlite");
        let connection = SignalStore::open_database_at(&path).unwrap();
        connection.execute_batch("CREATE TABLE signal_daily_summary(day_start_ms INTEGER NOT NULL, kind INTEGER NOT NULL, first_timestamp INTEGER NOT NULL, last_timestamp INTEGER NOT NULL, training_payload BLOB, PRIMARY KEY(day_start_ms, kind));").unwrap();
        for (symbol, timestamp) in records {
            let compressed = compress_record(&StoredSignalRecord::Snapshot(snapshot(symbol, *timestamp))).unwrap();
            connection.execute("INSERT INTO signal_records(timestamp, kind, payload) VALUES (?1, 0, ?2)", params![*timestamp as i64, compressed]).unwrap();
        }
        drop(connection);
        TempDatabase { directory, path }
    }

    #[test]
    fn snapshot_history_filters_symbol_and_paginates_in_order() {
        let database = database(&[("BTC-USDT", 100), ("ETH-USDT", 110), ("BTC-USDT", 120)]);
        let query = |cursor| StrategySnapshotHistoryQuery { symbol: "BTC-USDT".into(), from_ms: 0, through_ms: 200, cursor, limit: Some(1) };
        let first = SignalStore::strategy_snapshot_history_sync(&database.path, query(None), None, 1).unwrap();
        assert_eq!(first.records[0].snapshot.timestamp, 100);
        assert_eq!(first.next_cursor.as_deref(), Some("110:2"));
        let cursor = first.next_cursor.as_deref().map(parse_research_history_cursor).transpose().unwrap();
        let second = SignalStore::strategy_snapshot_history_sync(&database.path, query(first.next_cursor), cursor, 1).unwrap();
        assert_eq!(second.records[0].snapshot.timestamp, 120);
        assert!(second.range_exhausted);
    }

    #[test]
    fn snapshot_history_reports_truncation_instead_of_claiming_complete_coverage() {
        let rows = (0..SNAPSHOT_HISTORY_MAX_SCAN + 1).map(|index| ("ETH-USDT", index as u64 + 1)).collect::<Vec<_>>();
        let database = database(&rows);
        let page = SignalStore::strategy_snapshot_history_sync(&database.path, StrategySnapshotHistoryQuery {
            symbol: "BTC-USDT".into(), from_ms: 0, through_ms: i64::MAX as u64, cursor: None, limit: Some(1),
        }, None, 1).unwrap();
        assert_eq!(page.scanned_records, SNAPSHOT_HISTORY_MAX_SCAN);
        assert!(page.more_available);
        assert!(!page.history_complete);
    }
}
