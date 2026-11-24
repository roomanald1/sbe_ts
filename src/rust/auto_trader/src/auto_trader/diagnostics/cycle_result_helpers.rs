fn skipped_for_signal(signal: &Signal, reason: &str) -> CycleResult {
    let mut cycle = skipped(reason);
    cycle.symbol = Some(signal.symbol.clone());
    cycle.side = Some(signal.direction.clone());
    cycle
}
fn skipped(reason: &str) -> CycleResult {
    CycleResult {
        status: "skipped",
        reason: Some(reason.into()),
        symbol: None,
        side: None,
        quantity: None,
        order_type: None,
        order_response: None,
        filled_quantity: None,
        entry_diagnostic: None,
    }
}
#[cfg(test)]
mod cycle_result_helper_tests {
    use super::*;

    #[test]
    fn skipped_results_preserve_signal_identity_and_reason() {
        let signal = Signal {
            range_location: None,
            symbol: "ETH-USDT".into(), direction: "bearish".into(), strength: 0.8,
            timestamp: 1, stale: false, score: 0.8, ranging_probability: None,
            components: SignalComponents::default(), strategy_candidate: false,
            strategy: None, strategy_decision: None, market_regime: None,
            market_state: None,
            news_attribution: None, entry_gate: None,
        };
        let result = skipped_for_signal(&signal, "test reason");
        assert_eq!(result.status, "skipped");
        assert_eq!(result.reason.as_deref(), Some("test reason"));
        assert_eq!(result.symbol.as_deref(), Some("ETH-USDT"));
        assert_eq!(result.side.as_deref(), Some("bearish"));
        assert!(result.quantity.is_none());
        let generic = skipped("generic");
        assert_eq!(generic.reason.as_deref(), Some("generic"));
        assert!(generic.symbol.is_none());
    }

    #[test]
    fn entry_diagnostic_serializes_typed_outcome_and_blocker_details() {
        let result = with_entry_diagnostic(
            skipped("two candidates rejected"),
            LatestEntryDiagnostic {
                cycle_id: "cycle-1".into(),
                timestamp_ms: 123,
                worker_event_ids: vec!["worker-event-1".into()],
                candidate_decision_ids: vec!["decision-1".into()],
                gate_counts: vec![],
                candidate_gate_counts: vec![],
                candidate_details: vec![],
                outcome: EntryCycleOutcome::CandidatesRejected,
                raw_signal_count: 5,
                allowed_signal_count: 5,
                strategy_candidate_count: 2,
                fresh_directional_candidate_count: 3,
                stale_candidate_count: 1,
                non_directional_candidate_count: 1,
                position_blocked_candidate_count: 1,
                strategy_rejected_candidate_count: 1,
                evidence_present_candidate_count: 2,
                eligible_candidate_count: 2,
                rejected_candidate_count: 2,
                selected_candidate_count: 0,
                order_attempt_count: 0,
                submitted_order_count: 0,
                fill_confirmed: false,
                primary_gate_id: Some("candidate_selection_gates".into()),
                primary_blocker: Some("walk_forward_expectancy_not_proven".into()),
                detail: "lower 95% bound -0.120% requires > 0%".into(),
            },
        );
        let payload = serde_json::to_value(result).unwrap();
        let diagnostic = &payload["entry_diagnostic"];
        assert_eq!(diagnostic["outcome"], "candidates_rejected");
        assert_eq!(diagnostic["strategyCandidateCount"], 2);
        assert_eq!(diagnostic["freshDirectionalCandidateCount"], 3);
        assert_eq!(diagnostic["positionBlockedCandidateCount"], 1);
        assert_eq!(diagnostic["evidencePresentCandidateCount"], 2);
        assert_eq!(diagnostic["primaryGateId"], "candidate_selection_gates");
        assert_eq!(diagnostic["primaryBlocker"], "walk_forward_expectancy_not_proven");
        assert_eq!(diagnostic["detail"], "lower 95% bound -0.120% requires > 0%");
    }

    #[test]
    fn symbol_entry_result_serializes_backend_decision_and_identity() {
        let result = SymbolEntryResult {
            strategy: "live-momentum".into(),
            symbol: "ADA-USDT".into(),
            profile_id: Some("profile-1".into()),
            event_id: Some("event-1".into()),
            decision: Some(strategy::LiveStrategyDecision::Reject),
            reason: "Strength 0.111 < 0.300".into(),
            timestamp_ms: Some(123),
        };
        let payload = serde_json::to_value(result).unwrap();
        assert_eq!(payload["strategy"], "live-momentum");
        assert_eq!(payload["symbol"], "ADA-USDT");
        assert_eq!(payload["profile_id"], "profile-1");
        assert_eq!(payload["event_id"], "event-1");
        assert_eq!(payload["decision"], "reject");
        assert_eq!(payload["reason"], "Strength 0.111 < 0.300");
        assert_eq!(payload["timestamp_ms"], 123);
    }

    #[test]
    fn symbol_entry_result_can_explain_missing_worker_evaluation() {
        let result = SymbolEntryResult {
            strategy: "live-momentum".into(),
            symbol: "ADA-USDT".into(),
            profile_id: None,
            event_id: None,
            decision: None,
            reason: "No worker result recorded".into(),
            timestamp_ms: None,
        };
        let payload = serde_json::to_value(result).unwrap();
        assert_eq!(payload["decision"], serde_json::Value::Null);
        assert_eq!(payload["reason"], "No worker result recorded");
        assert_eq!(payload["timestamp_ms"], serde_json::Value::Null);
    }
}
