impl SignalStore {
    async fn read_outcomes(&self) -> Result<Vec<SignalOutcome>> {
        Ok(self
            .read_recent_records(1, WALK_FORWARD_OUTCOME_LIMIT)
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Outcome(outcome) => Some(outcome),
                _ => None,
            })
            .collect())
    }

    async fn read_snapshots(&self) -> Result<Vec<SignalSnapshot>> {
        Ok(self
            .read_recent_records(0, WALK_FORWARD_RECORD_LIMIT)
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Snapshot(snapshot) => Some(snapshot),
                _ => None,
            })
            .collect())
    }

    pub async fn append(&self, snapshot: &SignalSnapshot) -> Result<()> {
        self.append_record(StoredSignalRecord::Snapshot(snapshot.clone()))
            .await
    }

    pub async fn append_outcome(&self, outcome: &SignalOutcome) -> Result<()> {
        self.append_record(StoredSignalRecord::Outcome(outcome.clone()))
            .await
    }

    pub async fn append_evaluation(&self, evaluation: &SignalEvaluation) -> Result<()> {
        let mut records = vec![StoredSignalRecord::Evaluation(evaluation.clone())];
        if !evaluation.counterfactual_quote_path.is_empty() {
            records.push(StoredSignalRecord::CounterfactualQuotePath(
                CounterfactualQuotePath {
                    symbol: evaluation.symbol.clone(),
                    signal_timestamp: evaluation.signal_timestamp,
                    horizon_seconds: evaluation.horizon_seconds,
                    samples: evaluation.counterfactual_quote_path.clone(),
                },
            ));
        }
        self.append_records(records).await
    }

    async fn append_record(&self, record: StoredSignalRecord) -> Result<()> {
        self.append_records(vec![record]).await
    }

    pub async fn append_records(&self, records: Vec<StoredSignalRecord>) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        self.send_write(WriteOperation::Records(records)).await
    }

    async fn send_write(&self, operation: WriteOperation) -> Result<()> {
        let (ack, result) = oneshot::channel();
        self.writer
            .send(WriteRequest { operation, ack })
            .await
            .map_err(|_| anyhow::anyhow!("signal database writer stopped"))?;
        result
            .await
            .map_err(|_| anyhow::anyhow!("signal database writer dropped acknowledgement"))?
            .map_err(|error| anyhow::anyhow!(error))
    }
}
