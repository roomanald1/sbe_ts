use super::super::{Strategy, StrategyContract, StrategyDefinition};

pub(super) fn contract() -> StrategyContract {
    StrategyContract {
        insight: "closed daily candles across 16 Binance USDT pairs",
        entry: "BTC above SMA200 and positive average 10/20/40/80/160-day return",
        exit: super::evaluate::description(),
        entry_order: "paper orders adjust each open asset toward its inverse-volatility target weight when drift exceeds 5%",
        exit_order: "daily signal exit; fixed stop and target disabled",
        behavior: "long-only multi-asset trend portfolio; capped 25% target weight per asset; daily target rebalancing",
    }
}

pub(in crate::auto_trader) const DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::RiskAdjustedTrendPortfolio,
    name: "risk-adjusted-trend-portfolio",
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
