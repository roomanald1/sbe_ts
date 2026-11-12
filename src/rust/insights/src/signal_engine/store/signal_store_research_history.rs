const RESEARCH_HISTORY_MAX_PAGE: usize = 512;
const RESEARCH_HISTORY_MAX_SCAN: usize = 4_096;
const RESEARCH_HISTORY_MAX_COMPRESSED_BYTES: usize = 8 * 1024 * 1024;
const RESEARCH_HISTORY_MAX_RECORD_BYTES: usize = 128 * 1024;
const RESEARCH_HISTORY_MAX_RESPONSE_BYTES: usize = 512 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchFeatureHistoryQuery {
    pub symbol: String,
    pub from_ms: u64,
    pub through_ms: u64,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchFeatureHistoryPage {
    pub symbol: String,
    pub requested_from_ms: u64,
    pub requested_through_ms: u64,
    pub records: Vec<SignalResearchFeatures>,
    /// Exclusive timestamp/row-ID cursor. It advances across records for other
    /// symbols while preserving chronological order across pages.
    pub next_cursor: Option<String>,
    pub scanned_records: usize,
    pub coverage_start_ms: Option<u64>,
    pub coverage_through_ms: Option<u64>,
    pub more_available: bool,
    /// Archived day payloads overlap the requested range but are not traversed
    /// by this bounded raw-row endpoint, so their presence prevents completeness.
    pub archived_days_unscanned: usize,
    /// Summary rows whose original records and archived payload are both gone.
    pub archived_days_unavailable: usize,
    /// True only when the entire requested interval was scanned without page,
    /// scan, byte, or corrupt-record truncation. A later page can complete a
    /// range but must be combined with the caller's prior pages.
    pub history_complete: bool,
}

struct ResearchHistoryScan {
    page: ResearchFeatureHistoryPage,
}

impl SignalStore {
    pub async fn research_feature_history(
        &self,
        query: ResearchFeatureHistoryQuery,
    ) -> Result<ResearchFeatureHistoryPage> {
        anyhow::ensure!(!query.symbol.is_empty() && query.symbol.len() <= 32, "symbol must contain 1 to 32 bytes");
        anyhow::ensure!(query.from_ms <= query.through_ms, "fromMs must not exceed throughMs");
        anyhow::ensure!(query.from_ms <= i64::MAX as u64 && query.through_ms <= i64::MAX as u64, "time bounds exceed storage range");
        let cursor = query
            .cursor
            .as_deref()
            .map(parse_research_history_cursor)
            .transpose()?;
        let limit = query.limit.unwrap_or(256);
        anyhow::ensure!((1..=RESEARCH_HISTORY_MAX_PAGE).contains(&limit), "limit must be between 1 and {RESEARCH_HISTORY_MAX_PAGE}");
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::research_feature_history_sync(&path, query, cursor, limit)
        })
            .await
            .map_err(|error| anyhow::anyhow!("research history task failed: {error}"))?
    }

    fn research_feature_history_sync(
        path: &PathBuf,
        query: ResearchFeatureHistoryQuery,
        cursor: Option<(u64, i64)>,
        limit: usize,
    ) -> Result<ResearchFeatureHistoryPage> {
        let connection = Self::open_database_path(&path.to_path_buf())?;
        let mut statement = connection.prepare(
            "SELECT id, timestamp, length(payload) FROM signal_records WHERE kind = 4 AND timestamp >= ?1 AND timestamp <= ?2 AND (?3 IS NULL OR timestamp > ?3 OR (timestamp = ?3 AND id > ?4)) ORDER BY timestamp ASC, id ASC LIMIT ?5",
        )?;
        let rows = statement.query_map(
            params![query.from_ms as i64, query.through_ms as i64, cursor.map(|value| value.0 as i64), cursor.map(|value| value.1).unwrap_or(0), (RESEARCH_HISTORY_MAX_SCAN + 1) as i64],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
        )?;
        let mut scan = ResearchHistoryScan {
            page: ResearchFeatureHistoryPage {
                symbol: query.symbol.clone(),
                requested_from_ms: query.from_ms,
                requested_through_ms: query.through_ms,
                records: Vec::with_capacity(limit.min(64)),
                next_cursor: cursor.map(|(timestamp, id)| format!("{timestamp}:{id}")),
                scanned_records: 0,
                coverage_start_ms: None,
                coverage_through_ms: None,
                more_available: false,
                archived_days_unscanned: Self::archived_research_days(
                    &connection,
                    query.from_ms,
                    query.through_ms,
                )?,
                archived_days_unavailable: Self::unavailable_research_days(
                    &connection,
                    query.from_ms,
                    query.through_ms,
                )?,
                history_complete: false,
            },
        };
        let mut compressed_bytes = 0usize;
        let mut corrupt_record = false;
        let mut bounded = false;
        for row in rows {
            let (id, timestamp, compressed_len) = row?;
            if scan.page.scanned_records >= RESEARCH_HISTORY_MAX_SCAN {
                scan.page.more_available = true;
                bounded = true;
                break;
            }
            let compressed_len = compressed_len.max(0) as usize;
            if compressed_len > RESEARCH_HISTORY_MAX_COMPRESSED_BYTES.saturating_sub(compressed_bytes) {
                bounded = true;
                scan.page.more_available = true;
                break;
            }
            let compressed: Vec<u8> = connection.query_row(
                "SELECT payload FROM signal_records WHERE id = ?1 AND kind = 4",
                [id],
                |row| row.get(0),
            )?;
            compressed_bytes += compressed_len;
            scan.page.scanned_records += 1;
            let timestamp = timestamp.max(0) as u64;
            let previous_cursor = scan.page.next_cursor.clone();
            scan.page.next_cursor = Some(format!("{timestamp}:{id}"));
            scan.page.coverage_start_ms.get_or_insert(timestamp);
            scan.page.coverage_through_ms = Some(timestamp);
            let decoded = Self::decode_research_history_record(&compressed);
            let Ok(features) = decoded else {
                corrupt_record = true;
                continue;
            };
            if features.timestamp < query.from_ms || features.timestamp > query.through_ms || features.symbol != query.symbol {
                continue;
            }
            if scan.page.records.len() >= limit {
                scan.page.next_cursor = previous_cursor;
                scan.page.more_available = true;
                break;
            }
            scan.page.records.push(features);
            let candidate_bytes = serde_json::to_vec(&scan.page)?.len();
            if candidate_bytes > RESEARCH_HISTORY_MAX_RESPONSE_BYTES {
                scan.page.records.pop();
                bounded = true;
                scan.page.more_available = true;
                // Leave the cursor before this matching row to avoid silently
                // skipping it. The minimum feature record fits the payload cap.
                scan.page.next_cursor = previous_cursor;
                break;
            }
        }
        let exhausted_interval = !scan.page.more_available && !bounded;
        scan.page.history_complete = cursor.is_none()
            && exhausted_interval
            && !corrupt_record
            && scan.page.archived_days_unscanned == 0
            && scan.page.archived_days_unavailable == 0;
        let encoded = serde_json::to_vec(&scan.page)?;
        anyhow::ensure!(encoded.len() <= RESEARCH_HISTORY_MAX_RESPONSE_BYTES, "research history response exceeds payload limit");
        Ok(scan.page)
    }

    fn decode_research_history_record(compressed: &[u8]) -> Result<SignalResearchFeatures> {
        let decoder = MultiGzDecoder::new(compressed);
        let mut payload = Vec::new();
        decoder
            .take((RESEARCH_HISTORY_MAX_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut payload)?;
        anyhow::ensure!(payload.len() <= RESEARCH_HISTORY_MAX_RECORD_BYTES, "research feature record exceeds decoded size limit");
        match deserialize_record(4, &payload)? {
            StoredSignalRecord::ResearchFeatures(features) => Ok(features),
            _ => anyhow::bail!("unexpected record in research feature history"),
        }
    }

    fn archived_research_days(
        connection: &rusqlite::Connection,
        from_ms: u64,
        through_ms: u64,
    ) -> Result<usize> {
        let count = connection.query_row(
            "SELECT COUNT(*) FROM signal_daily_summary WHERE kind = 4 AND training_payload IS NOT NULL AND first_timestamp <= ?1 AND last_timestamp >= ?2",
            params![through_ms as i64, from_ms as i64],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count.max(0) as usize)
    }

    fn unavailable_research_days(
        connection: &rusqlite::Connection,
        from_ms: u64,
        through_ms: u64,
    ) -> Result<usize> {
        let count = connection.query_row(
            "SELECT COUNT(*) FROM signal_daily_summary AS archived WHERE kind = 4 AND training_payload IS NULL AND first_timestamp <= ?1 AND last_timestamp >= ?2 AND NOT EXISTS (SELECT 1 FROM signal_records AS raw WHERE raw.kind = 4 AND raw.timestamp >= archived.first_timestamp AND raw.timestamp <= archived.last_timestamp)",
            params![through_ms as i64, from_ms as i64],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count.max(0) as usize)
    }
}

