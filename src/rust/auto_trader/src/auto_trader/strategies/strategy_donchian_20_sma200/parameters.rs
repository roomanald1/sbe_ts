use super::super::{param, StrategyParameterSpec};
use crate::auto_trader::StrategyParameterKey;

pub(super) fn parameters() -> Vec<StrategyParameterSpec> {
    let mut specs = vec![
        param(
            StrategyParameterKey::BreakoutLookbackDays,
            "Breakout lookback",
            20.0,
            5.0,
            90.0,
            1.0,
            "days",
            1.0,
        ),
        param(
            StrategyParameterKey::TrendSmaDays,
            "Asset trend SMA",
            200.0,
            50.0,
            200.0,
            5.0,
            "days",
            1.0,
        ),
    ];
    specs.extend([
        param(
            StrategyParameterKey::StopLossPercent,
            "Protective stop distance",
            0.50,
            0.10,
            1.50,
            0.05,
            "%",
            -1.0,
        ),
        param(
            StrategyParameterKey::TakeProfitPercent,
            "Protective target distance",
            0.80,
            0.20,
            3.00,
            0.05,
            "%",
            1.0,
        ),
    ]);
    specs
}
