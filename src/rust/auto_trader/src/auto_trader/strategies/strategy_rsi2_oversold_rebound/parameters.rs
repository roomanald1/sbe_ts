use super::super::{StrategyParameterSpec, param};

pub(super) fn parameters() -> Vec<StrategyParameterSpec> {
    let mut specs = vec![
        param(
            "rsi_period_days",
            "RSI period",
            2.0,
            2.0,
            10.0,
            1.0,
            "days",
            1.0,
        ),
        param(
            "rsi_entry_max",
            "RSI entry ceiling",
            10.0,
            2.0,
            30.0,
            1.0,
            "RSI",
            -1.0,
        ),
        param(
            "rsi_exit_level",
            "RSI exit level",
            70.0,
            40.0,
            95.0,
            1.0,
            "RSI",
            1.0,
        ),
        param(
            "trend_sma_days",
            "Asset trend SMA",
            200.0,
            50.0,
            200.0,
            5.0,
            "days",
            1.0,
        ),
        param(
            "maximum_hold_days",
            "Maximum hold",
            5.0,
            1.0,
            15.0,
            1.0,
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
