#[cfg(test)]
mod shadow_outcome_tests {
    use super::*;

    fn spec() -> ParameterSpec {
        ParameterSpec {
            key: "minimum_momentum".into(),
            label: "Minimum momentum".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.1,
            unit: "rate".into(),
            learning_direction: 1.0,
        }
    }

    fn outcome(id: &str, net: f64) -> ShadowOutcomeRequest {
        ShadowOutcomeRequest {
            decision_id: id.into(),
            learning_context: "scout:BTC:trade:scenario:strategy".into(),
            simulated_net_pnl_usd: net,
            net_return_percent: net,
            signal_timestamp_ms: shadow_now_ms().saturating_sub(100),
            evaluated_timestamp_ms: shadow_now_ms(),
            taker_fee_rate: 0.001,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            strategy_parameters: BTreeMap::from([("minimum_momentum".into(), 0.5)]),
            exit_scenario: "strategy".into(),
            sample_group_id: "batch".into(),
            scout_parameter_key: None,
            scout_parameter_value: None,
            scout_parameter_profile_symbol: None,
            scout_comparison_id: None,
            scout_comparison_role: None,
            scout_comparison_zero_role: None,
            scout_comparison_parameter_key: None,
            scout_comparison_parameter_value: None,
            scout_comparison_parameter_profile_symbol: None,
            strategy_parameter_specs: vec![spec()],
        }
    }

    #[test]
    fn validates_probe_bounds_and_builds_profile_attribution() {
        let mut sample = outcome("probe", 1.25);
        sample.scout_parameter_key = Some("minimum_momentum".into());
        sample.scout_parameter_value = Some(0.7);
        let tested = validate_shadow_outcome(&sample).unwrap();
        let FeedbackEvent::ScoutOutcome {
            parameters,
            specs,
            learning_context,
            net_return_percent,
            ..
        } = outcome_event(&sample, &tested)
        else {
            panic!()
        };
        assert_eq!(parameters["minimum_momentum"], 0.7);
        assert_eq!(specs.len(), 1);
        assert_eq!(learning_context.as_deref(), Some("scout:BTC:trade"));
        assert_eq!(net_return_percent, 1.25);
        sample.scout_parameter_value = Some(0.75);
        assert!(validate_shadow_outcome(&sample).is_err());
    }

    #[test]
    fn paired_feedback_is_variant_minus_baseline_and_keeps_variant_profile() {
        let mut baseline = outcome("baseline", 0.4);
        baseline.scout_comparison_id = Some("cmp".into());
        baseline.scout_comparison_role = Some("baseline".into());
        baseline.strategy_parameter_specs.clear();
        let mut variant = outcome("variant", 1.1);
        variant.scout_comparison_id = Some("cmp".into());
        variant.scout_comparison_role = Some("variant".into());
        variant.scout_parameter_key = Some("minimum_momentum".into());
        variant.scout_parameter_value = Some(0.7);
        let event = paired_event("cmp", &baseline, &variant).unwrap();
        let FeedbackEvent::ScoutOutcome {
            event_id,
            parameters,
            specs,
            learning_context,
            net_return_percent,
            ..
        } = event
        else {
            panic!()
        };
        assert_eq!(event_id, "paired-scout:cmp");
        assert_eq!(parameters["minimum_momentum"], 0.7);
        assert_eq!(specs.len(), 1);
        assert!(learning_context.is_none());
        assert!((net_return_percent - 0.7).abs() < 1e-9);
    }

    #[test]
    fn legacy_pair_completes_with_new_variant_using_legacy_baseline_return() {
        let mut pair = ShadowPair {
            legacy_baseline_return_percent: Some(0.4),
            legacy_learning_key: "paired_probe:BTC-USDT:live-momentum:minimum_momentum:0.70000000"
                .into(),
            ..ShadowPair::default()
        };
        let mut variant = outcome("new-variant", 1.1);
        variant.scout_parameter_key = Some("minimum_momentum".into());
        variant.scout_parameter_value = Some(0.7);
        variant.scout_parameter_profile_symbol = Some("BTC-USDT".into());
        variant.scout_comparison_id = Some("legacy-cmp".into());
        variant.scout_comparison_role = Some("variant".into());
        variant.scout_comparison_parameter_key = Some("minimum_momentum".into());
        variant.scout_comparison_parameter_value = Some(0.7);
        pair.variant = Some(variant.clone());
        let event = paired_event_with_legacy_side("legacy-cmp", &pair, &variant).unwrap();
        let FeedbackEvent::ScoutOutcome {
            parameters,
            net_return_percent,
            specs,
            ..
        } = event
        else {
            panic!()
        };
        assert_eq!(parameters["minimum_momentum"], 0.7);
        assert_eq!(specs.len(), 1);
        assert!((net_return_percent - 0.7).abs() < 1e-9);
    }

    #[test]
    fn ineligible_variant_can_pair_as_zero_with_variant_metadata_from_baseline() {
        let mut baseline = outcome("baseline", 0.8);
        baseline.scout_comparison_id = Some("cmp-zero".into());
        baseline.scout_comparison_role = Some("baseline".into());
        baseline.scout_comparison_zero_role = Some("variant".into());
        baseline.scout_comparison_parameter_key = Some("minimum_momentum".into());
        baseline.scout_comparison_parameter_value = Some(0.7);
        baseline.scout_comparison_parameter_profile_symbol = Some("BTC-USDT".into());
        let mut variant_zero = baseline.clone();
        variant_zero.decision_id = "synthetic-zero".into();
        variant_zero.scout_comparison_role = Some("variant".into());
        variant_zero.scout_comparison_zero_role = None;
        variant_zero.scout_parameter_key = variant_zero.scout_comparison_parameter_key.clone();
        variant_zero.scout_parameter_value = variant_zero.scout_comparison_parameter_value;
        variant_zero.scout_parameter_profile_symbol = variant_zero
            .scout_comparison_parameter_profile_symbol
            .clone();
        variant_zero.net_return_percent = 0.0;
        let event = paired_event("cmp-zero", &baseline, &variant_zero).unwrap();
        let FeedbackEvent::ScoutOutcome {
            parameters,
            net_return_percent,
            ..
        } = event
        else {
            panic!()
        };
        assert_eq!(parameters["minimum_momentum"], 0.7);
        assert!((net_return_percent + 0.8).abs() < 1e-9);
    }

    #[test]
    fn outcome_dedupe_keeps_each_exit_scenario_and_suppresses_per_scenario_retries() {
        let mut seen = BTreeSet::new();
        for scenario in ["strategy", "tight", "wide"] {
            let mut sample = outcome("shared-decision", 0.1);
            sample.exit_scenario = scenario.into();
            let key = shadow_outcome_idempotency_key(&sample);
            assert!(
                seen.insert(key.clone()),
                "first delivery for {scenario} must be accepted"
            );
            assert!(
                !seen.insert(key),
                "retry for {scenario} must be deduplicated"
            );
        }
        assert_eq!(seen.len(), 3);
    }
}

