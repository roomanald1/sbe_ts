impl SignalStore {
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
            params![before_id, since.map(|value| value as i64), (limit + 1) as i64],
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

    pub async fn load_shadow_decisions_near(
        &self,
        symbol: String,
        since: u64,
        until: u64,
    ) -> Result<Vec<serde_json::Value>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let connection = Self::open_database_path(&path)?;
            let mut statement = connection.prepare(
                "SELECT payload FROM auto_trader_audit WHERE stage = 'decision' AND symbol = ?1 AND timestamp >= ?2 AND timestamp <= ?3",
            )?;
            let rows = statement.query_map(
                params![symbol, since as i64, until as i64],
                |row| row.get::<_, Vec<u8>>(0),
            )?;
            let compressed_rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            compressed_rows
                .into_iter()
                .map(|compressed| {
                    let mut decoder = MultiGzDecoder::new(compressed.as_slice());
                    let mut json = Vec::new();
                    decoder.read_to_end(&mut json)?;
                    Ok(serde_json::from_slice(&json)?)
                })
                .collect()
        })
        .await
        .map_err(|error| anyhow::anyhow!("shadow decision audit read task failed: {error}"))?
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

    async fn load_auto_trader_audit(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        self.load_auto_trader_audit_with_corrupt_rows(since, limit, false)
            .await
    }

    async fn load_auto_trader_audit_with_corrupt_rows(
        &self,
        since: Option<u64>,
        limit: usize,
        skip_corrupt_rows: bool,
    ) -> Result<Vec<serde_json::Value>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::load_auto_trader_audit_sync(&path, since, limit, skip_corrupt_rows)
        })
        .await
        .map_err(|error| anyhow::anyhow!("audit read task failed: {error}"))?
    }

    fn load_auto_trader_audit_sync(
        path: &PathBuf,
        since: Option<u64>,
        limit: usize,
        skip_corrupt_rows: bool,
    ) -> Result<Vec<serde_json::Value>> {
        let connection = Self::open_database_path(path)?;
        let mut statement = connection.prepare("SELECT payload FROM auto_trader_audit WHERE (?1 IS NULL OR timestamp >= ?1) ORDER BY timestamp DESC LIMIT ?2")?;
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
        let mut result = Vec::new();
        for compressed in compressed_rows {
            let mut decoder = MultiGzDecoder::new(compressed.as_slice());
            let mut json = Vec::new();
            let decoded = decoder
                .read_to_end(&mut json)
                .map_err(anyhow::Error::from)
                .and_then(|_| serde_json::from_slice(&json).map_err(anyhow::Error::from));
            match decoded {
                Ok(value) => result.push(value),
                Err(_) if skip_corrupt_rows => continue,
                Err(error) => return Err(error),
            }
        }
        result.reverse();
        Ok(result)
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
            params![since.map(|value| value as i64), limit.clamp(1, 50_000) as i64],
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditPage {
    pub items: Vec<serde_json::Value>,
    pub next_cursor: Option<i64>,
}
