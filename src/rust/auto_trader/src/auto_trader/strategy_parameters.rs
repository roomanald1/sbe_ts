fn strategy_parameter_key(symbol: &str, strategy: Strategy, parameter: &str) -> String {
    format!("{symbol}:{}:{parameter}", strategy.as_str())
}

fn legacy_strategy_parameter_name(parameter: &str) -> Option<&'static str> {
    match parameter {
        "profit_protection_activation_percent" => Some("shadow_profit_activation_percent"),
        "profit_protection_giveback_percent" => Some("shadow_profit_giveback_percent"),
        "profit_protection_minimum_lock_percent" => Some("shadow_profit_minimum_lock_percent"),
        _ => None,
    }
}

fn strategy_learning_stats(settings: &Settings, symbol: &str, strategy: Strategy) -> LearningStats {
    let marker = format!(":{symbol}:{}:", strategy.as_str());
    let now_ms = current_time_ms();
    settings
        .learning
        .iter()
        .filter(|(key, _)| key.contains(&marker))
        .map(|(_, stats)| {
            let mut recent = stats.clone();
            decay_learning_stats(&mut recent, now_ms);
            recent
        })
        .fold(LearningStats::default(), |mut total, stats| {
            total.samples += stats.samples;
            total.positive += stats.positive;
            total.realized_pnl += stats.realized_pnl;
            total
        })
}

fn strategy_parameter_profile(
    settings: &Settings,
    symbol: &str,
    strategy: Strategy,
) -> StrategyParameterProfile {
    let learning = strategy_learning_stats(settings, symbol, strategy);
    let positive_rate =
        (learning.samples > 0).then_some(learning.positive as f64 / learning.samples as f64);
    let performance = confident_win_rate_edge(learning.positive, learning.samples);
    let specs = all_strategy_parameter_specs(strategy);
    let learning_key = strategy_parameter_key(symbol, strategy, "learning_rate");
    let learning_rate = settings
        .strategy_parameter_overrides
        .get(&learning_key)
        .copied()
        .unwrap_or(0.05)
        .clamp(0.0, 0.25);
    let parameters = specs
        .iter()
        .map(|spec| {
            let key = strategy_parameter_key(symbol, strategy, spec.key);
            let configured = settings
                .strategy_parameter_overrides
                .get(&key)
                .copied()
                .or_else(|| {
                    legacy_strategy_parameter_name(spec.key).and_then(|legacy| {
                        settings
                            .strategy_parameter_overrides
                            .get(&strategy_parameter_key(symbol, strategy, legacy))
                            .copied()
                    })
                });
            let default_value = spec.default;
            let base = configured.unwrap_or(default_value);
            // Profitability alone cannot safely tune stop and target distances:
            // widening stops or tightening targets can increase risk. Keep these
            // fixed until outcomes are evaluated against alternative exit policies.
            let adaptive_step = if matches!(
                spec.key,
                "learning_rate" | "stop_loss_percent" | "take_profit_percent"
            ) {
                0.0
            } else {
                (spec.max - spec.min) * learning_rate * performance * spec.learning_direction
            };
            let candidate = (base + adaptive_step).clamp(spec.min, spec.max);
            let step_position = (candidate - spec.min) / spec.step;
            let learned_value =
                (spec.min + step_position.round() * spec.step).clamp(spec.min, spec.max);
            StrategyParameterValue {
                key: spec.key.into(),
                label: spec.label.into(),
                value: learned_value,
                learned_value,
                configured_value: configured,
                default_value,
                min: spec.min,
                max: spec.max,
                step: spec.step,
                unit: spec.unit.into(),
                overridden: configured.is_some(),
            }
        })
        .collect();
    StrategyParameterProfile {
        symbol: symbol.into(),
        strategy: strategy.as_str().into(),
        learning_samples: learning.samples,
        positive_rate,
        parameters,
    }
}

fn strategy_parameter_values(
    settings: &Settings,
    symbol: &str,
    strategy: Strategy,
) -> BTreeMap<String, f64> {
    strategy_parameter_profile(settings, symbol, strategy)
        .parameters
        .into_iter()
        .map(|parameter| (parameter.key, parameter.value))
        .collect()
}

fn validate_strategy_parameter_overrides(values: &BTreeMap<String, f64>) -> Result<(), String> {
    for (key, value) in values {
        if !value.is_finite() {
            return Err(format!("Strategy parameter {key} must be finite"));
        }
        let mut parts = key.splitn(3, ':');
        let Some(symbol) = parts.next() else {
            return Err(format!("Invalid strategy parameter key {key}"));
        };
        let Some(strategy_name) = parts.next() else {
            return Err(format!("Invalid strategy parameter key {key}"));
        };
        let Some(parameter_name) = parts.next() else {
            return Err(format!("Invalid strategy parameter key {key}"));
        };
        let Some(strategy) = Strategy::from_name(strategy_name) else {
            return Err(format!("Unknown strategy in parameter key {key}"));
        };
        let specs = all_strategy_parameter_specs(strategy);
        let Some(spec) = specs.iter().find(|spec| {
            spec.key == parameter_name
                || legacy_strategy_parameter_name(spec.key) == Some(parameter_name)
        }) else {
            return Err(format!("Unknown strategy parameter {key}"));
        };
        let step_position = (*value - spec.min) / spec.step;
        if symbol.is_empty()
            || *value < spec.min
            || *value > spec.max
            || (step_position - step_position.round()).abs() > 1e-6
        {
            return Err(format!(
                "Strategy parameter {key} must be between {} and {} in steps of {}",
                spec.min, spec.max, spec.step
            ));
        }
    }
    Ok(())
}

const STRATEGY_PARAMETER_MIN_SAMPLES: usize = 30;

fn confident_win_rate_edge(positive: usize, samples: usize) -> f64 {
    if samples < STRATEGY_PARAMETER_MIN_SAMPLES || positive > samples {
        return 0.0;
    }

    // Require the 95% Wilson interval to sit entirely on one side of 50%.
    // The edge stays zero while observed outcomes are statistically ambiguous.
    let n = samples as f64;
    let rate = positive as f64 / n;
    let z = 1.96;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let center = (rate + z2 / (2.0 * n)) / denominator;
    let margin = z * (rate * (1.0 - rate) / n + z2 / (4.0 * n * n)).sqrt() / denominator;
    let lower = center - margin;
    let upper = center + margin;

    if lower > 0.5 {
        ((lower - 0.5) * 2.0).clamp(0.0, 1.0)
    } else if upper < 0.5 {
        ((upper - 0.5) * 2.0).clamp(-1.0, 0.0)
    } else {
        0.0
    }
}

fn strategy_parameter_value(parameters: &BTreeMap<String, f64>, key: &str, default: f64) -> f64 {
    parameters.get(key).copied().unwrap_or(default)
}

fn strategy_profile_value(profile: &StrategyParameterProfile, key: &str) -> Option<f64> {
    profile
        .parameters
        .iter()
        .find(|parameter| parameter.key == key)
        .map(|parameter| parameter.value)
}
