use crate::{
    regime_conditioned_parameter_value, strategy_parameter_value, SignalDirection,
    StrategyDecision, StrategyParameterKey, StrategySignal,
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
    let Some(minimum) = regime_conditioned_parameter_value(
        signal,
        &insight.parameters,
        StrategyParameterKey::MinimumFlowImbalance,
    ) else {
        return Ok(StrategyDecision::Reject);
    };
    let Some(minimum_strength) = strategy_parameter_value(
        &insight.parameters,
        StrategyParameterKey::MinimumSignalStrength,
    ) else {
        return Ok(StrategyDecision::Reject);
    };
    if !signal.strength.is_finite() || signal.strength < minimum_strength {
        return Ok(StrategyDecision::Reject);
    }
    let side = match insight.direction {
        Some(SignalDirection::Long) => 1.0,
        Some(SignalDirection::Short) => -1.0,
        None => return Ok(StrategyDecision::Reject),
    };
    let flow_confirms = signal
        .components
        .trade_flow_imbalance
        .is_some_and(|value| side * value >= minimum);
    // In choppy or volatile regimes, rolling notional-flow sign can flip
    // without a price response. Confirm direction by sign only, avoiding a
    // comparison between the flow ratio and percent momentum units.
    let price_confirms = !requires_price_confirmation(signal)
        || signal
            .components
            .momentum_10s_percent
            .is_some_and(|value| value.is_finite() && side * value > 0.0);
    Ok(if flow_confirms && price_confirms {
        StrategyDecision::Enter
    } else {
        StrategyDecision::Reject
    })
}

pub(crate) fn entry_rejection_reason(insight: &StrategySignal<'_>) -> Option<String> {
    let Some(signal) = insight.signal.as_ref() else {
        return Some("No live insight signal was provided".to_owned());
    };
    let Some(minimum_strength) = strategy_parameter_value(
        &insight.parameters,
        StrategyParameterKey::MinimumSignalStrength,
    ) else {
        return Some("Learned parameter minimum_signal_strength is unavailable".to_owned());
    };
    if !signal.strength.is_finite() || signal.strength < minimum_strength {
        let observed = if signal.strength.is_finite() {
            format!("{:.3}", signal.strength)
        } else {
            "non-finite".to_owned()
        };
        return Some(format!("Strength {observed} < {:.3}", minimum_strength));
    }
    let Some(minimum) = regime_conditioned_parameter_value(
        signal,
        &insight.parameters,
        StrategyParameterKey::MinimumFlowImbalance,
    ) else {
        return Some("Learned parameter minimum_flow_imbalance is unavailable".to_owned());
    };
    let side = match insight.direction {
        Some(SignalDirection::Long) => 1.0,
        Some(SignalDirection::Short) => -1.0,
        None => return Some(SignalDirection::non_directional_reason(&signal.direction)),
    };
    let directional = signal
        .components
        .trade_flow_imbalance
        .map(|value| side * value);
    if !directional.is_some_and(|value| value >= minimum) {
        let observed = match directional {
            Some(value) if value.is_finite() => format!("{value:.3}"),
            Some(_) => "non-finite".to_owned(),
            None => "unavailable".to_owned(),
        };
        return Some(format!(
            "Directional trade-flow imbalance {observed}; requires at least {minimum:.3}"
        ));
    }
    if requires_price_confirmation(signal)
        && !signal
            .components
            .momentum_10s_percent
            .is_some_and(|value| value.is_finite() && side * value > 0.0)
    {
        let momentum = signal.components.momentum_10s_percent;
        let observed = match momentum {
            Some(value) if value.is_finite() => format!("{value:.4}%"),
            Some(_) => "non-finite".to_owned(),
            None => "unavailable".to_owned(),
        };
        return Some(format!(
            "Directional trade flow is not price-confirmed in the {} regime: 10s momentum {observed} does not confirm the selected side",
            signal.market_regime.as_deref().unwrap_or("unknown")
        ));
    }
    None
}

