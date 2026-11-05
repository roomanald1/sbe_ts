use super::*;
const DEFAULT_PROFIT_PROTECTION_ACTIVATION_PERCENT: f64 = 0.15;
const DEFAULT_PROFIT_PROTECTION_GIVEBACK_PERCENT: f64 = 0.10;
const DEFAULT_PROFIT_PROTECTION_MINIMUM_LOCK_PERCENT: f64 = 0.05;

fn registered_parameter_specs(strategy: Strategy) -> Vec<StrategyParameterSpec> {
    (strategy.definition().parameter_specs)()
}

const fn catalog_param(
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

fn strategy_rule_parameter_specs(strategy: Strategy) -> Vec<StrategyParameterSpec> {
    use StrategyParameterKey as K;
    let mut common_risk = Vec::new();
    if strategy != Strategy::DatedCarry && strategy.uses_protective_stops() {
        common_risk.push(catalog_param(
            K::StopLossPercent,
            "Protective stop distance",
            0.50,
            0.10,
            1.50,
            0.05,
            "%",
            -1.0,
        ));
    }
    if strategy != Strategy::DatedCarry && strategy.uses_fixed_take_profit() {
        common_risk.push(catalog_param(
            K::TakeProfitPercent,
            "Protective target distance",
            0.80,
            0.20,
            3.00,
            0.05,
            "%",
            1.0,
        ));
    }
    let mut specs = match strategy {
        Strategy::Donchian10Btc200 => vec![
            catalog_param(
                K::BreakoutLookbackDays,
                "Breakout lookback",
                10.0,
                5.0,
                40.0,
                1.0,
                "days",
                1.0,
            ),
            catalog_param(
                K::BtcTrendDays,
                "BTC regime SMA",
                200.0,
                100.0,
                300.0,
                10.0,
                "days",
                1.0,
            ),
        ],
        Strategy::Donchian20Btc200 => vec![
            catalog_param(
                K::BreakoutLookbackDays,
                "Breakout lookback",
                20.0,
                5.0,
                60.0,
                1.0,
                "days",
                1.0,
            ),
            catalog_param(
                K::BtcTrendDays,
                "BTC regime SMA",
                200.0,
                100.0,
                300.0,
                10.0,
                "days",
                1.0,
            ),
        ],
        Strategy::Donchian20Btc200Hold3 => vec![
            catalog_param(
                K::BreakoutLookbackDays,
                "Breakout lookback",
                20.0,
                5.0,
                60.0,
                1.0,
                "days",
                1.0,
            ),
            catalog_param(
                K::BtcTrendDays,
                "BTC regime SMA",
                200.0,
                100.0,
                300.0,
                10.0,
                "days",
                1.0,
            ),
            catalog_param(
                K::MinimumHoldDays,
                "Minimum hold",
                3.0,
                1.0,
                10.0,
                1.0,
                "days",
                1.0,
            ),
        ],
        Strategy::RiskAdjustedTrendPortfolio => vec![
            catalog_param(
                K::TrendPortfolioLookbackDays,
                "Trend return lookback",
                160.0,
                40.0,
                240.0,
                10.0,
                "days",
                1.0,
            ),
            catalog_param(
                K::TrendPortfolioBtcRegimeDays,
                "BTC regime SMA",
                200.0,
                100.0,
                300.0,
                10.0,
                "days",
                1.0,
            ),
            catalog_param(
                K::TrendPortfolioVolatilityDays,
                "Volatility lookback",
                30.0,
                10.0,
                90.0,
                5.0,
                "days",
                1.0,
            ),
            catalog_param(
                K::TrendPortfolioWeightCap,
                "Per asset weight cap",
                0.25,
                0.10,
                0.50,
                0.05,
                "weight",
                1.0,
            ),
            catalog_param(
                K::TrendPortfolioExitWeightFloor,
                "Early exit trend-sizing floor",
                0.05,
                0.0,
                0.50,
                0.01,
                "multiplier",
                -1.0,
            ),
        ],
        Strategy::RiskAdjustedMonthlyMomentum => vec![
            catalog_param(
                K::MonthlyMomentumLookbackDays,
                "Momentum lookback",
                90.0,
                30.0,
                180.0,
                5.0,
                "days",
                1.0,
            ),
            catalog_param(
                K::MonthlyMomentumVolatilityDays,
                "Volatility lookback",
                60.0,
                20.0,
                120.0,
                5.0,
                "days",
                1.0,
            ),
            catalog_param(
                K::MonthlyMomentumVolatilityTarget,
                "Annualized volatility target",
                0.40,
                0.10,
                0.80,
                0.05,
                "annualized",
                1.0,
            ),
            catalog_param(
                K::TrendSmaDays,
                "Trend SMA",
                100.0,
                50.0,
                250.0,
                5.0,
                "days",
                1.0,
            ),
            catalog_param(
                K::BtcRegimeSmaDays,
                "BTC regime SMA",
                200.0,
                100.0,
                300.0,
                10.0,
                "days",
                1.0,
            ),
        ],
        Strategy::DatedCarry => vec![
            catalog_param(
                K::CarryMinimumDaysToExpiry,
                "Minimum days to expiry",
                7.0,
                2.0,
                30.0,
                1.0,
                "days",
                -1.0,
            ),
            catalog_param(
                K::CarryCostGatePercent,
                "Minimum net carry edge",
                0.50,
                0.10,
                1.50,
                0.05,
                "%",
                -1.0,
            ),
        ],
        _ => Vec::new(),
    };
    if strategy.is_live_insight() && !strategy.is_legacy_composite() {
        specs.extend([
            catalog_param(
                K::ScoutMinimumWalkForwardSamples,
                "Scout minimum negative-evidence samples",
                10.0,
                3.0,
                30.0,
                1.0,
                "samples",
                -1.0,
            ),
            catalog_param(
                K::ScoutMinimumPositiveNetRate,
                "Scout minimum positive net rate",
                0.45,
                0.20,
                0.65,
                0.05,
                "rate",
                -1.0,
            ),
        ]);
    }
    if matches!(
        strategy,
        Strategy::LiveOrderBook | Strategy::LiveVwapDislocation
    ) {
        specs.push(catalog_param(
            K::MinimumCounterMomentumPercent,
            "Minimum counter-momentum for pullback confirmation",
            0.0,
            0.0,
            0.02,
            0.001,
            "%",
            -1.0,
        ));
    }
    if matches!(
        strategy,
        Strategy::Donchian10Btc200
            | Strategy::Donchian20Btc200
            | Strategy::Donchian20Btc200Hold3
            | Strategy::Donchian20Sma200
            | Strategy::Donchian55Btc200
    ) {
        specs.push(catalog_param(
            K::BreakoutEntryDistancePercent,
            "Entry distance below breakout",
            0.0,
            0.0,
            10.0,
            0.25,
            "%",
            1.0,
        ));
        specs.push(catalog_param(
            K::ScoutBreakoutBufferPercent,
            "Scout breakout buffer",
            10.0,
            0.0,
            20.0,
            0.5,
            "%",
            0.0,
        ));
    }
    let existing = specs.iter().map(|spec| spec.key).collect::<Vec<_>>();
    for spec in common_risk {
        if !existing.contains(&spec.key) {
            specs.push(spec);
        }
    }
    specs.push(catalog_param(
        StrategyParameterKey::PostOnlyTimeoutMinutes,
        "Post-only entry timeout",
        5.0,
        1.0,
        15.0,
        1.0,
        "minutes",
        0.0,
    ));
    specs
}

pub(super) fn complete_parameter_specs(strategy: Strategy) -> Vec<StrategyParameterSpec> {
    let mut specs = registered_parameter_specs(strategy);
    for spec in strategy_rule_parameter_specs(strategy) {
        if !specs.iter().any(|registered| registered.key == spec.key) {
            specs.push(spec);
        }
    }
    specs.push(catalog_param(
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
        catalog_param(
            StrategyParameterKey::ProfitProtectionActivationPercent,
            "Profit protection activation",
            DEFAULT_PROFIT_PROTECTION_ACTIVATION_PERCENT,
            0.10,
            0.50,
            0.05,
            "% net",
            1.0,
        ),
        catalog_param(
            StrategyParameterKey::ProfitProtectionGivebackPercent,
            "Profit protection giveback",
            DEFAULT_PROFIT_PROTECTION_GIVEBACK_PERCENT,
            0.05,
            0.30,
            0.05,
            "percentage points",
            1.0,
        ),
        catalog_param(
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
