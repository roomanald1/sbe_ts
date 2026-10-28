use super::super::{Strategy, StrategyContract, StrategyDefinition};

pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "closed daily candles; monthly anchor from the prior month's final close",
        entry: "BTC above SMA200; select the eligible positive 90-day return leader by 90-day return / 60-day realized volatility",
        exit: super::exit::description(),
        entry_order: "configured AutoOrderType (default IOC), scaled by the 40% volatility target",
        exit_order: "signal-driven monthly rotation; fixed stop and take-profit disabled",
        behavior: "long-only, one selected spot asset or cash; 7-symbol universe",
    }
}

pub(in crate::auto_trader) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::RiskAdjustedMonthlyMomentum,
    name: "risk-adjusted-monthly-momentum",
    live_insight: false,
    live_insight_fields: &[],
    legacy_composite: false,
    expands_to: &[],
    evaluate: super::super::insights_only_evaluate,
    contract,
    should_exit_when_disabled: super::exit::should_exit_when_disabled,
    should_exit_when_insight_inactive: Some(super::exit::should_exit_when_insight_inactive),
    parameter_specs: super::parameters::parameters,
};
