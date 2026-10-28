use super::super::{StrategyParameterSpec, param};

pub(super) fn parameters() -> Vec<StrategyParameterSpec> {
    let mut specs = vec![
        param(
            "breakout_lookback_days",
            "Breakout lookback",
            55.0,
            10.0,
            120.0,
            1.0,
            "days",
            1.0,
        ),
        param(
            "btc_trend_days",
            "BTC regime SMA",
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
