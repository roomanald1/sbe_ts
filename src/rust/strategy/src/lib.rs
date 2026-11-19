use serde::{Deserialize, Serialize};
extern crate self as strategies;
#[cfg(test)]
use std::collections::BTreeMap;
pub mod core;
pub use core::*;
mod benchmarks;
#[cfg(test)]
pub(crate) use benchmarks::benchmark_signal;
pub use benchmarks::benchmark_strategy_evaluation;
pub mod daily;
mod input;
mod parameter_catalog;
pub mod parameter_profiles;
mod parameter_scouting;
pub mod parameter_types;
pub mod scout_work;
pub use parameter_scouting::{
    ParameterVariantsRequest, StrategyEvidenceCandidate, combination_context, combination_key,
    next_parameter_combination, next_parameter_probe, parse_combination, probe_id,
    single_parameter_variants,
};
mod live_evaluation;
#[cfg(feature = "service")]
mod live_worker;
#[cfg(feature = "service")]
mod scout_worker;
#[cfg(feature = "service")]
mod scouts;
#[cfg(feature = "service")]
mod worker_metrics;
pub use input::{
    AbsorptionPressureValue, DailyStrategySignalFeatures, DatedCarryCandidate,
    DatedCarrySettlementSource, DepthSignalValue, Signal, SignalComponents, SignalDirection,
    StrategyInput, StrategyInputLike, StrategyMarketSignal, StrategySignal, StrategySpecificSignal,
    StrategyWalkForwardCoverage, StrategyWalkForwardEstimator, StrategyWalkForwardEvidence,
    StrategyWalkForwardEvidenceScope, StrategyWalkForwardValidationStatus, WindowedSignalValue,
    market_regime, prepare_live_strategy_signal, regime_conditioned_parameter_value,
    strategy_parameter_value,
};
pub use live_evaluation::{
    DatedCarryScanUpdate, DatedCarrySettlementUpdate, LIVE_EVALUATION_CONTRACT_VERSION,
    LiveEvaluationError, LiveEvaluationErrorCode, LiveEvaluationInput, LiveEvaluationRequest,
    LiveEvaluationResponse, LivePositionContext, LiveStrategyDecision, LiveStrategyDecisionReport,
    LiveStrategyOpenPosition, LiveStrategyProfile, LiveStrategyProfileSet,
    LiveStrategySocketRequest, LiveStrategySocketResponse, LiveStrategyWorkerDiagnostic,
    STRATEGY_INSIGHT_UPDATE_VERSION, StrategyInsightUpdate, decode_learning_snapshot_payload,
    encode_learning_snapshot_payload,
};
#[cfg(feature = "service")]
pub use live_worker::{LiveWorkerConfig, serve_live_worker};
pub use parameter_profiles::{
    confident_win_rate_edge, effective_parameter_values, legacy_strategy_parameter_name,
    scout_parameter_specs, signal_strategy_parameters, strategy_parameter_key,
    strategy_profile_value, validate_strategy_parameter_overrides,
};
pub use parameter_types::{
    StrategyParameterProfile, StrategyParameterScoutVariant, StrategyParameterValue,
};
pub use scout_work::{
    MAX_REPLAY_SIGNALS, ScoutCandidateBatch, ScoutCandidateBatchResult, ScoutCoordinatorRequest,
    ScoutCoordinatorResponse, ScoutCoordinatorRouteCommand, ScoutCoordinatorRouteResult,
    ScoutDataUpdate, ScoutFeedbackAcknowledgement, ScoutStreamEnvelope, ScoutStreamSubscription,
    ScoutWorkAssignment, ScoutWorkDecision, ScoutWorkMode, ScoutWorkResult, ScoutWorkerHttpMethod,
    ScoutWorkerRequest, ScoutWorkerResponse, ScoutWorkerRouteCommand, ScoutWorkerRouteResult,
    StrategyReplayDecisionPoint, StrategyReplayPoint, StrategyReplayRequest,
    StrategyReplayResponse, request_learning_route, request_scout_candidate_batch,
    request_scout_feedback, request_scout_profile_snapshot, request_scout_profile_snapshot_for,
    request_scout_service_route, request_scout_strategy_replay, request_scout_worker_route,
};
#[cfg(feature = "live_socket_client")]
pub use scout_work::{request_learning_snapshot_push, request_live_strategy_socket};
#[cfg(feature = "service")]
pub use scout_worker::{ScoutWorkerConfig, serve_strategy_worker};
#[cfg(feature = "service")]
pub use scouts::router;
pub fn is_autotrader_symbol_allowed(symbol: &str) -> bool {
    autotrader_allowed_symbols().contains(&symbol)
}
/// Canonical symbol universe accepted by AutoTrader, Learning, and Scout workers.
pub const AUTOTRADER_ALLOWED_SYMBOLS: &[&str] = &[
    "BTC-USDT",
    "ETH-USDT",
    "SOL-USDT",
    "BNB-USDT",
    "XRP-USDT",
    "ADA-USDT",
    "DOGE-USDT",
    "TRX-USDT",
    "LINK-USDT",
    "LTC-USDT",
    "AVAX-USDT",
];

pub fn autotrader_allowed_symbols() -> &'static [&'static str] {
    AUTOTRADER_ALLOWED_SYMBOLS
}

