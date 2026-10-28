use super::super::{Strategy, StrategyContract, StrategyDefinition};
pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "live order-book insight",
        entry: "signed book imbalance agrees with signal direction and magnitude is at least 0.15; wait for 10-second momentum to pull back against the signal",
        exit: super::exit::description(),
        entry_order: "configured AutoOrderType (default IOC); VWAP/TWAP use configured duration and participation",
        exit_order: "StopMarket protective order; IOC close order for strategy and take-profit exits",
        behavior: "directional entries supported by displayed bid/ask depth imbalance",
    }
}

pub(in crate::auto_trader) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::LiveOrderBook,
    name: "live-order-book",
    live_insight: true,
    live_insight_fields: super::data::SUBSCRIPTIONS,
    legacy_composite: false,
    expands_to: &[],
    evaluate: super::entry::evaluate,
    contract,
    should_exit_when_disabled: super::exit::should_exit_when_disabled,
    should_exit_when_insight_inactive: None,
    parameter_specs: super::parameters::parameters,
};
