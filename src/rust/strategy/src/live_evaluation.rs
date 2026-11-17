//! Versioned request/response contract for stateless live strategy evaluation.
//! Scout assignments intentionally use their separate contracts.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
    DailyStrategySignalFeatures, DatedCarryCandidate, DatedCarrySettlementSource, Strategy,
    StrategyMarketSignal, StrategyWalkForwardEvidence,
};

pub const LIVE_EVALUATION_CONTRACT_VERSION: u16 = 1;
pub const STRATEGY_INSIGHT_UPDATE_VERSION: u16 = 1;

/// A derived, bounded Insights update delivered to strategy workers.
/// The event timestamp is the deduplication key within a symbol stream.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StrategyInsightUpdate {
    pub contract_version: u16,
    pub event_id: String,
    pub event_time_ms: u64,
    #[serde(default)]
    pub signal: Option<crate::StrategyMarketSignal>,
    #[serde(default)]
    pub market: Option<serde_json::Value>,
    #[serde(default)]
    pub taker_fee_rate: Option<f64>,
    #[serde(default)]
    pub daily_snapshot: Option<crate::daily::DailyStrategySnapshot>,
    #[serde(default)]
    pub dated_carry_scan: Option<DatedCarryScanUpdate>,
    #[serde(default)]
    pub dated_carry_candidate: Option<crate::DatedCarryCandidate>,
    #[serde(default)]
    pub dated_carry_settlement: Option<DatedCarrySettlementUpdate>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DatedCarryScanUpdate {
    pub emitted_at_ms: u64,
    pub candidates: Vec<crate::DatedCarryCandidate>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DatedCarrySettlementUpdate {
    pub emitted_at_ms: u64,
    pub spot_symbol: String,
    pub future_symbol: String,
    pub expiry_ms: u64,
    pub spot_exit_twap: f64,
    pub settlement_price: f64,
    pub settlement_source: crate::DatedCarrySettlementSource,
}

/// Coordinator-owned live configuration installed atomically by AutoTrader.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveStrategyProfileSet {
    pub contract_version: u16,
    pub set_version: u64,
    pub strategy: Strategy,
    pub enabled: bool,
    pub profiles: Vec<LiveStrategyProfile>,
    #[serde(default)]
    pub open_positions: Vec<LiveStrategyOpenPosition>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveStrategyOpenPosition {
    pub symbol: String,
    pub profile_id: String,
    pub profile_symbol: String,
}

/// A strategy decision returned to AutoTrader; it is never an executable order.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveStrategyDecisionReport {
    pub contract_version: u16,
    pub event_id: String,
    pub strategy: Strategy,
    pub symbol: String,
    pub profile_id: String,
    pub profile_version: u64,
    pub parameter_hash: String,
    pub event_time_ms: u64,
    pub decision: LiveStrategyDecision,
    pub reason: Option<String>,
    pub walk_forward_evidence: Option<crate::StrategyWalkForwardEvidence>,
    pub signal: crate::StrategyMarketSignal,
    #[serde(default)]
    pub dated_carry_scan: Option<DatedCarryScanUpdate>,
    #[serde(default)]
    pub dated_carry_candidate: Option<crate::DatedCarryCandidate>,
    #[serde(default)]
    pub dated_carry_settlement: Option<DatedCarrySettlementUpdate>,
}

/// Typed AutoTrader/worker RSocket operations. Profile sets flow from
/// AutoTrader to workers; decisions flow from workers back to AutoTrader.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", content = "payload", rename_all = "snake_case")]
pub enum LiveStrategySocketRequest {
    UpdateProfiles(LiveStrategyProfileSet),
    UpdateLearningSnapshot(crate::LearnedProfileSnapshot),
    ReportDecision(LiveStrategyDecisionReport),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "operation", content = "payload", rename_all = "snake_case")]
