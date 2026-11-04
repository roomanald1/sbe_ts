use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type StrategyId = String;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StrategyDecision {
    Enter,
    Scout,
    Hold,
    Exit,
    Rebalance,
    Reject,
}

#[derive(Clone, Debug, Serialize)]
pub struct StrategyContract {
    pub insight: &'static str,
    pub entry: &'static str,
    pub exit: &'static str,
    pub entry_order: &'static str,
    pub exit_order: &'static str,
    pub behavior: &'static str,
}

/// Stable keys for bounded strategy parameters shared across runtime crates.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyParameterKey {
    BnbSmaDays,
    BreakoutLookbackDays,
    BreakoutEntryDistancePercent,
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
    MomentumLookbackSeconds,
    TrendLookbackSeconds,
    TradeFlowWindowSeconds,
    BookDepthLevels,
    VwapLookbackSeconds,
    VwapMinimumTradeCount,
    VwapMinimumTradeSpanSeconds,
    AbsorptionLookbackSeconds,
    AbsorptionReplenishmentSeconds,
    AbsorptionTouchToleranceBps,
    AbsorptionMaximumPriceMovePercent,
    AbsorptionMinimumFlowImbalance,
    MinimumCounterMomentumPercent,
    ScoutMinimumWalkForwardSamples,
    ScoutMinimumPositiveNetRate,
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
    PostOnlyTimeoutMinutes,
}

impl StrategyParameterKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BnbSmaDays => "bnb_sma_days",
            Self::BreakoutLookbackDays => "breakout_lookback_days",
            Self::BreakoutEntryDistancePercent => "breakout_entry_distance_percent",
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
            Self::MomentumLookbackSeconds => "momentum_lookback_seconds",
            Self::TrendLookbackSeconds => "trend_lookback_seconds",
            Self::TradeFlowWindowSeconds => "trade_flow_window_seconds",
            Self::BookDepthLevels => "book_depth_levels",
            Self::VwapLookbackSeconds => "vwap_lookback_seconds",
            Self::VwapMinimumTradeCount => "vwap_minimum_trade_count",
            Self::VwapMinimumTradeSpanSeconds => "vwap_minimum_trade_span_seconds",
            Self::AbsorptionLookbackSeconds => "absorption_lookback_seconds",
            Self::AbsorptionReplenishmentSeconds => "absorption_replenishment_seconds",
            Self::AbsorptionTouchToleranceBps => "absorption_touch_tolerance_bps",
            Self::AbsorptionMaximumPriceMovePercent => "absorption_maximum_price_move_percent",
            Self::AbsorptionMinimumFlowImbalance => "absorption_minimum_flow_imbalance",
            Self::MinimumCounterMomentumPercent => "minimum_counter_momentum_percent",
            Self::ScoutMinimumWalkForwardSamples => "scout_minimum_walk_forward_samples",
            Self::ScoutMinimumPositiveNetRate => "scout_minimum_positive_net_rate",
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
            Self::PostOnlyTimeoutMinutes => "post_only_timeout_minutes",
        }
    }
}

pub fn profile_id(symbol: &str, strategy: &str, parameters: &BTreeMap<String, f64>) -> String {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for byte in symbol.bytes().chain([b':']).chain(strategy.bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
    }
    for (key, value) in parameters {
        for byte in format!("{key}={value:.8};").bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
        }
    }
    format!("{hash:016x}")
}

/// Immutable, versioned parameter set consumed by the decision hot path.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StrategyProfile {
    pub profile_id: String,
    pub version: u64,
    pub symbol: String,
    pub strategy: String,
    pub parameters: BTreeMap<String, f64>,
    #[serde(default)]
    pub learning_samples: usize,
    #[serde(default)]
    pub positive_samples: usize,
    #[serde(default)]
    pub realized_pnl: f64,
}

/// Bounded tuning metadata exported with a strategy definition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParameterSpec {
    pub key: String,
    pub label: String,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub unit: String,
    /// +1 raises the parameter after a positive outcome; -1 lowers it.
    pub learning_direction: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StrategyProfileRegistration {
    pub profile: StrategyProfile,
    pub specs: Vec<ParameterSpec>,
}

/// Feedback event envelope; event IDs make retries safe to deduplicate.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeedbackEvent {
    ScoutOutcome {
        event_id: String,
        observed_at_ms: u64,
        symbol: String,
        strategy: String,
        profile_id: String,
        parameters: BTreeMap<String, f64>,
        specs: Vec<ParameterSpec>,
        net_return_percent: f64,
    },
    TradeOutcome {
        event_id: String,
        observed_at_ms: u64,
        symbol: String,
        strategy: String,
        profile_id: String,
        parameters: BTreeMap<String, f64>,
        specs: Vec<ParameterSpec>,
        scout: bool,
        realized_pnl_usd: f64,
        #[serde(default)]
        net_return_percent: f64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_wire_names_round_trip() {
        let profile = StrategyProfile {
            profile_id: "p1".into(),
            version: 1,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            parameters: BTreeMap::new(),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let encoded = serde_json::to_string(&profile).unwrap();
        let decoded: StrategyProfile = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, profile);
    }

    #[test]
    fn parameter_keys_keep_the_persisted_snake_case_name() {
        assert_eq!(
            serde_json::to_string(&StrategyParameterKey::MinimumMomentum).unwrap(),
            "\"minimum_momentum\""
        );
        assert_eq!(
            StrategyParameterKey::MinimumMomentum.as_str(),
            "minimum_momentum"
        );
    }
}
