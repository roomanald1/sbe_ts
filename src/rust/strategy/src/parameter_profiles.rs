use crate::{ParameterSpec, Signal, Strategy, StrategyParameterProfile};
use std::collections::BTreeMap;

pub fn signal_strategy_parameters(signal: &Signal) -> BTreeMap<String, f64> {
    signal
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("strategy_parameters"))
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_f64()?)))
        .collect()
}

pub fn strategy_parameter_key(symbol: &str, strategy: Strategy, parameter: &str) -> String {
    format!("{symbol}:{}:{parameter}", strategy.as_str())
}

pub fn legacy_strategy_parameter_name(parameter: &str) -> Option<&'static str> {
    match parameter {
        "profit_protection_activation_percent" => Some("shadow_profit_activation_percent"),
        "profit_protection_giveback_percent" => Some("shadow_profit_giveback_percent"),
        "profit_protection_minimum_lock_percent" => Some("shadow_profit_minimum_lock_percent"),
        _ => None,
    }
}

/// Validate persisted overrides against the owning strategy's current bounds and step size.
pub fn validate_strategy_parameter_overrides(values: &BTreeMap<String, f64>) -> Result<(), String> {
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
        let specs = strategy.parameter_specs();
        let Some(spec) = specs.iter().find(|spec| {
            spec.key.as_str() == parameter_name
                || legacy_strategy_parameter_name(spec.key.as_str()) == Some(parameter_name)
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

/// Combine learned values, bounded configured overrides, and a Scout probe variant.
pub fn effective_parameter_values(
    strategy: Strategy,
    profile_symbol: &str,
    learned: &BTreeMap<String, f64>,
    overrides: &BTreeMap<String, f64>,
    probe: Option<(&str, f64)>,
) -> BTreeMap<String, f64> {
    let mut parameters = strategy
        .parameter_specs()
        .into_iter()
        .map(|spec| (spec.key.as_str().to_owned(), spec.default))
        .collect::<BTreeMap<_, _>>();
    parameters.extend(learned.clone());
    for spec in strategy.parameter_specs() {
        let key = strategy_parameter_key(profile_symbol, strategy, spec.key.as_str());
        let configured = overrides.get(&key).copied().or_else(|| {
            legacy_strategy_parameter_name(spec.key.as_str()).and_then(|legacy| {
                overrides
                    .get(&strategy_parameter_key(profile_symbol, strategy, legacy))
                    .copied()
            })
        });
        if let Some(value) =
            configured.filter(|value| value.is_finite() && *value >= spec.min && *value <= spec.max)
        {
            parameters.insert(spec.key.as_str().to_owned(), value);
        }
    }
    if let Some((key, value)) = probe {
        if key.starts_with("combo:") {
            if let Some(variant) = crate::daily::parse_scout_combination(key, strategy) {
                parameters.extend(variant);
            }
        } else {
            parameters.insert(key.to_owned(), value);
        }
    }
    parameters
}

pub fn scout_parameter_specs(strategy: Strategy) -> Vec<ParameterSpec> {
    strategy
        .parameter_specs()
        .into_iter()
        .map(|spec| ParameterSpec {
            key: spec.key.as_str().to_owned(),
            label: spec.label.to_owned(),
            default: spec.default,
            min: spec.min,
            max: spec.max,
            step: spec.step,
            unit: spec.unit.to_owned(),
            learning_direction: spec.learning_direction,
        })
        .collect()
}

pub fn confident_win_rate_edge(positive: usize, samples: usize) -> f64 {
    if samples == 0 || positive > samples {
        return 0.0;
    }
    let n = samples as f64;
    let posterior_rate = (positive as f64 + 1.0) / (n + 2.0);
    let confidence = n / (n + 2.0);
    ((posterior_rate - 0.5) * 2.0 * confidence).clamp(-1.0, 1.0)
}

pub fn strategy_profile_value(profile: &StrategyParameterProfile, key: &str) -> Option<f64> {
    profile
        .parameters
        .iter()
        .find(|parameter| parameter.key == key)
        .map(|parameter| parameter.value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protective_strategies_expose_a_bounded_post_stop_cooldown_probe() {
        let specs = scout_parameter_specs(Strategy::LiveMomentum);
        let cooldown = specs
            .iter()
            .find(|spec| spec.key == "post_stop_cooldown_minutes")
            .unwrap();
        assert_eq!(
            (cooldown.default, cooldown.min, cooldown.max, cooldown.step),
            (2.0, 0.5, 10.0, 0.5)
        );
    }

    #[test]
    fn override_validation_checks_bounds_steps_and_legacy_names() {
        let strategy = Strategy::LiveMomentum;
        let spec = strategy.parameter_specs()[0];
        let key = strategy_parameter_key("BTC-USDT", strategy, spec.key.as_str());
        assert!(
            validate_strategy_parameter_overrides(&BTreeMap::from([(key.clone(), spec.min)]))
                .is_ok()
        );
        assert!(
            validate_strategy_parameter_overrides(&BTreeMap::from([(
                key.clone(),
                spec.min + spec.step / 2.0
            )]))
            .is_err()
        );
        assert!(validate_strategy_parameter_overrides(&BTreeMap::from([(key, f64::NAN)])).is_err());
        let legacy =
            strategy_parameter_key("BTC-USDT", strategy, "shadow_profit_activation_percent");
        assert_eq!(
            legacy_strategy_parameter_name("profit_protection_activation_percent"),
            Some("shadow_profit_activation_percent")
        );
        assert!(validate_strategy_parameter_overrides(&BTreeMap::from([(legacy, 0.5)])).is_ok());
    }

    #[test]
    fn profile_values_override_learning_and_apply_probe_variants() {
        let strategy = Strategy::LiveMomentum;
        let spec = strategy.parameter_specs()[0];
        let key = spec.key.as_str().to_owned();
        let learned = BTreeMap::from([(key.clone(), spec.default)]);
        let override_key = strategy_parameter_key("BTC-USDT", strategy, &key);
        let overrides = BTreeMap::from([(override_key, spec.min)]);
        let values = effective_parameter_values(
            strategy,
            "BTC-USDT",
            &learned,
            &overrides,
            Some((&key, spec.max)),
        );
        assert_eq!(values[&key], spec.max);
    }

    #[test]
    fn confidence_edge_is_bounded_and_empty_samples_are_neutral() {
        assert_eq!(confident_win_rate_edge(0, 0), 0.0);
        assert!(confident_win_rate_edge(10, 10) <= 1.0);
        assert!(confident_win_rate_edge(0, 10) >= -1.0);
        assert_eq!(confident_win_rate_edge(11, 10), 0.0);
    }
}
