#[cfg(test)]
mod scout_audit_plan_tests {
    use super::*;

    fn template(strategy_only: bool) -> ScoutAuditTemplate {
        ScoutAuditTemplate {
            base_audit: serde_json::json!({
                "decision_id": "decision-base|exit:strategy",
                "stage": "decision", "status": "scouted", "timestamp": 1234,
                "symbol": "BTC-USDT", "direction": "bullish", "strategy": "trend-50-200",
                "strategy_decision": {"observed_close": 100.0, "strategy_parameters": {"minimum_momentum": 0.5}}
            }),
            notional_usd: 5.0,
            learning_context: "scout:BTC:level:0.50:scenario".into(),
            stop_loss_percent: 0.5,
            take_profit_percent: 0.8,
            level_percent: 0.5,
            strategy_only,
            parameter_entry_eligible: None,
            parameter_specs: vec![ParameterSpec {
                key: "minimum_momentum".into(),
                label: "Minimum momentum".into(),
                default: 0.5,
                min: 0.0,
                max: 1.0,
                step: 0.1,
                unit: "rate".into(),
                learning_direction: 1.0,
            }],
        }
    }

    #[test]
    fn scouts_builds_scenarios_group_ids_and_preserves_profile_inputs() {
        let audits = build_scout_audits(&template(false), "daily-group");
        assert_eq!(audits.len(), 3);
        assert_eq!(audits[0]["scout_sample_group_id"], "daily-group");
        assert_eq!(audits[0]["scout_exit_scenario"], "strategy");
        assert_eq!(audits[1]["scout_exit_scenario"], "tight");
        assert_eq!(audits[2]["scout_exit_scenario"], "wide");
        assert_eq!(audits[1]["stop_loss_percent"], 0.25);
        assert_ne!(audits[0]["decision_id"], audits[1]["decision_id"]);
        assert_eq!(
            audits[0]["strategy_parameter_specs"][0]["key"],
            "minimum_momentum"
        );
        assert_eq!(audits[0]["scout_entry_price"], 100.0);
    }

    #[test]
    fn comparison_template_keeps_one_scenario_and_zero_side_metadata() {
        let mut template = template(true);
        template.base_audit["scout_comparison_role"] = serde_json::json!("baseline");
        template.base_audit["scout_comparison_zero_role"] = serde_json::json!("variant");
        template.base_audit["scout_comparison_parameter_key"] =
            serde_json::json!("minimum_momentum");
        template.base_audit["scout_comparison_parameter_value"] = serde_json::json!(0.7);
        let audits = build_scout_audits(&template, "pair-group");
        assert_eq!(audits.len(), 1);
        assert_eq!(
            audits[0]["scout_comparison_id"],
            format!("scout-pair:{}", probe_id("pair-group"))
        );
        assert_eq!(audits[0]["scout_comparison_role"], "baseline");
        assert_eq!(audits[0]["scout_comparison_zero_role"], "variant");
        assert_eq!(audits[0]["scout_comparison_parameter_value"], 0.7);
    }

    #[test]
    fn scout_registration_skips_historical_samples_outside_insights_retention() {
        let now = scout_now_ms();
        let fresh = serde_json::json!({
            "decision_id": "decision-fresh",
            "symbol": "BTC-USDT",
            "timestamp": now,
            "direction": "bullish",
            "strategy": "trend-50-200",
            "scout_entry_price": 100.0,
            "strategy_decision": {"strategy_parameters": {}},
            "stop_loss_percent": 0.5,
            "take_profit_percent": 0.8,
            "scout_exit_scenario": "strategy",
            "scout_comparison_role": "baseline",
            "scout_comparison_ids": ["pair-a", "pair-b"]
        });
        let registration = scout_registration(&fresh).unwrap();
        assert_eq!(registration["scout_comparison_role"], "baseline");
        assert_eq!(registration["scout_comparison_ids"], serde_json::json!(["pair-a", "pair-b"]));

        let mut stale = fresh;
        stale["timestamp"] = serde_json::json!(now.saturating_sub(7 * 86_400_000 + 1));
        assert!(scout_registration(&stale).is_none());
    }

    #[test]
    fn stored_scout_instances_drop_large_signal_payloads_but_keep_markout_fields() {
        let full = serde_json::json!({
            "decision_id": "decision-1",
            "symbol": "BTC-USDT",
            "timestamp": 1_000,
            "signal_timestamp": 1_000,
            "direction": "bullish",
            "strategy": "live-momentum",
            "scout_entry_price": 100.0,
            "scout_notional_usd": 5.0,
            "scout_learning_context": "context",
            "strategy_parameters": {"stop_loss_percent": 0.5},
            "strategy_parameter_specs": [],
            "strategy_decision": {
                "observed_close": 100.0,
                "strategy_order_size_multiplier": 0.5,
                "strategy_parameters": {"stop_loss_percent": 0.5},
                "market_snapshot": {"depth": [1, 2, 3]}
            },
            "feature_history": ["large", "signal", "payload"]
        });
        let compact = compact_scout_instance(&full);
        assert_eq!(compact["decision_id"], "decision-1");
        assert_eq!(compact["strategy_decision"]["observed_close"], 100.0);
        assert_eq!(compact["strategy_decision"]["strategy_order_size_multiplier"], 0.5);
        assert!(compact["strategy_decision"].get("market_snapshot").is_none());
        assert!(compact.get("feature_history").is_none());

        let mut state = PersistedState::default();
        state.scout_plan_results.insert(
            "request-1".into(),
            ScoutPlanResponse {
                audits: vec![full],
                selected_groups: vec!["group-1".into()],
            },
        );
        let (_, _, changed_plans) = compact_persisted_scout_payloads(&mut state);
        assert_eq!(changed_plans, vec!["request-1"]);
        assert!(state.scout_plan_results["request-1"].audits[0]
            .get("feature_history")
            .is_none());
    }
}

