const MIN_EXIT_POLICY_SAMPLES: usize = 40;
const MIN_EXIT_POLICY_TRAINING_SAMPLES: usize = 30;
const MIN_EXIT_POLICY_VALIDATION_SAMPLES: usize = 10;
const MAX_EXIT_POLICY_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_EXIT_POLICY_CLOCK_SKEW_MS: u64 = 5 * 60 * 1_000;

fn exit_protection_policy_key(symbol: &str, strategy: &str) -> String {
    format!("{symbol}:{strategy}")
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
        parameters.insert(StrategyParameterKey::StopLossPercent.as_str().to_owned(), value);
    }
    if let Some(value) = policy.take_profit_percent {
        parameters.insert(StrategyParameterKey::TakeProfitPercent.as_str().to_owned(), value);
    }
    parameters
}

fn active_exit_protection_policy<'a>(
    settings: &'a Settings,
    symbol: &str,
    strategy: &str,
) -> Option<&'a strategy::LearnedExitProtectionPolicy> {
    let policy = settings
        .learned_exit_protection_policies
        .get(&exit_protection_policy_key(symbol, strategy))?;
    valid_exit_protection_policy(policy, current_time_ms()).then_some(policy)
}

fn apply_exit_protection_policies(
    policies: Vec<strategy::LearnedExitProtectionPolicy>,
    now_ms: u64,
) -> BTreeMap<String, strategy::LearnedExitProtectionPolicy> {
    let mut accepted = BTreeMap::new();
    for policy in policies {
        if valid_exit_protection_policy(&policy, now_ms) {
            accepted
                .entry(exit_protection_policy_key(&policy.symbol, &policy.strategy))
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
            stop_loss_percent: Some(0.5),
            take_profit_percent: Some(0.8),
            activation_percent: 0.20,
            giveback_percent: 0.15,
            minimum_lock_percent: 0.10,
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
            exit_protection_policy_key("BTC-USDT", "live-momentum"),
            policy(now.saturating_sub(MAX_EXIT_POLICY_AGE_MS + 1)),
        );
        assert!(active_exit_protection_policy(&settings, "BTC-USDT", "live-momentum").is_none());
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
        let policy = policy(now);
        settings.learned_exit_protection_policies.insert(
            exit_protection_policy_key("BTC-USDT", kind.as_str()),
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

        let learned_policy = policy(now);
        settings.learned_exit_protection_policies.insert(
            exit_protection_policy_key("BTC-USDT", "live-momentum"),
            learned_policy,
        );
        inject_learned_parameters(&settings, &mut signal);
        let parameters = signal_strategy_parameters(&signal);
        assert_eq!(parameters["minimum_momentum"], 0.011);
        assert_eq!(parameters["profit_protection_activation_percent"], 0.20);
    }
}
