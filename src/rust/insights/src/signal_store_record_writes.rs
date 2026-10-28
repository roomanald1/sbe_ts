impl SignalStore {
    async fn read_outcomes(&self) -> Result<Vec<SignalOutcome>> {
        Ok(self
            .read_recent_records(1, 100_000)
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
            .read_recent_records(0, 100_000)
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
        self.append_record(StoredSignalRecord::Evaluation(evaluation.clone()))
            .await
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