fn parse_research_history_cursor(cursor: &str) -> Result<(u64, i64)> {
    anyhow::ensure!(cursor.len() <= 48, "cursor is too long");
    let (timestamp, id) = cursor.split_once(':').context("cursor must be timestamp:id")?;
    let timestamp = timestamp.parse::<u64>().context("cursor timestamp is invalid")?;
    let id = id.parse::<i64>().context("cursor row ID is invalid")?;
    anyhow::ensure!(timestamp <= i64::MAX as u64 && id > 0, "cursor is outside storage range");
    Ok((timestamp, id))
}

#[cfg(test)]
mod research_feature_history_tests {
    use super::*;

    struct TempDatabase {
        directory: PathBuf,
        path: PathBuf,
    }

    impl Drop for TempDatabase {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn feature(symbol: &str, timestamp: u64) -> SignalResearchFeatures {
        SignalResearchFeatures {
            symbol: symbol.to_owned(),
            timestamp,
            spread_bps: Some(1.0),
            microprice_offset_bps: None,
            bid_notional_10bps: None,
            ask_notional_10bps: None,
            depth_imbalance_10bps: None,
            depth_imbalance_change_1s: None,
            depth_imbalance_change_5s: None,
            trade_flow_5s: None,
            trade_flow_15s: None,
            trade_flow_60s: None,
            trade_flow_acceleration: None,
            signed_trade_notional_5s: None,
            signed_trade_notional_15s: None,
            trade_count_5s: 0,
            trade_count_60s: 0,
            average_trade_notional_5s: None,
            price_response_bps_per_1000usd_5s: None,
            btc_return_1m_percent: None,
            btc_return_5m_percent: None,
            eth_return_1m_percent: None,
            eth_return_5m_percent: None,
        }
    }

