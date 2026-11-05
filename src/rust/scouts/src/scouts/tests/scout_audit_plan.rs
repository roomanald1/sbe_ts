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
}

