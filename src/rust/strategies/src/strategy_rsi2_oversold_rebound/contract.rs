use super::super::{Strategy, StrategyContract, StrategyDefinition};
pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "daily candle insight",
        entry: "two-day RSI(2) at or below 10 and close above own SMA200",
        exit: super::evaluate::description(),
        entry_order: "passive PostOnlyLimit by default; IOC is reserved for high urgency; VWAP/TWAP use configured duration and participation",
        exit_order: "StopMarket protective order; IOC close order for strategy and take-profit exits",
        behavior: "long-only oversold rebound with five-day maximum hold",
    }
}

pub(in crate) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::Rsi2OversoldRebound,
    name: "rsi2-oversold-rebound",
    live_insight: false,
    live_insight_fields: &[],
    legacy_composite: false,
    expands_to: &[],
    evaluate: super::evaluate::evaluate,
    contract,
    should_exit_when_disabled: super::evaluate::should_exit_when_disabled,
    should_exit_when_insight_inactive: Some(super::evaluate::should_exit_when_insight_inactive),
    parameter_specs: super::parameters::parameters,
};
