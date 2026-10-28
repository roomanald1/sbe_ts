use super::super::{param, StrategyParameterSpec};
use crate::auto_trader::StrategyParameterKey;

pub(super) fn parameters() -> Vec<StrategyParameterSpec> {
    let mut specs = vec![];
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
