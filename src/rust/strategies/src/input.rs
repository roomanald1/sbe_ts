use crate::core::{StrategyDecision, StrategyParameterKey};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalDirection {
    Long,
    Short,
}

#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
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
    pub news_attribution: Option<serde_json::Value>,
    pub entry_gate: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
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

#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
pub struct WindowedSignalValue {
    pub window_seconds: u16,
    pub value: Option<f64>,
    pub sample_count: u16,
    pub span_ms: u32,
}
#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
pub struct DepthSignalValue {
    pub depth_levels: u8,
    pub value: Option<f64>,
}
#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
pub struct AbsorptionPressureValue {
    pub window_seconds: u16,
    pub replenishment_window_ms: u32,
    pub touch_tolerance_bps: f64,
    pub value: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, serde::Serialize)]
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

#[derive(Clone, Copy, Debug, Default, Deserialize, serde::Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StrategyWalkForwardEvidence {
    pub sample_count: usize,
    pub positive_net_rate: Option<f64>,
    pub average_net_return_percent: Option<f64>,
    pub last_sample_timestamp_ms: u64,
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
