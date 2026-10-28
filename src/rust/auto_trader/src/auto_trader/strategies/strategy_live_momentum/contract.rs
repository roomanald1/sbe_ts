use super::super::{Strategy, StrategyContract, StrategyDefinition};
pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "live momentum insight",
        entry: "direction agrees with nonzero 10-second momentum or 5-minute trend evidence; then shared walk-forward and risk gates apply",
        exit: super::exit::description(),
        entry_order: "configured AutoOrderType (default IOC); VWAP/TWAP use configured duration and participation",
        exit_order: "StopMarket protective order; IOC close order for strategy and take-profit exits",
        behavior: "directional continuation from short-horizon trend and momentum",
    }
}

pub(in crate::auto_trader) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::LiveMomentum,
    name: "live-momentum",
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
