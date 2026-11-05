//! Scout planning, shadow lifecycle, and service implementation.
//!
//! The files are grouped by responsibility. They are included in this shared
//! implementation module so the existing tightly-coupled private lifecycle
//! helpers remain private and the crate API stays unchanged.

include!("contracts.rs");
include!("candidate_planning.rs");
include!("service.rs");
include!("shadow_handlers.rs");

#[cfg(test)]
mod tests {
    use super::*;

    include!("tests/shadow_outcome.rs");
    include!("tests/shadow_audit_plan.rs");
    include!("tests/shadow_plan.rs");
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
    fn shadow_persistence_round_trips_state_and_outbox_dedupes_learning_events() {
        let path = std::env::temp_dir().join(format!(
            "scouts-persistence-test-{}-{}.json",
            std::process::id(),
            shadow_now_ms()
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
        let state = PersistedState {
            seen: BTreeSet::from(["event-1".into()]),
            outbox: vec![ShadowOutboxItem::Learning(event.clone())],
            ..PersistedState::default()
        };
        assert!(persist_shadow_state(&path, &state).is_ok());
        let loaded: PersistedState =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(loaded.seen.contains("event-1"));
        assert_eq!(loaded.outbox.len(), 1);
        assert_eq!(feedback_event_id(&event), "event-1");
        assert!(outbox_has_learning(&loaded, &event));

        let mut other = event.clone();
        if let FeedbackEvent::ScoutOutcome { event_id, .. } = &mut other {
            *event_id = "event-2".into();
        }
        assert!(!outbox_has_learning(&loaded, &other));
        let _ = std::fs::remove_file(path);
    }
}
