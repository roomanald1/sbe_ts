use super::super::{Strategy, StrategyContract, StrategyDefinition};

pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "high-confidence five-minute range with current price near a range edge",
        entry: "fade the range edge when range probability is at least 0.70 and price is within 12% of that edge",
        exit: super::evaluate::description(),
        entry_order: "passive PostOnlyLimit by default; IOC is reserved for high urgency; VWAP/TWAP use configured duration and participation",
        exit_order: "StopMarket protective order; IOC close order for strategy and take-profit exits",
        behavior: "contrarian entries at range extremes; no entries from the middle of a range",
    }
}

pub(crate) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::LiveRangeMeanReversion,
    name: "live-range-mean-reversion",
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
