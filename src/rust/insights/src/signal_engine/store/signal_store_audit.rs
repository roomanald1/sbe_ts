impl SignalStore {
    async fn load_auto_trader_decision_audit_window(
        &self,
        since: u64,
    ) -> Result<Vec<serde_json::Value>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::load_auto_trader_decision_audit_window_sync(&path, since)
        })
        .await
        .map_err(|error| anyhow::anyhow!("decision audit window read task failed: {error}"))?
    }

    fn load_auto_trader_decision_audit_window_sync(
        path: &PathBuf,
        since: u64,
    ) -> Result<Vec<serde_json::Value>> {
        const PAGE_SIZE: usize = 5_000;
        const MAX_ROWS: usize = 100_000;
        let connection = Self::open_database_path(path)?;
        let mut before_id = None;
        let mut scanned = 0;
        let mut corrupt_rows = 0;
        let mut result = Vec::new();
        loop {
            let limit = PAGE_SIZE.min(MAX_ROWS - scanned);
            let mut statement = connection.prepare(
                "SELECT rowid, payload FROM auto_trader_audit \
                 WHERE stage = 'decision' AND timestamp >= ?1 \
                 AND (?2 IS NULL OR rowid < ?2) ORDER BY rowid DESC LIMIT ?3",
            )?;
            let rows = statement
                .query_map(params![since as i64, before_id, limit as i64], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if rows.is_empty() {
                break;
            }
            before_id = rows.last().map(|(id, _)| *id);
            scanned += rows.len();
            for (_, compressed) in &rows {
                let decoded = (|| -> Result<serde_json::Value> {
                    let mut decoder = MultiGzDecoder::new(compressed.as_slice());
                    let mut json = Vec::new();
                    decoder.read_to_end(&mut json)?;
                    Ok(serde_json::from_slice(&json)?)
                })();
                match decoded {
                    Ok(value) => result.push(value),
                    Err(_) => corrupt_rows += 1,
                }
            }
            if rows.len() < limit {
                break;
            }
            if scanned >= MAX_ROWS {
                eprintln!("Missed opportunity decision audit scan reached its {MAX_ROWS}-row cap");
                break;
            }
        }
        if corrupt_rows > 0 {
            eprintln!(
                "Skipped {corrupt_rows} corrupt decision audit rows during missed opportunity scan"
            );
        }
        Ok(result)
    }

    pub async fn load_auto_trader_execution_audit_page(
        &self,
        before_id: Option<i64>,
        since: Option<u64>,
        limit: usize,
    ) -> Result<AuditPage> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::load_auto_trader_execution_audit_page_sync(&path, before_id, since, limit)
        })
        .await
        .map_err(|error| anyhow::anyhow!("execution audit page read task failed: {error}"))?
    }

    fn load_auto_trader_execution_audit_page_sync(
        path: &PathBuf,
        before_id: Option<i64>,
        since: Option<u64>,
        limit: usize,
    ) -> Result<AuditPage> {
        let limit = limit.clamp(1, 5_000);
        let connection = Self::open_database_path(path)?;
        let mut statement = connection.prepare(
            "SELECT rowid, payload FROM auto_trader_audit WHERE (stage = 'entry' OR stage LIKE 'exit%') AND (?1 IS NULL OR rowid < ?1) AND (?2 IS NULL OR timestamp >= ?2) ORDER BY rowid DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![
                before_id,
                since.map(|value| value as i64),
                (limit + 1) as i64
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )?;
        let mut rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        drop(statement);
        drop(connection);

        let next_cursor = has_more.then(|| rows.last().map(|(id, _)| *id)).flatten();
        let mut items = rows
            .into_iter()
            .map(|(_, compressed)| {
                let mut decoder = MultiGzDecoder::new(compressed.as_slice());
                let mut json = Vec::new();
                decoder.read_to_end(&mut json)?;
                Ok(serde_json::from_slice(&json)?)
            })
            .collect::<Result<Vec<_>>>()?;
        items.reverse();
        Ok(AuditPage { items, next_cursor })
    }

    pub async fn save_auto_trader_audit(&self, payload: serde_json::Value) -> Result<()> {
        let decision_id = payload
            .get("decision_id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("audit record is missing decision_id"))?;
        let stage = payload
            .get("stage")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("audit record is missing stage"))?;
        let timestamp = payload
            .get("timestamp")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| anyhow::anyhow!("audit record is missing timestamp"))?;
        let symbol = payload
            .get("symbol")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let json = serde_json::to_vec(&payload)?;
        self.send_write(WriteOperation::Audit {
            decision_id: decision_id.to_owned(),
            stage: stage.to_owned(),
            timestamp: timestamp as i64,
            symbol: symbol.to_owned(),
            payload: json,
        })
        .await
    }

    pub async fn load_auto_trader_audit_page(
        &self,
        since: Option<u64>,
        before_id: Option<i64>,
        limit: usize,
        stage: Option<String>,
    ) -> Result<AuditPage> {
        let since = if stage.as_deref() == Some("strategy_rejection") {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let cutoff = now_ms.saturating_sub(60 * 60 * 1_000);
            Some(since.unwrap_or(cutoff).max(cutoff))
        } else {
            since
        };
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let limit = limit.clamp(1, 10_000);
            let include_final_outcome = stage.as_deref() == Some("decision");
            let connection = Self::open_database_path(&path)?;
            let sql = match (since.is_some(), before_id.is_some(), stage.is_some()) {
                (true, _, true) => "SELECT rowid, payload FROM auto_trader_audit WHERE stage = ?1 AND timestamp >= ?2 AND (?3 IS NULL OR rowid < ?3) ORDER BY timestamp DESC, rowid DESC LIMIT ?4",
                (_, true, true) => "SELECT rowid, payload FROM auto_trader_audit WHERE stage = ?1 AND rowid < ?2 ORDER BY rowid DESC LIMIT ?3",
                (true, false, false) => "SELECT rowid, payload FROM auto_trader_audit WHERE timestamp >= ?1 ORDER BY rowid DESC LIMIT ?2",
                (false, true, false) => "SELECT rowid, payload FROM auto_trader_audit WHERE rowid < ?1 ORDER BY rowid DESC LIMIT ?2",
                (false, false, true) => "SELECT rowid, payload FROM auto_trader_audit WHERE stage = ?1 ORDER BY rowid DESC LIMIT ?2",
                _ => "SELECT rowid, payload FROM auto_trader_audit ORDER BY rowid DESC LIMIT ?1",
            };
            let mut statement = connection.prepare(sql)?;
            let rows = match (since, before_id, stage) {
                (Some(since), before_id, Some(stage)) => statement.query_map(
                    params![stage, since as i64, before_id, (limit + 1) as i64],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )?.collect::<rusqlite::Result<Vec<_>>>()?,
                (None, Some(before_id), Some(stage)) => statement.query_map(
                    params![stage, before_id, (limit + 1) as i64],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )?.collect::<rusqlite::Result<Vec<_>>>()?,
                (Some(since), None, None) => statement.query_map(
                    params![since as i64, (limit + 1) as i64],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )?.collect::<rusqlite::Result<Vec<_>>>()?,
                (None, Some(before_id), None) => statement.query_map(
                    params![before_id, (limit + 1) as i64],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )?.collect::<rusqlite::Result<Vec<_>>>()?,
                (None, None, Some(stage)) => statement.query_map(
                    params![stage, (limit + 1) as i64],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )?.collect::<rusqlite::Result<Vec<_>>>()?,
                _ => statement.query_map(params![(limit + 1) as i64],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )?.collect::<rusqlite::Result<Vec<_>>>()?,
            };
            let mut rows = rows;
            let has_more = rows.len() > limit;
            rows.truncate(limit);
            drop(statement);
            drop(connection);
            let next_cursor = has_more.then(|| rows.last().map(|(id, _)| *id)).flatten();
            let mut items = Vec::with_capacity(rows.len());
            for (_, compressed) in rows {
                let mut decoder = MultiGzDecoder::new(compressed.as_slice());
                let mut json = Vec::new();
                decoder.read_to_end(&mut json)?;
                items.push(serde_json::from_slice(&json)?);
            }
            items.reverse();
            if include_final_outcome {
                attach_final_decision_outcomes(&path, &mut items)?;
            }
            Ok(AuditPage { items, next_cursor })
        })
        .await
        .map_err(|error| anyhow::anyhow!("audit page read task failed: {error}"))?
    }

    async fn load_auto_trader_execution_audit(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::load_auto_trader_execution_audit_sync(&path, since, limit)
        })
        .await
        .map_err(|error| anyhow::anyhow!("execution audit read task failed: {error}"))?
    }

    fn load_auto_trader_execution_audit_sync(
        path: &PathBuf,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let connection = Self::open_database_path(path)?;
        let mut statement = connection.prepare(
            "SELECT payload FROM auto_trader_audit WHERE (stage = 'entry' OR stage LIKE 'exit%') AND (?1 IS NULL OR timestamp >= ?1) ORDER BY timestamp DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![
                since.map(|value| value as i64),
                limit.clamp(1, 50_000) as i64
            ],
            |row| row.get::<_, Vec<u8>>(0),
        )?;
        let compressed_rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        drop(connection);
        let mut result = compressed_rows
            .into_iter()
            .map(|compressed| {
                let mut decoder = MultiGzDecoder::new(compressed.as_slice());
                let mut json = Vec::new();
                decoder.read_to_end(&mut json)?;
                Ok(serde_json::from_slice(&json)?)
            })
            .collect::<Result<Vec<_>>>()?;
        result.reverse();
        Ok(result)
    }

    async fn count_auto_trader_decisions(&self, since: Option<u64>) -> Result<usize> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let connection = Self::open_database_path(&path)?;
            let count = connection.query_row(
                "SELECT COUNT(*) FROM auto_trader_audit WHERE stage = 'decision' AND (?1 IS NULL OR timestamp >= ?1)",
                params![since.map(|value| value as i64)],
                |row| row.get::<_, i64>(0),
            )?;
            Ok(count as usize)
        })
        .await
        .map_err(|error| anyhow::anyhow!("decision audit count task failed: {error}"))?
    }
}

