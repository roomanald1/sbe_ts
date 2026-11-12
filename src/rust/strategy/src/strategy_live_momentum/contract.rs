use super::super::{Strategy, StrategyContract, StrategyDefinition};
pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "live momentum insight",
        entry: "direction follows learned momentum and trend lookbacks; their minimum aligned values, lookbacks, and exits are bounded learned parameters",
        exit: super::evaluate::description(),
        entry_order: "passive PostOnlyLimit by default; IOC is reserved for high urgency; VWAP/TWAP use configured duration and participation",
        exit_order: "StopMarket protective order; IOC close order for strategy and take-profit exits",
        behavior: "directional continuation from short-horizon trend and momentum",
    }
}

pub(in crate) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::LiveMomentum,
    name: "live-momentum",
    live_insight: true,
    live_insight_fields: super::data::SUBSCRIPTIONS,
    legacy_composite: false,
    expands_to: &[],
    evaluate: super::evaluate::evaluate,
    contract,
    should_exit_when_disabled: super::evaluate::should_exit_when_disabled,
    should_exit_when_insight_inactive: None,
    parameter_specs: super::parameters::parameters,
};
