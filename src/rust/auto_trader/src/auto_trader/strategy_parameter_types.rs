#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StrategyParameterKey {
    BnbSmaDays,
    BreakoutLookbackDays,
    BtcEthSmaDays,
    BtcRegimeSmaDays,
    BtcTrendDays,
    FastSmaDays,
    LearningRate,
    MaximumHoldDays,
    MinimumHoldDays,
    MinimumAbsorptionPressure,
    MinimumBookImbalance,
    MinimumFlowImbalance,
    MinimumMomentum,
    MinimumVwapDislocationBps,
    ProfitProtectionActivationPercent,
    ProfitProtectionGivebackPercent,
    ProfitProtectionMinimumLockPercent,
    RelativeStrengthDays,
    RsiEntryMax,
    RsiExitLevel,
    RsiPeriodDays,
    SlowSmaDays,
    StopLossPercent,
    TakeProfitPercent,
    TrendPortfolioLookbackDays,
    TrendPortfolioVolatilityDays,
    TrendPortfolioBtcRegimeDays,
    TrendPortfolioWeightCap,
    MonthlyMomentumLookbackDays,
    MonthlyMomentumVolatilityDays,
    MonthlyMomentumVolatilityTarget,
    CarryMinimumDaysToExpiry,
    CarryCostGatePercent,
    TrendSmaDays,
}

impl StrategyParameterKey {
    const fn as_str(self) -> &'static str {
        match self {
            Self::BnbSmaDays => "bnb_sma_days",
            Self::BreakoutLookbackDays => "breakout_lookback_days",
            Self::BtcEthSmaDays => "btc_eth_sma_days",
            Self::BtcRegimeSmaDays => "btc_regime_sma_days",
            Self::BtcTrendDays => "btc_trend_days",
            Self::FastSmaDays => "fast_sma_days",
            Self::LearningRate => "learning_rate",
            Self::MaximumHoldDays => "maximum_hold_days",
            Self::MinimumHoldDays => "minimum_hold_days",
            Self::MinimumAbsorptionPressure => "minimum_absorption_pressure",
            Self::MinimumBookImbalance => "minimum_book_imbalance",
            Self::MinimumFlowImbalance => "minimum_flow_imbalance",
            Self::MinimumMomentum => "minimum_momentum",
            Self::MinimumVwapDislocationBps => "minimum_vwap_dislocation_bps",
            Self::ProfitProtectionActivationPercent => "profit_protection_activation_percent",
            Self::ProfitProtectionGivebackPercent => "profit_protection_giveback_percent",
            Self::ProfitProtectionMinimumLockPercent => "profit_protection_minimum_lock_percent",
            Self::RelativeStrengthDays => "relative_strength_days",
            Self::RsiEntryMax => "rsi_entry_max",
            Self::RsiExitLevel => "rsi_exit_level",
            Self::RsiPeriodDays => "rsi_period_days",
            Self::SlowSmaDays => "slow_sma_days",
            Self::StopLossPercent => "stop_loss_percent",
            Self::TakeProfitPercent => "take_profit_percent",
            Self::TrendPortfolioLookbackDays => "trend_portfolio_lookback_days",
            Self::TrendPortfolioVolatilityDays => "trend_portfolio_volatility_days",
            Self::TrendPortfolioBtcRegimeDays => "trend_portfolio_btc_regime_days",
            Self::TrendPortfolioWeightCap => "trend_portfolio_weight_cap",
            Self::MonthlyMomentumLookbackDays => "monthly_momentum_lookback_days",
            Self::MonthlyMomentumVolatilityDays => "monthly_momentum_volatility_days",
            Self::MonthlyMomentumVolatilityTarget => "monthly_momentum_volatility_target",
            Self::CarryMinimumDaysToExpiry => "carry_minimum_days_to_expiry",
            Self::CarryCostGatePercent => "carry_cost_gate_percent",
            Self::TrendSmaDays => "trend_sma_days",
        }
    }
}

#[derive(Clone, Copy)]
struct StrategyParameterSpec {
    key: StrategyParameterKey,
    label: &'static str,
    default: f64,
    min: f64,
    max: f64,
    step: f64,
    unit: &'static str,
    /// Positive means successful outcomes move this value upward; negative moves it downward.
    learning_direction: f64,
}

#[derive(Clone, Serialize)]
struct StrategyParameterValue {
    key: String,
    label: String,
    value: f64,
    learned_value: f64,
    configured_value: Option<f64>,
    default_value: f64,
    min: f64,
    max: f64,
    step: f64,
    unit: String,
    overridden: bool,
}

#[derive(Clone, Serialize)]
struct StrategyParameterProfile {
    symbol: String,
    strategy: String,
    learning_samples: usize,
    positive_rate: Option<f64>,
    parameters: Vec<StrategyParameterValue>,
}
