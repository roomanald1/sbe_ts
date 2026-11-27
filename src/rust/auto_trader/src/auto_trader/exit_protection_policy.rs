const MIN_EXIT_POLICY_SAMPLES: usize = 40;
const MIN_EXIT_POLICY_TRAINING_SAMPLES: usize = 30;
const MIN_EXIT_POLICY_VALIDATION_SAMPLES: usize = 10;
const MAX_EXIT_POLICY_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_EXIT_POLICY_CLOCK_SKEW_MS: u64 = 5 * 60 * 1_000;

fn exit_protection_policy_key(
    symbol: &str,
    strategy: &str,
    profile_id: &str,
    profile_version: u64,
) -> String {
    format!("{symbol}:{strategy}:{profile_id}:v{profile_version}")
}

fn exit_protection_parameter_valid(
    strategy: Strategy,
    key: StrategyParameterKey,
    value: f64,
) -> bool {
    if !strategy
        .parameter_specs()
        .into_iter()
        .any(|spec| spec.key == key)
    {
        return false;
    }
    value.is_finite()
        && strategy
            .parameter_specs()
            .into_iter()
            .find(|spec| spec.key == key)
            .is_some_and(|spec| value >= spec.min && value <= spec.max)
}

fn exit_protection_named_parameter_valid(strategy: Strategy, key: &str, value: f64) -> bool {
    strategy
        .parameter_specs()
        .into_iter()
        .find(|spec| spec.key.as_str() == key)
        .is_some_and(|spec| strategy_parameter_value_is_valid(value, &spec))
}

fn valid_exit_protection_policy(
    policy: &strategy::LearnedExitProtectionPolicy,
    now_ms: u64,
) -> bool {
    let Some(kind) =
        Strategy::from_name(&policy.strategy).filter(|kind| !kind.is_legacy_composite())
    else {
        return false;
    };
    let timestamp_valid = policy.as_of_ms <= now_ms.saturating_add(MAX_EXIT_POLICY_CLOCK_SKEW_MS)
        && now_ms.saturating_sub(policy.as_of_ms) <= MAX_EXIT_POLICY_AGE_MS;
    let profile_identity_valid = policy
        .profile_id
        .as_deref()
        .is_some_and(|profile_id| !profile_id.trim().is_empty())
        && policy.profile_version.is_some();
    let metrics_finite = [
        policy.baseline_validation_mean_percent,
        policy.selected_validation_mean_percent,
        policy.validation_gain_percent,
        policy.validation_positive_rate,
        policy.validation_negative_difference_rate,
        policy.validation_tenth_percentile_difference_percent,
        policy.validation_worst_difference_percent,
    ]
    .into_iter()
    .all(f64::is_finite);
    timestamp_valid
        && profile_identity_valid
        && strategy::is_autotrader_symbol_allowed(&policy.symbol)
        && policy.sample_count >= MIN_EXIT_POLICY_SAMPLES
        && policy.training_count >= MIN_EXIT_POLICY_TRAINING_SAMPLES
        && policy.validation_count >= MIN_EXIT_POLICY_VALIDATION_SAMPLES
        && policy
            .training_count
            .saturating_add(policy.validation_count)
            <= policy.sample_count
        && metrics_finite
        && policy.validation_gain_percent >= 0.02
        && (policy.selected_validation_mean_percent
            - policy.baseline_validation_mean_percent
            - policy.validation_gain_percent)
            .abs()
            <= 1e-6
        && (0.0..=1.0).contains(&policy.validation_positive_rate)
        && (0.0..=0.60).contains(&policy.validation_negative_difference_rate)
        && policy.validation_tenth_percentile_difference_percent >= -0.25
        && policy.validation_worst_difference_percent >= -0.50
        && policy.validation_worst_difference_percent
            <= policy.validation_tenth_percentile_difference_percent
        && policy.stop_loss_percent.is_some_and(|value| {
            exit_protection_parameter_valid(kind, StrategyParameterKey::StopLossPercent, value)
        })
        && policy.take_profit_percent.is_some_and(|value| {
            exit_protection_parameter_valid(kind, StrategyParameterKey::TakeProfitPercent, value)
        })
        && exit_protection_parameter_valid(
            kind,
            StrategyParameterKey::ProfitProtectionActivationPercent,
            policy.activation_percent,
        )
        && exit_protection_parameter_valid(
            kind,
            StrategyParameterKey::ProfitProtectionGivebackPercent,
            policy.giveback_percent,
        )
        && exit_protection_parameter_valid(
            kind,
            StrategyParameterKey::ProfitProtectionMinimumLockPercent,
            policy.minimum_lock_percent,
        )
        && policy.minimum_lock_percent <= policy.activation_percent
        && policy
            .maximum_holding_duration_ms
            .is_none_or(|duration_ms| {
                let days = duration_ms as f64 / 86_400_000.0;
                exit_protection_parameter_valid(kind, StrategyParameterKey::MaximumHoldDays, days)
                    && kind
                        .parameter_specs()
                        .iter()
                        .find(|spec| spec.key == StrategyParameterKey::MaximumHoldDays)
                        .is_some_and(|spec| strategy_parameter_value_is_valid(days, spec))
            })
        && policy.trailing_activation_percent.is_some()
            == policy.trailing_distance_percent.is_some()
        && policy.trailing_activation_percent.is_none_or(|value| {
            value > 0.0
                && exit_protection_named_parameter_valid(kind, "trailing_activation_percent", value)
        })
        && policy.trailing_distance_percent.is_none_or(|value| {
            value > 0.0
                && exit_protection_named_parameter_valid(kind, "trailing_distance_percent", value)
        })
        && policy.breakeven_activation_percent.is_none_or(|value| {
            value > 0.0
                && exit_protection_named_parameter_valid(
                    kind,
                    "breakeven_activation_percent",
                    value,
                )
        })
}

