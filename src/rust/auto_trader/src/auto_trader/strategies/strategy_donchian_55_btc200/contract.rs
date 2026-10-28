use super::super::{Strategy, StrategyContract, StrategyDefinition};
pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "daily candle insight",
        entry: "close above prior 55-day high while BTC is above SMA200",
        exit: super::exit::description(),
        entry_order: "configured AutoOrderType (default IOC); VWAP/TWAP use configured duration and participation",
        exit_order: "StopMarket protective order; IOC close order for strategy and take-profit exits",
        behavior: "long-only breakout; BTC regime gates eligible assets",
    }
}

pub(in crate::auto_trader) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::Donchian55Btc200,
    name: "donchian-55-btc200",
    live_insight: false,
    live_insight_fields: &[],
    legacy_composite: false,
    expands_to: &[],
    evaluate: super::super::insights_only_evaluate,
    contract,
    should_exit_when_disabled: super::exit::should_exit_when_disabled,
    should_exit_when_insight_inactive: Some(super::exit::should_exit_when_insight_inactive),
    parameter_specs: super::parameters::parameters,
};
