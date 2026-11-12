//! Versioned request/response contract for stateless live strategy evaluation.
//! Scout assignments intentionally use their separate contracts.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
    DailyStrategySignalFeatures, DatedCarryCandidate, DatedCarrySettlementSource, Strategy,
    StrategyMarketSignal, StrategyWalkForwardEvidence,
};

pub const LIVE_EVALUATION_CONTRACT_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveStrategyProfile {
    pub profile_id: String,
    pub profile_symbol: String,
    pub version: u64,
    pub parameters: BTreeMap<String, f64>,
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
