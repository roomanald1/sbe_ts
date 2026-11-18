use serde::{Deserialize, Serialize};

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
    ScoutBreakoutBufferPercent,
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
    ScoutMaximumEvidenceAgeSeconds,
    MinimumSignalStrength,
    MinimumMomentumTrend,
    MinimumMomentumRange,
    MinimumMomentumVolatile,
    MinimumFlowImbalanceTrend,
    MinimumFlowImbalanceRange,
    MinimumFlowImbalanceVolatile,
    MinimumBookImbalanceTrend,
    MinimumBookImbalanceRange,
    MinimumBookImbalanceVolatile,
    MinimumVwapDislocationBpsTrend,
    MinimumVwapDislocationBpsRange,
    MinimumVwapDislocationBpsVolatile,
    MinimumAbsorptionPressureTrend,
    MinimumAbsorptionPressureRange,
    MinimumAbsorptionPressureVolatile,
    ProfitProtectionActivationPercent,
    ProfitProtectionGivebackPercent,
    ProfitProtectionMinimumLockPercent,
    ThesisFailureLossActivationPercent,
    ThesisFailureConfirmationSamples,
    ThesisFailureMinimumSignalStrength,
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
    TrendPortfolioExitWeightFloor,
    MonthlyMomentumLookbackDays,
    MonthlyMomentumVolatilityDays,
    MonthlyMomentumVolatilityTarget,
    CarryMinimumDaysToExpiry,
    CarryCostGatePercent,
    TrendSmaDays,
    PostOnlyTimeoutMinutes,
    PostStopCooldownMinutes,
}

impl StrategyParameterKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BnbSmaDays => "bnb_sma_days",
            Self::BreakoutLookbackDays => "breakout_lookback_days",
            Self::BreakoutEntryDistancePercent => "breakout_entry_distance_percent",
            Self::ScoutBreakoutBufferPercent => "scout_breakout_buffer_percent",
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
            Self::ScoutMaximumEvidenceAgeSeconds => "scout_maximum_evidence_age_seconds",
            Self::MinimumSignalStrength => "minimum_signal_strength",
            Self::MinimumMomentumTrend => "minimum_momentum_trend",
            Self::MinimumMomentumRange => "minimum_momentum_range",
            Self::MinimumMomentumVolatile => "minimum_momentum_volatile",
            Self::MinimumFlowImbalanceTrend => "minimum_flow_imbalance_trend",
            Self::MinimumFlowImbalanceRange => "minimum_flow_imbalance_range",
            Self::MinimumFlowImbalanceVolatile => "minimum_flow_imbalance_volatile",
            Self::MinimumBookImbalanceTrend => "minimum_book_imbalance_trend",
            Self::MinimumBookImbalanceRange => "minimum_book_imbalance_range",
            Self::MinimumBookImbalanceVolatile => "minimum_book_imbalance_volatile",
            Self::MinimumVwapDislocationBpsTrend => "minimum_vwap_dislocation_bps_trend",
            Self::MinimumVwapDislocationBpsRange => "minimum_vwap_dislocation_bps_range",
            Self::MinimumVwapDislocationBpsVolatile => "minimum_vwap_dislocation_bps_volatile",
            Self::MinimumAbsorptionPressureTrend => "minimum_absorption_pressure_trend",
            Self::MinimumAbsorptionPressureRange => "minimum_absorption_pressure_range",
            Self::MinimumAbsorptionPressureVolatile => "minimum_absorption_pressure_volatile",
            Self::ProfitProtectionActivationPercent => "profit_protection_activation_percent",
            Self::ProfitProtectionGivebackPercent => "profit_protection_giveback_percent",
            Self::ProfitProtectionMinimumLockPercent => "profit_protection_minimum_lock_percent",
            Self::ThesisFailureLossActivationPercent => "thesis_failure_loss_activation_percent",
            Self::ThesisFailureConfirmationSamples => "thesis_failure_confirmation_samples",
            Self::ThesisFailureMinimumSignalStrength => "thesis_failure_minimum_signal_strength",
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
            Self::TrendPortfolioExitWeightFloor => "trend_portfolio_exit_weight_floor",
            Self::MonthlyMomentumLookbackDays => "monthly_momentum_lookback_days",
            Self::MonthlyMomentumVolatilityDays => "monthly_momentum_volatility_days",
            Self::MonthlyMomentumVolatilityTarget => "monthly_momentum_volatility_target",
            Self::CarryMinimumDaysToExpiry => "carry_minimum_days_to_expiry",
            Self::CarryCostGatePercent => "carry_cost_gate_percent",
            Self::TrendSmaDays => "trend_sma_days",
            Self::PostOnlyTimeoutMinutes => "post_only_timeout_minutes",
            Self::PostStopCooldownMinutes => "post_stop_cooldown_minutes",
        }
    }

    pub fn regime_threshold(self, regime: &str) -> Option<Self> {
        let regime = regime.to_ascii_lowercase();
        let regime = if matches!(regime.as_str(), "trend" | "trending")
            || regime.starts_with("btc-above-sma")
            || regime.starts_with("btc-below-sma")
        {
            "trend"
        } else if matches!(regime.as_str(), "range" | "ranging") {
            "range"
        } else if matches!(regime.as_str(), "volatile" | "volatility") {
            "volatile"
        } else {
            return None;
        };
        Some(match (self, regime) {
            (Self::MinimumMomentum, "trend") => Self::MinimumMomentumTrend,
            (Self::MinimumMomentum, "range") => Self::MinimumMomentumRange,
            (Self::MinimumMomentum, "volatile") => Self::MinimumMomentumVolatile,
            (Self::MinimumFlowImbalance, "trend") => Self::MinimumFlowImbalanceTrend,
            (Self::MinimumFlowImbalance, "range") => Self::MinimumFlowImbalanceRange,
            (Self::MinimumFlowImbalance, "volatile") => Self::MinimumFlowImbalanceVolatile,
            (Self::MinimumBookImbalance, "trend") => Self::MinimumBookImbalanceTrend,
            (Self::MinimumBookImbalance, "range") => Self::MinimumBookImbalanceRange,
            (Self::MinimumBookImbalance, "volatile") => Self::MinimumBookImbalanceVolatile,
            (Self::MinimumVwapDislocationBps, "trend") => Self::MinimumVwapDislocationBpsTrend,
            (Self::MinimumVwapDislocationBps, "range") => Self::MinimumVwapDislocationBpsRange,
            (Self::MinimumVwapDislocationBps, "volatile") => {
                Self::MinimumVwapDislocationBpsVolatile
            }
            (Self::MinimumAbsorptionPressure, "trend") => Self::MinimumAbsorptionPressureTrend,
            (Self::MinimumAbsorptionPressure, "range") => Self::MinimumAbsorptionPressureRange,
            (Self::MinimumAbsorptionPressure, "volatile") => {
                Self::MinimumAbsorptionPressureVolatile
            }
            _ => return None,
        })
    }
}
