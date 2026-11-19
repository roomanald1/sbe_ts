use crate::Strategy;
use crate::core::{StrategyDecision, StrategyParameterKey};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalDirection {
    Long,
    Short,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, serde::Serialize)]
#[serde(default)]
pub struct StrategyMarketSignal {
    pub symbol: String,
    pub direction: String,
    pub strength: f64,
    pub timestamp: u64,
    pub stale: bool,
    pub score: f64,
    pub ranging_probability: Option<f64>,
    pub components: SignalComponents,
    #[serde(alias = "research_strategy_candidate")]
    pub strategy_candidate: bool,
    #[serde(alias = "research_strategy")]
    pub strategy: Option<String>,
    pub strategy_decision: Option<serde_json::Value>,
    pub market_regime: Option<String>,
    #[serde(default)]
    pub market_state: Option<LiveMarketStateClassification>,
    pub news_attribution: Option<serde_json::Value>,
    pub entry_gate: Option<serde_json::Value>,
}

/// Return the canonical regime attached to a signal, deriving it from the
/// shared market features when the producer did not provide one.
pub fn market_regime(signal: &StrategyMarketSignal) -> Option<&'static str> {
    if let Some(regime) = signal.market_regime.as_deref() {
        match regime {
            "range" => return Some("range"),
            "trend" => return Some("trend"),
            "volatile" => return Some("volatile"),
            "mixed" => return Some("mixed"),
            _ => {}
        }
    }
    if signal
        .ranging_probability
        .is_some_and(|probability| probability.is_finite() && probability >= 0.65)
    {
        return Some("range");
    }
    if signal
        .components
        .trend_5m_volatility_units
        .is_some_and(|value| value.is_finite() && value.abs() >= 1.5)
    {
        return Some("trend");
    }
    if signal
        .components
        .volatility_1m_percent
        .is_some_and(|value| value.is_finite() && value.abs() >= 1.0)
    {
        return Some("volatile");
    }
    let has_regime_features = signal.ranging_probability.is_some()
        || signal.components.trend_5m_volatility_units.is_some()
        || signal.components.volatility_1m_percent.is_some();
    has_regime_features.then_some("mixed")
}

/// Versioned live classifier. Confidence describes separation from the
/// decision boundary; missing or non-finite features remain uncertain.
pub const LIVE_MARKET_STATE_VERSION: u16 = 2;

#[derive(Clone, Debug, Deserialize, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveMarketStateClassification {
    pub version: u16,
    /// Direction and structure combined: trend_up, trend_down, range or mixed.
    pub structure: String,
    /// Independent volatility dimension: low, normal, high or unknown.
    pub volatility_overlay: String,
    /// Legacy coarse label retained for existing Learning evidence attribution.
    pub evidence_regime: String,
    pub profile_key: String,
    pub confidence: f64,
    pub observed_at_ms: u64,
}

impl LiveMarketStateClassification {
    pub fn key_is_supported(key: &str) -> bool {
        let Some((version, dimensions)) = key.split_once(':') else {
            return false;
        };
        if version != "v2" {
            return false;
        }
        matches!(
            dimensions,
            "trend_up:low"
                | "trend_up:normal"
                | "trend_up:high"
                | "trend_up:unknown"
                | "trend_down:low"
                | "trend_down:normal"
                | "trend_down:high"
                | "trend_down:unknown"
                | "range:low"
                | "range:normal"
                | "range:high"
                | "range:unknown"
                | "mixed:low"
                | "mixed:normal"
                | "mixed:high"
                | "mixed:unknown"
        )
    }
}