fn exit_protection_policy_parameters(
    policy: &strategy::LearnedExitProtectionPolicy,
) -> BTreeMap<String, f64> {
    let mut parameters = BTreeMap::from([
        (
            StrategyParameterKey::ProfitProtectionActivationPercent
                .as_str()
                .to_owned(),
            policy.activation_percent,
        ),
        (
            StrategyParameterKey::ProfitProtectionGivebackPercent
                .as_str()
                .to_owned(),
            policy.giveback_percent,
        ),
        (
            StrategyParameterKey::ProfitProtectionMinimumLockPercent
                .as_str()
                .to_owned(),
            policy.minimum_lock_percent,
        ),
    ]);
    if let Some(value) = policy.stop_loss_percent {
        parameters.insert(
            StrategyParameterKey::StopLossPercent.as_str().to_owned(),
            value,
        );
    }
    if let Some(value) = policy.take_profit_percent {
        parameters.insert(
            StrategyParameterKey::TakeProfitPercent.as_str().to_owned(),
            value,
        );
    }
    if let Some(duration_ms) = policy.maximum_holding_duration_ms {
        parameters.insert(
            StrategyParameterKey::MaximumHoldDays.as_str().to_owned(),
            duration_ms as f64 / 86_400_000.0,
        );
    }
    for (key, value) in [
        (
            "trailing_activation_percent",
            policy.trailing_activation_percent,
        ),
        (
            "trailing_distance_percent",
            policy.trailing_distance_percent,
        ),
        (
            "breakeven_activation_percent",
            policy.breakeven_activation_percent,
        ),
    ] {
        if let Some(value) = value {
            parameters.insert(key.to_owned(), value);
        }
    }
    parameters
}

fn active_exit_protection_policy<'a>(
    settings: &'a Settings,
    symbol: &str,
    strategy: &str,
    profile_id: Option<&str>,
    profile_version: Option<u64>,
) -> Option<&'a strategy::LearnedExitProtectionPolicy> {
    let (Some(profile_id), Some(profile_version)) = (profile_id, profile_version) else {
        return None;
    };
    let policy = settings
        .learned_exit_protection_policies
        .get(&exit_protection_policy_key(
            symbol,
            strategy,
            profile_id,
            profile_version,
        ))?;
    valid_exit_protection_policy(policy, current_time_ms()).then_some(policy)
}

