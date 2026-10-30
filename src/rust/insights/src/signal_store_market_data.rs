impl SignalStore {
    pub async fn book_snapshots(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<SignalBookSnapshot>> {
        let mut snapshots = self
            .read_records(3, since, Some(limit))
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::BookSnapshot(snapshot) => Some(snapshot),
                _ => None,
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.symbol.cmp(&b.symbol)));
        let start = snapshots.len().saturating_sub(limit);
        Ok(snapshots.drain(start..).collect())
    }

    pub async fn append_book_snapshot(&self, snapshot: &SignalBookSnapshot) -> Result<()> {
        self.append_record(StoredSignalRecord::BookSnapshot(snapshot.clone()))
            .await
    }

    pub async fn append_external_market_features(
        &self,
        features: &ExternalMarketFeatures,
    ) -> Result<()> {
        self.append_record(StoredSignalRecord::ExternalMarket(features.clone()))
            .await
    }

    pub async fn external_market_features(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<ExternalMarketFeatures>> {
        let mut features = self
            .read_records(5, since, Some(limit))
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::ExternalMarket(features) => Some(features),
                _ => None,
            })
            .collect::<Vec<_>>();
        features.sort_by(|a, b| {
            a.timestamp
                .cmp(&b.timestamp)
                .then(a.underlying.cmp(&b.underlying))
        });
        let start = features.len().saturating_sub(limit);
        Ok(features.drain(start..).collect())
    }

    pub async fn append_research_features(&self, features: &SignalResearchFeatures) -> Result<()> {
        self.append_record(StoredSignalRecord::ResearchFeatures(features.clone()))
            .await
    }

    pub async fn research_features(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<SignalResearchFeatures>> {
        let records = if since.is_none() {
            self.read_recent_records(4, limit.min(100_000)).await?
        } else {
            self.read_records(4, since, Some(limit.min(100_000)))
                .await?
        };
        let mut features = records
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::ResearchFeatures(features) => Some(features),
                _ => None,
            })
            .collect::<Vec<_>>();
        features.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.symbol.cmp(&b.symbol)));
        let start = features.len().saturating_sub(limit);
        Ok(features.drain(start..).collect())
    }

    pub async fn evaluations(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<SignalEvaluation>> {
        let lower_bound = since.map(|timestamp| timestamp.saturating_sub(3_600_000));
        let mut evaluations = self
            .read_records(2, lower_bound, Some(limit))
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Evaluation(evaluation)
                    if since
                        .is_none_or(|timestamp| evaluation.evaluated_timestamp >= timestamp) =>
                {
                    Some(evaluation)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        evaluations.sort_by(|a, b| {
            a.evaluated_timestamp
                .cmp(&b.evaluated_timestamp)
                .then(a.symbol.cmp(&b.symbol))
                .then(a.horizon_seconds.cmp(&b.horizon_seconds))
        });
        let start = evaluations.len().saturating_sub(limit);
        Ok(evaluations.drain(start..).collect())
    }
}
