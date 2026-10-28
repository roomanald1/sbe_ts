fn strategy_parameter_specs(strategy: Strategy) -> Vec<StrategyParameterSpec> {
    strategy.parameter_specs()
}

const fn param(
    key: StrategyParameterKey,
    label: &'static str,
    default: f64,
    min: f64,
    max: f64,
    step: f64,
    unit: &'static str,
    learning_direction: f64,
) -> StrategyParameterSpec {
    StrategyParameterSpec {
        key,
        label,
        default,
        min,
        max,
        step,
        unit,
        learning_direction,
    }
}

fn all_strategy_parameter_specs(strategy: Strategy) -> Vec<StrategyParameterSpec> {
    let mut specs = strategy_parameter_specs(strategy);
    specs.push(param(
        StrategyParameterKey::LearningRate,
        "Outcome adaptation weight",
        0.05,
        0.0,
        0.25,
        0.01,
        "weight",
        0.0,
    ));
    specs.extend([
        param(
            StrategyParameterKey::ProfitProtectionActivationPercent,
            "Profit protection activation",
            DEFAULT_PROFIT_PROTECTION_ACTIVATION_PERCENT,
            0.10,
            0.50,
            0.05,
            "% net",
            1.0,
        ),
        param(
            StrategyParameterKey::ProfitProtectionGivebackPercent,
            "Profit protection giveback",
            DEFAULT_PROFIT_PROTECTION_GIVEBACK_PERCENT,
            0.05,
            0.30,
            0.05,
            "percentage points",
            1.0,
        ),
        param(
            StrategyParameterKey::ProfitProtectionMinimumLockPercent,
            "Minimum profit lock",
            DEFAULT_PROFIT_PROTECTION_MINIMUM_LOCK_PERCENT,
            0.0,
            0.20,
            0.05,
            "% net",
            -1.0,
        ),
    ]);
    specs
}
