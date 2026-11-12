impl SignalStore {
    pub async fn append(&self, snapshot: &SignalSnapshot) -> Result<()> {
        self.append_record(StoredSignalRecord::Snapshot(snapshot.clone()))
            .await
    }

    pub async fn append_outcome(&self, outcome: &SignalOutcome) -> Result<()> {
        self.append_record(StoredSignalRecord::Outcome(outcome.clone()))
            .await
    }

    pub async fn append_evaluation(&self, evaluation: &SignalEvaluation) -> Result<()> {
        self.append_records(evaluation_records(evaluation)).await
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

fn evaluation_records(evaluation: &SignalEvaluation) -> Vec<StoredSignalRecord> {
    vec![StoredSignalRecord::Evaluation(evaluation.clone())]
}

#[cfg(test)]
mod evaluation_write_tests {
    use super::*;

    #[test]
    fn appending_evaluation_does_not_generate_a_quote_path_record() {
        let evaluation = SignalEvaluation {
            symbol: "BTC-USDT".into(),
            signal_timestamp: 10_000,
            evaluated_timestamp: 11_000,
            horizon_seconds: 1,
            signal_direction: "bullish".into(),
            signal_score: 0.5,
            ranging_probability: None,
            reference_entry_price: 100.0,
            reference_exit_price: 101.0,
            entry_bid: Some(99.9),
            entry_ask: Some(100.1),
            entry_quote_timestamp: Some(10_000),
            exit_bid: Some(100.9),
            exit_ask: Some(101.1),
            exit_quote_timestamp: Some(11_000),
            taker_fee_rate: 0.001,
            long_gross_percent: 1.0,
            short_gross_percent: -1.0,
            long_net_percent: Some(0.8),
            short_net_percent: Some(-1.2),
            path_high: 101.0,
            path_high_timestamp: 11_000,
            path_low: 100.0,
            path_low_timestamp: 10_000,
            long_time_to_positive_net_seconds: None,
            short_time_to_positive_net_seconds: None,
        };

        let records = evaluation_records(&evaluation);
        assert_eq!(records.len(), 1);
        assert!(matches!(records.as_slice(), [StoredSignalRecord::Evaluation(_)]));
    }
}
