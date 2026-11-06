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
        strategy_parameter_value(&parameters, StrategyParameterKey::MinimumFlowImbalance)
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
        .trade_flow_imbalance
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
mod parameter_bounds_live_trade_flow {
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
    fn parameter_bounds_live_trade_flow() {
        let strategy = Strategy::LiveTradeFlow;
        let specs = all_strategy_parameter_specs(strategy);
        assert!(!specs.is_empty(), "strategy must define parameters");
        for spec in &specs {
            assert_bounds(spec, strategy);
        }
    }
}

#[cfg(test)]
mod strategy_walk_forward_tests {
    use super::*;
    use crate::{Signal, SignalComponents, Strategy, StrategyWalkForwardEvidence};
    use std::collections::BTreeMap;

    fn signal() -> Signal {
        Signal {
            symbol: "BTC-USDT".into(),
            direction: "bullish".into(),
            strength: 0.8,
            timestamp: 100_000,
            stale: false,
            score: 0.8,
            ranging_probability: None,
            components: SignalComponents {
                trade_flow_imbalance: Some(0.5),
                ..SignalComponents::default()
            },
            strategy_candidate: false,
            strategy: Some("live-trade-flow".into()),
            strategy_decision: None,
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    #[test]
    fn only_mature_negative_strategy_evidence_blocks_an_entry() {
        let signal = signal();
        let parameters = BTreeMap::from([
            ("minimum_flow_imbalance".into(), 0.2),
            ("scout_minimum_walk_forward_samples".into(), 3.0),
            ("scout_minimum_positive_net_rate".into(), 0.5),
        ]);
        let mut no_evidence = StrategySignal::entry(&signal, &parameters);
        assert_eq!(Strategy::LiveTradeFlow.evaluate(&no_evidence).unwrap(), StrategyDecision::Enter);

        no_evidence.walk_forward_evidence = Some(StrategyWalkForwardEvidence {
            sample_count: 1,
            positive_net_rate: Some(0.0),
            average_net_return_percent: Some(-0.1),
            last_sample_timestamp_ms: 90_000,
        });
        let immature = no_evidence.clone();
        assert_eq!(Strategy::LiveTradeFlow.evaluate(&immature).unwrap(), StrategyDecision::Enter);

        no_evidence.walk_forward_evidence = Some(StrategyWalkForwardEvidence {
            sample_count: 3,
            positive_net_rate: Some(0.0),
            average_net_return_percent: Some(-0.1),
            last_sample_timestamp_ms: 90_000,
        });
        let mature_negative = no_evidence;
        assert_eq!(Strategy::LiveTradeFlow.evaluate(&mature_negative).unwrap(), StrategyDecision::Reject);
    }
}
