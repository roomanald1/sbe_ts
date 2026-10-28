use super::super::{Strategy, StrategyContract, StrategyDefinition};
pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "legacy live composite market insight",
        entry: "legacy stored selection; migrates to the enabled live component strategies",
        exit: super::exit::description(),
        entry_order: "configured AutoOrderType (default IOC); VWAP/TWAP use configured duration and participation",
        exit_order: "StopMarket protective order; IOC close order for strategy and take-profit exits",
        behavior: "legacy configuration compatibility",
    }
}

pub(in crate::auto_trader) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::LiveInsights,
    name: "live-insights",
    live_insight: true,
    live_insight_fields: super::data::SUBSCRIPTIONS,
    legacy_composite: true,
    expands_to: &[
        Strategy::LiveMomentum,
        Strategy::LiveTradeFlow,
        Strategy::LiveOrderBook,
        Strategy::LiveVwapDislocation,
        Strategy::LiveAbsorptionPressure,
    ],
    evaluate: super::entry::evaluate,
    contract,
    should_exit_when_disabled: super::exit::should_exit_when_disabled,
    should_exit_when_insight_inactive: None,
    parameter_specs: super::parameters::parameters,
};