pub fn classify_live_market_state(
    signal: &StrategyMarketSignal,
) -> Option<LiveMarketStateClassification> {
    let range_probability = signal.ranging_probability.filter(|value| value.is_finite());
    let trend = signal
        .components
        .trend_5m_volatility_units
        .filter(|value| value.is_finite());
    let volatility = signal
        .components
        .volatility_1m_percent
        .filter(|value| value.is_finite())
        .map(f64::abs);

    let (structure, evidence_regime, structural_confidence) =
        if let Some(probability) = range_probability.filter(|value| *value >= 0.65) {
            (
                "range",
                "range",
                ((probability - 0.5) * 2.0).clamp(0.0, 1.0),
            )
        } else if let Some(trend) = trend.filter(|value| value.abs() >= 1.5) {
            if trend.is_sign_positive() {
                (
                    "trend_up",
                    "trend",
                    ((trend.abs() - 1.0) / 1.0).clamp(0.0, 1.0),
                )
            } else {
                (
                    "trend_down",
                    "trend",
                    ((trend.abs() - 1.0) / 1.0).clamp(0.0, 1.0),
                )
            }
        } else if range_probability.is_some() || trend.is_some() || volatility.is_some() {
            let legacy = market_regime(signal);
            let direction = match signal.direction.to_ascii_lowercase().as_str() {
                "bullish" | "long" => "trend_up",
                "bearish" | "short" => "trend_down",
                _ => "mixed",
            };
            match legacy {
                Some("range") => ("range", "range", 0.5),
                Some("trend") if direction != "mixed" => (direction, "trend", 0.5),
                Some("trend") => ("mixed", "trend", 0.5),
                Some("volatile") | Some("mixed") | None => ("mixed", "mixed", 0.5),
                _ => ("mixed", "mixed", 0.5),
            }
        } else {
            return None;
        };
    let volatility_overlay = match volatility {
        Some(value) if value >= 1.0 => "high",
        Some(value) if value <= 0.2 => "low",
        Some(_) => "normal",
        None => "unknown",
    };
    let evidence_regime =
        if volatility.is_some_and(|value| value >= 1.0) && evidence_regime == "mixed" {
            "volatile"
        } else {
            evidence_regime
        };
    let completeness = if volatility.is_some() { 1.0 } else { 0.85 };
    Some(LiveMarketStateClassification {
        version: LIVE_MARKET_STATE_VERSION,
        evidence_regime: evidence_regime.to_owned(),
        profile_key: format!("v2:{structure}:{volatility_overlay}"),
        structure: structure.to_owned(),
        volatility_overlay: volatility_overlay.to_owned(),
        confidence: (structural_confidence * completeness).clamp(0.0, 1.0),
        observed_at_ms: signal.timestamp,
    })
}

