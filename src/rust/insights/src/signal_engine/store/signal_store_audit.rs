impl SignalStore {
    async fn load_auto_trader_decision_audit_window(
        &self,
        since: u64,
    ) -> Result<Vec<serde_json::Value>> {
        let path = self.audit_path.clone();
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
        let path = self.audit_path.clone();
        tokio::task::spawn_blocking(move || {
            Self::load_auto_trader_execution_audit_page_sync(&path, before_id, since, limit)
        })
        .await
        .map_err(|error| anyhow::anyhow!("execution audit page read task failed: {error}"))?
    }

    pub async fn load_auto_trader_entry_audits(
        &self,
        decision_ids: Vec<String>,
    ) -> Result<Vec<serde_json::Value>> {
        let path = self.audit_path.clone();
        tokio::task::spawn_blocking(move || {
            Self::load_auto_trader_entry_audits_sync(&path, decision_ids)
        })
        .await
        .map_err(|error| anyhow::anyhow!("entry audit lookup task failed: {error}"))?
    }

    fn load_auto_trader_entry_audits_sync(
        path: &PathBuf,
        decision_ids: Vec<String>,
    ) -> Result<Vec<serde_json::Value>> {
        const MAX_DECISION_IDS: usize = 500;
        let decision_ids = decision_ids
            .into_iter()
            .filter(|id| !id.is_empty())
            .take(MAX_DECISION_IDS)
            .collect::<Vec<_>>();
        if decision_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat("?")
            .take(decision_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT payload FROM auto_trader_audit WHERE stage = 'entry' AND decision_id IN ({placeholders})"
        );
        let connection = Self::open_database_path(path)?;
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(decision_ids.iter()), |row| {
            row.get::<_, Vec<u8>>(0)
        })?;
        rows.map(|compressed| {
            let compressed = compressed?;
            let mut decoder = MultiGzDecoder::new(compressed.as_slice());
            let mut json = Vec::new();
            decoder.read_to_end(&mut json)?;
            Ok(serde_json::from_slice(&json)?)
        })
        .collect()
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
        self.send_audit_write(WriteOperation::Audit {
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
        let path = self.audit_path.clone();
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
        let path = self.audit_path.clone();
        tokio::task::spawn_blocking(move || {
            Self::load_auto_trader_execution_audit_sync(&path, since, limit)
        })
        .await
        .map_err(|error| anyhow::anyhow!("execution audit read task failed: {error}"))?
    }

    // Decision quality needs cumulative entry-fill snapshots to reconstruct the
    // final filled quantity and weighted price. Keep these out of the shared
    // execution-quality sample, where each snapshot would be counted as an
    // additional order sample.
    async fn load_auto_trader_decision_quality_audit(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let path = self.audit_path.clone();
        tokio::task::spawn_blocking(move || {
            Self::load_auto_trader_decision_quality_audit_sync(&path, since, limit)
        })
        .await
        .map_err(|error| anyhow::anyhow!("decision quality audit read task failed: {error}"))?
    }

    fn load_auto_trader_decision_quality_audit_sync(
        path: &PathBuf,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let connection = Self::open_database_path(path)?;
        let sql = if since.is_some() {
            "SELECT payload FROM auto_trader_audit WHERE (stage = 'entry' OR stage = 'entry_fill' OR stage LIKE 'exit%') AND timestamp >= ?1 ORDER BY timestamp DESC, rowid DESC LIMIT ?2"
        } else {
            "SELECT payload FROM auto_trader_audit WHERE (stage = 'entry' OR stage = 'entry_fill' OR stage LIKE 'exit%') ORDER BY timestamp DESC, rowid DESC LIMIT ?1"
        };
        let mut statement = connection.prepare(sql)?;
        let row_limit = limit.clamp(1, 50_000) as i64;
        let rows = if let Some(since) = since {
            statement.query_map(params![since as i64, row_limit], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            statement.query_map(params![row_limit], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        drop(statement);
        drop(connection);
        let mut result = rows.into_iter().map(|compressed| {
            let mut decoder = MultiGzDecoder::new(compressed.as_slice());
            let mut json = Vec::new();
            decoder.read_to_end(&mut json)?;
            Ok(serde_json::from_slice(&json)?)
        }).collect::<Result<Vec<_>>>()?;
        result.reverse();
        Ok(result)
    }

    fn load_auto_trader_execution_audit_sync(
        path: &PathBuf,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let connection = Self::open_database_path(path)?;
        let sql = if since.is_some() {
            "SELECT payload FROM auto_trader_audit WHERE (stage = 'entry' OR stage LIKE 'exit%') AND timestamp >= ?1 ORDER BY timestamp DESC, rowid DESC LIMIT ?2"
        } else {
            "SELECT payload FROM auto_trader_audit WHERE (stage = 'entry' OR stage LIKE 'exit%') ORDER BY timestamp DESC, rowid DESC LIMIT ?1"
        };
        let mut statement = connection.prepare(sql)?;
        let row_limit = limit.clamp(1, 50_000) as i64;
        let compressed_rows = if let Some(since) = since {
            statement.query_map(params![since as i64, row_limit], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            statement.query_map(params![row_limit], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
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
        let path = self.audit_path.clone();
        tokio::task::spawn_blocking(move || {
            let connection = Self::open_database_path(&path)?;
            let count = if let Some(since) = since {
                connection.query_row(
                    "SELECT COUNT(*) FROM auto_trader_audit WHERE stage = 'decision' AND timestamp >= ?1",
                    params![since as i64],
                    |row| row.get::<_, i64>(0),
                )?
            } else {
                connection.query_row(
                    "SELECT COUNT(*) FROM auto_trader_audit WHERE stage = 'decision'",
                    [],
                    |row| row.get::<_, i64>(0),
                )?
            };
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
    let mut entry_status_statement = connection.prepare(
        "SELECT payload FROM auto_trader_audit WHERE decision_id = ?1 AND stage = 'entry_status'",
    )?;
    // The stage/timestamp index makes SQLite scan every retained rejection to
    // satisfy the time ordering. These prefix ranges are selective by
    // decision_id, so use the existing (decision_id, stage) primary-key index.
    let mut skipped_statement = connection.prepare(
        "WITH final_entry AS (
             SELECT payload, timestamp FROM auto_trader_audit INDEXED BY sqlite_autoindex_auto_trader_audit_1
             WHERE stage = 'strategy_rejection' AND decision_id >= ?1 AND decision_id < ?2
             ORDER BY timestamp DESC LIMIT 1
         ), daily_execution AS (
             SELECT payload, timestamp FROM auto_trader_audit INDEXED BY sqlite_autoindex_auto_trader_audit_1
             WHERE stage = 'strategy_rejection' AND decision_id >= ?3 AND decision_id < ?4
             ORDER BY timestamp DESC LIMIT 1
         ), async_submission AS (
             SELECT payload, timestamp FROM auto_trader_audit INDEXED BY sqlite_autoindex_auto_trader_audit_1
             WHERE stage = 'strategy_rejection' AND decision_id >= ?5 AND decision_id < ?6
             ORDER BY timestamp DESC LIMIT 1
         ) SELECT payload FROM (
             SELECT * FROM final_entry
             UNION ALL SELECT * FROM daily_execution
             UNION ALL SELECT * FROM async_submission
         ) ORDER BY timestamp DESC LIMIT 1",
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

        let (mut entry, skipped_outcome) = if let Some(decision_id) = decision
            .get("decision_id")
            .and_then(serde_json::Value::as_str)
        {
            let mut rows = statement.query(params![decision_id])?;
            let entry = rows.next()?
                .map(|row| row.get::<_, Vec<u8>>(0))
                .transpose()?
                .map(|compressed| -> Result<serde_json::Value> {
                    let mut decoder = MultiGzDecoder::new(compressed.as_slice());
                    let mut json = Vec::new();
                    decoder.read_to_end(&mut json)?;
                    Ok(serde_json::from_slice(&json)?)
                })
                .transpose()?;
            if entry.is_some() {
                (entry, None)
            } else {
                let strategy = decision
                    .get("strategy")
                    .or_else(|| decision.get("strategy_decision").and_then(|value| value.get("strategy")))
                    .and_then(serde_json::Value::as_str);
                if let Some(strategy) = strategy {
                    let ranges = ["final_entry", "daily_execution", "async_submission"]
                        .map(|phase| {
                            let prefix = format!(
                                "{decision_id}|strategy:{strategy}|phase:{phase}|audit:"
                            );
                            let mut upper_bound = prefix.as_bytes().to_vec();
                            *upper_bound.last_mut().expect("audit prefix is nonempty") += 1;
                            let upper_bound = String::from_utf8(upper_bound)
                                .expect("incremented audit prefix remains valid UTF-8");
                            (prefix, upper_bound)
                        });
                    let mut rows = skipped_statement.query(params![
                        ranges[0].0,
                        ranges[0].1,
                        ranges[1].0,
                        ranges[1].1,
                        ranges[2].0,
                        ranges[2].1,
                    ])?;
                    let skipped_outcome = rows
                        .next()?
                        .map(|row| row.get::<_, Vec<u8>>(0))
                        .transpose()?
                        .map(|compressed| -> Result<serde_json::Value> {
                            let mut decoder = MultiGzDecoder::new(compressed.as_slice());
                            let mut json = Vec::new();
                            decoder.read_to_end(&mut json)?;
                            Ok(serde_json::from_slice(&json)?)
                        })
                        .transpose()?;
                    (None, skipped_outcome)
                } else {
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        if let (Some(decision_id), Some(entry)) = (
            decision.get("decision_id").and_then(serde_json::Value::as_str),
            entry.as_mut(),
        ) {
            let mut rows = entry_status_statement.query(params![decision_id])?;
            let status = rows
                .next()?
                .map(|row| row.get::<_, Vec<u8>>(0))
                .transpose()?
                .map(|compressed| -> Result<serde_json::Value> {
                    let mut decoder = MultiGzDecoder::new(compressed.as_slice());
                    let mut json = Vec::new();
                    decoder.read_to_end(&mut json)?;
                    Ok(serde_json::from_slice(&json)?)
                })
                .transpose()?;
            if let Some(status) = status {
                for field in [
                    "status",
                    "reason",
                    "execution_order_id",
                    "order_type",
                    "order_quantity",
                    "filled_quantity",
                    "average_price",
                    "fees",
                    "fill_rate",
                ] {
                    if let Some(value) = status.get(field) {
                        entry[field] = value.clone();
                    }
                }
                if let Some(execution_response) = entry
                    .get_mut("execution_response")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    execution_response.insert(
                        "status".into(),
                        status.get("status").cloned().unwrap_or(serde_json::Value::Null),
                    );
                    execution_response.insert(
                        "failure_reason".into(),
                        status.get("reason").cloned().unwrap_or(serde_json::Value::Null),
                    );
                }
            }

            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let still_active = matches!(
                entry.get("status").and_then(serde_json::Value::as_str),
                Some("submitting" | "accepted" | "resting" | "partially_filled")
            );
            let expired = entry
                .get("expires_at")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|expires_at| expires_at <= now_ms);
            if still_active && expired {
                let partially_filled = entry
                    .get("filled_quantity")
                    .and_then(serde_json::Value::as_f64)
                    .is_some_and(|quantity| quantity > 1e-12);
                let status = if partially_filled {
                    "partially_filled_closed"
                } else {
                    "expired_or_cancelled"
                };
                let reason = if partially_filled {
                    "expired_with_partial_fill"
                } else {
                    "expired_without_full_fill"
                };
                entry["status"] = serde_json::json!(status);
                entry["reason"] = serde_json::json!(reason);
                if let Some(execution_response) = entry
                    .get_mut("execution_response")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    execution_response.insert("status".into(), serde_json::json!(status));
                    execution_response.insert("failure_reason".into(), serde_json::json!(reason));
                }
            }
        }

        let mut final_outcome = if let Some(entry) = entry {
            serde_json::json!({
                "status": entry.get("status"),
                "reason": entry.get("reason"),
                "entry": entry,
                "execution_gate": {
                    "gate_id": "order_submission",
                    "stage": "post_selection_execution",
                    "status": entry.get("status"),
                    "order_submitted": true,
                    "reason": entry.get("reason"),
                },
                "gate_path": [
                    {
                        "stage": "candidate_selection",
                        "status": "selected",
                        "selection_mode": decision.get("entry_gate").and_then(|gate| gate.get("selection_mode")),
                        "gate": decision.get("entry_gate"),
                    },
                    {
                        "stage": "execution",
                        "status": entry.get("status"),
                        "gate_id": "order_submission",
                        "order_submitted": true,
                        "entry": entry,
                    }
                ],
            })
        } else if let Some(outcome) = skipped_outcome {
            let selection_gate = decision.get("entry_gate");
            let execution_gate = outcome.get("execution_gate").cloned().unwrap_or_else(|| {
                serde_json::json!({
                    "gate_id": "execution_or_risk_gate",
                    "stage": "post_selection_execution",
                    "status": "blocked",
                    "result_status": outcome.get("status"),
                    "reason": outcome.get("reason"),
                    "evaluated": true,
                    "order_submitted": false,
                    "short_circuited": true,
                    "subsequent_checks": "not_evaluated_after_block",
                })
            });
            let gate_path = outcome.get("gate_path").cloned().unwrap_or_else(|| {
                serde_json::json!([
                    {
                        "stage": "candidate_selection",
                        "status": "selected",
                        "selection_mode": selection_gate.and_then(|gate| gate.get("selection_mode")),
                        "gate": selection_gate,
                    },
                    {
                        "stage": "execution",
                        "status": "blocked",
                        "gate_id": "execution_or_risk_gate",
                        "reason": outcome.get("reason"),
                        "evaluated": true,
                        "short_circuited": true,
                        "subsequent_checks": "not_evaluated_after_block",
                    }
                ])
            });
            serde_json::json!({
                "status": outcome.get("status"),
                "reason": outcome.get("reason"),
                "entry": null,
                "execution_gate": execution_gate,
                "gate_path": gate_path,
            })
        } else {
            serde_json::json!({
                "status": "pending",
                "reason": null,
                "entry": null,
                "execution_gate": {
                    "gate_id": "order_submission",
                    "stage": "post_selection_execution",
                    "status": "pending",
                    "order_submitted": null,
                    "reason": "No entry or final-entry gate audit is linked to this decision yet.",
                },
                "gate_path": [
                    {
                        "stage": "candidate_selection",
                        "status": "selected",
                        "selection_mode": decision.get("entry_gate").and_then(|gate| gate.get("selection_mode")),
                        "gate": decision.get("entry_gate"),
                    },
                    {
                        "stage": "execution",
                        "status": "pending",
                        "gate_id": "order_submission",
                        "order_submitted": null,
                        "reason": "No downstream execution result has been recorded.",
                    }
                ],
            })
        };
        if let Some(entry) = final_outcome
            .get_mut("entry")
            .filter(|entry| !entry.is_null())
        {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let still_active = matches!(
                entry.get("status").and_then(serde_json::Value::as_str),
                Some("submitting" | "accepted" | "resting" | "partially_filled")
            );
            let expired = entry
                .get("expires_at")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|expires_at| expires_at <= now_ms);
            if still_active && expired {
                let partially_filled = entry
                    .get("filled_quantity")
                    .and_then(serde_json::Value::as_f64)
                    .is_some_and(|quantity| quantity > 1e-12);
                let status = if partially_filled {
                    "partially_filled_closed"
                } else {
                    "expired_or_cancelled"
                };
                let reason = if partially_filled {
                    "expired_with_partial_fill"
                } else {
                    "expired_without_full_fill"
                };
                entry["status"] = serde_json::json!(status);
                entry["reason"] = serde_json::json!(reason);
                final_outcome["status"] = serde_json::json!(status);
                final_outcome["reason"] = serde_json::json!(reason);
                final_outcome["execution_gate"]["status"] = serde_json::json!(status);
                final_outcome["execution_gate"]["reason"] = serde_json::json!(reason);
                if let Some(execution) = final_outcome
                    .get_mut("gate_path")
                    .and_then(serde_json::Value::as_array_mut)
                    .and_then(|path| path.get_mut(1))
                {
                    execution["status"] = serde_json::json!(status);
                    execution["reason"] = serde_json::json!(reason);
                    if let Some(path_entry) = execution.get_mut("entry") {
                        path_entry["status"] = serde_json::json!(status);
                        path_entry["reason"] = serde_json::json!(reason);
                    }
                }
            }
        }
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
        let audit_path = path.with_file_name(format!(
            "{}-audit.sqlite",
            path.file_stem().unwrap().to_string_lossy()
        ));
        let audit_connection = SignalStore::open_audit_database_at(&audit_path).unwrap();
        migrate_legacy_audit_tables(&path, &audit_path).unwrap();
        let (audit_writer, audit_receiver) = mpsc::channel(128);
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
            "reason": "Strength 0.12 < 0.30"
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
        let audit_path = store.audit_path.clone();
        drop(store);
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(audit_path);
    }

    fn insert_compressed_audit(
        connection: &Connection,
        stage: &str,
        timestamp: u64,
        id: &str,
    ) {
        let payload = serde_json::json!({
            "stage": stage,
            "timestamp": timestamp,
            "trade_id": "trade-1",
            "record_id": id,
            "filled_quantity": if id == "partial" { 1.0 } else { 3.0 },
            "average_price": if id == "partial" { 100.0 } else { 101.0 },
            "fees": if id == "partial" { 0.1 } else { 0.3 }
        });
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&serde_json::to_vec(&payload).unwrap()).unwrap();
        connection.execute(
            "INSERT INTO auto_trader_audit (stage, timestamp, payload) VALUES (?1, ?2, ?3)",
            params![stage, timestamp as i64, encoder.finish().unwrap()],
        ).unwrap();
    }

    fn insert_audit_payload(connection: &Connection, payload: serde_json::Value) {
        let stage = payload["stage"].as_str().unwrap();
        let timestamp = payload["timestamp"].as_u64().unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&serde_json::to_vec(&payload).unwrap()).unwrap();
        connection.execute(
            "INSERT INTO auto_trader_audit (decision_id, stage, timestamp, symbol, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![payload["decision_id"].as_str().unwrap(), stage, timestamp as i64,
                   payload["symbol"].as_str(), encoder.finish().unwrap()],
        ).unwrap();
    }

    #[test]
    fn decision_quality_loads_cumulative_entry_fills_without_changing_execution_samples() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insights-entry-fill-audit-{}-{unique}.sqlite",
            std::process::id()
        ));
        let connection = Connection::open(&path).unwrap();
        connection.execute(
            "CREATE TABLE auto_trader_audit (stage TEXT, timestamp INTEGER, payload BLOB)",
            [],
        ).unwrap();
        insert_compressed_audit(&connection, "entry", 100, "entry");
        insert_compressed_audit(&connection, "entry_fill", 110, "partial");
        insert_compressed_audit(&connection, "entry_fill", 120, "complete");
        insert_compressed_audit(&connection, "exit_fill", 130, "exit");
        drop(connection);

        let execution = SignalStore::load_auto_trader_execution_audit_sync(&path, None, 100).unwrap();
        assert_eq!(execution.iter().map(|row| row["stage"].as_str().unwrap()).collect::<Vec<_>>(),
            vec!["entry", "exit_fill"]);

        let decision = SignalStore::load_auto_trader_decision_quality_audit_sync(&path, Some(110), 100).unwrap();
        assert_eq!(decision.iter().map(|row| row["record_id"].as_str().unwrap()).collect::<Vec<_>>(),
            vec!["partial", "complete", "exit"]);
        assert_eq!(decision.last().unwrap()["filled_quantity"], 3.0);

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn decision_quality_orders_equal_timestamp_fill_snapshots_by_rowid() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insights-entry-fill-order-{}-{unique}.sqlite",
            std::process::id()
        ));
        let connection = Connection::open(&path).unwrap();
        connection.execute(
            "CREATE TABLE auto_trader_audit (stage TEXT, timestamp INTEGER, payload BLOB)",
            [],
        ).unwrap();
        insert_compressed_audit(&connection, "entry_fill", 110, "first");
        insert_compressed_audit(&connection, "entry_fill", 110, "second");
        drop(connection);

        let records = SignalStore::load_auto_trader_decision_quality_audit_sync(&path, Some(110), 100).unwrap();
        assert_eq!(records.iter().map(|row| row["record_id"].as_str().unwrap()).collect::<Vec<_>>(),
            vec!["first", "second"]);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn decision_quality_uses_final_cumulative_entry_fill_for_closed_trade_return() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insights-decision-entry-fill-summary-{}-{unique}.sqlite",
            std::process::id()
        ));
        let store = test_store(path.clone());
        let audit_path = store.audit_path.clone();
        let connection = Connection::open(&audit_path).unwrap();
        insert_audit_payload(&connection, serde_json::json!({
            "decision_id":"decision-1", "trade_id":"trade-1", "stage":"decision", "timestamp":100,
            "symbol":"BTC-USDT"
        }));
        insert_audit_payload(&connection, serde_json::json!({
            "decision_id":"decision-1", "trade_id":"trade-1", "stage":"entry", "timestamp":101,
            "symbol":"BTC-USDT", "strategy":"fixture", "side":"buy", "filled_quantity":0.0,
            "average_price":100.0, "fees":0.0, "quote_currency":"USDT", "direction":"long"
        }));
        insert_audit_payload(&connection, serde_json::json!({
            "decision_id":"decision-1", "trade_id":"trade-1", "stage":"entry_fill", "timestamp":103,
            "symbol":"BTC-USDT", "filled_quantity":2.0, "average_price":101.0, "fees":0.2
        }));
        insert_audit_payload(&connection, serde_json::json!({
            "decision_id":"decision-1", "trade_id":"trade-1", "stage":"exit_fill", "timestamp":104,
            "symbol":"BTC-USDT", "exit_filled_quantity":2.0, "realized_pnl":4.0, "trade_closed":true
        }));
        drop(connection);

        let summary = store.compute_auto_trader_decision_quality(None).await.unwrap();
        assert_eq!(summary.decision_samples, 1);
        assert_eq!(summary.filled_entry_samples, 1);
        assert_eq!(summary.completed_trade_samples, 1);
        assert_eq!(summary.unresolved_trade_samples, 0);
        assert!(summary.cohorts.iter().any(|cohort| {
            cohort.total_net_pnl == Some(4.0)
                && (cohort.average_net_return_percent - (4.0 / 202.0 * 100.0)).abs() < 1e-9
        }));

        drop(store);
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(audit_path);
    }
}
