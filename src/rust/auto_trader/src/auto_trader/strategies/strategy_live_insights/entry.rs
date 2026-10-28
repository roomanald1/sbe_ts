use crate::auto_trader::{StrategyEvaluation, StrategyInsight};
pub(crate) fn evaluate(
    insight: &StrategyInsight<'_>,
) -> Result<Option<StrategyEvaluation>, String> {
    let StrategyInsight::LiveSignal { .. } = insight;
    Ok(Some(StrategyEvaluation::LiveSignal))
}
