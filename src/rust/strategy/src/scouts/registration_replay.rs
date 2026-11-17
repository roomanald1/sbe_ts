use serde_json::Value;

/// Holds only registrations that have not yet been acknowledged by Insights.
pub(crate) struct RegistrationReplayQueue {
    pending: Vec<Value>,
}

impl RegistrationReplayQueue {
    pub(crate) fn new(pending: Vec<Value>) -> Self {
        Self { pending }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn take_pending(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.pending)
    }

    pub(crate) fn retry_failed(&mut self, failed: Vec<Value>) {
        self.pending = failed;
    }
}

#[cfg(test)]
mod tests {
    use super::RegistrationReplayQueue;

    #[test]
    fn successful_registrations_are_not_replayed() {
        let registrations = vec![serde_json::json!({"id":"one"})];
        let mut queue = RegistrationReplayQueue::new(registrations);

        let first_attempt = queue.take_pending();
        assert_eq!(first_attempt.len(), 1);
        queue.retry_failed(Vec::new());

        assert!(queue.is_empty());
        assert!(queue.take_pending().is_empty());
    }

    #[test]
    fn only_failed_registrations_are_retried() {
        let registrations = vec![
            serde_json::json!({"id":"success"}),
            serde_json::json!({"id":"failed"}),
        ];
        let mut queue = RegistrationReplayQueue::new(registrations);
        let first_attempt = queue.take_pending();
        queue.retry_failed(vec![first_attempt[1].clone()]);

        let retry = queue.take_pending();
        assert_eq!(retry, vec![serde_json::json!({"id":"failed"})]);
        queue.retry_failed(Vec::new());
        assert!(queue.is_empty());
    }
}
