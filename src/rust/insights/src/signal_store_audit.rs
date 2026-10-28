impl SignalStore {
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
    ) -> Result<Vec<serde_json::Value>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::load_auto_trader_execution_audit_sync(&path, since)
        })
        .await
        .map_err(|error| anyhow::anyhow!("execution audit read task failed: {error}"))?
    }

    fn load_auto_trader_execution_audit_sync(
        path: &PathBuf,
        since: Option<u64>,
    ) -> Result<Vec<serde_json::Value>> {
        let connection = Self::open_database_path(path)?;
        let mut statement = connection.prepare(
            "SELECT payload FROM auto_trader_audit WHERE (stage = 'entry' OR stage LIKE 'exit%') AND (?1 IS NULL OR timestamp >= ?1) ORDER BY timestamp ASC",
        )?;
        let rows = statement.query_map(params![since.map(|value| value as i64)], |row| {
            row.get::<_, Vec<u8>>(0)
        })?;
        let compressed_rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        drop(connection);
        compressed_rows
            .into_iter()
            .map(|compressed| {
                let mut decoder = MultiGzDecoder::new(compressed.as_slice());
                let mut json = Vec::new();
                decoder.read_to_end(&mut json)?;
                Ok(serde_json::from_slice(&json)?)
            })
            .collect()
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