fn attach_final_decision_outcomes(
    path: &PathBuf,
    decisions: &mut [serde_json::Value],
) -> Result<()> {
    let connection = SignalStore::open_database_path(path)?;
    let mut statement = connection.prepare(
        "SELECT payload FROM auto_trader_audit WHERE decision_id = ?1 AND stage = 'entry'",
    )?;

    for decision in decisions {
        if decision.get("stage").and_then(serde_json::Value::as_str) != Some("decision") {
            continue;
        }

        let selected = matches!(
            decision.get("status").and_then(serde_json::Value::as_str),
            Some("selected" | "scouted")
        );
        if !selected {
            decision["final_outcome"] = serde_json::json!({
                "status": decision.get("status"),
                "reason": decision.get("reason"),
                "entry": null,
            });
            continue;
        }

        let entry = if let Some(decision_id) = decision
            .get("decision_id")
            .and_then(serde_json::Value::as_str)
        {
            let mut rows = statement.query(params![decision_id])?;
            rows.next()?
                .map(|row| row.get::<_, Vec<u8>>(0))
                .transpose()?
                .map(|compressed| -> Result<serde_json::Value> {
                    let mut decoder = MultiGzDecoder::new(compressed.as_slice());
                    let mut json = Vec::new();
                    decoder.read_to_end(&mut json)?;
                    Ok(serde_json::from_slice(&json)?)
                })
                .transpose()?
        } else {
            None
        };

        let final_outcome = match entry {
            Some(entry) => serde_json::json!({
                "status": entry.get("status"),
                "reason": entry.get("reason"),
                "entry": entry,
            }),
            None => serde_json::json!({
                "status": "pending",
                "reason": null,
                "entry": null,
            }),
        };
        decision["final_outcome"] = final_outcome;
    }

    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditPage {
    pub items: Vec<serde_json::Value>,
    pub next_cursor: Option<i64>,
}

#[cfg(test)]
mod decision_audit_window_tests {
    use super::*;

    fn test_store(path: PathBuf) -> SignalStore {
        let connection = SignalStore::open_database_at(&path).unwrap();
        let (writer, receiver) = mpsc::channel(128);
        tokio::task::spawn_blocking(move || run_database_writer(connection, receiver));
        SignalStore {
            path,
            writer,
            recent_cache: Arc::new(Mutex::new(None)),
            best_cache: Arc::new(Mutex::new(None)),
            performance_cache: Arc::new(Mutex::new(None)),
            execution_quality_cache: Arc::new(Mutex::new(None)),
            decision_quality_cache: Arc::new(Mutex::new(None)),
            calibration_records_cache: Arc::new(Mutex::new(None)),
        }
    }

    #[test]
    fn scans_recent_decision_records_in_pages_and_skips_other_stages() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insights-decision-audit-{}-{unique}.sqlite",
            std::process::id()
        ));
        let mut connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "CREATE TABLE auto_trader_audit (stage TEXT, timestamp INTEGER, payload BLOB)",
                [],
            )
            .unwrap();
        let transaction = connection.transaction().unwrap();
        for timestamp in 1_000..=6_000 {
            let payload = serde_json::json!({ "timestamp": timestamp });
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(&serde_json::to_vec(&payload).unwrap())
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO auto_trader_audit (stage, timestamp, payload) VALUES (?1, ?2, ?3)",
                    params!["decision", timestamp, encoder.finish().unwrap()],
                )
                .unwrap();
        }
        transaction
            .execute(
                "INSERT INTO auto_trader_audit (stage, timestamp, payload) VALUES (?1, ?2, ?3)",
                params![
                    "entry",
                    6_000,
                    GzEncoder::new(Vec::new(), Compression::default())
                        .finish()
                        .unwrap()
                ],
            )
            .unwrap();
        transaction.commit().unwrap();
        drop(connection);

        let records =
            SignalStore::load_auto_trader_decision_audit_window_sync(&path, 1_000).unwrap();
        assert_eq!(records.len(), 5_001);
        assert_eq!(records[0]["timestamp"], 6_000);

        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn strategy_rejection_audit_round_trips_through_persisted_history() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insights-strategy-rejection-audit-{}-{unique}.sqlite",
            std::process::id()
        ));
        let store = test_store(path.clone());
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let payload = serde_json::json!({
            "decision_id": "signal-1|strategy:live-order-book|phase:live_worker",
            "stage": "strategy_rejection",
            "timestamp": timestamp,
            "symbol": "LINK-USDT",
            "strategy": "live-order-book",
            "status": "rejected",
            "reason": "Signal strength 0.12 is below the required minimum 0.30"
        });
        let mut expired_payload = payload.clone();
        expired_payload["decision_id"] = serde_json::json!("expired-rejection");
        expired_payload["timestamp"] = serde_json::json!(
            timestamp.saturating_sub(2 * 60 * 60 * 1_000)
        );
        store.save_auto_trader_audit(expired_payload).await.unwrap();
        store.save_auto_trader_audit(payload.clone()).await.unwrap();

        let page = store
            .load_auto_trader_audit_page(
                Some(0),
                None,
                100,
                Some("strategy_rejection".to_owned()),
            )
            .await
            .unwrap();

        assert_eq!(page.items, vec![payload]);
        drop(store);
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = std::fs::remove_file(path);
    }
}