fn apply_exit_protection_policies(
    policies: Vec<strategy::LearnedExitProtectionPolicy>,
    now_ms: u64,
) -> BTreeMap<String, strategy::LearnedExitProtectionPolicy> {
    let mut accepted = BTreeMap::new();
    for policy in policies {
        if valid_exit_protection_policy(&policy, now_ms) {
            let (Some(profile_id), Some(profile_version)) =
                (policy.profile_id.as_deref(), policy.profile_version)
            else {
                continue;
            };
            accepted
                .entry(exit_protection_policy_key(
                    &policy.symbol,
                    &policy.strategy,
                    profile_id,
                    profile_version,
                ))
                .or_insert(policy);
        }
    }
    accepted
}

#[cfg(test)]
mod exit_protection_policy_tests {
    use super::*;

    fn policy(as_of_ms: u64) -> strategy::LearnedExitProtectionPolicy {
        strategy::LearnedExitProtectionPolicy {
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: Some("base-profile".into()),
            profile_version: Some(1),
            stop_loss_percent: Some(0.5),
            take_profit_percent: Some(0.8),
            activation_percent: 0.20,
            giveback_percent: 0.15,
            minimum_lock_percent: 0.10,
            maximum_holding_duration_ms: None,
            trailing_activation_percent: None,
            trailing_distance_percent: None,
            breakeven_activation_percent: None,
            sample_count: 40,
            training_count: 30,
            validation_count: 10,
            baseline_validation_mean_percent: 0.10,
            selected_validation_mean_percent: 0.15,
            validation_gain_percent: 0.05,
            validation_positive_rate: 0.70,
            validation_negative_difference_rate: 0.20,
            validation_tenth_percentile_difference_percent: -0.10,
            validation_worst_difference_percent: -0.25,
            as_of_ms,
        }
    }

    fn test_signal(symbol: &str, strategy: &str, timestamp: u64) -> Signal {
        Signal {
            range_location: None,
            symbol: symbol.to_owned(),
            direction: "bullish".into(),
            strength: 1.0,
            timestamp,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: true,
            strategy: Some(strategy.to_owned()),
            strategy_decision: Some(serde_json::json!({})),
            market_regime: None,
            market_state: None,
            news_attribution: None,
            entry_gate: Some(serde_json::json!({"selection_mode":"exploratory"})),
        }
    }

    #[test]
    fn snapshot_policy_validation_rejects_bad_bounds_evidence_and_staleness() {
        let now = current_time_ms();
        let valid = policy(now);
        assert!(valid_exit_protection_policy(&valid, now));

        let mut invalid_bound = valid.clone();
        invalid_bound.giveback_percent = 0.04;
        assert!(!valid_exit_protection_policy(&invalid_bound, now));
        let mut invalid_evidence = valid.clone();
        invalid_evidence.validation_count = 9;
        assert!(!valid_exit_protection_policy(&invalid_evidence, now));
        let stale = policy(now.saturating_sub(MAX_EXIT_POLICY_AGE_MS + 1));
        assert!(!valid_exit_protection_policy(&stale, now));

        let mut incomplete_trailing_pair = valid.clone();
        incomplete_trailing_pair.trailing_activation_percent = Some(0.5);
        assert!(!valid_exit_protection_policy(
            &incomplete_trailing_pair,
            now
        ));
        let mut unsafe_trailing_distance = valid.clone();
        unsafe_trailing_distance.trailing_activation_percent = Some(0.5);
        unsafe_trailing_distance.trailing_distance_percent = Some(1.2);
        assert!(!valid_exit_protection_policy(
            &unsafe_trailing_distance,
            now
        ));
        let mut disabled_breakeven_candidate = valid.clone();
        disabled_breakeven_candidate.breakeven_activation_percent = Some(0.0);
        assert!(!valid_exit_protection_policy(
            &disabled_breakeven_candidate,
            now
        ));
    }

    #[test]
    fn replacement_snapshot_drops_missing_or_invalid_exit_policies() {
        let now = current_time_ms();
        let valid = policy(now);
        let accepted = apply_exit_protection_policies(vec![valid.clone()], now);
        assert_eq!(accepted.len(), 1);
        assert!(apply_exit_protection_policies(Vec::new(), now).is_empty());

        let mut invalid = valid;
        invalid.validation_gain_percent = -0.1;
        assert!(apply_exit_protection_policies(vec![invalid], now).is_empty());
    }

