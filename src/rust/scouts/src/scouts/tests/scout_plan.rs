#[cfg(test)]
mod scout_plan_tests {
    use super::*;

    fn service() -> ServiceState {
        let path = std::env::temp_dir().join(format!(
            "scouts-plan-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let persisted = PersistedState::default();
        let strategy_outcomes_by_profile = Arc::clone(&persisted.strategy_outcomes_by_profile);
        let portfolio_instances_by_group = Arc::clone(&persisted.portfolio_instances_by_group);
        ServiceState {
            client: reqwest::Client::new(),
            learning_url: "http://127.0.0.1:1".into(),
            auto_trader_url: "http://127.0.0.1:1".into(),
            seen: Arc::new(Mutex::new(BTreeSet::new())),
            strategy_outcomes_by_profile,
            portfolio_instances_by_group,
            strategy_outcome_profile_index_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            scout: ScoutState {
                data: Arc::new(Mutex::new(persisted)),
                path,
                flush_lock: Arc::new(Mutex::new(())),
            },
        }
    }

    fn candidate(group: &str, profile: &str) -> ScoutPlanCandidate {
        ScoutPlanCandidate {
            generate_group_id: false,
            sample_group_id: group.into(),
            cooldown_key: group.into(),
            profile_key: profile.into(),
            eligible: true,
            parameter_variant: false,
            cooldown_ms: 300_000,
            audit_plans: vec![serde_json::json!({
                "decision_id": format!("{group}-decision"), "scout_exit_scenario": "strategy",
                "status": "scouted", "stage": "decision"
            })],
            audit_templates: Vec::new(),
            sample_payload: None,
            daily_policy: None,
            live_policy: None,
        }
    }

    #[tokio::test]
    async fn scouts_persists_its_own_configuration() {
        let service = service();
        let configuration = ScoutConfiguration {
            strategies: vec!["live-momentum".into()],
            max_active_samples: 5_000,
            max_active_positions: 10_000,
            ..ScoutConfiguration::default()
        };
        set_scout_configuration(State(service.clone()), Json(configuration))
            .await
            .unwrap();
        let returned = get_scout_configuration(State(service.clone())).await.0;
        assert_eq!(returned.strategies, vec!["live-momentum"]);
        assert_eq!(returned.max_active_samples, 5_000);
        assert_eq!(returned.max_active_positions, 10_000);
        let persisted = load_scout_state(&service.scout.path).unwrap();
        assert_eq!(persisted.scout_configuration.strategies, returned.strategies);
        let _ = std::fs::remove_file(&service.scout.path);
    }

    #[tokio::test]
    async fn plan_applies_batch_cap_cooldown_and_idempotent_retries() {
        let service = service();
        let request = ScoutPlanRequest {
            request_id: "batch-1".into(),
            now_ms: scout_now_ms(),
            max_active_batches: 1,
            max_active_variants_per_profile: 2,
            legacy_cooldowns: BTreeMap::new(),
            legacy_comparisons: Vec::new(),
            candidates: vec![
                candidate("group-a", "BTC:trend"),
                candidate("group-b", "ETH:trend"),
            ],
        };
        let first = create_scout_plan(State(service.clone()), Json(request.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(first.selected_groups.len(), 1);
        assert_eq!(first.audits[0]["scout"], true);
        assert_eq!(
            first.audits[0]["scout_sample_group_id"],
            first.selected_groups[0]
        );
        let retry = create_scout_plan(State(service.clone()), Json(request.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(retry.selected_groups, first.selected_groups);
        {
            let mut state = service.scout.data.lock().await;
            state.scout_sample_active_scenarios.clear();
        }
        let repeated_group = &first.selected_groups[0];
        let cooldown = create_scout_plan(
            State(service.clone()),
            Json(ScoutPlanRequest {
                request_id: "batch-2".into(),
                now_ms: request.now_ms + 1,
                max_active_batches: 5,
                legacy_cooldowns: BTreeMap::new(),
                legacy_comparisons: Vec::new(),
                candidates: vec![candidate(repeated_group, "BTC:trend")],
                ..request
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(cooldown.selected_groups.is_empty());
        let _ = std::fs::remove_file(&service.scout.path);
    }

    #[tokio::test]
    async fn rejects_oversized_plan_batches() {
        let service = service();
        let request = ScoutPlanRequest {
            request_id: "oversized".into(),
            now_ms: scout_now_ms(),
            max_active_batches: 1,
            max_active_variants_per_profile: 1,
            legacy_cooldowns: BTreeMap::new(),
            legacy_comparisons: Vec::new(),
            candidates: vec![candidate("x", "p"); 513],
        };
        assert!(
            create_scout_plan(State(service), Json(request))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn plan_limits_active_variants_per_profile() {
        let service = service();
        let mut first = candidate("variant-a", "BTC:momentum");
        first.parameter_variant = true;
        let mut second = candidate("variant-b", "BTC:momentum");
        second.parameter_variant = true;
        service
            .scout
            .data
            .lock()
            .await
            .scout_plan_profiles
            .insert("variant-existing".into(), "BTC:momentum".into());
        let response = create_scout_plan(
            State(service.clone()),
            Json(ScoutPlanRequest {
                request_id: "profile-cap".into(),
                now_ms: scout_now_ms(),
                max_active_batches: 5,
                max_active_variants_per_profile: 1,
                legacy_cooldowns: BTreeMap::new(),
                legacy_comparisons: Vec::new(),
                candidates: vec![first, second],
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(response.selected_groups.is_empty());
        let _ = std::fs::remove_file(service.scout.path);
    }

    #[tokio::test]
    async fn plan_imports_legacy_cooldowns_and_partial_comparison_without_replacing_state() {
        let service = service();
        {
            let mut state = service.scout.data.lock().await;
            state.scout_sample_active_scenarios.insert(
                "owned-active".into(),
                BTreeSet::from(["decision|strategy".into()]),
            );
            state.scout_variant_rotation_cursor = 7;
        }
        let _ = create_scout_plan(
            State(service.clone()),
            Json(ScoutPlanRequest {
                request_id: "legacy-import".into(),
                now_ms: scout_now_ms(),
                max_active_batches: 5,
                max_active_variants_per_profile: 2,
                legacy_cooldowns: BTreeMap::from([("legacy-key".into(), 123)]),
                legacy_comparisons: vec![LegacyScoutComparison {
                    comparison_id: "legacy-pair".into(),
                    baseline_return_percent: Some(0.3),
                    variant_return_percent: None,
                    learning_key: "paired_probe:BTC-USDT:live-momentum:minimum_momentum:0.7".into(),
                    updated_at_ms: 100,
                }],
                candidates: Vec::new(),
            }),
        )
        .await
        .unwrap();
        let state = service.scout.data.lock().await;
        assert_eq!(state.scout_sample_at_ms["legacy-key"], 123);
        assert!(
            state
                .scout_sample_active_scenarios
                .contains_key("owned-active")
        );
        assert_eq!(state.scout_variant_rotation_cursor, 7);
        assert_eq!(
            state.pairs["legacy-pair"].legacy_baseline_return_percent,
            Some(0.3)
        );
        let _ = std::fs::remove_file(service.scout.path.clone());
    }

    #[test]
    fn daily_policy_selects_levels_and_builds_stable_profile_groups() {
        let mut candidate = candidate("", "");
        candidate.generate_group_id = true;
        candidate.sample_group_id.clear();
        candidate.cooldown_key.clear();
        candidate.audit_plans.clear();
        candidate.audit_templates = vec![ScoutAuditTemplate {
            base_audit: serde_json::json!({
                "decision_id": "daily-base", "strategy": "donchian-20-btc200",
                "symbol": "BTC-USDT", "direction": "bullish", "timestamp": 1000,
                "signal_timestamp": 1000, "market_context": "bullish:trend:BTC-USDT:donchian-20-btc200",
                "strategy_decision": {"strategy_parameters": {}, "primary_level": 100.0, "observed_close": 99.6}
            }),
            notional_usd: 10.0,
            learning_context: "scout:context".into(),
            stop_loss_percent: 0.5,
            take_profit_percent: 0.8,
            level_percent: 0.0,
            strategy_only: false,
            parameter_entry_eligible: None,
            parameter_specs: vec![],
        }];
        candidate.daily_policy = Some(DailyScoutPolicy {
            evaluator_decision: "scout".into(),
            selected_scout: false,
            strategy_candidate: true,
            daily_features_present: true,
            independent_non_donchian_strategy: false,
            parameter_key: None,
            parameter_value: None,
        });
        let selected = expand_daily_scout_candidates(&[candidate]);
        assert_eq!(selected.len(), 7);
        assert_eq!(selected[0].audit_templates[0].level_percent, 0.5);
        let (group, cooldown) = generated_scout_group_and_cooldown(&selected[0]).unwrap();
        assert!(group.starts_with("daily-scout-level:0.50|donchian-20-btc200|BTC-USDT|bullish"));
        assert_eq!(
            cooldown,
            "daily|donchian-20-btc200|BTC-USDT|bullish|level:0.50"
        );
    }

    #[test]
    fn daily_policy_promotes_only_eligible_independent_rejects_to_scout_samples() {
        let mut candidate = candidate("", "");
        candidate.generate_group_id = true;
        candidate.sample_group_id.clear();
        candidate.cooldown_key.clear();
        candidate.audit_plans.clear();
        candidate.audit_templates = vec![ScoutAuditTemplate {
            base_audit: serde_json::json!({
                "decision_id": "independent-reject", "strategy": "trend-50-200", "symbol": "ETH-USDT",
                "direction": "bullish", "timestamp": 1000, "signal_timestamp": 1000,
                "market_context": "bullish:trend:ETH-USDT:trend-50-200",
                "strategy_decision": {"strategy_parameters": {}, "observed_close": 100.0}
            }),
            notional_usd: 5.0,
            learning_context: "scout:test".into(),
            stop_loss_percent: 0.5,
            take_profit_percent: 0.8,
            level_percent: 0.0,
            strategy_only: false,
            parameter_entry_eligible: None,
            parameter_specs: vec![],
        }];
        candidate.daily_policy = Some(DailyScoutPolicy {
            evaluator_decision: "reject".into(),
            selected_scout: false,
            strategy_candidate: true,
            daily_features_present: true,
            independent_non_donchian_strategy: true,
            parameter_key: None,
            parameter_value: None,
        });
        assert_eq!(
            expand_daily_scout_candidates(&[candidate.clone()]).len(),
            8
        );
        candidate
            .daily_policy
            .as_mut()
            .unwrap()
            .daily_features_present = false;
        assert!(expand_daily_scout_candidates(&[candidate]).is_empty());
    }

    #[test]
    fn service_drops_identical_entry_only_pairs_and_keeps_exit_or_zero_side_pairs() {
        let mut candidate = candidate("pair", "BTC:strategy");
        let make_template = |role: &str, eligible: bool, key: &str| ScoutAuditTemplate {
            base_audit: serde_json::json!({
                "scout_comparison_role": role,
                "scout_comparison_parameter_key": key,
            }),
            notional_usd: 1.0,
            learning_context: "scout:test".into(),
            stop_loss_percent: 0.5,
            take_profit_percent: 0.8,
            level_percent: 0.0,
            strategy_only: true,
            parameter_entry_eligible: Some(eligible),
            parameter_specs: vec![],
        };
        candidate.audit_templates = vec![
            make_template("baseline", true, "minimum_momentum"),
            make_template("variant", true, "minimum_momentum"),
        ];
        assert!(!scout_comparison_changes_exit(&candidate));
        candidate.audit_templates[0].base_audit["scout_comparison_parameter_key"] =
            serde_json::json!("take_profit_percent");
        candidate.audit_templates[1].base_audit["scout_comparison_parameter_key"] =
            serde_json::json!("take_profit_percent");
        assert!(scout_comparison_changes_exit(&candidate));
        candidate.audit_templates[0].parameter_entry_eligible = Some(false);
        candidate.audit_templates[0].base_audit["scout_comparison_parameter_key"] =
            serde_json::json!("minimum_momentum");
        candidate.audit_templates[1].base_audit["scout_comparison_parameter_key"] =
            serde_json::json!("minimum_momentum");
        assert!(scout_comparison_changes_exit(&candidate));
    }

    #[test]
    fn live_policy_checks_enabled_strategy_fresh_book_and_sets_midpoint() {
        let mut candidate = candidate("", "live-momentum|BTC-USDT");
        candidate.generate_group_id = true;
        candidate.sample_group_id.clear();
        candidate.cooldown_key.clear();
        candidate.audit_plans.clear();
        candidate.audit_templates = vec![ScoutAuditTemplate {
            base_audit: serde_json::json!({
                "decision_id": "live", "strategy": "live-momentum", "symbol": "BTC-USDT",
                "direction": "bullish", "timestamp": 1000,
                "strategy_decision": {"strategy_parameters": {}}
            }),
            notional_usd: 5.0,
            learning_context: "scout:live:trade:scenario".into(),
            stop_loss_percent: 0.5,
            take_profit_percent: 0.8,
            level_percent: 0.0,
            strategy_only: false,
            parameter_entry_eligible: None,
            parameter_specs: vec![],
        }];
        candidate.live_policy = Some(LiveScoutPolicy {
            now_ms: 10_000,
            strategy_enabled: true,
            book_updated_at_ms: Some(8_000),
            best_bid: Some(99.0),
            best_ask: Some(101.0),
        });
        let selected = expand_daily_scout_candidates(&[candidate.clone()]);
        assert!(selected[0].eligible);
        assert_eq!(
            selected[0].audit_templates[0].base_audit["strategy_decision"]["observed_close"],
            100.0
        );
        candidate.live_policy.as_mut().unwrap().strategy_enabled = false;
        assert!(!expand_daily_scout_candidates(&[candidate])[0].eligible);
    }
}

