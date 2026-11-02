use super::super::{Strategy, StrategyContract, StrategyDefinition};
pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "live VWAP dislocation insight",
        entry: "signed VWAP dislocation agrees with signal direction and magnitude is at least 1 bp; wait for 10-second momentum to pull back against the signal",
        exit: super::evaluate::description(),
        entry_order: "passive PostOnlyLimit by default; IOC is reserved for high urgency; VWAP/TWAP use configured duration and participation",
        exit_order: "StopMarket protective order; IOC close order for strategy and take-profit exits",
        behavior: "directional entries supported by price displacement from VWAP",
    }
}

pub(in crate::auto_trader) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::LiveVwapDislocation,
    name: "live-vwap-dislocation",
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