/// Select strategy-specific inputs from one shared Insights snapshot.
/// This adapter belongs to Strategy so coordinators do not interpret features.
pub fn prepare_live_strategy_signal(
    signal: &StrategyMarketSignal,
    strategy: Strategy,
    parameters: &BTreeMap<String, f64>,
) -> Result<StrategyMarketSignal, String> {
    let parameter = |key: StrategyParameterKey| {
        parameters
            .get(key.as_str())
            .copied()
            .ok_or_else(|| format!("Configured parameter '{}' is unavailable", key.as_str()))
    };
    let mut configured = signal.clone();
    match strategy {
        Strategy::LiveMomentum => {
            let momentum_window = parameter(StrategyParameterKey::MomentumLookbackSeconds)? as u16;
            let trend_window = parameter(StrategyParameterKey::TrendLookbackSeconds)? as u16;
            configured.components.momentum_10s_percent = Some(
                signal
                    .components
                    .momentum_by_window
                    .iter()
                    .find(|feature| feature.window_seconds == momentum_window)
                    .and_then(|feature| feature.value)
                    .unwrap_or(0.0),
            );
            configured.components.trend_5m_volatility_units = Some(
                signal
                    .components
                    .trend_volatility_units_by_window
                    .iter()
                    .find(|feature| feature.window_seconds == trend_window)
                    .and_then(|feature| feature.value)
                    .unwrap_or(0.0),
            );
        }
        Strategy::LiveTradeFlow => {
            let configured_window = parameter(StrategyParameterKey::TradeFlowWindowSeconds)?;
            // Learned values can be fractional while Insights exposes discrete
            // windows; use the nearest supported feature instead of skipping the symbol.
            configured.components.trade_flow_imbalance = signal
                .components
                .trade_flow_by_window
                .iter()
                .min_by(|left, right| {
                    (f64::from(left.window_seconds) - configured_window)
                        .abs()
                        .total_cmp(&(f64::from(right.window_seconds) - configured_window).abs())
                })
                .ok_or_else(|| "Insights supplied no trade-flow windows".to_owned())?
                .value;
        }
        Strategy::LiveOrderBook => {
            let depth = parameter(StrategyParameterKey::BookDepthLevels)? as u8;
            let momentum_window = parameter(StrategyParameterKey::MomentumLookbackSeconds)? as u16;
            configured.components.book_imbalance = signal
                .components
                .book_imbalance_by_depth
                .iter()
                .find(|feature| feature.depth_levels == depth)
                .ok_or_else(|| format!("Order-book feature at depth {depth} is missing"))?
                .value;
            configured.components.momentum_10s_percent = signal
                .components
                .momentum_by_window
                .iter()
                .find(|feature| feature.window_seconds == momentum_window)
                .ok_or_else(|| format!("Momentum feature window {momentum_window}s is missing"))?
                .value;
        }
        Strategy::LiveVwapDislocation => {
            let window = parameter(StrategyParameterKey::VwapLookbackSeconds)? as u16;
            let momentum_window = parameter(StrategyParameterKey::MomentumLookbackSeconds)? as u16;
            let minimum_count = parameter(StrategyParameterKey::VwapMinimumTradeCount)? as u16;
            let minimum_span_ms =
                (parameter(StrategyParameterKey::VwapMinimumTradeSpanSeconds)? * 1_000.0) as u32;
            configured.components.momentum_10s_percent = signal
                .components
                .momentum_by_window
                .iter()
                .find(|feature| feature.window_seconds == momentum_window)
                .ok_or_else(|| format!("Momentum feature window {momentum_window}s is missing"))?
                .value;
            let feature = signal
                .components
                .vwap_dislocation_by_window
                .iter()
                .find(|feature| feature.window_seconds == window)
                .ok_or_else(|| format!("VWAP-dislocation feature window {window}s is missing"))?;
            configured.components.mispricing_vs_vwap_bps = (feature.sample_count >= minimum_count
                && feature.span_ms >= minimum_span_ms)
                .then_some(feature.value)
                .flatten();
        }
        Strategy::LiveAbsorptionPressure => {
            let window = parameter(StrategyParameterKey::AbsorptionLookbackSeconds)? as u16;
            let replenishment_ms =
                (parameter(StrategyParameterKey::AbsorptionReplenishmentSeconds)? * 1_000.0) as u32;
            let tolerance = parameter(StrategyParameterKey::AbsorptionTouchToleranceBps)?;
            let maximum_move = parameter(StrategyParameterKey::AbsorptionMaximumPriceMovePercent)?;
            let momentum_window = parameter(StrategyParameterKey::MomentumLookbackSeconds)? as u16;
            let movement = signal
                .components
                .momentum_by_window
                .iter()
                .find(|feature| feature.window_seconds == momentum_window)
                .ok_or_else(|| format!("Momentum feature window {momentum_window}s is missing"))?
                .value
                .ok_or_else(|| {
                    format!("Momentum feature window {momentum_window}s has no value yet")
                })?;
            configured.components.momentum_10s_percent = Some(movement);
            if movement.abs() > maximum_move {
                return Err(format!(
                    "Price movement {movement:.4}% exceeds the configured maximum {maximum_move:.4}%"
                ));
            }
            let pressure = signal
                .components
                .absorption_pressure_by_window
                .iter()
                .find(|feature| {
                    feature.window_seconds == window
                        && feature.replenishment_window_ms == replenishment_ms
                        && (feature.touch_tolerance_bps - tolerance).abs() < 1e-6
                })
                .and_then(|feature| feature.value);
            let flow_window = parameter(StrategyParameterKey::TradeFlowWindowSeconds)?;
            let minimum_flow = parameter(StrategyParameterKey::AbsorptionMinimumFlowImbalance)?;
            let flow_fallback = signal
                .components
                .trade_flow_by_window
                .iter()
                .min_by(|left, right| {
                    (f64::from(left.window_seconds) - flow_window)
                        .abs()
                        .total_cmp(&(f64::from(right.window_seconds) - flow_window).abs())
                })
                .and_then(|feature| feature.value)
                .filter(|flow| flow.abs() >= minimum_flow)
                .map(|flow| -flow);
            configured.components.pressure_without_price_move = pressure.or(flow_fallback);
        }
        _ => {
            return Err(format!(
                "Strategy '{}' has no live-feature adapter",
                strategy.as_str()
            ));
        }
    }
    configured.strategy = Some(strategy.as_str().to_owned());
    configured.components.momentum_by_window.clear();
    configured
        .components
        .trend_volatility_units_by_window
        .clear();
    configured.components.trade_flow_by_window.clear();
    configured.components.book_imbalance_by_depth.clear();
    configured.components.vwap_dislocation_by_window.clear();
    configured.components.absorption_pressure_by_window.clear();
    Ok(configured)
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, serde::Serialize)]
#[serde(default)]
pub struct SignalComponents {
    pub momentum_10s_percent: Option<f64>,
    pub trend_5m_volatility_units: Option<f64>,
    pub trade_flow_imbalance: Option<f64>,
    pub book_imbalance: Option<f64>,
    pub volatility_1m_percent: Option<f64>,
    pub mispricing_vs_vwap_bps: Option<f64>,
    pub pressure_without_price_move: Option<f64>,
    pub aggressor_cost_bps: Option<f64>,
    pub momentum_by_window: Vec<WindowedSignalValue>,
    pub trend_volatility_units_by_window: Vec<WindowedSignalValue>,
    pub trade_flow_by_window: Vec<WindowedSignalValue>,
    pub book_imbalance_by_depth: Vec<DepthSignalValue>,
    pub vwap_dislocation_by_window: Vec<WindowedSignalValue>,
    pub absorption_pressure_by_window: Vec<AbsorptionPressureValue>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, serde::Serialize)]