    fn database(records: &[(&str, u64)]) -> TempDatabase {
        static NEXT_TEMP_DB: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let unique = NEXT_TEMP_DB.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "insights-research-history-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("research.sqlite");
        let connection = SignalStore::open_database_at(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE signal_daily_summary(day_start_ms INTEGER NOT NULL, kind INTEGER NOT NULL, first_timestamp INTEGER NOT NULL, last_timestamp INTEGER NOT NULL, training_payload BLOB, PRIMARY KEY(day_start_ms, kind));",
            )
            .unwrap();
        for (symbol, timestamp) in records {
            let record = StoredSignalRecord::ResearchFeatures(feature(symbol, *timestamp));
            let compressed = compress_record(&record).unwrap();
            connection
                .execute(
                    "INSERT INTO signal_records(timestamp, kind, payload) VALUES (?1, 4, ?2)",
                    params![*timestamp as i64, compressed],
                )
                .unwrap();
        }
        drop(connection);
        TempDatabase { directory, path }
    }

    #[test]
    fn applies_symbol_and_inclusive_time_bounds() {
        let database = database(&[
            ("BTC-USDT", 99),
            ("BTC-USDT", 100),
            ("ETH-USDT", 120),
            ("BTC-USDT", 150),
            ("BTC-USDT", 151),
        ]);
        let page = SignalStore::research_feature_history_sync(
            &database.path,
            ResearchFeatureHistoryQuery {
                symbol: "BTC-USDT".into(),
                from_ms: 100,
                through_ms: 150,
                cursor: None,
                limit: Some(10),
            },
            None,
            10,
        )
        .unwrap();
        assert_eq!(page.records.iter().map(|record| record.timestamp).collect::<Vec<_>>(), [100, 150]);
        assert_eq!(page.coverage_start_ms, Some(100));
        assert_eq!(page.coverage_through_ms, Some(150));
        assert!(page.history_complete);
    }

