use super::super::{StrategyParameterSpec, param};

pub(super) fn parameters() -> Vec<StrategyParameterSpec> {
    let mut specs = vec![];
    specs.extend([
        param("stop_loss_percent", "Protective stop distance", 0.50, 0.10, 1.50, 0.05, "%", -1.0),
        param("take_profit_percent", "Protective target distance", 0.80, 0.20, 3.00, 0.05, "%", 1.0),
    ]);
    specs
}