pub enum LiveStrategySocketResponse {
    ProfilesApplied {
        strategy: Strategy,
        set_version: u64,
    },
    LearningSnapshotApplied,
    DecisionAccepted {
        status: u16,
        detail: Option<String>,
    },
    Rejected {
        status: u16,
        detail: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveStrategyProfile {
    pub profile_id: String,
    pub profile_symbol: String,
    pub version: u64,
    pub parameters: BTreeMap<String, f64>,
    /// Updated by Learning and delivered through AutoTrader's profile push.
    #[serde(default)]
    pub walk_forward_evidence: Option<StrategyWalkForwardEvidence>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct LivePositionContext {
    pub is_open: bool,
    pub strategy_enabled: bool,
    pub signal_active: Option<bool>,
    pub rebalance_required: bool,
    pub daily_features: Option<DailyStrategySignalFeatures>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "stage", content = "input", rename_all = "snake_case")]
pub enum LiveEvaluationInput {
    Entry {
        signal: StrategyMarketSignal,
        daily_features: Option<DailyStrategySignalFeatures>,
        walk_forward_evidence: Option<StrategyWalkForwardEvidence>,
    },
    OpenPosition {
        position: LivePositionContext,
    },
    DatedCarryEntry {
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        candidate: DatedCarryCandidate,
    },
    Settlement {
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveEvaluationRequest {
    pub contract_version: u16,
    pub request_id: String,
    pub strategy: Strategy,
    pub symbol: String,
    pub profile: LiveStrategyProfile,
    pub input: LiveEvaluationInput,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveStrategyDecision {
    Enter,
    Hold,
    Exit,
    Rebalance,
    Reject,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveEvaluationErrorCode {
    InvalidRequest,
    UnsupportedVersion,
    WrongStrategy,
    InvalidSymbol,
    InvalidProfile,
    InvalidParameters,
    EvaluationFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LiveEvaluationError {
    pub code: LiveEvaluationErrorCode,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveEvaluationResponse {
    pub contract_version: u16,
    pub request_id: String,
    pub strategy: Strategy,
    pub symbol: String,
    pub profile_id: String,
    pub profile_symbol: String,
    pub profile_version: u64,
    pub decision: Option<LiveStrategyDecision>,
    pub error: Option<LiveEvaluationError>,
}

#[cfg(test)]
mod socket_contract_tests {
    use super::*;

    #[test]
    fn profile_push_and_decision_report_use_distinct_typed_socket_operations() {
        let profile_set = LiveStrategyProfileSet {
            contract_version: LIVE_EVALUATION_CONTRACT_VERSION,
            set_version: 42,
            strategy: Strategy::LiveMomentum,
            enabled: true,
            profiles: Vec::new(),
            open_positions: Vec::new(),
        };
        let profile_request = LiveStrategySocketRequest::UpdateProfiles(profile_set);
        let profile_wire = serde_json::to_value(&profile_request).unwrap();
        assert_eq!(profile_wire["operation"], "update_profiles");
        assert!(profile_wire.get("path").is_none());
        assert_eq!(
            serde_json::to_value(
                serde_json::from_value::<LiveStrategySocketRequest>(profile_wire).unwrap()
            )
            .unwrap(),
            serde_json::to_value(profile_request).unwrap()
        );

        let decision_request =
            LiveStrategySocketRequest::ReportDecision(LiveStrategyDecisionReport {
                contract_version: LIVE_EVALUATION_CONTRACT_VERSION,
                event_id: "event-1".into(),
                strategy: Strategy::LiveMomentum,
                symbol: "BTC-USDT".into(),
                profile_id: "profile-1".into(),
                profile_version: 1,
                parameter_hash: "profile-1".into(),
                event_time_ms: 1_700_000_000_000,
                decision: LiveStrategyDecision::Reject,
                reason: Some("neutral signal".into()),
                walk_forward_evidence: None,
                signal: crate::StrategyMarketSignal::default(),
                dated_carry_scan: None,
                dated_carry_candidate: None,
                dated_carry_settlement: None,
            });
        let decision_wire = serde_json::to_value(&decision_request).unwrap();
        assert_eq!(decision_wire["operation"], "report_decision");
        assert!(decision_wire.get("path").is_none());
        assert_eq!(
            serde_json::to_value(
                serde_json::from_value::<LiveStrategySocketRequest>(decision_wire).unwrap()
            )
            .unwrap(),
            serde_json::to_value(decision_request).unwrap()
        );
    }
}
