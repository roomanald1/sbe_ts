//! Scout planning, scout lifecycle, and service implementation.
//!
//! The files are grouped by responsibility. They are included in this shared
//! implementation module so the existing tightly-coupled private lifecycle
//! helpers remain private and the crate API stays unchanged.

include!("contracts.rs");
include!("candidate_planning.rs");
include!("service.rs");
include!("scout_handlers.rs");
include!("scout_report.rs");

#[cfg(test)]
mod tests {
    use super::*;

    include!("tests/scout_outcome.rs");
    include!("tests/scout_audit_plan.rs");
    include!("tests/scout_plan.rs");
    include!("tests/dated_carry_lifecycle.rs");
    include!("tests/live_candidates.rs");

    #[test]
    fn probe_ids_and_context_hashes_are_stable_and_exclusions_remove_context() {
        let first = probe_id("BTC-USDT|carry");
        assert_eq!(first, probe_id("BTC-USDT|carry"));
        assert_eq!(first.len(), 16);
        assert_ne!(first, probe_id("ETH-USDT|carry"));

        let profile = BTreeMap::from([
            ("entry_threshold".to_owned(), 1.25),
            ("exit_threshold".to_owned(), 2.5),
        ]);
        let excluded = BTreeMap::from([("entry_threshold".to_owned(), 9.0)]);
        assert_eq!(
            context_hash(&profile, &excluded),
            context_hash(
                &BTreeMap::from([("exit_threshold".to_owned(), 2.5)]),
                &BTreeMap::new()
            )
        );
    }

    #[test]
    fn combination_keys_round_trip_and_reject_malformed_or_out_of_bounds_values() {
        let specs = vec![
            ParameterSpec {
                key: "alpha".into(),
                label: "Alpha".into(),
                default: 1.0,
                min: 0.0,
                max: 4.0,
                step: 0.5,
                unit: "".into(),
                learning_direction: 1.0,
            },
            ParameterSpec {
                key: "beta".into(),
                label: "Beta".into(),
                default: 2.0,
                min: 0.0,
                max: 4.0,
                step: 0.5,
                unit: "".into(),
                learning_direction: 1.0,
            },
        ];
        let values = BTreeMap::from([("alpha".into(), 1.5), ("beta".into(), 2.5)]);
        let full = BTreeMap::from([
            ("alpha".into(), 1.0),
            ("beta".into(), 2.0),
            ("gamma".into(), 3.0),
        ]);
        let key = combination_key(&values, &full).unwrap();
        assert_eq!(combination_context(&key).unwrap().len(), 16);
        assert_eq!(parse_combination(&key, &specs), Some(values));
        assert!(
            parse_combination("combo:alpha=99,beta=2.5|ctx:0000000000000000", &specs).is_none()
        );
        assert!(combination_context("combo:alpha=1,beta=2|ctx:not-a-hash").is_none());
    }

    #[test]
    fn bounded_parameter_sweeps_cover_single_and_pair_candidates() {
        let specs = vec![
            ParameterSpec {
                key: "alpha".into(),
                label: "Alpha".into(),
                default: 1.0,
                min: 0.0,
                max: 4.0,
                step: 0.5,
                unit: "".into(),
                learning_direction: 1.0,
            },
            ParameterSpec {
                key: "beta".into(),
                label: "Beta".into(),
                default: 2.0,
                min: 0.0,
                max: 4.0,
                step: 0.5,
                unit: "".into(),
                learning_direction: 1.0,
            },
        ];
        let profile = BTreeMap::new();
        assert!(next_parameter_probe("BTC", "s", &profile, &specs, 0).is_some());
        assert_eq!(
            next_parameter_combination("BTC", "s", &profile, &specs, 0)
                .unwrap()
                .len(),
            2
        );
        let variants = all_parameter_variants(&profile, &specs, 0).unwrap();
        assert_eq!(variants.len(), 3);
        assert!(variants.iter().all(|variant| !variant.is_empty()));
        let too_many = (0..17)
            .map(|index| ParameterSpec {
                key: format!("p{index}"),
                label: format!("P{index}"),
                default: 1.0,
                min: 0.0,
                max: 2.0,
                step: 0.1,
                unit: "".into(),
                learning_direction: 1.0,
            })
            .collect::<Vec<_>>();
        assert!(all_parameter_variants(&profile, &too_many, 0).is_none());
    }