    #[test]
    fn persisted_policy_expires_at_use_when_learning_stops_refreshing() {
        let mut settings = Settings::default();
        let now = current_time_ms();
        settings.learned_exit_protection_policies.insert(
            exit_protection_policy_key("BTC-USDT", "live-momentum", "base-profile", 1),
            policy(now.saturating_sub(MAX_EXIT_POLICY_AGE_MS + 1)),
        );
        assert!(
            active_exit_protection_policy(
                &settings,
                "BTC-USDT",
                "live-momentum",
                Some("base-profile"),
                Some(1)
            )
            .is_none()
        );
    }

    #[test]
    fn maximum_hold_policy_is_applied_only_to_matching_profile_version_and_captured_on_entry() {
        const DAY_MS: u64 = 86_400_000;
        let now = current_time_ms();
        let kind = Strategy::Rsi2OversoldRebound;
        let parameters = kind
            .parameter_specs()
            .iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let profile_id = strategy::profile_id("BTC-USDT", kind.as_str(), &parameters);
        let profile_version = 7;
        let mut settings = Settings::default();
        settings.learned_profiles.insert(
            format!("BTC-USDT:{}", kind.as_str()),
            strategy::StrategyProfile {
                profile_id: profile_id.clone(),
                version: profile_version,
                symbol: "BTC-USDT".into(),
                strategy: kind.as_str().into(),
                parameters,
                learning_samples: 40,
                positive_samples: 24,
                realized_pnl: 2.0,
            },
        );
        let mut selected_policy = policy(now);
        selected_policy.strategy = kind.as_str().into();
        selected_policy.profile_id = Some(profile_id.clone());
        selected_policy.profile_version = Some(profile_version);
        selected_policy.maximum_holding_duration_ms = Some(3 * DAY_MS);
        selected_policy.trailing_activation_percent = Some(0.55);
        selected_policy.trailing_distance_percent = Some(0.20);
        selected_policy.breakeven_activation_percent = Some(0.30);
        assert!(valid_exit_protection_policy(&selected_policy, now));
        settings.learned_exit_protection_policies.insert(
            exit_protection_policy_key("BTC-USDT", kind.as_str(), &profile_id, profile_version),
            selected_policy,
        );

        let mut signal = test_signal("BTC-USDT", kind.as_str(), now / 1_000);
        inject_learned_parameters(&settings, &mut signal);
        let decision = signal.strategy_decision.as_ref().unwrap();
        let selected_parameters = signal_strategy_parameters(&signal);
        assert_eq!(selected_parameters["maximum_hold_days"], 3.0);
        assert_eq!(selected_parameters["trailing_activation_percent"], 0.55);
        assert_eq!(selected_parameters["trailing_distance_percent"], 0.20);
        assert_eq!(selected_parameters["breakeven_activation_percent"], 0.30);
        assert_eq!(
            decision["learned_exit_protection_policy"]["profile_id"],
            profile_id
        );
        let position = TraderPosition {
            quantity: 1.0,
            strategy: Some(kind.as_str().to_owned()),
            entry_timestamp_ms: now,
            strategy_decision: signal.strategy_decision.clone(),
            ..TraderPosition::default()
        };
        let hold_status = maximum_hold_status(&position, now + 4 * DAY_MS);
        assert_eq!(hold_status.maximum_hold_days, Some(3.0));
        assert!(hold_status.expired);

        let mut newer_profile_settings = settings.clone();
        newer_profile_settings
            .learned_profiles
            .get_mut(&format!("BTC-USDT:{}", kind.as_str()))
            .unwrap()
            .version = profile_version + 1;
        let mut newer_signal = test_signal("BTC-USDT", kind.as_str(), now / 1_000);
        inject_learned_parameters(&newer_profile_settings, &mut newer_signal);
        assert_eq!(
            signal_strategy_parameters(&newer_signal)["maximum_hold_days"],
            5.0
        );
        assert!(
            newer_signal
                .strategy_decision
                .as_ref()
                .unwrap()
                .get("learned_exit_protection_policy")
                .is_none()
        );
    }