pub struct WindowedSignalValue {
    pub window_seconds: u16,
    pub value: Option<f64>,
    pub sample_count: u16,
    pub span_ms: u32,
}
#[derive(Clone, Debug, Default, Deserialize, PartialEq, serde::Serialize)]
pub struct DepthSignalValue {
    pub depth_levels: u8,
    pub value: Option<f64>,
}
#[derive(Clone, Debug, Default, Deserialize, PartialEq, serde::Serialize)]
pub struct AbsorptionPressureValue {
    pub window_seconds: u16,
    pub replenishment_window_ms: u32,
    pub touch_tolerance_bps: f64,
    pub value: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, serde::Serialize)]
#[serde(default)]
pub struct DailyStrategySignalFeatures {
    pub observed_close: Option<f64>,
    pub primary_level: Option<f64>,
    pub secondary_level: Option<f64>,
    pub btc_gate_required: bool,
    pub btc_close: Option<f64>,
    pub btc_sma: Option<f64>,
    pub primary_metric_value: Option<f64>,
    pub strategy_order_size_multiplier: Option<f64>,
    pub monthly_anchor_close_time_ms: Option<u64>,
    pub cross_sectional_metric_value: Option<f64>,
    pub cross_sectional_threshold: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyWalkForwardEvidenceScope {
    ExactProfileRegime,
    ExactProfileUnconditioned,
    SymbolStrategyRegimeFallback,
    SymbolStrategyUnconditionedFallback,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyWalkForwardEstimator {
    /// Legacy bootstrap over completed outcomes, not an OOS portfolio stream.
    CompletedOutcomeClusters,
    RollingOriginPortfolio,
    #[default]
    Unspecified,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyWalkForwardCoverage {
    Partial,
    Complete,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyWalkForwardValidationStatus {
    Validated,
    Failed,
    #[default]
    NotValidated,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, serde::Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StrategyWalkForwardEvidence {
    /// Contract version carried with Learning snapshots, AutoTrader profiles,
    /// and Strategy decisions. Version zero identifies pre-contract payloads.
    pub evidence_contract_version: u16,
    pub estimator: StrategyWalkForwardEstimator,
    pub history_coverage: StrategyWalkForwardCoverage,
    pub cost_coverage: StrategyWalkForwardCoverage,
    pub search_family_coverage: StrategyWalkForwardCoverage,
    pub point_in_time_cutoff_timestamp_ms: Option<u64>,
    pub final_holdout_status: StrategyWalkForwardValidationStatus,
    pub oos_fold_count: usize,
    pub spa_p_value: Option<f64>,
    pub sample_count: usize,
    pub positive_net_rate: Option<f64>,
    pub average_net_return_percent: Option<f64>,
    pub last_sample_timestamp_ms: u64,
    /// Cluster-robust 95% interval around net expectancy, in percent.
    pub expectancy_ci_low_percent: Option<f64>,
    pub expectancy_ci_high_percent: Option<f64>,
    /// Kish effective sample size after outcome-age weighting.
    pub effective_sample_size: Option<f64>,
    /// Number of distinct UTC outcome-date windows represented.
    pub window_count: usize,
    /// Peak-to-trough drawdown of chronological compounded net returns.
    pub max_drawdown_percent: Option<f64>,
    /// Mean loss magnitude across negative net outcomes.
    pub downside_loss_percent: Option<f64>,
    /// Identifies exact-profile evidence versus a broader symbol/strategy fallback.
    pub evidence_scope: StrategyWalkForwardEvidenceScope,
}
impl DailyStrategySignalFeatures {
    pub fn close_above(&self, level: Option<f64>) -> bool {
        self.observed_close
            .zip(level)
            .is_some_and(|(c, l)| c.is_finite() && l.is_finite() && c > l)
    }
    pub fn btc_gate_passes(&self) -> bool {
        !self.btc_gate_required
            || self
                .btc_close
                .zip(self.btc_sma)
                .is_some_and(|(c, s)| c.is_finite() && s.is_finite() && c > s)
    }
    pub fn cross_sectional_leader(&self) -> bool {
        self.cross_sectional_metric_value
            .zip(self.cross_sectional_threshold)
            .is_some_and(|(v, t)| v.is_finite() && t.is_finite() && v >= t)
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DatedCarryCandidate {
    pub spot_symbol: String,
    pub future_symbol: String,
    pub expiry_ms: u64,
    pub days_to_expiry: f64,
    pub quantity: f64,
    pub spot_ask: f64,
    pub future_bid: f64,
    #[serde(default)]
    pub spot_bid: f64,
    #[serde(default)]
    pub future_ask: f64,
    pub basis_percent: f64,
    pub net_after_budget_percent: f64,
    pub eligible: bool,
    pub reason: String,
}
#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DatedCarrySettlementSource {
    Official,
    #[serde(rename = "index_1m_estimate")]
    Index1mEstimate,
}
impl DatedCarrySettlementSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Index1mEstimate => "index_1m_estimate",
        }
    }
}
#[derive(Clone, Debug)]
pub enum StrategySpecificSignal {
    DatedCarryEntry {
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        candidate: DatedCarryCandidate,
    },
    DatedCarrySettlement {
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        spot_symbol: String,
        future_symbol: String,
        expiry_ms: u64,
        spot_exit_twap: f64,
        settlement_price: f64,
        settlement_source: Option<DatedCarrySettlementSource>,
    },
}

/// Typed strategy-only input shared by AutoTrader and Scouts. Market calculations and
/// lifecycle context are prepared by the caller; evaluators do not read feeds.
#[derive(Clone, Debug)]
pub struct StrategyInput {
    pub signal: Option<StrategyMarketSignal>,
    pub specific_signal: Option<StrategySpecificSignal>,
    pub daily_features: Option<DailyStrategySignalFeatures>,
    pub walk_forward_evidence: Option<StrategyWalkForwardEvidence>,
    pub direction: Option<SignalDirection>,
    pub strategy: Option<crate::Strategy>,
    pub parameters: BTreeMap<String, f64>,
    pub position_open: bool,
    pub strategy_enabled: bool,
    pub signal_active: Option<bool>,
    pub rebalance_required: bool,
}
impl StrategyInput {
    /// Build the same typed evaluator input from a serialized signal candidate.
    /// The candidate contract is the `Signal` JSON published by Insights.
    pub fn from_candidate_json(
        candidate: serde_json::Value,
        parameters: BTreeMap<String, f64>,
    ) -> Result<Self, String> {
        let signal: StrategyMarketSignal = serde_json::from_value(candidate)
            .map_err(|error| format!("invalid strategy candidate: {error}"))?;
        Ok(Self::entry(&signal, &parameters))
    }
    pub fn entry(signal: &StrategyMarketSignal, parameters: &BTreeMap<String, f64>) -> Self {
        let daily_features = signal
            .strategy_decision
            .clone()
            .and_then(|value| serde_json::from_value(value).ok());
        let walk_forward_evidence = signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("strategy_walk_forward"))
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        let strategy = signal
            .strategy
            .as_deref()
            .and_then(crate::Strategy::from_name);
        Self {
            signal: Some(signal.clone()),
            specific_signal: None,
            daily_features,
            walk_forward_evidence,
            direction: SignalDirection::from_wire(&signal.direction),
            strategy,
            parameters: parameters.clone(),
            position_open: false,
            strategy_enabled: true,
            signal_active: None,
            rebalance_required: false,
        }
    }
    pub fn position(
        parameters: &BTreeMap<String, f64>,
        strategy_enabled: bool,
        signal_active: Option<bool>,
        rebalance_required: bool,
    ) -> Self {
        Self {
            signal: None,
            specific_signal: None,
            daily_features: None,
            walk_forward_evidence: None,
            direction: None,
            strategy: None,
            parameters: parameters.clone(),
            position_open: true,
            strategy_enabled,
            signal_active,
            rebalance_required,
        }
    }
    pub fn dated_carry_entry(
        candidate: &DatedCarryCandidate,
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        parameters: &BTreeMap<String, f64>,
    ) -> Self {
        Self {
            signal: None,
            specific_signal: Some(StrategySpecificSignal::DatedCarryEntry {
                emitted_at_ms,
                evaluated_at_ms,
                candidate: candidate.clone(),
            }),
            daily_features: None,
            walk_forward_evidence: None,
            direction: None,
            strategy: Some(crate::Strategy::DatedCarry),
            parameters: parameters.clone(),
            position_open: false,
            strategy_enabled: true,
            signal_active: None,
            rebalance_required: false,
        }
    }
    pub fn dated_carry_settlement(
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        spot_symbol: &str,
        future_symbol: &str,
        expiry_ms: u64,
        spot_exit_twap: f64,
        settlement_price: f64,
        settlement_source: Option<DatedCarrySettlementSource>,
        parameters: &BTreeMap<String, f64>,
    ) -> Self {
        Self {
            signal: None,
            specific_signal: Some(StrategySpecificSignal::DatedCarrySettlement {
                emitted_at_ms,
                evaluated_at_ms,
                spot_symbol: spot_symbol.into(),
                future_symbol: future_symbol.into(),
                expiry_ms,
                spot_exit_twap,
                settlement_price,
                settlement_source,
            }),
            daily_features: None,
            walk_forward_evidence: None,
            direction: None,
            strategy: Some(crate::Strategy::DatedCarry),
            parameters: parameters.clone(),
            position_open: true,
            strategy_enabled: true,
            signal_active: None,
            rebalance_required: false,
        }
    }
}

impl SignalDirection {
    pub fn from_wire(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "UP" | "LONG" | "BULLISH" => Some(Self::Long),
            "DOWN" | "SHORT" | "BEARISH" => Some(Self::Short),
            _ => None,
        }
    }

    pub fn non_directional_reason(value: &str) -> String {
        let value = value.trim();
        if value.eq_ignore_ascii_case("neutral") {
            "Signal direction is neutral".to_owned()
        } else if value.is_empty() {
            "Signal direction is unavailable (missing or empty)".to_owned()
        } else {
            format!("Signal direction '{value}' is unsupported")
        }
    }
}

#[cfg(test)]
mod signal_direction_tests {
    use super::SignalDirection;