fn requires_price_confirmation(signal: &crate::Signal) -> bool {
    matches!(
        signal.market_regime.as_deref(),
        Some("range" | "mixed" | "volatile")
    )
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
            range_location: None,
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
            market_state: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    #[test]
    fn only_mature_negative_strategy_evidence_blocks_an_entry() {
        let signal = signal();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let parameters = BTreeMap::from([
            ("minimum_flow_imbalance".into(), 0.2),
            ("scout_minimum_walk_forward_samples".into(), 3.0),
            ("scout_minimum_positive_net_rate".into(), 0.5),
            ("scout_maximum_evidence_age_seconds".into(), 86400.0),
            ("minimum_signal_strength".into(), 0.0),
        ]);
        let mut no_evidence = StrategySignal::entry(&signal, &parameters);
        assert_eq!(
            Strategy::LiveTradeFlow.evaluate(&no_evidence).unwrap(),
            StrategyDecision::Enter
        );

        no_evidence.walk_forward_evidence = Some(StrategyWalkForwardEvidence {
            sample_count: 1,
            positive_net_rate: Some(0.0),
            average_net_return_percent: Some(-0.1),
            last_sample_timestamp_ms: now_ms,
            ..Default::default()
        });
        let immature = no_evidence.clone();
        assert_eq!(
            Strategy::LiveTradeFlow.evaluate(&immature).unwrap(),
            StrategyDecision::Enter
        );

        no_evidence.walk_forward_evidence = Some(StrategyWalkForwardEvidence {
            sample_count: 3,
            positive_net_rate: Some(0.0),
            average_net_return_percent: Some(-0.1),
            last_sample_timestamp_ms: now_ms,
            ..Default::default()
        });
        let mature_negative = no_evidence;
        assert_eq!(
            Strategy::LiveTradeFlow.evaluate(&mature_negative).unwrap(),
            StrategyDecision::Reject
        );
    }
}

#[cfg(test)]
mod regime_price_confirmation_tests {
    use super::*;
    use crate::{Signal, SignalComponents, Strategy};
    use std::collections::BTreeMap;

    fn signal(regime: &str, direction: &str, flow: f64, momentum: Option<f64>) -> Signal {
        Signal {
            range_location: None,
            symbol: "OP-USDT".into(),
            direction: direction.into(),
            strength: 0.8,
            timestamp: 100_000,
            stale: false,
            score: 0.8,
            ranging_probability: None,
            components: SignalComponents {
                trade_flow_imbalance: Some(flow),
                momentum_10s_percent: momentum,
                ..SignalComponents::default()
            },
            strategy_candidate: true,
            strategy: Some("live-trade-flow".into()),
            strategy_decision: None,
            market_regime: Some(regime.to_owned()),
            market_state: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    fn parameters() -> BTreeMap<String, f64> {
        BTreeMap::from([
            ("minimum_flow_imbalance".into(), 0.2),
            ("minimum_signal_strength".into(), 0.0),
        ])
    }

    #[test]
    fn choppy_regimes_require_same_side_price_confirmation_without_changing_flow_threshold() {
        let parameters = parameters();
        let range_opposing_signal = signal("range", "bullish", 0.5, Some(-0.01));
        let range_flow_without_confirmation =
            StrategySignal::entry(&range_opposing_signal, &parameters);
        assert_eq!(
            Strategy::LiveTradeFlow
                .evaluate(&range_flow_without_confirmation)
                .unwrap(),
            StrategyDecision::Reject
        );
        assert!(Strategy::LiveTradeFlow
            .entry_rejection_reason(&range_flow_without_confirmation)
            .unwrap()
            .contains("not price-confirmed"));

        let range_confirmed_signal = signal("range", "bullish", 0.5, Some(0.01));
        let range_confirmed = StrategySignal::entry(&range_confirmed_signal, &parameters);
        assert_eq!(
            Strategy::LiveTradeFlow.evaluate(&range_confirmed).unwrap(),
            StrategyDecision::Enter
        );

        let volatile_signal = signal("volatile", "bearish", -0.5, None);
        let volatile_without_confirmation = StrategySignal::entry(&volatile_signal, &parameters);
        assert_eq!(
            Strategy::LiveTradeFlow
                .evaluate(&volatile_without_confirmation)
                .unwrap(),
            StrategyDecision::Reject
        );

        let trend_signal = signal("trend", "bullish", 0.5, Some(-0.01));
        let trend_flow_leads_price = StrategySignal::entry(&trend_signal, &parameters);
        assert_eq!(
            Strategy::LiveTradeFlow
                .evaluate(&trend_flow_leads_price)
                .unwrap(),
            StrategyDecision::Enter
        );

        let below_threshold_signal = signal("range", "bullish", 0.19, Some(0.01));
        let below_threshold = StrategySignal::entry(&below_threshold_signal, &parameters);
        assert_eq!(
            Strategy::LiveTradeFlow.evaluate(&below_threshold).unwrap(),
            StrategyDecision::Reject
        );
        assert!(Strategy::LiveTradeFlow
            .entry_rejection_reason(&below_threshold)
            .unwrap()
            .contains("requires at least 0.200"));
    }
}
