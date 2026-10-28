use crate::auto_trader::{StrategyEvaluation, StrategyInsight, strategy_parameter_value};
pub(crate) fn evaluate(
    insight: &StrategyInsight<'_>,
) -> Result<Option<StrategyEvaluation>, String> {
    let StrategyInsight::LiveSignal { signal, parameters } = insight;
    let side = match signal.direction.to_ascii_uppercase().as_str() {
        "UP" | "LONG" | "BULLISH" => 1.0,
        "DOWN" | "SHORT" | "BEARISH" => -1.0,
        _ => return Ok(None),
    };
    let aligned = |value: Option<f64>| {
        value.is_some_and(|value| {
            value.is_finite()
                && side * value > strategy_parameter_value(parameters, "minimum_momentum", 0.0)
        })
    };
    if aligned(signal.components.momentum_10s_percent)
        || aligned(signal.components.trend_5m_volatility_units)
    {
        Ok(Some(StrategyEvaluation::LiveSignal))
    } else {
        Ok(None)
    }
}