#[cfg(test)]
mod autotrader_symbol_allowlist_tests {
    #[test]
    fn shared_allowlist_is_the_scout_symbol_source() {
        for symbol in crate::autotrader_allowed_symbols() {
            assert!(crate::is_autotrader_symbol_allowed(symbol));
        }
        assert!(!crate::is_autotrader_symbol_allowed("XMSTR-USDT"));
        assert!(!crate::is_autotrader_symbol_allowed("BTC-USDC"));
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StrategyParameterSpec {
    pub key: StrategyParameterKey,
    pub label: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub unit: &'static str,
    pub learning_direction: f64,
}
pub const fn param(
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

#[derive(Clone, Copy, Debug)]
pub enum LiveInsightField {
    Momentum10s,
    Trend5m,
    TradeFlow,
    OrderBook,
    VwapDislocation,
    AbsorptionPressure,
}
impl LiveInsightField {
    fn is_present(self, signal: &Signal) -> bool {
        match self {
            Self::Momentum10s => signal.components.momentum_10s_percent.is_some(),
            Self::Trend5m => signal.components.trend_5m_volatility_units.is_some(),
            Self::TradeFlow => signal.components.trade_flow_imbalance.is_some(),
            Self::OrderBook => signal.components.book_imbalance.is_some(),
            Self::VwapDislocation => signal.components.mispricing_vs_vwap_bps.is_some(),
            Self::AbsorptionPressure => signal.components.pressure_without_price_move.is_some(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    #[serde(rename = "live-momentum")]
    LiveMomentum,
    #[serde(rename = "live-trade-flow")]
    LiveTradeFlow,
    #[serde(rename = "live-order-book")]
    LiveOrderBook,
    #[serde(rename = "live-vwap-dislocation")]
    LiveVwapDislocation,
    #[serde(rename = "live-absorption-pressure")]
    LiveAbsorptionPressure,
    #[default]
    #[serde(rename = "donchian-55-btc200")]
    Donchian55Btc200,
    #[serde(rename = "donchian-20-sma200")]
    Donchian20Sma200,
    #[serde(rename = "donchian-20-btc200")]
    Donchian20Btc200,
    #[serde(rename = "donchian-10-btc200")]
    Donchian10Btc200,
    #[serde(rename = "donchian-20-btc200-hold3")]
    Donchian20Btc200Hold3,
    #[serde(rename = "risk-adjusted-trend-portfolio")]
    RiskAdjustedTrendPortfolio,
    #[serde(rename = "trend-sleeves-btc200")]
    TrendSleevesBtc200,
    #[serde(rename = "trend-50-200")]
    Trend50200,
    #[serde(rename = "relative-strength-90-sma200")]
    RelativeStrength90Sma200,
    #[serde(rename = "risk-adjusted-monthly-momentum")]
    RiskAdjustedMonthlyMomentum,
    #[serde(rename = "rsi2-oversold-rebound")]
    Rsi2OversoldRebound,
    #[serde(rename = "dated-futures-carry")]
    DatedCarry,
}

pub struct StrategyDefinition {
    pub strategy: Strategy,
    pub name: &'static str,
    live_insight: bool,
    live_insight_fields: &'static [LiveInsightField],
    legacy_composite: bool,
    expands_to: &'static [Strategy],
    evaluate: fn(&StrategySignal<'_>) -> Result<StrategyDecision, String>,
    contract: fn() -> StrategyContract,
    should_exit_when_disabled: fn(bool) -> bool,
    should_exit_when_insight_inactive: Option<fn(bool) -> bool>,
    parameter_specs: fn() -> Vec<StrategyParameterSpec>,
}

fn strategy_decision_rejection_reason(
    strategy: Strategy,
    input: &StrategySignal<'_>,
) -> Option<String> {
    let Some(signal) = input.signal else {
        return Some(format!(
            "{} rejected entry: no signal data was provided to evaluate its entry rules",
            strategy.as_str()
        ));
    };
    let Some(details) = signal.strategy_decision.as_ref() else {
        return Some(format!(
            "{} rejected entry: the signal did not satisfy its entry rules, and no rule metrics were attached",
            strategy.as_str()
        ));
    };
    let boolean = |key: &str| details.get(key).and_then(serde_json::Value::as_bool);
    let number = |key: &str| details.get(key).and_then(serde_json::Value::as_f64);
    let string = |key: &str| details.get(key).and_then(serde_json::Value::as_str);

    if boolean("btc_gate_required") == Some(true) && boolean("btc_gate_passed") == Some(false) {
        let window = number("btc_window_days")
            .map(|days| format!("SMA{}", days as u64))
            .unwrap_or_else(|| "trend average".to_owned());
        return Some(match (number("btc_close"), number("btc_sma")) {
            (Some(close), Some(sma)) => {
                format!("BTC trend gate failed: close {close:.2} vs {window} {sma:.2}")
            }
            _ => format!("BTC trend gate failed ({window})"),
        });
    }
    if boolean("primary_passed") == Some(false) {
        let rule = string("primary_rule").unwrap_or("primary strategy rule");
        let unit = string("primary_metric_unit").unwrap_or("");
        let observed = number("primary_metric_value")
            .map(|value| format!("{value:.2} {unit}"))
            .unwrap_or_else(|| "unavailable".to_owned());
        let threshold = number("primary_level")
            .map(|value| format!("{value:.2} {unit}"))
            .unwrap_or_else(|| "unavailable".to_owned());
        return Some(format!(
            "{rule} failed: observed {observed}, threshold {threshold}"
        ));
    }
    if boolean("secondary_passed") == Some(false) {
        let rule = string("secondary_rule").unwrap_or("secondary strategy rule");
        let distance = number("secondary_distance_percent")
            .map(|value| format!(" (distance {value:.2}%)"))
            .unwrap_or_default();
        return Some(format!("{rule} failed{distance}"));
    }

    Some(format!(
        "{} rejected entry: the signal did not satisfy its entry rules{}",
        strategy.as_str(),
        if signal.strategy_candidate {
            ""
        } else {
            " because it was not marked as a strategy candidate"
        }
    ))
}

fn no_strategy_parameters() -> Vec<StrategyParameterSpec> {
    Vec::new()
}

/// Evaluate the registered entry or open-position rule for a strategy.
pub fn evaluate<I: StrategyInputLike>(
    strategy: Strategy,
    input: &I,
) -> Result<StrategyDecision, String> {
    strategy.evaluate(input)
}

/// Return strategy-specific learned parameter specs.
pub fn parameter_specs(strategy: Strategy) -> Vec<StrategyParameterSpec> {
    parameter_catalog::complete_parameter_specs(strategy)
}

fn all_strategy_parameter_specs(strategy: Strategy) -> Vec<StrategyParameterSpec> {
    parameter_specs(strategy)
}

/// Return the stable behavior contract registered for this strategy.
pub fn contract(strategy: Strategy) -> StrategyContract {
    strategy.contract()
}

pub mod strategy_live_momentum {
    pub mod evaluate {
        include!("strategy_live_momentum/evaluate.rs");
    }
    pub mod data {
        include!("strategy_live_momentum/data.rs");
    }
    pub mod definition {
        include!("strategy_live_momentum/contract.rs");
    }
    pub mod parameters {
        include!("strategy_live_momentum/parameters.rs");
    }
}
pub mod strategy_live_trade_flow {
    pub mod evaluate {
        include!("strategy_live_trade_flow/evaluate.rs");
    }
    pub mod data {
        include!("strategy_live_trade_flow/data.rs");
    }
    pub mod definition {
        include!("strategy_live_trade_flow/contract.rs");
    }
    pub mod parameters {
        include!("strategy_live_trade_flow/parameters.rs");
    }
}
pub mod strategy_live_order_book {
    pub mod evaluate {
        include!("strategy_live_order_book/evaluate.rs");
    }
    pub mod data {
        include!("strategy_live_order_book/data.rs");
    }
    pub mod definition {
        include!("strategy_live_order_book/contract.rs");
    }
    pub mod parameters {
        include!("strategy_live_order_book/parameters.rs");
    }
}
pub mod strategy_live_vwap_dislocation {
    pub mod evaluate {
        include!("strategy_live_vwap_dislocation/evaluate.rs");
    }
    pub mod data {
        include!("strategy_live_vwap_dislocation/data.rs");
    }
    pub mod definition {
        include!("strategy_live_vwap_dislocation/contract.rs");
    }
    pub mod parameters {
        include!("strategy_live_vwap_dislocation/parameters.rs");
    }
}
pub mod strategy_live_absorption_pressure {
    pub mod evaluate {
        include!("strategy_live_absorption_pressure/evaluate.rs");
    }
    pub mod data {
        include!("strategy_live_absorption_pressure/data.rs");
    }
    pub mod definition {
        include!("strategy_live_absorption_pressure/contract.rs");
    }
    pub mod parameters {
        include!("strategy_live_absorption_pressure/parameters.rs");
    }
}
pub mod strategy_donchian_55_btc200 {
    pub mod evaluate {
        include!("strategy_donchian_55_btc200/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_donchian_55_btc200/contract.rs");
    }
    pub mod parameters {
        include!("strategy_donchian_55_btc200/parameters.rs");
    }
}
pub mod strategy_donchian_20_sma200 {
    pub mod evaluate {
        include!("strategy_donchian_20_sma200/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_donchian_20_sma200/contract.rs");
    }
    pub mod parameters {
        include!("strategy_donchian_20_sma200/parameters.rs");
    }
}
pub mod strategy_donchian_20_btc200 {
    pub mod evaluate {
        include!("strategy_donchian_20_btc200/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_donchian_20_btc200/contract.rs");
    }
}
pub mod strategy_donchian_10_btc200 {
    pub mod evaluate {
        include!("strategy_donchian_10_btc200/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_donchian_10_btc200/contract.rs");
    }
}
pub mod strategy_donchian_20_btc200_hold3 {
    pub mod evaluate {
        include!("strategy_donchian_20_btc200_hold3/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_donchian_20_btc200_hold3/contract.rs");
    }
}
pub mod strategy_risk_adjusted_trend_portfolio {
    pub mod evaluate {
        include!("strategy_risk_adjusted_trend_portfolio/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_risk_adjusted_trend_portfolio/contract.rs");
    }
    pub mod parameters {
        include!("strategy_risk_adjusted_trend_portfolio/parameters.rs");
    }
}
pub mod strategy_trend_sleeves_btc200 {
    pub mod evaluate {
        include!("strategy_trend_sleeves_btc200/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_trend_sleeves_btc200/contract.rs");
    }
    pub mod parameters {
        include!("strategy_trend_sleeves_btc200/parameters.rs");
    }
}
pub mod strategy_trend_50_200 {
    pub mod evaluate {
        include!("strategy_trend_50_200/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_trend_50_200/contract.rs");
    }
    pub mod parameters {
        include!("strategy_trend_50_200/parameters.rs");
    }
}
pub mod strategy_relative_strength_90_sma200 {
    pub mod evaluate {
        include!("strategy_relative_strength_90_sma200/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_relative_strength_90_sma200/contract.rs");
    }
    pub mod parameters {
        include!("strategy_relative_strength_90_sma200/parameters.rs");
    }
}
pub mod strategy_risk_adjusted_monthly_momentum {
    pub mod evaluate {
        include!("strategy_risk_adjusted_monthly_momentum/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_risk_adjusted_monthly_momentum/contract.rs");
    }
}
pub mod strategy_rsi2_oversold_rebound {
    pub mod evaluate {
        include!("strategy_rsi2_oversold_rebound/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_rsi2_oversold_rebound/contract.rs");
    }
    pub mod parameters {
        include!("strategy_rsi2_oversold_rebound/parameters.rs");
    }
}
pub mod strategy_dated_futures_carry {
    pub mod evaluate {
        include!("strategy_dated_futures_carry/evaluate.rs");
    }
    pub mod definition {
        include!("strategy_dated_futures_carry/contract.rs");
    }
}
fn walk_forward_rejection_reason(strategy: Strategy, input: &StrategySignal<'_>) -> Option<String> {
    let evidence = input.walk_forward_evidence?;
    let specs = strategy.parameter_specs();
    let bounded_parameter = |key: StrategyParameterKey| {
        let spec = specs.iter().find(|spec| spec.key == key)?;
        let value = input.parameters.get(key.as_str()).copied()?;
        let step_position = (value - spec.min) / spec.step;
        (value.is_finite()
            && value >= spec.min
            && value <= spec.max
            && spec.step > 0.0
            && (step_position - step_position.round()).abs() <= 1e-6)
            .then_some(value)
    };
    let Some(minimum_samples) =
        bounded_parameter(StrategyParameterKey::ScoutMinimumWalkForwardSamples)
            .map(|value| value.round() as usize)
    else {
        return Some(
            "walk_forward_invalid_config: minimum sample count is missing or out of bounds".into(),
        );
    };
    let Some(max_evidence_age_seconds) =
        bounded_parameter(StrategyParameterKey::ScoutMaximumEvidenceAgeSeconds)
            .map(|value| value as u64)
    else {
        return Some(
            "walk_forward_invalid_config: maximum evidence age is missing or out of bounds".into(),
        );
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let age_ms = now_ms.saturating_sub(evidence.last_sample_timestamp_ms);

    // New and sparsely sampled profiles remain eligible for the separately
    // bounded exploration path. The quality gate activates only once enough
    // fresh evidence exists to estimate expectancy.
    if evidence.sample_count < minimum_samples || age_ms > max_evidence_age_seconds * 1_000 {
        return None;
    }
    if input
        .signal
        .is_some_and(|signal| evidence.last_sample_timestamp_ms >= signal.timestamp)
    {
        return Some(format!(
            "walk_forward_not_point_in_time: evidence timestamp {} is not before the candidate",
            evidence.last_sample_timestamp_ms
        ));
    }

    let effective_samples = evidence
        .effective_sample_size
        .unwrap_or(evidence.sample_count as f64);
    if !effective_samples.is_finite() || effective_samples < minimum_samples as f64 {
        return Some(format!(
            "walk_forward_insufficient_effective_samples: {} outcomes, effective n {:.1}, requires at least {}",
            evidence.sample_count, effective_samples, minimum_samples
        ));
    }
    let Some(lower_bound) = evidence
        .expectancy_ci_low_percent
        .filter(|value| value.is_finite())
    else {
        return Some(format!(
            "walk_forward_interval_unavailable: {} outcomes, lower confidence bound is missing",
            evidence.sample_count
        ));
    };
    (lower_bound <= 0.0).then(|| {
        format!(
            "walk_forward_expectancy_not_proven: lower 95% confidence bound {:.3}% requires > 0.000% (n={}, effective n={:.1}, age={}s)",
            lower_bound,
            evidence.sample_count,
            effective_samples,
            age_ms / 1_000
        )
    })
}

impl Strategy {
    pub fn all() -> impl Iterator<Item = Strategy> {
        STRATEGY_DEFINITIONS
            .iter()
            .map(|definition| definition.strategy)
    }
    pub fn live_strategies() -> impl Iterator<Item = Strategy> {
        Self::all().filter(|strategy| strategy.is_live_insight())
    }
    fn definition(self) -> &'static StrategyDefinition {
        STRATEGY_DEFINITIONS
            .iter()
            .find(|definition| definition.strategy == self)
            .expect("every Strategy variant must have a strategy definition")
    }
    pub fn as_str(self) -> &'static str {
        self.definition().name
    }
    pub fn from_name(name: &str) -> Option<Self> {
        STRATEGY_DEFINITIONS
            .iter()
            .find(|definition| definition.name == name)
            .map(|definition| definition.strategy)
    }
    pub fn is_dated_carry(self) -> bool {
        matches!(self, Self::DatedCarry)
    }
    pub fn is_live_insight(self) -> bool {
        self.definition().live_insight
    }
    pub fn receives_live_insight(self, signal: &Signal) -> bool {
        self.definition()
            .live_insight_fields
            .iter()
            .any(|field| field.is_present(signal))
    }
    pub fn any_live_strategy_receives(signal: &Signal) -> bool {
        STRATEGY_DEFINITIONS.iter().any(|definition| {
            definition
                .live_insight_fields
                .iter()
                .any(|field| field.is_present(signal))
        })
    }
    pub fn is_legacy_composite(self) -> bool {
        self.definition().legacy_composite
    }
    pub fn parameter_specs(self) -> Vec<StrategyParameterSpec> {
        parameter_catalog::complete_parameter_specs(self)
    }
    pub fn expanded_strategies(self) -> &'static [Strategy] {
        self.definition().expands_to
    }
    /// Entry and exit rules for this strategy are evaluated from signal context.
    pub fn evaluate<I: StrategyInputLike>(self, input: &I) -> Result<StrategyDecision, String> {
        input.with_strategy_signal(|input| {
            if input.position_open
                && input.signal.is_none()
                && input.daily_features.is_none()
                && input.specific_signal.is_none()
            {
                if self == Strategy::DatedCarry {
                    return Ok(StrategyDecision::Hold);
                }
                let exit_when_inactive = input
                    .signal_active
                    .is_some_and(|active| self.should_exit_when_insight_inactive(active));
                if let Some(decision) = input.position_decision(
                    self.should_exit_when_disabled(input.strategy_enabled),
                    exit_when_inactive,
                ) {
                    return Ok(decision);
                }
            }
            let decision = (self.definition().evaluate)(input)?;
            if decision != StrategyDecision::Enter {
                return Ok(decision);
            }
            if walk_forward_rejection_reason(self, input).is_some() {
                return Ok(StrategyDecision::Reject);
            }
            Ok(decision)
        })
    }
    pub fn entry_rejection_reason(self, input: &StrategySignal<'_>) -> Option<String> {
        let reason = match self {
            Self::LiveMomentum => strategy_live_momentum::evaluate::entry_rejection_reason(input),
            Self::LiveTradeFlow => {
                strategy_live_trade_flow::evaluate::entry_rejection_reason(input)
            }
            Self::LiveOrderBook => {
                strategy_live_order_book::evaluate::entry_rejection_reason(input)
            }
            Self::LiveVwapDislocation => {
                strategy_live_vwap_dislocation::evaluate::entry_rejection_reason(input)
            }
            Self::LiveAbsorptionPressure => {
                strategy_live_absorption_pressure::evaluate::entry_rejection_reason(input)
            }
            _ => return strategy_decision_rejection_reason(self, input),
        };
        if reason.is_some() {
            return reason;
        }

        if (self.definition().evaluate)(input).ok()? != StrategyDecision::Enter {
            return Some("Live strategy evaluator rejected the signal".to_owned());
        }
        walk_forward_rejection_reason(self, input)
    }
    pub fn contract(self) -> StrategyContract {
        (self.definition().contract)()
    }
    pub fn should_exit_when_disabled(self, enabled: bool) -> bool {
        (self.definition().should_exit_when_disabled)(enabled)
    }
    pub fn should_exit_when_insight_inactive(self, active: bool) -> bool {
        self.definition()
            .should_exit_when_insight_inactive
            .is_some_and(|evaluate_exit| evaluate_exit(active))
    }

    pub fn uses_protective_stops(self) -> bool {
        match self {
            Self::Donchian20Btc200 => {
                strategy_donchian_20_btc200::evaluate::uses_protective_stops()
            }
            Self::Donchian10Btc200 => {
                strategy_donchian_10_btc200::evaluate::uses_protective_stops()
            }
            Self::Donchian20Btc200Hold3 => {
                strategy_donchian_20_btc200_hold3::evaluate::uses_protective_stops()
            }
            Self::RiskAdjustedTrendPortfolio => {
                strategy_risk_adjusted_trend_portfolio::evaluate::uses_protective_stops()
            }
            Self::RiskAdjustedMonthlyMomentum => {
                strategy_risk_adjusted_monthly_momentum::evaluate::uses_protective_stops()
            }
            _ => true,
        }
    }

    pub fn uses_fixed_take_profit(self) -> bool {
        !matches!(
            self,
            Self::Donchian55Btc200
                | Self::Donchian20Btc200
                | Self::Donchian10Btc200
                | Self::Donchian20Btc200Hold3
                | Self::RiskAdjustedTrendPortfolio
                | Self::RiskAdjustedMonthlyMomentum
        )
    }
}

static STRATEGY_DEFINITIONS: [&StrategyDefinition; 17] = [
    &strategy_live_momentum::definition::DEFINITION,
    &strategy_live_trade_flow::definition::DEFINITION,
    &strategy_live_order_book::definition::DEFINITION,
    &strategy_live_vwap_dislocation::definition::DEFINITION,
    &strategy_live_absorption_pressure::definition::DEFINITION,
    &strategy_donchian_55_btc200::definition::DEFINITION,
    &strategy_donchian_20_sma200::definition::DEFINITION,
    &strategy_donchian_20_btc200::definition::DEFINITION,
    &strategy_donchian_10_btc200::definition::DEFINITION,
    &strategy_donchian_20_btc200_hold3::definition::DEFINITION,
    &strategy_risk_adjusted_trend_portfolio::definition::DEFINITION,
    &strategy_trend_sleeves_btc200::definition::DEFINITION,
    &strategy_trend_50_200::definition::DEFINITION,
    &strategy_relative_strength_90_sma200::definition::DEFINITION,
    &strategy_risk_adjusted_monthly_momentum::definition::DEFINITION,
    &strategy_rsi2_oversold_rebound::definition::DEFINITION,
    &strategy_dated_futures_carry::definition::DEFINITION,
];

#[cfg(test)]
mod registered_strategy_contract_tests {
    use super::*;

    #[test]
    fn every_registered_strategy_has_a_complete_contract() {
        assert_eq!(STRATEGY_DEFINITIONS.len(), Strategy::all().count());
        for definition in STRATEGY_DEFINITIONS {
            let contract = (definition.contract)();
            assert!(!definition.name.is_empty(), "strategy name");
            assert!(
                !contract.insight.trim().is_empty(),
                "{} insight",
                definition.name
            );
            assert!(
                !contract.entry.trim().is_empty(),
                "{} entry",
                definition.name
            );
            assert!(!contract.exit.trim().is_empty(), "{} exit", definition.name);
            assert!(
                !contract.entry_order.trim().is_empty(),
                "{} entry order",
                definition.name
            );
            assert!(
                !contract.exit_order.trim().is_empty(),
                "{} exit order",
                definition.name
            );
            assert!(
                !contract.behavior.trim().is_empty(),
                "{} behavior",
                definition.name
            );
            assert_eq!(definition.strategy.as_str(), definition.name);
        }
    }

    #[test]
    fn removed_live_insights_alias_is_not_a_registered_strategy() {
        assert!(Strategy::from_name("live-insights").is_none());
        assert!(Strategy::all().all(|strategy| strategy.as_str() != "live-insights"));
    }

    #[test]
    fn serialized_candidate_constructor_uses_the_registered_evaluator() {
        let candidate = serde_json::json!({
            "symbol": "BTC-USDT", "direction": "bullish", "strength": 1.0,
            "timestamp": 10, "stale": false, "strategy_candidate": true,
            "strategy": "donchian-55-btc200", "components": {},
            "strategy_decision": { "observed_close": 101.0, "primary_level": 100.0,
                "secondary_level": 100.0, "btc_gate_required": true, "btc_close": 101.0, "btc_sma": 100.0 }
        });
        let parameters = Strategy::Donchian55Btc200
            .parameter_specs()
            .into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect();
        let input = StrategySignal::from_candidate_json(candidate, parameters).unwrap();
        assert_eq!(
            evaluate(Strategy::Donchian55Btc200, &input).unwrap(),
            StrategyDecision::Enter
        );
    }
}

#[cfg(test)]
mod risk_adjusted_monthly_momentum_registry_tests {
    use super::*;

    #[test]
    fn strategy_is_registered_and_uses_signal_exit_without_fixed_stops() {
        let strategy = Strategy::from_name("risk-adjusted-monthly-momentum").unwrap();
        assert_eq!(strategy, Strategy::RiskAdjustedMonthlyMomentum);
        assert!(!strategy.uses_protective_stops());
        assert!(strategy.should_exit_when_insight_inactive(false));
        assert!(!strategy.should_exit_when_insight_inactive(true));
        assert!(strategy.should_exit_when_disabled(false));
    }
}

#[cfg(test)]
mod strategy_signal_contract_tests {
    use super::*;

    #[test]
    fn every_daily_strategy_enters_only_from_its_own_benchmark_signal() {
        let daily = Strategy::all()
            .filter(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry());
        let mut checked = 0;
        for strategy in daily {
            checked += 1;
            let own_candidate = benchmark_signal(strategy.as_str(), true);
            let other_candidate = benchmark_signal("another-strategy", true);
            let inactive = benchmark_signal(strategy.as_str(), false);
            let parameters = all_strategy_parameter_specs(strategy)
                .into_iter()
                .map(|spec| (spec.key.as_str().to_owned(), spec.default))
                .collect::<BTreeMap<_, _>>();
            let evaluate = |signal| {
                strategy
                    .evaluate(&StrategySignal::entry(signal, &parameters))
                    .unwrap()
            };
            assert_eq!(evaluate(&own_candidate), StrategyDecision::Enter);
            assert_eq!(evaluate(&other_candidate), StrategyDecision::Reject);
            assert_eq!(evaluate(&inactive), StrategyDecision::Reject);
        }
        assert!(checked > 0, "registry must include daily strategies");
    }

    #[test]
    fn every_registered_strategy_returns_an_explicit_position_decision() {
        let parameters = BTreeMap::new();
        for strategy in Strategy::all() {
            let eval = |strategy_enabled, signal_active, rebalance_required| {
                strategy
                    .evaluate(&StrategySignal::position(
                        &parameters,
                        strategy_enabled,
                        signal_active,
                        rebalance_required,
                    ))
                    .unwrap()
            };
            let inactive = eval(true, Some(false), false);
            if !strategy.is_live_insight() && !strategy.is_dated_carry() {
                assert_eq!(inactive, StrategyDecision::Exit, "{}", strategy.as_str());
            } else {
                assert_eq!(inactive, StrategyDecision::Hold, "{}", strategy.as_str());
            }
            assert_eq!(
                eval(true, Some(true), false),
                StrategyDecision::Hold,
                "{}",
                strategy.as_str()
            );
            if strategy != Strategy::DatedCarry {
                assert_eq!(
                    eval(false, None, false),
                    StrategyDecision::Exit,
                    "{}",
                    strategy.as_str()
                );
            }
            if strategy == Strategy::RiskAdjustedMonthlyMomentum {
                assert_eq!(eval(true, Some(true), true), StrategyDecision::Rebalance);
            }
        }
    }

    #[test]
    fn every_live_strategy_evaluates_its_signal_and_rejects_missing_signal() {
        for strategy in Strategy::live_strategies() {
            let parameters = strategy
                .parameter_specs()
                .into_iter()
                .map(|spec| (spec.key.as_str().to_owned(), spec.default))
                .collect::<BTreeMap<_, _>>();
            let mut components = SignalComponents::default();
            match strategy {
                Strategy::LiveMomentum => components.momentum_10s_percent = Some(0.1),
                Strategy::LiveTradeFlow => components.trade_flow_imbalance = Some(0.5),
                Strategy::LiveOrderBook => components.book_imbalance = Some(0.5),
                Strategy::LiveVwapDislocation => components.mispricing_vs_vwap_bps = Some(2.0),
                Strategy::LiveAbsorptionPressure => {
                    components.pressure_without_price_move = Some(-0.5)
                }
                _ => continue,
            }
            let signal = Signal {
                symbol: "BTC-USDT".into(),
                direction: "bullish".into(),
                strength: 1.0,
                timestamp: 1,
                stale: false,
                score: 1.0,
                ranging_probability: None,
                components,
                strategy_candidate: false,
                strategy: None,
                strategy_decision: None,
                market_regime: None,
                news_attribution: None,
                entry_gate: None,
            };
            let input = StrategySignal::entry(&signal, &parameters);
            assert_eq!(
                strategy.evaluate(&input).unwrap(),
                StrategyDecision::Enter,
                "{}",
                strategy.as_str()
            );
            let mut weak_signal = signal.clone();
            weak_signal.strength = 0.1;
            let weak_input = StrategySignal::entry(&weak_signal, &parameters);
            assert_eq!(
                strategy.evaluate(&weak_input).unwrap(),
                StrategyDecision::Reject,
                "{} accepted a signal below its learned strength threshold",
                strategy.as_str()
            );
            let rejection = strategy.entry_rejection_reason(&weak_input).unwrap();
            assert!(
                rejection.contains("Strength 0.100 < 0.300"),
                "{} returned unexpected strength rejection: {rejection}",
                strategy.as_str()
            );
            weak_signal.strength = f64::NAN;
            let non_finite_input = StrategySignal::entry(&weak_signal, &parameters);
            assert_eq!(
                strategy.evaluate(&non_finite_input).unwrap(),
                StrategyDecision::Reject,
                "{} accepted non-finite signal strength",
                strategy.as_str()
            );
            let mut opposing = signal.clone();
            match strategy {
                Strategy::LiveMomentum => opposing.components.momentum_10s_percent = Some(-0.1),
                Strategy::LiveTradeFlow => opposing.components.trade_flow_imbalance = Some(-0.5),
                Strategy::LiveOrderBook => opposing.components.book_imbalance = Some(-0.5),
                Strategy::LiveVwapDislocation => {
                    opposing.components.mispricing_vs_vwap_bps = Some(-2.0)
                }
                Strategy::LiveAbsorptionPressure => {
                    opposing.components.pressure_without_price_move = Some(0.5)
                }
                _ => continue,
            }
            assert_eq!(
                strategy
                    .evaluate(&StrategySignal::entry(&opposing, &parameters))
                    .unwrap(),
                StrategyDecision::Reject,
                "{} accepted an opposing feature signal",
                strategy.as_str()
            );
            let empty_parameters = BTreeMap::new();
            let missing_thresholds = StrategySignal::entry(&signal, &empty_parameters);
            assert_eq!(
                strategy.evaluate(&missing_thresholds).unwrap(),
                StrategyDecision::Reject,
                "{} must fail closed when its parameter profile is missing",
                strategy.as_str()
            );
            let missing = StrategySignal::position(&parameters, true, None, false);
            assert_eq!(
                strategy.evaluate(&missing).unwrap(),
                StrategyDecision::Hold,
                "{}",
                strategy.as_str()
            );
        }
    }

    #[test]
    fn live_rejections_report_observed_value_and_required_threshold() {
        let cases = [
            (
                Strategy::LiveMomentum,
                "bullish",
                SignalComponents {
                    momentum_10s_percent: Some(0.01),
                    trend_5m_volatility_units: Some(0.02),
                    ..Default::default()
                },
                StrategyParameterKey::MinimumMomentum,
                0.03,
                "0.0100%",
            ),
            (
                Strategy::LiveTradeFlow,
                "bullish",
                SignalComponents {
                    trade_flow_imbalance: Some(-0.2),
                    ..Default::default()
                },
                StrategyParameterKey::MinimumFlowImbalance,
                0.3,
                "-0.200",
            ),
            (
                Strategy::LiveOrderBook,
                "bullish",
                SignalComponents {
                    book_imbalance: Some(0.1),
                    ..Default::default()
                },
                StrategyParameterKey::MinimumBookImbalance,
                0.4,
                "0.100",
            ),
            (
                Strategy::LiveVwapDislocation,
                "bullish",
                SignalComponents {
                    mispricing_vs_vwap_bps: Some(-1.2),
                    ..Default::default()
                },
                StrategyParameterKey::MinimumVwapDislocationBps,
                2.0,
                "-1.20 bps",
            ),
            (
                Strategy::LiveAbsorptionPressure,
                "bullish",
                SignalComponents {
                    pressure_without_price_move: Some(0.1),
                    ..Default::default()
                },
                StrategyParameterKey::MinimumAbsorptionPressure,
                0.2,
                "-0.100",
            ),
        ];

        for (strategy, direction, components, parameter, threshold, expected_value) in cases {
            let signal = Signal {
                symbol: "XRP-USDC".to_owned(),
                direction: direction.to_owned(),
                strength: 1.0,
                timestamp: 1,
                stale: false,
                score: 1.0,
                ranging_probability: None,
                components,
                strategy_candidate: false,
                strategy: None,
                strategy_decision: None,
                market_regime: None,
                news_attribution: None,
                entry_gate: None,
            };
            let parameters = BTreeMap::from([
                (parameter.as_str().to_owned(), threshold),
                (
                    StrategyParameterKey::MinimumSignalStrength
                        .as_str()
                        .to_owned(),
                    0.0,
                ),
            ]);
            let input = StrategySignal::entry(&signal, &parameters);

            assert_eq!(strategy.evaluate(&input).unwrap(), StrategyDecision::Reject);
            let reason = strategy.entry_rejection_reason(&input).unwrap();
            assert!(
                reason.contains(expected_value),
                "{}: {reason}",
                strategy.as_str()
            );
            assert!(
                reason.contains(&format!("{threshold:.2}")),
                "{}: {reason}",
                strategy.as_str()
            );
        }
    }

    #[test]
    fn live_rejection_reports_failed_walk_forward_evidence() {
        let strategy = Strategy::LiveMomentum;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let signal = Signal {
            symbol: "XRP-USDC".to_owned(),
            direction: "bullish".to_owned(),
            strength: 1.0,
            timestamp: now_ms,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents {
                momentum_10s_percent: Some(0.01),
                ..Default::default()
            },
            strategy_candidate: false,
            strategy: None,
            strategy_decision: Some(serde_json::json!({
                "strategy_walk_forward": {
                    "sampleCount": 10000,
                    "effectiveSampleSize": 10000.0,
                    "positiveNetRate": 0.6,
                    "averageNetReturnPercent": 0.1,
                    "expectancyCiLowPercent": -0.1,
                    "lastSampleTimestampMs": now_ms - 1000
                }
            })),
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        };
        let parameters = strategy
            .parameter_specs()
            .into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let input = StrategySignal::entry(&signal, &parameters);

        assert_eq!(strategy.evaluate(&input).unwrap(), StrategyDecision::Reject);
        let reason = strategy.entry_rejection_reason(&input).unwrap();
        assert!(reason.contains("n=10000"));
        assert!(reason.contains("walk_forward_expectancy_not_proven"));
        assert!(reason.contains("lower 95% confidence bound -0.100%"));
    }

    #[test]
    fn every_registered_strategy_has_bounded_parameters() {
        for strategy in Strategy::all() {
            assert_parameter_bounds_for_strategy(strategy);
        }
    }

    fn assert_parameter_bounds_for_strategy(strategy: Strategy) {
        for spec in all_strategy_parameter_specs(strategy) {
            let key = format!("BTC-USDT:{}:{}", strategy.as_str(), spec.key.as_str());
            let mut values = BTreeMap::from([(key.clone(), spec.min)]);
            assert!(
                validate_strategy_parameter_overrides(&values).is_ok(),
                "{}:{} minimum",
                strategy.as_str(),
                spec.key.as_str()
            );
            values.insert(key.clone(), spec.max);
            assert!(
                validate_strategy_parameter_overrides(&values).is_ok(),
                "{}:{} maximum",
                strategy.as_str(),
                spec.key.as_str()
            );
            values.insert(key.clone(), spec.min - spec.step);
            assert!(
                validate_strategy_parameter_overrides(&values).is_err(),
                "{}:{} below minimum",
                strategy.as_str(),
                spec.key.as_str()
            );
            values.insert(key, spec.max + spec.step);
            assert!(
                validate_strategy_parameter_overrides(&values).is_err(),
                "{}:{} above maximum",
                strategy.as_str(),
                spec.key.as_str()
            );
        }
    }
}
