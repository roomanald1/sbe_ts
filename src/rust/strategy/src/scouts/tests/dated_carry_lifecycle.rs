#[cfg(test)]
mod dated_carry_lifecycle_tests {
    use super::*;

    fn carry_candidate(opened_at_ms: u64) -> strategies::DatedCarryEntryCandidate {
        let spot_ask = 100.0;
        let future_bid = 101.5;
        let basis_percent = (future_bid / spot_ask - 1.0) * 100.0;
        strategies::DatedCarryEntryCandidate {
            spot_symbol: "BTCUSDT".into(),
            future_symbol: "BTCUSDT_20261231".into(),
            expiry_ms: opened_at_ms + 10 * 86_400_000,
            days_to_expiry: 10.0,
            quantity: 1.0,
            spot_ask,
            future_bid,
            spot_bid: 99.9,
            future_ask: 101.6,
            basis_percent,
            net_after_budget_percent: basis_percent
                - strategies::DATED_CARRY_COST_BUDGET_PERCENT,
            eligible: true,
        }
    }

    fn carry_probe(opened_at_ms: u64, would_enter: bool) -> DatedCarryProbe {
        let tested = BTreeMap::from([("carry_minimum_days_to_expiry".into(), 6.0)]);
        let spec = ParameterSpec {
            key: "carry_minimum_days_to_expiry".into(),
            label: "Minimum days".into(),
            default: 7.0,
            min: 2.0,
            max: 30.0,
            step: 1.0,
            unit: "days".into(),
            learning_direction: -1.0,
        };
        DatedCarryProbe {
            sample_id: "carry-probe-1".into(),
            parameter_key: "carry_minimum_days_to_expiry".into(),
            parameter_value: 6.0,
            comparison_role: None,
            comparison_id: None,
            comparison_ids: Vec::new(),
            opened_at_ms,
            candidate: carry_candidate(opened_at_ms),
            strategy: "dated-carry".into(),
            profile_symbol: "BTC-USDT".into(),
            parameters: BTreeMap::from([
                ("carry_minimum_days_to_expiry".into(), 6.0),
                ("carry_cost_gate_percent".into(), 0.5),
            ]),
            tested_parameters: tested,
            parameter_specs: vec![spec],
            would_enter,
        }
    }

    #[test]
    fn lifecycle_persistence_reports_only_changed_record_keys() {
        let opened_at_ms = scout_now_ms().saturating_sub(4 * 86_400_000);
        let probe = carry_probe(opened_at_ms, false);
        let mut state = PersistedState::default();
        state.scout_pending_samples.insert(
            probe.sample_id.clone(),
            ScoutSamplePayload::DatedCarryProbe(probe),
        );
        state.scout_sample_active_scenarios.insert(
            "carry-probe-1".into(),
            BTreeSet::from(["carry-probe-1|carry".into()]),
        );
        state
            .scout_plan_profiles
            .insert("carry-probe-1".into(), "dated-carry|BTC-USDT".into());
        let request = ScoutLifecycleObserveRequest {
            request_id: "expired-carry-probe".into(),
            now_ms: scout_now_ms(),
            candidates: Vec::new(),
        };

        let (_, changed_records) =
            apply_scout_lifecycle_observation_with_changes(&mut state, &request);

        assert!(changed_records.contains("scout_pending_samples@carry-probe-1"));
        assert!(changed_records.contains("scout_sample_active_scenarios@carry-probe-1"));
        assert!(changed_records.contains("scout_plan_profiles@carry-probe-1"));
        assert!(changed_records.contains("scout_observation_results@expired-carry-probe"));
        assert!(!changed_records.contains("outcomes"));
        assert!(!changed_records.contains("scout_instances"));
    }

    #[test]
    fn zero_return_carry_variant_finalizes_after_one_day_and_retry_is_idempotent() {
        let opened_at_ms = scout_now_ms().saturating_sub(86_400_000);
        let probe = carry_probe(opened_at_ms, false);
        let mut state = PersistedState::default();
        state.scout_pending_samples.insert(
            probe.sample_id.clone(),
            ScoutSamplePayload::DatedCarryProbe(probe),
        );
        state.scout_sample_active_scenarios.insert(
            "carry-probe-1".into(),
            BTreeSet::from(["carry-probe-1|carry".into()]),
        );
        state
            .scout_plan_profiles
            .insert("carry-probe-1".into(), "dated-carry|BTC-USDT".into());
        let request = ScoutLifecycleObserveRequest {
            request_id: "scan-1".into(),
            now_ms: scout_now_ms(),
            candidates: vec![],
        };
        let response = apply_scout_lifecycle_observation(&mut state, &request);
        assert_eq!(response.finalized_count, 1);
        let retry = apply_scout_lifecycle_observation(&mut state, &request);
        assert_eq!(retry.finalized_count, 1);
        assert!(state.scout_pending_samples.is_empty());
        assert_eq!(state.outbox.len(), 1);
        assert!(state.outcomes.is_empty(), "rejected carry candidates are not performance samples");
        let ScoutOutboxItem::Learning(FeedbackEvent::ScoutOutcome {
            net_return_percent,
            symbol,
            profile_id,
            specs,
            ..
        }) = &state.outbox[0]
        else {
            panic!()
        };
        assert_eq!(*net_return_percent, 0.0);
        assert_eq!(symbol, "BTC-USDT");
        assert_eq!(
            profile_id,
            &strategies::profile_id(
                "BTC-USDT",
                "dated-carry",
                &BTreeMap::from([
                    ("carry_minimum_days_to_expiry".into(), 6.0),
                    ("carry_cost_gate_percent".into(), 0.5),
                ])
            )
        );
        assert_eq!(specs.len(), 1);
    }