    #[test]
    fn cursor_advances_through_other_symbols() {
        let database = database(&[("BTC-USDT", 100), ("ETH-USDT", 110), ("BTC-USDT", 120)]);
        let query = |cursor| ResearchFeatureHistoryQuery {
            symbol: "BTC-USDT".into(),
            from_ms: 0,
            through_ms: 200,
            cursor,
            limit: Some(1),
        };
        let first = SignalStore::research_feature_history_sync(&database.path, query(None), None, 1).unwrap();
        assert_eq!(first.records.len(), 1);
        assert_eq!(first.next_cursor.as_deref(), Some("110:2"));
        let second = SignalStore::research_feature_history_sync(
            &database.path,
            query(first.next_cursor.clone()),
            first
                .next_cursor
                .as_deref()
                .map(parse_research_history_cursor)
                .transpose()
                .unwrap(),
            1,
        )
        .unwrap();
        assert_eq!(second.records.len(), 1);
        assert_eq!(second.records[0].timestamp, 120);
        assert_eq!(second.next_cursor.as_deref(), Some("120:3"));
        assert_eq!(second.scanned_records, 1);
    }

    #[test]
    fn bounded_scan_reports_incomplete_coverage_and_continuation() {
        let rows = (0..(RESEARCH_HISTORY_MAX_SCAN + 2))
            .map(|index| ("ETH-USDT", index as u64 + 1))
            .collect::<Vec<_>>();
        let database = database(&rows);
        let page = SignalStore::research_feature_history_sync(
            &database.path,
            ResearchFeatureHistoryQuery {
                symbol: "BTC-USDT".into(),
                from_ms: 0,
                through_ms: u64::MAX.min(i64::MAX as u64),
                cursor: None,
                limit: Some(10),
            },
            None,
            10,
        )
        .unwrap();
        assert_eq!(page.scanned_records, RESEARCH_HISTORY_MAX_SCAN);
        assert!(page.records.is_empty());
        assert!(page.more_available);
        assert!(!page.history_complete);
        assert_eq!(page.next_cursor.as_deref(), Some("4096:4096"));
    }

    #[test]
    fn overlapping_archived_days_are_reported_as_incomplete_coverage() {
        let database = database(&[("BTC-USDT", 100)]);
        let connection = SignalStore::open_database_path(&database.path).unwrap();
        connection
            .execute(
                "INSERT INTO signal_daily_summary(day_start_ms, kind, first_timestamp, last_timestamp, training_payload) VALUES (0, 4, 50, 150, X'0102')",
                [],
            )
            .unwrap();
        drop(connection);

        let page = SignalStore::research_feature_history_sync(
            &database.path,
            ResearchFeatureHistoryQuery {
                symbol: "BTC-USDT".into(),
                from_ms: 75,
                through_ms: 125,
                cursor: None,
                limit: Some(10),
            },
            None,
            10,
        )
        .unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.archived_days_unscanned, 1);
        assert!(!page.history_complete);
    }
}
