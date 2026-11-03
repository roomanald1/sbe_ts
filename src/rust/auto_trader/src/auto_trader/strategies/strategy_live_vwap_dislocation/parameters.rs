use super::super::{param, StrategyParameterSpec};
use crate::auto_trader::StrategyParameterKey;

pub(super) fn parameters() -> Vec<StrategyParameterSpec> {
    let mut specs = vec![param(
        StrategyParameterKey::MinimumVwapDislocationBps,
        "Minimum VWAP dislocation",
        1.0,
        0.1,
        20.0,
        0.1,
        "bps",
        -1.0,
    )];
    specs.extend([
        param(StrategyParameterKey::VwapLookbackSeconds, "VWAP lookback", 60.0, 10.0, 300.0, 10.0, "seconds", 0.0),
        param(StrategyParameterKey::VwapMinimumTradeCount, "Minimum VWAP trades", 5.0, 3.0, 30.0, 1.0, "trades", -1.0),
        param(StrategyParameterKey::VwapMinimumTradeSpanSeconds, "Minimum VWAP trade span", 10.0, 5.0, 60.0, 5.0, "seconds", -1.0),
        param(StrategyParameterKey::MomentumLookbackSeconds, "Pullback momentum lookback", 10.0, 5.0, 30.0, 5.0, "seconds", 0.0),
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