    #[test]
    fn different_profile_policies_reach_new_positions_and_drive_distinct_exit_states() {
        let now = current_time_ms();
        let mut settings = Settings::default();
        let definitions = [
            (
                Strategy::LiveMomentum,
                "profile-momentum-v1",
                0.30,
                0.10,
                0.25,
            ),
            (Strategy::LiveOrderBook, "profile-book-v1", 0.60, 0.20, 0.50),
        ];
        let mut positions = Vec::new();
        for (kind, profile_id, trailing_activation, trailing_distance, breakeven_activation) in
            definitions
        {
            let profile_version = 1;
            let parameters = kind
                .parameter_specs()
                .iter()
                .map(|spec| (spec.key.as_str().to_owned(), spec.default))
                .collect::<BTreeMap<_, _>>();
            settings.learned_profiles.insert(
                format!("BTC-USDT:{}", kind.as_str()),
                strategy::StrategyProfile {
                    profile_id: profile_id.to_owned(),
                    version: profile_version,
                    symbol: "BTC-USDT".into(),
                    strategy: kind.as_str().into(),
                    parameters,
                    learning_samples: 40,
                    positive_samples: 24,
                    realized_pnl: 2.0,
                },
            );
            let mut selected_policy = policy(now);
            selected_policy.strategy = kind.as_str().into();
            selected_policy.profile_id = Some(profile_id.to_owned());
            selected_policy.profile_version = Some(profile_version);
            selected_policy.trailing_activation_percent = Some(trailing_activation);
            selected_policy.trailing_distance_percent = Some(trailing_distance);
            selected_policy.breakeven_activation_percent = Some(breakeven_activation);
            assert!(valid_exit_protection_policy(&selected_policy, now));
            settings.learned_exit_protection_policies.insert(
                exit_protection_policy_key("BTC-USDT", kind.as_str(), profile_id, profile_version),
                selected_policy,
            );

            let mut signal = test_signal("BTC-USDT", kind.as_str(), now / 1_000);
            inject_learned_parameters(&settings, &mut signal);
            let selected_parameters = signal_strategy_parameters(&signal);
            assert_eq!(
                selected_parameters["trailing_activation_percent"],
                trailing_activation
            );
            assert_eq!(
                selected_parameters["trailing_distance_percent"],
                trailing_distance
            );
            assert_eq!(
                selected_parameters["breakeven_activation_percent"],
                breakeven_activation
            );
            positions.push(TraderPosition {
                quantity: 1.0,
                average_entry_price: 100.0,
                strategy: Some(kind.as_str().to_owned()),
                strategy_decision: signal.strategy_decision,
                trailing_peak_return_percent: Some(0.40),
                breakeven_protection_activated: false,
                initial_risk_reference: Some(InitialRiskReference {
                    schema_version: 1,
                    quote_currency: Some("USDT".into()),
                    stop_loss_percent: Some(0.5),
                    quote_amount: Some(0.5),
                    fill_ids: vec![format!("entry-{}", kind.as_str())],
                    status: "complete".into(),
                }),
                ..TraderPosition::default()
            });
        }

        let momentum = profile_exit_protection_state(&positions[0], 0.25);
        let order_book = profile_exit_protection_state(&positions[1], 0.25);
        assert!(momentum.trailing_stop_exit);
        assert!(!order_book.trailing_stop_exit);
        assert!(momentum.breakeven_activated);
        assert!(!order_book.breakeven_activated);
        assert_eq!(
            positions[0]
                .initial_risk_reference
                .as_ref()
                .unwrap()
                .quote_amount,
            Some(0.5)
        );
        assert_eq!(
            positions[1]
                .initial_risk_reference
                .as_ref()
                .unwrap()
                .quote_amount,
            Some(0.5)
        );
    }

