use crate::{StrategyDecision, StrategyParameterKey, StrategySignal, strategy_parameter_value};

const MINIMUM_RANGE_PROBABILITY: f64 = 0.70;
const RANGE_EDGE_FRACTION: f64 = 0.12;

pub(crate) fn evaluate(input: &StrategySignal<'_>) -> Result<StrategyDecision, String> {
    if let Some(decision) = input.position_decision(!input.strategy_enabled, false) {
        return Ok(decision);
    }
    let Some(signal) = input.signal.as_ref() else {
        return Ok(StrategyDecision::Reject);
    };
    let Some(minimum_strength) = strategy_parameter_value(
        &input.parameters,
        StrategyParameterKey::MinimumSignalStrength,
    ) else {
        return Ok(StrategyDecision::Reject);
    };
    let range_probability = signal.ranging_probability.unwrap_or_default();
    let location = signal.range_location.unwrap_or(f64::NAN);
    let is_edge = (signal.direction == "bullish" && location <= RANGE_EDGE_FRACTION)
        || (signal.direction == "bearish" && location >= 1.0 - RANGE_EDGE_FRACTION);
    Ok(
        if signal.strength.is_finite()
            && signal.strength >= minimum_strength
            && range_probability.is_finite()
            && range_probability >= MINIMUM_RANGE_PROBABILITY
            && location.is_finite()
            && (0.0..=1.0).contains(&location)
            && is_edge
        {
            StrategyDecision::Enter
        } else {
            StrategyDecision::Reject
        },
    )
}

pub(crate) fn entry_rejection_reason(input: &StrategySignal<'_>) -> Option<String> {
    let signal = input.signal.as_ref()?;
    let probability = signal.ranging_probability.unwrap_or_default();
    let location = signal.range_location.unwrap_or(f64::NAN);
    if probability < MINIMUM_RANGE_PROBABILITY {
        return Some(format!(
            "Range probability {probability:.3} is below {MINIMUM_RANGE_PROBABILITY:.3}"
        ));
    }
    if !location.is_finite() || !(0.0..=1.0).contains(&location) {
        return Some("Range location is unavailable or invalid".to_owned());
    }
    if !((signal.direction == "bullish" && location <= RANGE_EDGE_FRACTION)
        || (signal.direction == "bearish" && location >= 1.0 - RANGE_EDGE_FRACTION))
    {
        return Some(format!(
            "Price location {location:.3} is not at the range edge for {} mean reversion",
            signal.direction
        ));
    }
    None
}

pub(super) fn should_exit_when_disabled(enabled: bool) -> bool {
    !enabled
}

pub(super) fn description() -> &'static str {
    "shared StopMarket protective stop and IOC stop/target exits; disabling this strategy closes its attributed positions"
}
