use super::super::{param, StrategyParameterSpec, StrategyParameterKey as K};

pub(super) fn parameters() -> Vec<StrategyParameterSpec> {
    vec![
        param(K::TrendPortfolioLookbackDays, "Trend return lookback", 160.0, 40.0, 240.0, 10.0, "days", 1.0),
        param(K::TrendPortfolioBtcRegimeDays, "BTC regime SMA", 200.0, 100.0, 300.0, 10.0, "days", 1.0),
        param(K::TrendPortfolioVolatilityDays, "Volatility lookback", 30.0, 10.0, 90.0, 5.0, "days", 1.0),
        param(K::TrendPortfolioWeightCap, "Per asset weight cap", 0.25, 0.10, 0.50, 0.05, "weight", 1.0),
        param(K::TrendPortfolioExitWeightFloor, "Early exit trend-sizing floor", 0.05, 0.0, 0.50, 0.01, "multiplier", -1.0),
    ]
}
