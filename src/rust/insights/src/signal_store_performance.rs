impl SignalStore {
    pub async fn performance(&self) -> Result<Vec<SignalPerformance>> {
        let mut cache = self.performance_cache.lock().await;
        if let Some((created, items)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(60) {
                return Ok(items.clone());
            }
        }
        let items = self.compute_performance().await?;
        *cache = Some((Instant::now(), items.clone()));
        Ok(items)
    }

    async fn compute_performance(&self) -> Result<Vec<SignalPerformance>> {
        let records = self.read_calibration_records().await?;
        let mut totals: HashMap<(String, u64), PerformanceTotals> = HashMap::new();
        for outcome in records.iter().filter_map(|record| match record {
            StoredSignalRecord::Outcome(outcome) => Some(outcome),
            _ => None,
        }) {
            let total = totals
                .entry((outcome.symbol.clone(), outcome.horizon_seconds))
                .or_default();
            total.sample_count += 1;
            total.raw_return_total += outcome.return_percent;
            match outcome.direction.as_str() {
                "bullish" => {
                    total.directional_count += 1;
                    total.bullish_count += 1;
                    if outcome.direction_match {
                        total.bullish_matches += 1;
                        total.directional_matches += 1;
                    }
                }
                "bearish" => {
                    total.directional_count += 1;
                    total.bearish_count += 1;
                    if outcome.direction_match {
                        total.bearish_matches += 1;
                        total.directional_matches += 1;
                    }
                }
                "neutral" => total.neutral_count += 1,
                _ => {}
            }
            if let Some(net_return) = outcome.net_return_percent {
                total.net_count += 1;
                total.net_return_total += net_return;
            }
        }
        let mut result = totals
            .into_iter()
            .map(|((symbol, horizon_seconds), total)| SignalPerformance {
                symbol,
                horizon_seconds,
                sample_count: total.sample_count,
                directional_sample_count: total.directional_count,
                neutral_sample_count: total.neutral_count,
                bullish_match_rate: (total.bullish_count > 0)
                    .then_some(total.bullish_matches as f64 / total.bullish_count as f64),
                bearish_match_rate: (total.bearish_count > 0)
                    .then_some(total.bearish_matches as f64 / total.bearish_count as f64),
                direction_match_rate: (total.directional_count > 0)
                    .then_some(total.directional_matches as f64 / total.directional_count as f64),
                average_return_percent: total.raw_return_total / total.sample_count as f64,
                net_sample_count: total.net_count,
                average_net_return_percent: (total.net_count > 0)
                    .then_some(total.net_return_total / total.net_count as f64),
            })
            .collect::<Vec<_>>();
        result.sort_by(|a, b| {
            a.symbol
                .cmp(&b.symbol)
                .then(a.horizon_seconds.cmp(&b.horizon_seconds))
        });
        Ok(result)
    }
}
