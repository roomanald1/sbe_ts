use super::super::{StrategyParameterSpec, param};

pub(super) fn parameters() -> Vec<StrategyParameterSpec> {
    let mut specs = vec![param(
        "minimum_vwap_dislocation_bps",
        "Minimum VWAP dislocation",
        1.0,
        0.1,
        20.0,
        0.1,
        "bps",
        -1.0,
    )];
    specs.extend([
        param("stop_loss_percent", "Protective stop distance", 0.50, 0.10, 1.50, 0.05, "%", -1.0),
        param("take_profit_percent", "Protective target distance", 0.80, 0.20, 3.00, 0.05, "%", 1.0),
    ]);
    specs
}
