use super::super::{Strategy, StrategyContract, StrategyDefinition};
pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "live absorption insight",
        entry: "pressure-without-price-move points against the signal direction with magnitude at least 0.2",
        exit: super::exit::description(),
        entry_order: "configured AutoOrderType (default IOC); VWAP/TWAP use configured duration and participation",
        exit_order: "StopMarket protective order; IOC close order for strategy and take-profit exits",
        behavior: "directional entries supported by observed absorption pressure",
    }
}

pub(in crate::auto_trader) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::LiveAbsorptionPressure,
    name: "live-absorption-pressure",
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
