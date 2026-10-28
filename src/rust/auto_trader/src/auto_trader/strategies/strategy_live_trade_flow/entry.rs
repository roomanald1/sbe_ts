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
    if signal.components.trade_flow_imbalance.is_some_and(|value| {
        side * value >= strategy_parameter_value(parameters, "minimum_flow_imbalance", 0.15)
    }) {
        Ok(Some(StrategyEvaluation::LiveSignal))
    } else {
        Ok(None)
    }
}