    #[test]
    fn explains_neutral_missing_and_unsupported_directions_separately() {
        assert_eq!(
            SignalDirection::non_directional_reason("neutral"),
            "Signal direction is neutral"
        );
        assert_eq!(
            SignalDirection::non_directional_reason("  "),
            "Signal direction is unavailable (missing or empty)"
        );
        assert_eq!(
            SignalDirection::non_directional_reason("ranging"),
            "Signal direction 'ranging' is unsupported"
        );
    }
}

#[cfg(test)]
mod market_regime_tests {
    use super::{
        LiveMarketStateClassification, StrategyMarketSignal, classify_live_market_state,
        market_regime,
    };

    #[test]
    fn uses_canonical_regime_then_derives_it_from_market_features() {
        let explicit = StrategyMarketSignal {
            market_regime: Some("range".into()),
            ranging_probability: Some(0.1),
            ..Default::default()
        };
        assert_eq!(market_regime(&explicit), Some("range"));

        let volatile = StrategyMarketSignal {
            components: super::SignalComponents {
                volatility_1m_percent: Some(1.2),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(market_regime(&volatile), Some("volatile"));
        assert_eq!(market_regime(&StrategyMarketSignal::default()), None);
    }

    #[test]
    fn live_market_state_reports_confidence_and_uncertainty() {
        let range = StrategyMarketSignal {
            ranging_probability: Some(0.9),
            ..Default::default()
        };
        let state = classify_live_market_state(&range).unwrap();
        assert_eq!(state.version, 2);
        assert_eq!(state.structure, "range");
        assert_eq!(state.volatility_overlay, "unknown");
        assert_eq!(state.evidence_regime, "range");
        assert_eq!(state.profile_key, "v2:range:unknown");
        assert_eq!(state.confidence, 0.68);
        assert_eq!(state.observed_at_ms, 0);
        let missing = StrategyMarketSignal::default();
        assert!(classify_live_market_state(&missing).is_none());
    }

    #[test]
    fn live_market_state_keeps_direction_and_volatility_as_orthogonal_dimensions() {
        let bullish = StrategyMarketSignal {
            timestamp: 123,
            components: super::SignalComponents {
                trend_5m_volatility_units: Some(2.0),
                volatility_1m_percent: Some(1.2),
                ..Default::default()
            },
            ..Default::default()
        };
        let state = classify_live_market_state(&bullish).unwrap();
        assert_eq!(state.structure, "trend_up");
        assert_eq!(state.volatility_overlay, "high");
        assert_eq!(state.evidence_regime, "trend");
        assert_eq!(state.profile_key, "v2:trend_up:high");
        assert_eq!(state.observed_at_ms, 123);

        let bearish = StrategyMarketSignal {
            components: super::SignalComponents {
                trend_5m_volatility_units: Some(-2.0),
                volatility_1m_percent: Some(0.1),
                ..Default::default()
            },
            ..Default::default()
        };
        let state = classify_live_market_state(&bearish).unwrap();
        assert_eq!(state.structure, "trend_down");
        assert_eq!(state.volatility_overlay, "low");
        assert_eq!(state.profile_key, "v2:trend_down:low");
        assert!(LiveMarketStateClassification::key_is_supported(
            &state.profile_key
        ));
        assert!(!LiveMarketStateClassification::key_is_supported(
            "v3:trend_down:low"
        ));
    }
}

#[cfg(test)]
mod walk_forward_evidence_contract_tests {
    use super::*;

    #[test]
    fn evidence_contract_is_versioned_and_legacy_payloads_are_unvalidated() {
        let legacy: StrategyWalkForwardEvidence = serde_json::from_value(serde_json::json!({
            "sampleCount": 40,
            "expectancyCiLowPercent": 0.1
        }))
        .unwrap();
        assert_eq!(legacy.evidence_contract_version, 0);
        assert_eq!(legacy.estimator, StrategyWalkForwardEstimator::Unspecified);
        assert_eq!(
            legacy.final_holdout_status,
            StrategyWalkForwardValidationStatus::NotValidated
        );

        let versioned = StrategyWalkForwardEvidence {
            evidence_contract_version: 2,
            estimator: StrategyWalkForwardEstimator::RollingOriginPortfolio,
            history_coverage: StrategyWalkForwardCoverage::Complete,
            cost_coverage: StrategyWalkForwardCoverage::Complete,
            search_family_coverage: StrategyWalkForwardCoverage::Complete,
            point_in_time_cutoff_timestamp_ms: Some(123),
            final_holdout_status: StrategyWalkForwardValidationStatus::Validated,
            oos_fold_count: 8,
            spa_p_value: Some(0.02),
            ..Default::default()
        };
        let payload = serde_json::to_value(versioned).unwrap();
        assert_eq!(payload["evidenceContractVersion"], 2);
        assert_eq!(payload["estimator"], "rolling_origin_portfolio");
        assert_eq!(payload["pointInTimeCutoffTimestampMs"], 123);
        assert_eq!(payload["finalHoldoutStatus"], "validated");
        assert_eq!(payload["spaPValue"], 0.02);
    }
}
pub type Signal = StrategyMarketSignal;
/// Evaluator-facing borrowed view. AutoTrader can pass its published signal and learned
/// profile directly without cloning the signal's component vectors or parameter map.
#[derive(Clone, Debug)]
pub struct StrategySignal<'a> {
    pub signal: Option<&'a StrategyMarketSignal>,
    pub specific_signal: Option<StrategySpecificSignal>,
    pub daily_features: Option<DailyStrategySignalFeatures>,
    pub walk_forward_evidence: Option<StrategyWalkForwardEvidence>,
    pub direction: Option<SignalDirection>,
    pub strategy: Option<crate::Strategy>,
    pub parameters: &'a BTreeMap<String, f64>,
    pub position_open: bool,
    pub strategy_enabled: bool,
    pub signal_active: Option<bool>,
    pub rebalance_required: bool,
}
impl<'a> StrategySignal<'a> {
    pub fn from_candidate_json(
        candidate: serde_json::Value,
        parameters: BTreeMap<String, f64>,
    ) -> Result<StrategyInput, String> {
        StrategyInput::from_candidate_json(candidate, parameters)
    }
    pub fn entry(signal: &'a StrategyMarketSignal, parameters: &'a BTreeMap<String, f64>) -> Self {
        let daily_features = signal
            .strategy_decision
            .clone()
            .and_then(|value| serde_json::from_value(value).ok());
        let walk_forward_evidence = signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("strategy_walk_forward"))
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        Self {
            signal: Some(signal),
            specific_signal: None,
            daily_features,
            walk_forward_evidence,
            direction: SignalDirection::from_wire(&signal.direction),
            strategy: signal
                .strategy
                .as_deref()
                .and_then(crate::Strategy::from_name),
            parameters,
            position_open: false,
            strategy_enabled: true,
            signal_active: None,
            rebalance_required: false,
        }
    }
    pub fn position(
        parameters: &'a BTreeMap<String, f64>,
        strategy_enabled: bool,
        signal_active: Option<bool>,
        rebalance_required: bool,
    ) -> Self {
        Self {
            signal: None,
            specific_signal: None,
            daily_features: None,
            walk_forward_evidence: None,
            direction: None,
            strategy: None,
            parameters,
            position_open: true,
            strategy_enabled,
            signal_active,
            rebalance_required,
        }
    }
    pub fn dated_carry_entry(
        candidate: &DatedCarryCandidate,
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        parameters: &'a BTreeMap<String, f64>,
    ) -> Self {
        Self {
            signal: None,
            specific_signal: Some(StrategySpecificSignal::DatedCarryEntry {
                emitted_at_ms,
                evaluated_at_ms,
                candidate: candidate.clone(),
            }),
            daily_features: None,
            walk_forward_evidence: None,
            direction: None,
            strategy: Some(crate::Strategy::DatedCarry),
            parameters,
            position_open: false,
            strategy_enabled: true,
            signal_active: None,
            rebalance_required: false,
        }
    }
    pub fn dated_carry_settlement(
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        spot_symbol: &str,
        future_symbol: &str,
        expiry_ms: u64,
        spot_exit_twap: f64,
        settlement_price: f64,
        settlement_source: Option<DatedCarrySettlementSource>,
        parameters: &'a BTreeMap<String, f64>,
    ) -> Self {
        Self {
            signal: None,
            specific_signal: Some(StrategySpecificSignal::DatedCarrySettlement {
                emitted_at_ms,
                evaluated_at_ms,
                spot_symbol: spot_symbol.into(),
                future_symbol: future_symbol.into(),
                expiry_ms,
                spot_exit_twap,
                settlement_price,
                settlement_source,
            }),
            daily_features: None,
            walk_forward_evidence: None,
            direction: None,
            strategy: Some(crate::Strategy::DatedCarry),
            parameters,
            position_open: true,
            strategy_enabled: true,
            signal_active: None,
            rebalance_required: false,
        }
    }
    pub fn position_decision(
        &self,
        exit_when_disabled: bool,
        exit_when_signal_inactive: bool,
    ) -> Option<StrategyDecision> {
        if !self.position_open {
            return None;
        }
        if !self.strategy_enabled && exit_when_disabled {
            return Some(StrategyDecision::Exit);
        }
        if exit_when_signal_inactive {
            return Some(StrategyDecision::Exit);
        }
        if self.rebalance_required {
            return Some(StrategyDecision::Rebalance);
        }
        Some(StrategyDecision::Hold)
    }
    pub fn scout_breakout_distance_percent(&self) -> Option<f64> {
        self.daily_features.and_then(|f| {
            f.observed_close.zip(f.primary_level).and_then(|(c, l)| {
                (c.is_finite() && c > 0.0 && l.is_finite() && l > 0.0)
                    .then_some((c / l - 1.0) * 100.0)
            })
        })
    }
    pub fn within_learned_breakout_entry_distance(&self, d: f64) -> bool {
        self.daily_features.is_some_and(|f| {
            f.close_above(f.primary_level)
                || (d > 0.0
                    && f.observed_close
                        .zip(f.primary_level)
                        .is_some_and(|(c, l)| (c / l - 1.0) * 100.0 >= -d))
        })
    }
    pub fn within_scout_breakout_buffer(&self) -> bool {
        let Some(buffer) = strategy_parameter_value(
            self.parameters,
            StrategyParameterKey::ScoutBreakoutBufferPercent,
        ) else {
            return false;
        };
        self.scout_breakout_distance_percent()
            .is_some_and(|d| (-buffer..=0.0).contains(&d))
    }
}

/// Adapter hook for downstream services with owned candidate payloads.
pub trait StrategyInputLike {
    fn with_strategy_signal<R>(&self, evaluate: impl FnOnce(&StrategySignal<'_>) -> R) -> R;
}
impl StrategyInputLike for StrategyInput {
    #[inline]
    fn with_strategy_signal<R>(&self, evaluate: impl FnOnce(&StrategySignal<'_>) -> R) -> R {
        let input = StrategySignal {
            signal: self.signal.as_ref(),
            specific_signal: self.specific_signal.clone(),
            daily_features: self.daily_features,
            walk_forward_evidence: self.walk_forward_evidence,
            direction: self.direction,
            strategy: self.strategy,
            parameters: &self.parameters,
            position_open: self.position_open,
            strategy_enabled: self.strategy_enabled,
            signal_active: self.signal_active,
            rebalance_required: self.rebalance_required,
        };
        evaluate(&input)
    }
}
impl StrategyInputLike for StrategySignal<'_> {
    #[inline]
    fn with_strategy_signal<R>(&self, evaluate: impl FnOnce(&StrategySignal<'_>) -> R) -> R {
        evaluate(self)
    }
}

pub fn strategy_parameter_value(
    parameters: &BTreeMap<String, f64>,
    key: StrategyParameterKey,
) -> Option<f64> {
    parameters
        .get(key.as_str())
        .copied()
        .filter(|v| v.is_finite())
}

pub fn regime_conditioned_parameter_value(
    signal: &StrategyMarketSignal,
    parameters: &BTreeMap<String, f64>,
    key: StrategyParameterKey,
) -> Option<f64> {
    signal
        .market_regime
        .as_deref()
        .and_then(|regime| key.regime_threshold(regime))
        .and_then(|regime_key| strategy_parameter_value(parameters, regime_key))
        .or_else(|| strategy_parameter_value(parameters, key))
}
