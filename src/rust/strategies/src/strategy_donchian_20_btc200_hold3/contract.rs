use super::super::{Strategy, StrategyContract, StrategyDefinition};

pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "closed daily candles",
        entry: "close above prior 20-day high while BTC closes above SMA200",
        exit: super::evaluate::description(),
        entry_order:
            "configured paper order; size multiplier capped at 0.25 of the normal per-order cap",
        exit_order:
            "daily signal exit after a minimum 72-hour hold; fixed stop and target disabled",
        behavior: "long-only 20-day BTC-gated breakout with a three-day minimum hold",
    }
}

pub(in crate) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::Donchian20Btc200Hold3,
    name: "donchian-20-btc200-hold3",
    live_insight: false,
    live_insight_fields: &[],
    legacy_composite: false,
    expands_to: &[],
    evaluate: super::evaluate::evaluate,
    contract,
    should_exit_when_disabled: super::evaluate::should_exit_when_disabled,
    should_exit_when_insight_inactive: Some(super::evaluate::should_exit_when_insight_inactive),
    parameter_specs: super::super::no_strategy_parameters,
};