    #[test]
    fn profitable_carry_probe_waits_for_matching_mark_through_three_days_then_finalizes() {
        let opened_at_ms = scout_now_ms().saturating_sub(3 * 86_400_000 - 5_000);
        let probe = carry_probe(opened_at_ms, true);
        let mut state = PersistedState::default();
        state.scout_pending_samples.insert(
            probe.sample_id.clone(),
            ScoutSamplePayload::DatedCarryProbe(probe),
        );
        let request = ScoutLifecycleObserveRequest {
            request_id: "scan-missing".into(),
            now_ms: scout_now_ms(),
            candidates: vec![],
        };
        assert_eq!(
            apply_scout_lifecycle_observation(&mut state, &request).finalized_count,
            0
        );
        assert_eq!(state.scout_pending_samples.len(), 1);
        let mut mark = carry_candidate(opened_at_ms);
        mark.spot_bid = 101.0;
        mark.future_ask = 100.2;
        let request = ScoutLifecycleObserveRequest {
            request_id: "scan-arrived".into(),
            now_ms: scout_now_ms(),
            candidates: vec![mark],
        };
        assert_eq!(
            apply_scout_lifecycle_observation(&mut state, &request).finalized_count,
            1
        );
        let ScoutOutboxItem::Learning(FeedbackEvent::ScoutOutcome {
            net_return_percent,
            event_id,
            ..
        }) = &state.outbox[0]
        else {
            panic!()
        };
        assert!(net_return_percent.is_finite());
        assert!(event_id.ends_with("carry_minimum_days_to_expiry"));
        assert_eq!(state.outcomes.len(), 1);
        let completed = state.outcomes.values().next().unwrap();
        assert_eq!(completed.strategy, "dated-futures-carry");
        assert_eq!(completed.symbol, "BTC-USDT");
        assert_eq!(completed.exit_scenario, "strategy");
        assert_eq!(completed.strategy_parameters["carry_minimum_days_to_expiry"], 6.0);
    }

    #[test]
    fn one_carry_baseline_pairs_with_multiple_variants_and_zero_trade_sides() {
        let opened_at_ms = scout_now_ms().saturating_sub(86_400_000);
        let mut baseline = carry_probe(opened_at_ms, true);
        baseline.sample_id = "carry-batch|baseline".into();
        baseline.parameter_key.clear();
        baseline.parameter_value = 0.0;
        baseline.comparison_role = Some("baseline".into());
        baseline.comparison_ids = vec!["carry-pair-a".into(), "carry-pair-b".into()];
        baseline.parameters.insert("carry_minimum_days_to_expiry".into(), 7.0);
        baseline.tested_parameters.clear();

        let mut variant_a = carry_probe(opened_at_ms, true);
        variant_a.sample_id = "carry-batch|variant-a".into();
        variant_a.comparison_role = Some("variant".into());
        variant_a.comparison_id = Some("carry-pair-a".into());
        variant_a.comparison_ids.clear();

        let mut variant_b = carry_probe(opened_at_ms, false);
        variant_b.sample_id = "carry-batch|variant-b".into();
        variant_b.comparison_role = Some("variant".into());
        variant_b.comparison_id = Some("carry-pair-b".into());
        variant_b.comparison_ids.clear();

        let mut state = PersistedState::default();
        for probe in [baseline, variant_a, variant_b] {
            state.scout_pending_samples.insert(
                probe.sample_id.clone(),
                ScoutSamplePayload::DatedCarryProbe(probe),
            );
        }
        let mut exit = carry_candidate(opened_at_ms);
        exit.spot_bid = 101.0;
        exit.future_ask = 100.2;
        let response = apply_scout_lifecycle_observation(
            &mut state,
            &ScoutLifecycleObserveRequest {
                request_id: "carry-pair-observe".into(),
                now_ms: scout_now_ms(),
                candidates: vec![exit],
            },
        );
        assert_eq!(response.finalized_count, 3);
        let paired = state.outbox.iter().filter_map(|item| match item {
            ScoutOutboxItem::Learning(FeedbackEvent::ScoutOutcome {
                comparison: strategies::ScoutComparisonMetadata::Paired {
                    group_id,
                    ..
                },
                net_return_percent,
                ..
            }) => Some((group_id.clone(), *net_return_percent)),
            _ => None,
        }).collect::<BTreeMap<_, _>>();
        assert_eq!(paired.len(), 2);
        assert!(paired["carry-pair-a"].abs() < 1e-8);
        assert!(paired["carry-pair-b"] < 0.0);
    }
}

