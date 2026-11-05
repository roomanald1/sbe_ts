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
    fn zero_return_carry_variant_finalizes_after_one_day_and_retry_is_idempotent() {
        let opened_at_ms = shadow_now_ms().saturating_sub(86_400_000);
        let probe = carry_probe(opened_at_ms, false);
        let mut state = PersistedState::default();
        state.shadow_pending_samples.insert(
            probe.sample_id.clone(),
            ShadowSamplePayload::DatedCarryProbe(probe),
        );
        state.shadow_sample_active_scenarios.insert(
            "carry-probe-1".into(),
            BTreeSet::from(["carry-probe-1|carry".into()]),
        );
        state
            .shadow_plan_profiles
            .insert("carry-probe-1".into(), "dated-carry|BTC-USDT".into());
        let request = ShadowLifecycleObserveRequest {
            request_id: "scan-1".into(),
            now_ms: shadow_now_ms(),
            candidates: vec![],
        };
        let response = apply_shadow_lifecycle_observation(&mut state, &request);
        assert_eq!(response.finalized_count, 1);
        let retry = apply_shadow_lifecycle_observation(&mut state, &request);
        assert_eq!(retry.finalized_count, 1);
        assert!(state.shadow_pending_samples.is_empty());
        assert_eq!(state.outbox.len(), 1);
        let ShadowOutboxItem::Learning(FeedbackEvent::ScoutOutcome {
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
        let opened_at_ms = shadow_now_ms().saturating_sub(3 * 86_400_000 - 5_000);
        let probe = carry_probe(opened_at_ms, true);
        let mut state = PersistedState::default();
        state.shadow_pending_samples.insert(
            probe.sample_id.clone(),
            ShadowSamplePayload::DatedCarryProbe(probe),
        );
        let request = ShadowLifecycleObserveRequest {
            request_id: "scan-missing".into(),
            now_ms: shadow_now_ms(),
            candidates: vec![],
        };
        assert_eq!(
            apply_shadow_lifecycle_observation(&mut state, &request).finalized_count,
            0
        );
        assert_eq!(state.shadow_pending_samples.len(), 1);
        let mut mark = carry_candidate(opened_at_ms);
        mark.spot_bid = 101.0;
        mark.future_ask = 100.2;
        let request = ShadowLifecycleObserveRequest {
            request_id: "scan-arrived".into(),
            now_ms: shadow_now_ms(),
            candidates: vec![mark],
        };
        assert_eq!(
            apply_shadow_lifecycle_observation(&mut state, &request).finalized_count,
            1
        );
        let ShadowOutboxItem::Learning(FeedbackEvent::ScoutOutcome {
            net_return_percent,
            event_id,
            ..
        }) = &state.outbox[0]
        else {
            panic!()
        };
        assert!(net_return_percent.is_finite());
        assert!(event_id.ends_with("carry_minimum_days_to_expiry"));
    }
}

