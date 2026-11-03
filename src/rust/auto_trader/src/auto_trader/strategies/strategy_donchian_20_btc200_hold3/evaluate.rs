use crate::auto_trader::{strategy_parameter_value, Strategy, StrategyDecision, StrategyParameterKey, StrategySignal};

pub(crate) fn evaluate(input: &StrategySignal<'_>) -> Result<StrategyDecision, String> {
    let exit_when_inactive = input
        .signal_active
        .is_some_and(|active| should_exit_when_insight_inactive(active));
    if let Some(decision) = input.position_decision(
        should_exit_when_disabled(input.strategy_enabled),
        exit_when_inactive,
    ) {
        return Ok(decision);
    }
    let Some(signal) = input.signal else {
        return Ok(StrategyDecision::Reject);
    };
    let Some(entry_distance) = strategy_parameter_value(input.parameters, StrategyParameterKey::BreakoutEntryDistancePercent) else {
        return Err("Missing learned breakout entry distance".into());
    };
    let qualifies = input.within_learned_breakout_entry_distance(entry_distance)
        && input.daily_features.is_some_and(|features| features.btc_gate_passes());
    if signal.strategy_candidate
        && input.strategy == Some(Strategy::Donchian20Btc200Hold3)
        && qualifies
    {
        Ok(StrategyDecision::Enter)
    } else if signal.strategy_candidate
        && input.strategy == Some(Strategy::Donchian20Btc200Hold3)
        && input.within_scout_breakout_buffer()
        && input.daily_features.is_some_and(|features| features.btc_gate_passes())
    {
        Ok(StrategyDecision::Scout)
    } else {
        Ok(StrategyDecision::Reject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_trader::{Signal, SignalComponents};
    use std::collections::BTreeMap;

    fn signal(strategy: &str, candidate: bool) -> Signal {
        Signal {
            symbol: "BTC-USDT".into(),
            direction: "bullish".into(),
            strength: 1.0,
            timestamp: 1,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: candidate,
            strategy: Some(strategy.into()),
            strategy_decision: Some(serde_json::json!({
                "observed_close": 120.0,
                "primary_level": 100.0,
                "secondary_level": 100.0,
                "btc_gate_required": true,
                "btc_close": 120.0,
                "btc_sma": 100.0,
                "primary_metric_value": 10.0,
                "strategy_order_size_multiplier": 0.5,
                "monthly_anchor_close_time_ms": 1,
                "cross_sectional_metric_value": 10.0,
                "cross_sectional_threshold": 5.0
            })),
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    #[test]
    fn accepts_only_its_own_published_candidate_signal() {
        let parameters = BTreeMap::from([("breakout_entry_distance_percent".to_owned(), 0.0)]);
        let own = signal("donchian-20-btc200-hold3", true);
        let wrong = signal("another-strategy", true);
        let inactive = signal("donchian-20-btc200-hold3", false);
        assert_eq!(
            evaluate(&StrategySignal::entry(&own, &parameters)).unwrap(),
            StrategyDecision::Enter
        );
        assert_eq!(
            evaluate(&StrategySignal::entry(&wrong, &parameters)).unwrap(),
            StrategyDecision::Reject
        );
        assert_eq!(
            evaluate(&StrategySignal::entry(&inactive, &parameters)).unwrap(),
            StrategyDecision::Reject
        );
        let mut failed_rule = own.clone();
        let features = failed_rule.strategy_decision.as_mut().unwrap();
        features["observed_close"] = serde_json::json!(89.9);
        assert_eq!(
            evaluate(&StrategySignal::entry(&failed_rule, &parameters)).unwrap(),
            StrategyDecision::Reject
        );

        let invalidated = StrategySignal::position(&parameters, true, Some(false), false);
        let still_active = StrategySignal::position(&parameters, true, Some(true), false);
        let disabled = StrategySignal::position(&parameters, false, None, false);
        assert_eq!(evaluate(&invalidated).unwrap(), StrategyDecision::Exit);
        assert_eq!(evaluate(&still_active).unwrap(), StrategyDecision::Hold);
        assert_eq!(evaluate(&disabled).unwrap(), StrategyDecision::Exit);
    }
}

pub(super) fn should_exit_when_insight_inactive(active: bool) -> bool {
    !active
}
pub(super) fn should_exit_when_disabled(enabled: bool) -> bool {
    !enabled
}
pub(crate) fn uses_protective_stops() -> bool {
    false
}
pub(super) fn description() -> &'static str {
    "exit when the 20-day breakout or BTC SMA200 regime becomes inactive, after the minimum 72-hour hold"
}

#[cfg(test)]
mod parameter_bounds_donchian_20_btc200_hold3 {
    use crate::auto_trader::{
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
    fn parameter_bounds_donchian_20_btc200_hold3() {
        let strategy = Strategy::Donchian20Btc200Hold3;
        let specs = all_strategy_parameter_specs(strategy);
        assert!(!specs.is_empty(), "strategy must define parameters");
        for spec in &specs {
            assert_bounds(spec, strategy);
        }
    }
}
