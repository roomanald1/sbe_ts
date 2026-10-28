use super::super::{StrategyParameterSpec, param};

pub(super) fn parameters() -> Vec<StrategyParameterSpec> {
    let mut specs = vec![
        param(
            "fast_sma_days",
            "Fast trend SMA",
            50.0,
            10.0,
            150.0,
            1.0,
            "days",
            1.0,
        ),
        param(
            "slow_sma_days",
            "Slow trend SMA",
            200.0,
            50.0,
            200.0,
            5.0,
            "days",
            1.0,
        ),
    ];
    specs.extend([
        param("stop_loss_percent", "Protective stop distance", 0.50, 0.10, 1.50, 0.05, "%", -1.0),
        param("take_profit_percent", "Protective target distance", 0.80, 0.20, 3.00, 0.05, "%", 1.0),
    ]);
    specs
}