    #[test]
    fn scout_persistence_round_trips_state_and_outbox_dedupes_learning_events() {
        let path = std::env::temp_dir().join(format!(
            "scouts-persistence-test-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let event = FeedbackEvent::ScoutOutcome {
            event_id: "event-1".into(),
            observed_at_ms: 10,
            learning_context: Some("context".into()),
            symbol: "BTC-USDT".into(),
            strategy: "momentum".into(),
            profile_id: "profile".into(),
            parameters: BTreeMap::new(),
            specs: Vec::new(),
            net_return_percent: 0.5,
        };
        let mut second_event = event.clone();
        if let FeedbackEvent::ScoutOutcome { event_id, .. } = &mut second_event {
            *event_id = "event-2".into();
        }
        let mut state = PersistedState {
            seen: BTreeSet::from(["event-1".into()]),
            outbox: vec![
                ScoutOutboxItem::Learning(event.clone()),
                ScoutOutboxItem::Learning(second_event.clone()),
            ],
            ..PersistedState::default()
        };
        assert!(persist_scout_state(&path, &state, &["seen", "outbox"]).is_ok());
        let loaded = load_scout_state(&path).unwrap();
        assert!(loaded.seen.contains("event-1"));
        assert_eq!(loaded.outbox.len(), 2);
        assert_eq!(feedback_event_id(&event), "event-1");
        assert!(outbox_has_learning(&loaded, &event));

        let mut other = event.clone();
        if let FeedbackEvent::ScoutOutcome { event_id, .. } = &mut other {
            *event_id = "event-3".into();
        }
        assert!(!outbox_has_learning(&loaded, &other));
        state.outbox.remove(0);
        assert!(persist_scout_state(&path, &state, &["outbox"]).is_ok());
        let loaded = load_scout_state(&path).unwrap();
        assert_eq!(loaded.outbox.len(), 1);
        assert!(outbox_has_learning(&loaded, &second_event));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn scout_persistence_updates_one_plan_result_without_dropping_siblings() {
        let path = std::env::temp_dir().join(format!(
            "scouts-incremental-persistence-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let mut state = PersistedState::default();
        state.scout_plan_results.insert(
            "request-a".into(),
            ScoutPlanResponse {
                audits: Vec::new(),
                selected_groups: vec!["group-a".into()],
            },
        );
        assert!(persist_scout_state(&path, &state, &["scout_plan_results@request-a"]).is_ok());
        state.scout_plan_results.insert(
            "request-b".into(),
            ScoutPlanResponse {
                audits: Vec::new(),
                selected_groups: vec!["group-b".into()],
            },
        );
        assert!(persist_scout_state(&path, &state, &["scout_plan_results@request-b"]).is_ok());
        let loaded = load_scout_state(&path).unwrap();
        assert_eq!(
            loaded.scout_plan_results["request-a"].selected_groups,
            ["group-a"]
        );
        assert_eq!(
            loaded.scout_plan_results["request-b"].selected_groups,
            ["group-b"]
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn active_audit_replay_builds_registrations_from_loaded_state() {
        let path = std::env::temp_dir().join(format!(
            "scouts-audit-replay-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let audit = serde_json::json!({
            "decision_id": "durable-decision",
            "strategy": "live-momentum",
            "symbol": "BTC-USDT",
            "direction": "bullish",
            "timestamp": scout_now_ms(),
            "scout_entry_price": 100.0,
            "strategy_decision": {"strategy_parameters": {}}
        });
        let mut state = PersistedState::default();
        state
            .scout_active_audits
            .insert("durable-group".into(), vec![audit]);
        assert!(persist_scout_state(&path, &state, &["scout_active_audits@durable-group"]).is_ok());

        let loaded = load_scout_state(&path).unwrap();
        let registrations = active_scout_registrations(&loaded);
        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0]["decision_id"], "durable-decision");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}
