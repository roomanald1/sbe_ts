use crate::{
    strategy_parameter_value, SignalDirection, StrategyDecision, StrategyParameterKey,
    StrategySignal,
};
pub(crate) fn evaluate(insight: &StrategySignal<'_>) -> Result<StrategyDecision, String> {
    if let Some(decision) =
        insight.position_decision(should_exit_when_disabled(insight.strategy_enabled), false)
    {
        return Ok(decision);
    }
    let Some(signal) = insight.signal.as_ref() else {
        return Ok(StrategyDecision::Reject);
    };
    let Some(minimum) = strategy_parameter_value(
        &insight.parameters,
        StrategyParameterKey::MinimumAbsorptionPressure,
    ) else {
        return Ok(StrategyDecision::Reject);
    };
    let side = match insight.direction {
        Some(SignalDirection::Long) => 1.0,
        Some(SignalDirection::Short) => -1.0,
        None => return Ok(StrategyDecision::Reject),
    };
    Ok(if signal
        .components
        .pressure_without_price_move
        .is_some_and(|value| -side * value >= minimum)
    {
        StrategyDecision::Enter
    } else {
        StrategyDecision::Reject
    })
}

pub(crate) fn entry_rejection_reason(insight: &StrategySignal<'_>) -> Option<String> {
    let Some(signal) = insight.signal.as_ref() else {
        return Some("No live insight signal was provided".to_owned());
    };
    let Some(minimum) =
        strategy_parameter_value(
            &insight.parameters,
            StrategyParameterKey::MinimumAbsorptionPressure,
        )
    else {
        return Some("Learned parameter minimum_absorption_pressure is unavailable".to_owned());
    };
    let side = match insight.direction {
        Some(SignalDirection::Long) => 1.0,
        Some(SignalDirection::Short) => -1.0,
        None => return Some(SignalDirection::non_directional_reason(&signal.direction)),
    };
    let directional = signal
        .components
        .pressure_without_price_move
        .map(|value| -side * value);
    if directional.is_some_and(|value| value >= minimum) {
        return None;
    }
    let observed = match directional {
        Some(value) if value.is_finite() => format!("{value:.3}"),
        Some(_) => "non-finite".to_owned(),
        None => "unavailable".to_owned(),
    };
    Some(format!(
        "Opposing absorption pressure {observed}; requires at least {minimum:.3}"
    ))
}

pub(super) fn should_exit_when_disabled(enabled: bool) -> bool {
    !enabled
}
pub(super) fn description() -> &'static str {
    "shared StopMarket protective stop and IOC stop/target exits; disabling this strategy closes its attributed positions"
}

#[cfg(test)]
mod parameter_bounds_live_absorption_pressure {
    use crate::{
        all_strategy_parameter_specs, validate_strategy_parameter_overrides, Strategy,
        StrategyParameterSpec,
    };
    use std::collections::BTreeMap;

    fn assert_bounds(spec: &StrategyParameterSpec, strategy: Strategy) {
        let key = format!("BOUNDS:{}:{}", strategy.as_str(), spec.key.as_str());
        let mut values = BTreeMap::from([(key.clone(), spec.min)]);
        assert!(
            validate_strategy_parameter_overrides(&values).is_ok(),
            "{}:{} should accept its minimum",
            strategy.as_str(),
            spec.key.as_str()
        );
        values.insert(key.clone(), spec.max);
        assert!(
            validate_strategy_parameter_overrides(&values).is_ok(),
            "{}:{} should accept its maximum",
            strategy.as_str(),
            spec.key.as_str()
        );
        values.insert(key.clone(), spec.min - spec.step);
        assert!(
            validate_strategy_parameter_overrides(&values).is_err(),
            "{}:{} should reject below minimum",
            strategy.as_str(),
            spec.key.as_str()
        );
        values.insert(key, spec.max + spec.step);
        assert!(
            validate_strategy_parameter_overrides(&values).is_err(),
            "{}:{} should reject above maximum",
            strategy.as_str(),
            spec.key.as_str()
        );
    }

    #[test]
    fn parameter_bounds_live_absorption_pressure() {
        let strategy = Strategy::LiveAbsorptionPressure;
        let specs = all_strategy_parameter_specs(strategy);
        assert!(!specs.is_empty(), "strategy must define parameters");
        for spec in &specs {
            assert_bounds(spec, strategy);
        }
    }
}