    #[test]
    fn injection_applies_policy_then_configured_override_and_updates_profile_identity() {
        let now = current_time_ms();
        let kind = Strategy::LiveMomentum;
        let mut settings = Settings::default();
        settings.learned_profiles.insert(
            "BTC-USDT:live-momentum".into(),
            strategy::StrategyProfile {
                profile_id: "base-profile".into(),
                version: 12,
                symbol: "BTC-USDT".into(),
                strategy: kind.as_str().into(),
                parameters: BTreeMap::from([("minimum_momentum".into(), 0.003)]),
                learning_samples: 40,
                positive_samples: 25,
                realized_pnl: 3.0,
            },
        );
        let mut policy = policy(now);
        policy.profile_version = Some(12);
        settings.learned_exit_protection_policies.insert(
            exit_protection_policy_key("BTC-USDT", kind.as_str(), "base-profile", 12),
            policy.clone(),
        );
        settings.strategy_parameter_overrides.insert(
            strategy_parameter_key(
                "BTC-USDT",
                kind,
                StrategyParameterKey::ProfitProtectionActivationPercent.as_str(),
            ),
            0.25,
        );
        let mut signal = Signal {
            range_location: None,
            symbol: "BTC-USDT".into(),
            direction: "bullish".into(),
            strength: 1.0,
            timestamp: now / 1_000,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: true,
            strategy: Some(kind.as_str().into()),
            strategy_decision: Some(serde_json::json!({})),
            market_regime: None,
            market_state: None,
            news_attribution: None,
            entry_gate: None,
        };

        inject_learned_parameters(&settings, &mut signal);
        let parameters = signal_strategy_parameters(&signal);
        let decision = signal.strategy_decision.as_ref().unwrap();
        assert_eq!(parameters["profit_protection_activation_percent"], 0.25);
        assert_eq!(parameters["profit_protection_giveback_percent"], 0.15);
        assert_eq!(parameters["profit_protection_minimum_lock_percent"], 0.10);
        assert_eq!(parameters["minimum_momentum"], 0.003);
        assert_eq!(decision["learning_profile_version"], 12);
        assert_eq!(
            decision["learning_profile_id"],
            strategy::profile_id("BTC-USDT", kind.as_str(), &parameters)
        );
        assert_eq!(decision["learned_exit_protection_policy"]["as_of_ms"], now);
        assert_eq!(
            decision["effective_exit_protection_parameters"]["profit_protection_activation_percent"],
            0.25
        );
        let profile = learned_strategy_parameter_profile(&settings, "BTC-USDT", kind);
        let activation = profile
            .parameters
            .iter()
            .find(|parameter| {
                parameter.key == StrategyParameterKey::ProfitProtectionActivationPercent.as_str()
            })
            .unwrap();
        assert_eq!(activation.learned_value, 0.20);
        assert_eq!(activation.configured_value, Some(0.25));
        assert_eq!(activation.value, 0.25);

        signal.strategy_decision.as_mut().unwrap()["scout_parameter_key"] =
            serde_json::json!(StrategyParameterKey::ProfitProtectionActivationPercent.as_str());
        signal.strategy_decision.as_mut().unwrap()["scout_parameter_value"] =
            serde_json::json!(0.30);
        inject_learned_parameters(&settings, &mut signal);
        assert_eq!(
            signal_strategy_parameters(&signal)["profit_protection_activation_percent"],
            0.30
        );
    }

    #[test]
    fn missing_policy_and_base_profile_preserve_worker_parameters() {
        let now = current_time_ms();
        let mut settings = Settings::default();
        let mut signal = Signal {
            range_location: None,
            symbol: "BTC-USDT".into(),
            direction: "bullish".into(),
            strength: 1.0,
            timestamp: now / 1_000,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: true,
            strategy: Some("live-momentum".into()),
            strategy_decision: Some(serde_json::json!({
                "strategy_parameters": { "minimum_momentum": 0.011 },
                "learning_profile_id": "worker-profile",
                "learning_profile_version": 7
            })),
            market_regime: None,
            market_state: None,
            news_attribution: None,
            entry_gate: None,
        };
        let original = signal.strategy_decision.clone();
        inject_learned_parameters(&settings, &mut signal);
        assert_eq!(signal.strategy_decision, original);

        let mut learned_policy = policy(now);
        learned_policy.profile_id = Some("worker-profile".into());
        learned_policy.profile_version = Some(7);
        settings.learned_exit_protection_policies.insert(
            exit_protection_policy_key("BTC-USDT", "live-momentum", "worker-profile", 7),
            learned_policy,
        );
        inject_learned_parameters(&settings, &mut signal);
        let parameters = signal_strategy_parameters(&signal);
        assert_eq!(parameters["minimum_momentum"], 0.011);
        assert_eq!(parameters["profit_protection_activation_percent"], 0.20);
    }
}
