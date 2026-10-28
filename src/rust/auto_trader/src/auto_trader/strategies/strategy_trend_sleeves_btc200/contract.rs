use super::super::{Strategy, StrategyContract, StrategyDefinition};
pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "daily candle insight",
        entry: "BTC/ETH above own SMA50 or BNB above own SMA100, with BTC above SMA200",
        exit: super::exit::description(),
        entry_order: "configured AutoOrderType (default IOC); VWAP/TWAP use configured duration and participation",
        exit_order: "StopMarket protective order; IOC close order for strategy and take-profit exits",
        behavior: "long-only BTC/ETH/BNB sleeves; other symbols are ineligible",
    }
}

pub(in crate::auto_trader) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::TrendSleevesBtc200,
    name: "trend-sleeves-btc200",
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
