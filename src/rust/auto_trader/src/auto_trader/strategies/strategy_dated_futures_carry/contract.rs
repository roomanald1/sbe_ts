use super::super::{Strategy, StrategyContract, StrategyDefinition};

fn contract() -> StrategyContract {
    StrategyContract {
        insight: "Insights publishes a paired dated-futures carry signal",
        entry: "executable future bid exceeds spot ask by the cost budget near expiry",
        exit: super::evaluate::description(),
        entry_order: "paired paper spot buy and dated-future short",
        exit_order: "paper spot TWAP and future delivery",
        behavior: "signal-driven, paired paper carry strategy",
    }
}

pub(in crate::auto_trader) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::DatedCarry,
    name: "dated-futures-carry",
    live_insight: false,
    live_insight_fields: &[],
    legacy_composite: false,
    expands_to: &[],
    evaluate: super::evaluate::evaluate,
    contract,
    should_exit_when_disabled: super::evaluate::should_exit_when_disabled,
    should_exit_when_insight_inactive: None,
    parameter_specs: super::super::no_strategy_parameters,
};
