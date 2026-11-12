use super::super::{param, StrategyParameterSpec};
use crate::StrategyParameterKey;

pub(super) fn parameters() -> Vec<StrategyParameterSpec> {
    let mut specs = vec![param(
        StrategyParameterKey::MinimumAbsorptionPressure,
        "Minimum opposing absorption pressure",
        0.2,
        0.02,
        0.9,
        0.01,
        "ratio",
        -1.0,
    )];
    specs.extend([
        param(StrategyParameterKey::TradeFlowWindowSeconds, "Trade-flow fallback window", 60.0, 10.0, 120.0, 10.0, "seconds", 0.0),
        param(StrategyParameterKey::MomentumLookbackSeconds, "Price-movement lookback", 10.0, 5.0, 30.0, 5.0, "seconds", 0.0),
        param(StrategyParameterKey::AbsorptionLookbackSeconds, "Absorption lookback", 30.0, 10.0, 60.0, 10.0, "seconds", 0.0),
        param(StrategyParameterKey::AbsorptionReplenishmentSeconds, "Replenishment window", 2.0, 1.0, 5.0, 1.0, "seconds", 0.0),
        param(StrategyParameterKey::AbsorptionTouchToleranceBps, "Touch tolerance", 2.0, 1.0, 5.0, 1.0, "bps", 0.0),
        param(StrategyParameterKey::AbsorptionMaximumPriceMovePercent, "Maximum price movement", 0.025, 0.005, 0.100, 0.005, "%", -1.0),
        param(StrategyParameterKey::AbsorptionMinimumFlowImbalance, "Minimum flow without price movement", 0.55, 0.20, 0.90, 0.05, "ratio", -1.0),
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
