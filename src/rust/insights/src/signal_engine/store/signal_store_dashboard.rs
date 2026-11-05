impl SignalStore {
    pub async fn auto_trader_dashboard(&self) -> Result<AutoTraderDashboard> {
        let (best, walk_forward) = tokio::try_join!(self.best(100), self.walk_forward())?;
        let missed_opportunity_bands = self.missed_opportunity_bands().await?;
        Ok(AutoTraderDashboard {
            best,
            walk_forward,
            missed_opportunity_bands,
        })
    }

    pub async fn dashboard(&self) -> Result<SignalDashboard> {
        let (best, history, performance, walk_forward) = tokio::try_join!(
            self.best(10),
            self.recent(200),
            self.performance(),
            self.walk_forward(),
        )?;
        Ok(SignalDashboard {
            best,
            history,
            performance,
            walk_forward,
        })
    }

    pub async fn recent(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        if limit > 200 {
            return self.load_recent(limit).await;
        }
        let mut cache = self.recent_cache.lock().await;
        if let Some((created, items)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(60) {
                let start = items.len().saturating_sub(limit);
                return Ok(items[start..].to_vec());
            }
        }
        let items = self.load_recent(200).await?;
        let start = items.len().saturating_sub(limit);
        *cache = Some((Instant::now(), items.clone()));
        Ok(items[start..].to_vec())
    }

    async fn load_recent(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        let records = self.read_calibration_records().await?;
        let mut items = records
            .iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Snapshot(snapshot) => Some(snapshot.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        calibrate_signal_scores(&mut items, &records);
        let start = items.len().saturating_sub(limit);
        Ok(items.drain(start..).collect())
    }

    pub async fn best(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        // The AutoTrader asks for more than the UI's ten rows. Keep a wider
        // cache so its five-second cycle does not repeatedly decompress and
        // recalibrate the entire signal history.
        if limit > 100 {
            return self.load_best(limit).await;
        }
        let mut cache = self.best_cache.lock().await;
        if let Some((created, items)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(60) {
                return Ok(items.iter().take(limit).cloned().collect());
            }
        }
        let items = self.load_best(100).await?;
        *cache = Some((Instant::now(), items.clone()));
        Ok(items.into_iter().take(limit).collect())
    }

    async fn load_best(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        let records = self.read_calibration_records().await?;
        let mut latest: HashMap<String, SignalSnapshot> = HashMap::new();
        for record in records.iter() {
            let StoredSignalRecord::Snapshot(snapshot) = record else {
                continue;
            };
            let snapshot = snapshot.clone();
            // Retain the latest ranging setup from disk across a service restart.
            // It remains visible until fresh feed data replaces it, with staleness
            // set below so clients can distinguish it from a live signal.
            if snapshot.stale && snapshot.direction != "ranging" {
                continue;
            }
            let replace = latest
                .get(&snapshot.symbol)
                .is_none_or(|current| current.timestamp < snapshot.timestamp);
            if replace {
                latest.insert(snapshot.symbol.clone(), snapshot);
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut items = latest
            .into_values()
            .filter_map(|mut item| {
                let age = now.saturating_sub(item.timestamp);
                if age > 60_000 && item.direction != "ranging" {
                    return None;
                }
                item.stale |= age > 60_000;
                Some(item)
            })
            .collect::<Vec<_>>();
        calibrate_signal_scores(&mut items, &records);
        items.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        items.truncate(limit);
        Ok(items)
    }
}
