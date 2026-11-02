use super::super::{Strategy, StrategyContract, StrategyDefinition};

pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "closed daily candles",
        entry: "close above prior 10-day high while BTC closes above SMA200",
        exit: super::evaluate::description(),
        entry_order: "passive PostOnlyLimit by default; IOC is reserved for high urgency; daily strategy budget and account limits apply",
        exit_order: "IOC close when the daily breakout or BTC regime becomes inactive",
        behavior: "long-only 10-day breakout; fixed BTC 200-day regime gate; paper signal exit",
    }
}

pub(in crate::auto_trader) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::Donchian10Btc200,
    name: "donchian-10-btc200",
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
