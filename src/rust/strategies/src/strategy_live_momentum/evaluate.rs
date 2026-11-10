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
    let side = match insight.direction {
        Some(SignalDirection::Long) => 1.0,
        Some(SignalDirection::Short) => -1.0,
        None => return Ok(StrategyDecision::Reject),
    };
    let Some(minimum) =
        strategy_parameter_value(&insight.parameters, StrategyParameterKey::MinimumMomentum)
    else {
        return Ok(StrategyDecision::Reject);
    };
    let entry_condition =
        |value: Option<f64>| value.is_some_and(|value| value.is_finite() && side * value > minimum);
    Ok(if entry_condition(signal.components.momentum_10s_percent)
        || entry_condition(signal.components.trend_5m_volatility_units)
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
    let side = match insight.direction {
        Some(SignalDirection::Long) => 1.0,
        Some(SignalDirection::Short) => -1.0,
        None => return Some("Signal direction is neutral or unavailable".to_owned()),
    };
    let Some(minimum) =
        strategy_parameter_value(&insight.parameters, StrategyParameterKey::MinimumMomentum)
    else {
        return Some("Learned parameter minimum_momentum is unavailable".to_owned());
    };
    let momentum = signal.components.momentum_10s_percent.map(|value| side * value);
    let trend = signal.components.trend_5m_volatility_units.map(|value| side * value);
    let passes = |value: Option<f64>| {
        value.is_some_and(|value| value.is_finite() && value > minimum)
    };
    if passes(momentum) || passes(trend) {
        return None;
    }
    let describe = |value: Option<f64>| match value {
        Some(value) if value.is_finite() => format!("{value:.4}%"),
        Some(_) => "non-finite".to_owned(),
        None => "unavailable".to_owned(),
    };
    Some(format!(
        "Directional momentum {} (threshold > {:.4}%) and trend strength {} volatility units (threshold > {:.4}); neither met its threshold",
        describe(momentum),
        minimum,
        trend.map(|value| {
            if value.is_finite() {
                format!("{value:.4}")
            } else {
                "non-finite".to_owned()
            }
        }).unwrap_or_else(|| "unavailable".to_owned()),
        minimum
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Signal, SignalComponents};
    use std::collections::BTreeMap;

    fn signal(direction: &str, momentum: Option<f64>, trend: Option<f64>) -> Signal {
        Signal {
            symbol: "BTC-USDT".into(),
            direction: direction.into(),
            strength: 1.0,
            timestamp: 1,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents {
                momentum_10s_percent: momentum,
                trend_5m_volatility_units: trend,
                ..Default::default()
            },
            strategy_candidate: false,
            strategy: None,
            strategy_decision: None,
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    fn enters(direction: &str, momentum: Option<f64>, trend: Option<f64>) -> bool {
        let signal = signal(direction, momentum, trend);
        let parameters = BTreeMap::from([("minimum_momentum".to_owned(), 0.0)]);
        let signal_input = StrategySignal::entry(&signal, &parameters);
        matches!(evaluate(&signal_input), Ok(StrategyDecision::Enter))
    }

    #[test]
    fn entry_requires_signal_aligned_momentum_or_trend() {
        assert!(enters("bullish", Some(0.1), None));
        assert!(enters("bearish", Some(-0.1), None));
        assert!(enters("bullish", None, Some(0.1)));
        assert!(!enters("bullish", Some(-0.1), None));
        assert!(!enters("bearish", Some(0.1), None));
        assert!(!enters("neutral", Some(1.0), Some(1.0)));
        assert!(!enters("bullish", None, None));
    }
}

pub(super) fn should_exit_when_disabled(enabled: bool) -> bool {
    !enabled
}
pub(super) fn description() -> &'static str {
    "shared StopMarket protective stop and IOC stop/target exits; disabling this strategy closes its attributed positions"
}

#[cfg(test)]
mod parameter_bounds_live_momentum {
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
    fn parameter_bounds_live_momentum() {
        let strategy = Strategy::LiveMomentum;
        let specs = all_strategy_parameter_specs(strategy);
        assert!(!specs.is_empty(), "strategy must define parameters");
        for spec in &specs {
            assert_bounds(spec, strategy);
        }
    }
}
