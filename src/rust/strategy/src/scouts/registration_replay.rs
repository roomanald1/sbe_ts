use serde_json::Value;

/// Holds only registrations that have not yet been acknowledged by Insights.
pub(crate) struct RegistrationReplayQueue {
    pending: Vec<Value>,
}

#[derive(Default)]
pub(crate) struct ProcessEpochTracker {
    observed: Option<String>,
}

impl ProcessEpochTracker {
    pub(crate) fn is_new_epoch(&self, epoch: &str) -> bool {
        !epoch.is_empty() && self.observed.as_deref() != Some(epoch)
    }

    pub(crate) fn commit(&mut self, epoch: String) {
        self.observed = Some(epoch);
    }
}

impl RegistrationReplayQueue {
    pub(crate) fn new(pending: Vec<Value>) -> Self {
        Self {
            pending: deduplicate_registrations(pending),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn take_batch(&mut self, limit: usize) -> Vec<Value> {
        let count = self.pending.len().min(limit);
        self.pending.drain(..count).collect()
    }

    pub(crate) fn replace(&mut self, registrations: Vec<Value>) {
        self.pending = deduplicate_registrations(registrations);
    }

    pub(crate) fn retry_failed(&mut self, failed: Vec<Value>) {
        self.pending.extend(failed);
        self.pending = deduplicate_registrations(std::mem::take(&mut self.pending));
    }
}

pub(crate) fn retryable_http_status(status: u16) -> bool {
    status == 408 || status == 429 || status >= 500
}

fn deduplicate_registrations(registrations: Vec<Value>) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    registrations
        .into_iter()
        .filter(|registration| {
            let key = registration
                .get("decision_id")
                .or_else(|| registration.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| registration.to_string());
            seen.insert(key)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ProcessEpochTracker, RegistrationReplayQueue};

    #[test]
    fn unchanged_process_epoch_is_a_noop_and_changed_epoch_replays_once() {
        let mut tracker = ProcessEpochTracker::default();
        assert!(tracker.is_new_epoch("epoch-1"));
        tracker.commit("epoch-1".into());
        assert!(!tracker.is_new_epoch("epoch-1"));
        assert!(tracker.is_new_epoch("epoch-2"));
        tracker.commit("epoch-2".into());
        assert!(!tracker.is_new_epoch("epoch-2"));
    }

    #[test]
    fn empty_process_epoch_is_ignored() {
        assert!(!ProcessEpochTracker::default().is_new_epoch(""));
    }

    #[test]
    fn successful_registrations_are_not_replayed() {
        let registrations = vec![serde_json::json!({"id":"one"})];
        let mut queue = RegistrationReplayQueue::new(registrations);

        let first_attempt = queue.take_batch(1);
        assert_eq!(first_attempt.len(), 1);
        queue.retry_failed(Vec::new());

        assert!(queue.is_empty());
        assert!(queue.take_batch(1).is_empty());
    }

    #[test]
    fn only_failed_registrations_are_retried() {
        let registrations = vec![
            serde_json::json!({"id":"success"}),
            serde_json::json!({"id":"failed"}),
            serde_json::json!({"id":"later"}),
        ];
        let mut queue = RegistrationReplayQueue::new(registrations);
        let first_attempt = queue.take_batch(2);
        assert_eq!(first_attempt[0], serde_json::json!({"id":"success"}));
        let failed_attempt = vec![first_attempt[1].clone()];
        assert_eq!(failed_attempt, vec![serde_json::json!({"id":"failed"})]);
        queue.retry_failed(failed_attempt);

        assert_eq!(queue.take_batch(1), vec![serde_json::json!({"id":"later"})]);
        let retry = queue.take_batch(1);
        assert_eq!(retry, vec![serde_json::json!({"id":"failed"})]);
        assert!(queue.is_empty());
    }

    #[test]
    fn duplicate_decision_ids_are_registered_once() {
        let registration = serde_json::json!({"decision_id":"same"});
        let mut queue =
            RegistrationReplayQueue::new(vec![registration.clone(), registration.clone()]);
        assert_eq!(queue.pending_len(), 1);
        queue.replace(vec![registration.clone(), registration]);
        assert_eq!(queue.take_batch(8).len(), 1);
    }

    #[test]
    fn transient_http_responses_are_retried_but_permanent_client_errors_are_not() {
        assert!(retryable_http_status(408));
        assert!(retryable_http_status(429));
        assert!(retryable_http_status(503));
        assert!(!retryable_http_status(400));
        assert!(!retryable_http_status(422));
    }
}
