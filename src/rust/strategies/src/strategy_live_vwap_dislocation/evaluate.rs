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
    let parameters = &insight.parameters;
    let Some(minimum) =
        strategy_parameter_value(&parameters, StrategyParameterKey::MinimumVwapDislocationBps)
    else {
        return Ok(StrategyDecision::Reject);
    };
    let side = match insight.direction {
        Some(SignalDirection::Long) => 1.0,
        Some(SignalDirection::Short) => -1.0,
        None => return Ok(StrategyDecision::Reject),
    };
    if signal
        .components
        .mispricing_vs_vwap_bps
        .is_some_and(|value| side * value >= minimum)
    {
        Ok(StrategyDecision::Enter)
    } else {
        Ok(StrategyDecision::Reject)
    }
}

pub(super) fn should_exit_when_disabled(enabled: bool) -> bool {
    !enabled
}
pub(super) fn description() -> &'static str {
    "shared StopMarket protective stop and IOC stop/target exits; disabling this strategy closes its attributed positions"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Signal, SignalComponents};
    use std::collections::BTreeMap;

    fn signal(direction: &str, dislocation: Option<f64>) -> Signal {
        Signal {
            symbol: "BTC-USDT".into(),
            direction: direction.into(),
            strength: 1.0,
            timestamp: 1,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents {
                mispricing_vs_vwap_bps: dislocation,
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

    #[test]
    fn entry_requires_directional_dislocation_to_meet_learned_threshold() {
        let parameters = BTreeMap::from([("minimum_vwap_dislocation_bps".to_owned(), 2.0)]);
        for (direction, dislocation, expected) in [
            ("bullish", Some(2.0), StrategyDecision::Enter),
            ("bullish", Some(1.99), StrategyDecision::Reject),
            ("bearish", Some(-2.0), StrategyDecision::Enter),
            ("bearish", Some(2.0), StrategyDecision::Reject),
            ("neutral", Some(3.0), StrategyDecision::Reject),
            ("bullish", None, StrategyDecision::Reject),
        ] {
            let signal = signal(direction, dislocation);
            let input = StrategySignal::entry(&signal, &parameters);
            assert_eq!(evaluate(&input).unwrap(), expected);
        }

        let signal = signal("bullish", Some(3.0));
        let empty_parameters = BTreeMap::new();
        let missing_parameter = StrategySignal::entry(&signal, &empty_parameters);
        assert_eq!(evaluate(&missing_parameter).unwrap(), StrategyDecision::Reject);
    }

    #[test]
    fn position_decision_handles_disabled_and_active_strategies() {
        let parameters = BTreeMap::new();
        assert_eq!(
            evaluate(&StrategySignal::position(&parameters, true, Some(true), false)).unwrap(),
            StrategyDecision::Hold
        );
        assert_eq!(
            evaluate(&StrategySignal::position(&parameters, true, Some(false), false)).unwrap(),
            StrategyDecision::Hold
        );
        assert_eq!(
            evaluate(&StrategySignal::position(&parameters, false, None, false)).unwrap(),
            StrategyDecision::Exit
        );
    }
}

#[cfg(test)]
mod parameter_bounds_live_vwap_dislocation {
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
    fn parameter_bounds_live_vwap_dislocation() {
        let strategy = Strategy::LiveVwapDislocation;
        let specs = all_strategy_parameter_specs(strategy);
        assert!(!specs.is_empty(), "strategy must define parameters");
        for spec in &specs {
            assert_bounds(spec, strategy);
        }
    }
}
