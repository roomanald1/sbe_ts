use super::super::{param, StrategyParameterSpec};
use crate::StrategyParameterKey;

pub(super) fn parameters() -> Vec<StrategyParameterSpec> {
    let mut specs = vec![param(
        StrategyParameterKey::MinimumMomentum,
        "Minimum aligned momentum",
        0.0,
        0.0,
        0.02,
        0.001,
        "percent",
        -1.0,
    )];
    specs.extend([
        param(StrategyParameterKey::MomentumLookbackSeconds, "Momentum lookback", 10.0, 5.0, 30.0, 5.0, "seconds", 0.0),
        param(StrategyParameterKey::TrendLookbackSeconds, "Trend lookback", 300.0, 60.0, 900.0, 60.0, "seconds", 0.0),
    ]);
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
