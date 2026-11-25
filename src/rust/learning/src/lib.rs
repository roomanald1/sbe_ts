//! Learning service and profile update engine. It is intentionally independent
//! of the trader's decision and order-execution state.

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};
#[cfg(test)]
use strategy::StrategyThresholdObservation;
use strategy::{
    ContextLearningStat, ContextMissedOpportunityBand, ContextThresholdInput,
    ContextThresholdRequest, ContextThresholdResponse, ContextThresholds, ExecutionLearningRequest,
    ExecutionLearningResponse, ExecutionLearningStat, ExecutionOutcomeEvent, FeedbackEvent,
    LearnedProfileSnapshot, ParameterSpec, ScoutComparisonMetadata, StrategyProfile,
    StrategyProfileRegistration, TradeReplayContext,
};
use tokio::sync::Mutex;

mod context;
mod execution;
mod exit_policy;
mod feedback;
mod profiles;
mod retention;
mod scout_coordinator;
mod scout_replay;
mod scout_replay_config;
mod thresholds;
mod walk_forward_statistics;

use context::*;
use execution::*;
use feedback::*;
use profiles::*;
pub use scout_coordinator::ScoutCoordinator;
pub use scout_replay_config::{
    ScoutReplayBootstrapMethod, ScoutReplaySelectionObjective, ScoutReplayTrainingWindow,
    ScoutWalkForwardReplayConfig,
};
use thresholds::*;
use walk_forward_statistics::*;

#[derive(Clone, Serialize, Deserialize)]
struct RegisteredProfile {
    profile: StrategyProfile,
    specs: Vec<ParameterSpec>,
    samples: usize,
    positive: usize,
    realized_pnl: f64,
    #[serde(default)]
    outcomes: Vec<LearningOutcome>,
}

#[derive(Clone, Serialize, Deserialize)]
struct LearningOutcome {
    observed_at_ms: u64,
    net_return_percent: f64,
    #[serde(default)]
    evidence_group_id: Option<String>,
    #[serde(default)]
    learning_context: Option<String>,
    #[serde(default)]
    market_regime: Option<String>,
    #[serde(default)]
    horizon_seconds: Option<u64>,
    source: LearningOutcomeSource,
    #[serde(default)]
    realized_pnl_usd: Option<f64>,
    #[serde(default)]
    paired_comparison: bool,
    #[serde(default)]
    tested_parameter_keys: Vec<String>,
    #[serde(default)]
    baseline_parameters: BTreeMap<String, f64>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum LearningOutcomeSource {
    Scout,
    Trade,
}

#[derive(Clone, Serialize, Deserialize)]
struct ThresholdTrainingSample {
    symbol: String,
    strategy: String,
    parameter_key: String,
    observed_at_ms: u64,
    threshold: f64,
    feature_value: f64,
    net_return_percent: f64,
}

#[derive(Serialize)]
struct StrategySummary {
    strategy: String,
    learning_samples: usize,
    positive_rate: Option<f64>,
    average_net_return_percent: Option<f64>,
    scout_outcome_samples: usize,
    scout_positive_rate: Option<f64>,
    scout_average_net_return_percent: Option<f64>,
    active_profile_trade_samples: usize,
    active_profile_trade_positive_rate: Option<f64>,
    active_profile_average_net_return_percent: Option<f64>,
    other_profile_trade_samples: usize,
    other_profile_trade_positive_rate: Option<f64>,
    other_profile_average_net_return_percent: Option<f64>,
    paired_scout_samples: usize,
    paired_scout_positive_rate: Option<f64>,
    average_paired_scout_lift_percent: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize)]
struct LearningState {
    #[serde(default = "default_live_scout_parameter_samples")]
    minimum_live_scout_parameter_samples: usize,
    variants: BTreeMap<String, RegisteredProfile>,
    active_profiles: BTreeMap<String, StrategyProfile>,
    #[serde(default)]
    context_learning: BTreeMap<String, ContextLearningStat>,
    // Bounded Learning-owned counterfactual evidence. Existing state files
    // deserialize this as empty; do not fabricate evidence from raw signals.
    #[serde(default)]
    missed_opportunity_bands: Vec<ContextMissedOpportunityBand>,
    #[serde(default)]
    execution_learning: BTreeMap<String, ExecutionLearningStat>,
    #[serde(default)]
    threshold_samples: Vec<ThresholdTrainingSample>,
    #[serde(default)]
    seen_execution_event_ids: BTreeSet<String>,
    #[serde(default)]
    exit_timing_observations: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    strategy_replay_jobs: BTreeMap<String, StrategyReplayJob>,
    #[serde(default)]
    missed_opportunity_jobs: BTreeMap<String, MissedOpportunityJob>,
    #[serde(default)]
    scout_replay_runs: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    evidence_requests: BTreeMap<String, EvidenceRequest>,
    seen_event_ids: BTreeSet<String>,
    accepted_events: u64,
    #[serde(skip)]
    outcome_history_truncated: bool,
    #[serde(skip)]
    seen_event_ids_overflowed: bool,
    #[serde(skip)]
    seen_execution_event_ids_overflowed: bool,
    #[serde(skip)]
    state_updates: Option<Arc<tokio::sync::Notify>>,
}

fn default_live_scout_parameter_samples() -> usize {
    MINIMUM_SCOUT_PARAMETER_SAMPLES
}

#[cfg(test)]
mod scout_sample_defaults_tests {
    use super::*;

    #[test]
    fn scout_sample_default_uses_the_enforced_minimum() {
        assert_eq!(default_live_scout_parameter_samples(), 10);
        assert_eq!(
            LearningState::default().minimum_live_scout_parameter_samples,
            10
        );
    }
}

#[cfg(test)]
mod evidence_request_tests {
    use super::*;

    #[test]
    fn equivalent_gaps_deduplicate_and_freshness_completes_them() {
        let mut state = LearningState::default();
        assert!(
            schedule_evidence_request(
                &mut state,
                "BTC-USDT",
                "live-momentum",
                "profile-1",
                "trend",
                "5m",
                "fresh",
                999,
            )
            .is_none()
        );
        assert!(state.evidence_requests.is_empty());
        let first = schedule_evidence_request(
            &mut state,
            "BTC-USDT",
            "live-momentum",
            "profile-1",
            "trend",
            "5m",
            "missing",
            1_000,
        )
        .unwrap();
        assert!(
            schedule_evidence_request(
                &mut state,
                "BTC-USDT",
                "live-momentum",
                "profile-1",
                "trend",
                "5m",
                "stale",
                1_001,
            )
            .is_none()
        );
        assert!(
            schedule_evidence_request(
                &mut state,
                "BTC-USDT",
                "live-momentum",
                "profile-1",
                "range",
                "5m",
                "missing",
                1_002,
            )
            .is_some()
        );
        assert!(start_evidence_request(&mut state, &first, 1_003));
        finish_evidence_request(&mut state, &first, 1_004, Ok(()));
        assert!(
            schedule_evidence_request(
                &mut state,
                "BTC-USDT",
                "live-momentum",
                "profile-1",
                "trend",
                "5m",
                "fresh",
                1_005,
            )
            .is_none()
        );
        assert_eq!(
            state.evidence_requests[&first].status,
            EvidenceRequestStatus::Completed
        );
    }

    #[test]
    fn evidence_request_retry_is_backed_off_and_capped() {
        let mut state = LearningState::default();
        let id = schedule_evidence_request(
            &mut state,
            "ETH-USDT",
            "live-momentum",
            "profile-2",
            "unknown",
            "default",
            "stale",
            10_000,
        )
        .unwrap();
        assert!(start_evidence_request(&mut state, &id, 10_000));
        finish_evidence_request(&mut state, &id, 10_001, Err("history unavailable".into()));
        assert!(
            schedule_evidence_request(
                &mut state,
                "ETH-USDT",
                "live-momentum",
                "profile-2",
                "unknown",
                "default",
                "stale",
                15_000,
            )
            .is_none()
        );
        assert!(
            schedule_evidence_request(
                &mut state,
                "ETH-USDT",
                "live-momentum",
                "profile-2",
                "unknown",
                "default",
                "stale",
                15_001,
            )
            .is_some()
        );
        assert_eq!(evidence_request_retry_delay_ms(1), 5_000);
        assert_eq!(evidence_request_retry_delay_ms(2), 10_000);
        assert_eq!(
            evidence_request_retry_delay_ms(100),
            EVIDENCE_REQUEST_RETRY_MAX_MS
        );
    }

    #[tokio::test]
    async fn evidence_requests_survive_learning_restart_and_running_work_is_requeued() {
        let path = std::env::temp_dir().join(format!(
            "learning-evidence-requests-{}-{}.sqlite",
            std::process::id(),
            now_ms()
        ));
        let mut state = LearningState::default();
        state.active_profiles.insert(
            "BTC-USDT:live-momentum".into(),
            StrategyProfile {
                profile_id: "profile-1".into(),
                version: 1,
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                parameters: BTreeMap::from([("minimum_momentum".into(), 0.002)]),
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            },
        );
        let id = schedule_evidence_request(
            &mut state,
            "BTC-USDT",
            "live-momentum",
            "profile-1",
            "trend",
            "30m",
            "missing",
            123,
        )
        .unwrap();
        assert!(start_evidence_request(&mut state, &id, 124));
        let connection = open_state_db(&path).unwrap();
        persist_normalized(&connection, &state).unwrap();
        drop(connection);

        let recovered = service_state(path.clone()).unwrap();
        let recovered_state = recovered.state.lock().await;
        assert_eq!(
            recovered_state.evidence_requests[&id].status,
            EvidenceRequestStatus::Pending
        );
        assert_eq!(recovered_state.evidence_requests[&id].attempts, 1);
        assert_eq!(
            recovered_state.evidence_requests[&id].reason.as_deref(),
            Some("resumed after Learning restarted")
        );
        drop(recovered_state);
        let telemetry = get_evidence_requests(axum::extract::State(recovered))
            .await
            .0;
        assert_eq!(telemetry.requests.len(), 1);
        assert_eq!(
            telemetry.requests[0].request.status,
            EvidenceRequestStatus::Pending
        );
        assert_eq!(
            telemetry.requests[0].coverage_status.as_deref(),
            Some("missing")
        );
        assert_eq!(telemetry.requests[0].sample_count, Some(0));
        assert_eq!(telemetry.requests[0].effective_sample_size, Some(0.0));
        assert_eq!(telemetry.requests[0].last_sample_timestamp_ms, None);
        assert_eq!(telemetry.requests[0].regime_horizon_coverage, "measured");
        assert_eq!(telemetry.total_coverage_cohorts, 1);
        assert_eq!(telemetry.omitted_coverage_cohorts, 0);
        assert_eq!(telemetry.coverage_cohorts[0].symbol, "BTC-USDT");
        assert_eq!(telemetry.coverage_cohorts[0].profile_id, "profile-1");
        assert_eq!(
            telemetry.coverage_cohorts[0].coverage_status.as_deref(),
            Some("missing")
        );
        assert_eq!(telemetry.coverage_cohorts[0].sample_count, Some(0));
        assert_eq!(telemetry.coverage_cohorts[0].last_sample_timestamp_ms, None);
        std::fs::remove_file(&path).unwrap();
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}

impl Default for LearningState {
    fn default() -> Self {
        Self {
            minimum_live_scout_parameter_samples: default_live_scout_parameter_samples(),
            variants: BTreeMap::new(),
            active_profiles: BTreeMap::new(),
            context_learning: BTreeMap::new(),
            missed_opportunity_bands: Vec::new(),
            execution_learning: BTreeMap::new(),
            threshold_samples: Vec::new(),
            seen_execution_event_ids: BTreeSet::new(),
            exit_timing_observations: BTreeMap::new(),
            strategy_replay_jobs: BTreeMap::new(),
            missed_opportunity_jobs: BTreeMap::new(),
            scout_replay_runs: BTreeMap::new(),
            evidence_requests: BTreeMap::new(),
            seen_event_ids: BTreeSet::new(),
            accepted_events: 0,
            outcome_history_truncated: false,
            seen_event_ids_overflowed: false,
            seen_execution_event_ids_overflowed: false,
            state_updates: None,
        }
    }
}

const MAX_EVIDENCE_REQUESTS: usize = 10_000;
const EVIDENCE_REQUEST_MAX_ATTEMPTS: u32 = 5;
const EVIDENCE_REQUEST_RETRY_BASE_MS: u64 = 5_000;
const EVIDENCE_REQUEST_RETRY_MAX_MS: u64 = 60 * 60 * 1_000;
const EVIDENCE_REQUEST_DISPATCH_TIMEOUT_MS: u64 = 6 * 60 * 60 * 1_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct EvidenceRequest {
    symbol: String,
    strategy: String,
    profile_id: String,
    regime: String,
    horizon: String,
    status: EvidenceRequestStatus,
    attempts: u32,
    created_at_ms: u64,
    updated_at_ms: u64,
    next_attempt_at_ms: u64,
    reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EvidenceRequestStatus {
    Pending,
    Running,
    Dispatched,
    Completed,
    Failed,
}

fn evidence_request_key(
    symbol: &str,
    strategy: &str,
    profile_id: &str,
    regime: &str,
    horizon: &str,
) -> String {
    [symbol, strategy, profile_id, regime, horizon]
        .iter()
        .map(|part| format!("{}:{part}", part.len()))
        .collect::<Vec<_>>()
        .join("|")
}

fn evidence_request_retry_delay_ms(attempts: u32) -> u64 {
    EVIDENCE_REQUEST_RETRY_BASE_MS
        .saturating_mul(1_u64 << attempts.saturating_sub(1).min(20))
        .min(EVIDENCE_REQUEST_RETRY_MAX_MS)
}

fn schedule_evidence_request(
    state: &mut LearningState,
    symbol: &str,
    strategy: &str,
    profile_id: &str,
    regime: &str,
    horizon: &str,
    freshness: &str,
    now_ms: u64,
) -> Option<String> {
    let id = evidence_request_key(symbol, strategy, profile_id, regime, horizon);
    let gap = matches!(freshness, "missing" | "stale");
    if !gap {
        if let Some(request) = state.evidence_requests.get_mut(&id)
            && request.status != EvidenceRequestStatus::Completed
        {
            request.status = EvidenceRequestStatus::Completed;
            request.updated_at_ms = now_ms;
            request.reason = Some("coverage is fresh".into());
        }
        return None;
    }
    if let Some(request) = state.evidence_requests.get_mut(&id) {
        if matches!(
            request.status,
            EvidenceRequestStatus::Pending | EvidenceRequestStatus::Running
        ) {
            return None;
        }
        if request.status == EvidenceRequestStatus::Dispatched {
            if now_ms < request.next_attempt_at_ms {
                return None;
            }
            request.status = EvidenceRequestStatus::Failed;
            request.updated_at_ms = now_ms;
            request.next_attempt_at_ms =
                now_ms.saturating_add(evidence_request_retry_delay_ms(request.attempts.max(1)));
            request.reason = Some("no fresh Scout outcome before dispatch timeout".into());
            return None;
        }
        if request.status == EvidenceRequestStatus::Completed {
            request.attempts = 0;
            request.next_attempt_at_ms = now_ms;
        } else if request.attempts >= EVIDENCE_REQUEST_MAX_ATTEMPTS
            || now_ms < request.next_attempt_at_ms
        {
            return None;
        }
        request.status = EvidenceRequestStatus::Pending;
        request.updated_at_ms = now_ms;
        request.reason = None;
        return Some(id);
    }
    if state.evidence_requests.len() >= MAX_EVIDENCE_REQUESTS {
        let evict = state
            .evidence_requests
            .iter()
            .filter(|(_, request)| {
                matches!(
                    request.status,
                    EvidenceRequestStatus::Completed | EvidenceRequestStatus::Failed
                )
            })
            .min_by_key(|(_, request)| request.updated_at_ms)
            .map(|(id, _)| id.clone())?;
        state.evidence_requests.remove(&evict);
    }
    state.evidence_requests.insert(
        id.clone(),
        EvidenceRequest {
            symbol: symbol.into(),
            strategy: strategy.into(),
            profile_id: profile_id.into(),
            regime: regime.into(),
            horizon: horizon.into(),
            status: EvidenceRequestStatus::Pending,
            attempts: 0,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            next_attempt_at_ms: now_ms,
            reason: None,
        },
    );
    Some(id)
}

fn start_evidence_request(state: &mut LearningState, id: &str, now_ms: u64) -> bool {
    let Some(request) = state.evidence_requests.get_mut(id) else {
        return false;
    };
    if request.status != EvidenceRequestStatus::Pending || now_ms < request.next_attempt_at_ms {
        return false;
    }
    request.status = EvidenceRequestStatus::Running;
    request.attempts = request.attempts.saturating_add(1);
    request.updated_at_ms = now_ms;
    true
}

fn finish_evidence_request(
    state: &mut LearningState,
    id: &str,
    now_ms: u64,
    result: Result<(), String>,
) {
    let Some(request) = state.evidence_requests.get_mut(id) else {
        return;
    };
    if request.status == EvidenceRequestStatus::Completed {
        return;
    }
    request.updated_at_ms = now_ms;
    match result {
        Ok(()) => {
            request.status = EvidenceRequestStatus::Dispatched;
            request.next_attempt_at_ms =
                now_ms.saturating_add(EVIDENCE_REQUEST_DISPATCH_TIMEOUT_MS);
            request.reason = Some("awaiting Scout outcome feedback".into());
        }
        Err(reason) => {
            request.status = EvidenceRequestStatus::Failed;
            request.next_attempt_at_ms =
                now_ms.saturating_add(evidence_request_retry_delay_ms(request.attempts.max(1)));
            request.reason = Some(reason);
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StrategyReplayStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StrategyReplayJob {
    event_id: String,
    /// Optional identity and provenance copied from versioned TradeOutcome
    /// feedback. Older stored jobs deserialize without invented attribution.
    #[serde(default)]
    schema_version: Option<u16>,
    #[serde(default)]
    trade_id: Option<String>,
    #[serde(default)]
    opportunity_id: Option<String>,
    #[serde(default)]
    episode_id: Option<String>,
    /// Profile captured at entry; kept distinct from the replay/training key.
    #[serde(default)]
    entry_profile_id: Option<String>,
    #[serde(default)]
    profile_hash: Option<String>,
    #[serde(default)]
    profile_version: Option<u64>,
    #[serde(default)]
    execution_mode: Option<String>,
    #[serde(default)]
    entry_policy: Option<String>,
    #[serde(default)]
    source: Option<String>,
    symbol: String,
    strategy: String,
    profile_id: String,
    parameters: BTreeMap<String, f64>,
    context: Option<TradeReplayContext>,
    status: StrategyReplayStatus,
    attempts: u32,
    created_at_ms: u64,
    updated_at_ms: u64,
    reason: Option<String>,
    result: Option<StrategyReplayResult>,
    #[serde(default)]
    history_cursor: Option<String>,
    #[serde(default = "default_replay_history_status")]
    history_status: String,
    #[serde(default)]
    pages_scanned: usize,
    #[serde(default)]
    records_scanned: usize,
    #[serde(default)]
    signals_scanned: usize,
    #[serde(default)]
    coverage_start_ms: Option<u64>,
    #[serde(default)]
    coverage_through_ms: Option<u64>,
    #[serde(default)]
    last_signal_timestamp_ms: Option<u64>,
    #[serde(default)]
    cadence_gap_detected: bool,
    #[serde(default)]
    cadence_gap_at_ms: Option<u64>,
    #[serde(default)]
    counterfactual_entry_found: bool,
    #[serde(default)]
    counterfactual_position_open: bool,
    #[serde(default)]
    counterfactual_exit_timestamp_ms: Option<u64>,
    #[serde(default)]
    counterfactual_exit_mark_price: Option<f64>,
    #[serde(default)]
    mark_at_actual_exit: Option<f64>,
    #[serde(default)]
    mark_at_observation_end: Option<f64>,
    #[serde(default)]
    archived_days_unscanned: usize,
    #[serde(default)]
    archived_days_unavailable: usize,
    #[serde(default)]
    corrupt_history_records: usize,
    #[serde(default)]
    horizon_mark_prices: BTreeMap<u64, f64>,
    #[serde(default)]
    horizon_mark_timestamps: BTreeMap<u64, u64>,
    #[serde(default)]
    horizons_ms: Vec<u64>,
}

fn default_replay_history_status() -> String {
    "pending".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StrategyReplayResult {
    classification: String,
    input_count: usize,
    evaluated_count: usize,
    first_exit_timestamp_ms: Option<u64>,
    estimated_mark_return_percent: Option<f64>,
    execution_costs_complete: bool,
    reason: Option<String>,
    horizons: Vec<StrategyReplayHorizonResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StrategyReplayHorizonResult {
    horizon_ms: u64,
    classification: String,
    counterfactual_exit_timestamp_ms: Option<u64>,
    estimated_mark_return_percent: Option<f64>,
    complete: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MissedOpportunityJob {
    event_id: String,
    observed_at_ms: u64,
    symbol: String,
    strategy: String,
    profile_id: String,
    parameters: BTreeMap<String, f64>,
    direction: String,
    strength: f64,
    score: f64,
    learning_context: String,
    status: StrategyReplayStatus,
    reason: Option<String>,
    horizons: BTreeMap<u64, MissedOpportunityHorizon>,
    #[serde(default)]
    history_cursor: Option<String>,
    #[serde(default)]
    pages_scanned: usize,
    #[serde(default)]
    coverage_start_ms: Option<u64>,
    #[serde(default)]
    coverage_through_ms: Option<u64>,
    #[serde(default)]
    corrupt_records: usize,
    #[serde(default)]
    archived_days_unscanned: usize,
    #[serde(default)]
    archived_days_unavailable: usize,
    #[serde(default)]
    cadence_gap_detected: bool,
    #[serde(default)]
    cadence_gap_at_ms: Option<u64>,
    #[serde(default)]
    reference_price: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MissedOpportunityHorizon {
    mark_price: Option<f64>,
    timestamp_ms: Option<u64>,
    gross_directional_return_percent: Option<f64>,
    complete: bool,
}

#[derive(Clone)]
struct ServiceState {
    state: Arc<Mutex<LearningState>>,
    path: PathBuf,
    state_updates: Arc<tokio::sync::Notify>,
}

#[derive(Deserialize, Serialize)]
struct FeedbackResult {
    accepted: bool,
    duplicate: bool,
    profile: Option<StrategyProfile>,
}

async fn accept_learning_feedback(
    State(service): State<ServiceState>,
    Json(event): Json<FeedbackEvent>,
) -> Result<Json<FeedbackResult>, (StatusCode, String)> {
    let scout_identity = match &event {
        FeedbackEvent::ScoutOutcome {
            symbol,
            strategy,
            profile_id,
            market_regime,
            ..
        } => Some((
            symbol.clone(),
            strategy.clone(),
            profile_id.clone(),
            market_regime.clone().unwrap_or_else(|| "unknown".into()),
        )),
        _ => None,
    };
    let result = feedback::accept_feedback(State(service.clone()), Json(event)).await?;
    if let Some((symbol, strategy, profile_id, regime)) = scout_identity {
        let mut state = service.state.lock().await;
        let now = now_ms();
        let mut changed = false;
        for request in state.evidence_requests.values_mut().filter(|request| {
            request.symbol == symbol
                && request.strategy == strategy
                && request.profile_id == profile_id
                && (request.regime == regime || request.regime == "unknown")
        }) {
            if request.status != EvidenceRequestStatus::Completed {
                request.status = EvidenceRequestStatus::Completed;
                request.updated_at_ms = now;
                request.reason = Some("Scout outcome recorded".into());
                changed = true;
            }
        }
        if changed {
            persist(&service.path, &state)?;
        }
    }
    Ok(result)
}

fn key(symbol: &str, strategy: &str) -> String {
    format!("{symbol}:{strategy}")
}

fn variant_key(profile: &StrategyProfile) -> String {
    format!(
        "{}:{}:{}:{:?}",
        profile.symbol, profile.strategy, profile.profile_id, profile.parameters
    )
}

fn event_parts(event: &FeedbackEvent) -> (&str, &str, &str, &BTreeMap<String, f64>, String) {
    match event {
        FeedbackEvent::ScoutOutcome {
            event_id,
            symbol,
            strategy,
            profile_id,
            parameters,
            ..
        } => (event_id, symbol, strategy, parameters, profile_id.clone()),
        FeedbackEvent::TradeOutcome {
            event_id,
            symbol,
            strategy,
            profile_id,
            parameters,
            ..
        } => (event_id, symbol, strategy, parameters, profile_id.clone()),
        FeedbackEvent::MissedOpportunityCandidate {
            event_id,
            symbol,
            strategy,
            profile_id,
            parameters,
            ..
        } => (event_id, symbol, strategy, parameters, profile_id.clone()),
    }
}

pub fn router(path: PathBuf) -> Result<Router, String> {
    let service = service_state(path)?;
    Ok(service_router(service))
}

pub fn router_with_scout_coordinator(
    path: PathBuf,
    worker_endpoints: &str,
) -> Result<(Router, ScoutCoordinator), String> {
    let service = service_state(path)?;
    let app = service_router(service.clone());
    let coordinator = ScoutCoordinator::new(service, worker_endpoints, app.clone())?;
    Ok((app, coordinator))
}

fn service_state(path: PathBuf) -> Result<ServiceState, String> {
    let mut stored = load_state(&path)?;
    let state_updates = Arc::new(tokio::sync::Notify::new());
    stored.state_updates = Some(Arc::clone(&state_updates));
    for job in stored.strategy_replay_jobs.values_mut() {
        if job.status == StrategyReplayStatus::Running {
            job.status = StrategyReplayStatus::Pending;
            job.updated_at_ms = now_ms();
            job.reason =
                Some("resumed from its durable history cursor after Learning restarted".into());
        }
    }
    for job in stored.missed_opportunity_jobs.values_mut() {
        if job.status == StrategyReplayStatus::Running {
            job.status = StrategyReplayStatus::Pending;
            job.reason = Some(
                "resumed from durable snapshot-history cursor after Learning restarted".into(),
            );
        }
    }
    for request in stored.evidence_requests.values_mut() {
        if request.status == EvidenceRequestStatus::Running {
            request.status = EvidenceRequestStatus::Pending;
            request.updated_at_ms = now_ms();
            request.next_attempt_at_ms = request.updated_at_ms;
            request.reason = Some("resumed after Learning restarted".into());
        }
    }
    feedback::reconcile_scout_profiles(&mut stored);
    if !stored.outcome_history_truncated {
        rebuild_trade_profile_metrics(&mut stored, now_ms());
    }
    feedback::persist(&path, &stored).map_err(|(_, error)| error)?;
    Ok(ServiceState {
        state: Arc::new(Mutex::new(stored)),
        path,
        state_updates,
    })
}

fn service_router(service: ServiceState) -> Router {
    Router::new()
        .route("/profiles", get(get_profiles).post(register_profile))
        .route("/decision-snapshot", get(get_decision_snapshot))
        .route("/summary", get(get_summary))
        .route("/feedback", post(accept_learning_feedback))
        .route("/exit-timing", post(accept_exit_timing_observation))
        .route("/strategy-replays", get(get_strategy_replays))
        .route("/evidence-requests", get(get_evidence_requests))
        .route("/strategy-replays/{event_id}", get(get_strategy_replay))
        .route(
            "/strategy-replays/{event_id}/retry",
            post(retry_strategy_replay),
        )
        .route(
            "/config/scout-parameter-samples",
            axum::routing::put(update_scout_parameter_samples),
        )
        .route("/context-thresholds", post(context_thresholds))
        .route("/execution-decision", post(execution_decision))
        .route("/execution-outcomes", post(execution_outcome))
        .with_state(service)
}

/// Operations-only HTTP surface. Domain requests are served through the
/// Learning RSocket coordinator.
pub fn operational_router() -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/process-memory-bytes", get(process_memory_bytes))
}

async fn process_cpu_seconds() -> String {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return "0".to_owned();
    }
    let usage = unsafe { usage.assume_init() };
    let user = usage.ru_utime.tv_sec as f64 + usage.ru_utime.tv_usec as f64 / 1_000_000.0;
    let system = usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1_000_000.0;
    (user + system).to_string()
}

async fn process_memory_bytes() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmRSS:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
        })
        .map(|kilobytes| (kilobytes * 1024).to_string())
        .unwrap_or_else(|| "0".to_owned())
}

fn open_state_db(path: &Path) -> Result<rusqlite::Connection, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create learning state directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let connection = rusqlite::Connection::open(path).map_err(|error| {
        format!(
            "could not open learning database {}: {error}",
            path.display()
        )
    })?;
    //TODO - do we really need to do then whenever we open the db
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS learning_metadata (id INTEGER PRIMARY KEY CHECK (id = 1), accepted_events INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_config (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_variants (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_variant_recency (id TEXT PRIMARY KEY, last_seen_ms INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS learning_variant_recency_latest ON learning_variant_recency(last_seen_ms DESC, id DESC);
             CREATE TABLE IF NOT EXISTS learning_active_profiles (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_context_stats (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_execution_stats (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_threshold_samples (id INTEGER PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_seen_event_ids (id TEXT PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS learning_seen_execution_event_ids (id TEXT PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS learning_exit_timing_observations (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_strategy_replay_jobs (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_scout_replay_runs (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_missed_opportunity_jobs (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_evidence_requests (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_missed_opportunity_bands (id INTEGER PRIMARY KEY, payload TEXT NOT NULL);",
        )
        .map_err(|error| {
            format!(
                "could not initialize learning database {}: {error}",
                path.display()
            )
        })?;
    Ok(connection)
}

fn durable_event_id_exists(path: &Path, table: &str, event_id: &str) -> Result<bool, String> {
    let table = match table {
        "learning_seen_event_ids" | "learning_seen_execution_event_ids" => table,
        _ => return Err(format!("unsupported Learning event-ID table {table}")),
    };
    let connection = open_state_db(path)?;
    connection
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id = ?1)"),
            [event_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("could not check durable Learning event ID: {error}"))
}

fn backfill_variant_recency(connection: &rusqlite::Connection) -> Result<(), String> {
    connection
        .execute(
            "INSERT OR IGNORE INTO learning_variant_recency (id, last_seen_ms)
             SELECT v.id, COALESCE((
                 SELECT MAX(CAST(json_extract(outcome.value, '$.observed_at_ms') AS INTEGER))
                 FROM json_each(v.payload, '$.outcomes') AS outcome
             ), 0)
             FROM learning_variants AS v
             WHERE NOT EXISTS (
                 SELECT 1 FROM learning_variant_recency AS r WHERE r.id = v.id
             )",
            [],
        )
        .map_err(|error| format!("could not backfill Learning variant recency: {error}"))?;
    Ok(())
}

fn load_recent_map<T: for<'de> Deserialize<'de>>(
    connection: &rusqlite::Connection,
    table: &str,
    limit: usize,
    target: &mut BTreeMap<String, T>,
) -> Result<(), String> {
    let updated_at = match table {
        "learning_context_stats" | "learning_execution_stats" => "$.updated_at_ms",
        "learning_exit_timing_observations" => "$.observed_at_ms",
        _ => return Err(format!("unsupported bounded Learning table {table}")),
    };
    let mut statement = connection
        .prepare(&format!(
            "SELECT id, payload FROM {table}
             ORDER BY CAST(json_extract(payload, '{updated_at}') AS INTEGER) DESC, id DESC
             LIMIT ?1"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (id, payload) = row.map_err(|error| error.to_string())?;
        target.insert(
            id,
            serde_json::from_str(&payload).map_err(|error| error.to_string())?,
        );
    }
    Ok(())
}

fn load_recent_rows<T: for<'de> Deserialize<'de>>(
    connection: &rusqlite::Connection,
    table: &str,
    limit: usize,
) -> Result<Vec<T>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT payload FROM {table} ORDER BY id DESC LIMIT ?1"
        ))
        .map_err(|error| error.to_string())?;
    let mut rows = statement
        .query_map([limit as i64], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .map(|row| {
            let payload = row.map_err(|error| error.to_string())?;
            serde_json::from_str(&payload).map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    rows.reverse();
    Ok(rows)
}

fn load_recent_variants(
    connection: &rusqlite::Connection,
    limit: usize,
    target: &mut BTreeMap<String, RegisteredProfile>,
) -> Result<bool, String> {
    let available = limit.saturating_sub(target.len());
    if available == 0 {
        return Ok(false);
    }
    let mut statement = connection
        .prepare(
            "SELECT v.id, v.payload FROM learning_variants AS v
             LEFT JOIN learning_variant_recency AS r ON r.id = v.id
             ORDER BY COALESCE(r.last_seen_ms, 0) DESC, v.id DESC LIMIT ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([available as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut loaded = 0usize;
    let mut truncated = false;
    for row in rows {
        let (id, payload) = row.map_err(|error| error.to_string())?;
        let mut variant: RegisteredProfile =
            serde_json::from_str(&payload).map_err(|error| error.to_string())?;
        truncated |= retention::compact_variant_outcomes(&mut variant, now_ms());
        target.entry(id).or_insert(variant);
        loaded += 1;
        if loaded % 100 == 0 {
            truncated |= retention::compact_outcome_working_set(target);
        }
    }
    truncated |= retention::compact_outcome_working_set(target);
    Ok(truncated)
}

fn load_one_variant_entry(
    connection: &rusqlite::Connection,
    id: &str,
    target: &mut BTreeMap<String, RegisteredProfile>,
) -> Result<bool, String> {
    use rusqlite::OptionalExtension;
    let payload = connection
        .query_row(
            "SELECT payload FROM learning_variants WHERE id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(payload) = payload {
        let mut variant: RegisteredProfile =
            serde_json::from_str(&payload).map_err(|error| error.to_string())?;
        let mut truncated = retention::compact_variant_outcomes(&mut variant, now_ms());
        target.insert(id.to_owned(), variant);
        if target.len() % 100 == 0 {
            truncated |= retention::compact_outcome_working_set(target);
        }
        return Ok(truncated);
    }
    Ok(false)
}

async fn accept_exit_timing_observation(
    State(service): State<ServiceState>,
    Json(observation): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let event_id = observation["event_id"].as_str().unwrap_or_default();
    let valid_number = |key: &str| observation[key].as_f64().is_some_and(f64::is_finite);
    if event_id.is_empty()
        || event_id.len() > 256 //TODO remove hardcoding
        || observation["symbol"].as_str().is_none()
        || observation["strategy"].as_str().is_none()
        || !valid_number("exit_return_percent")
        || !valid_number("post_exit_return_percent")
        || !valid_number("missed_return_percent")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid exit-timing observation".into(),
        ));
    }
    if !strategy::is_autotrader_symbol_allowed(observation["symbol"].as_str().unwrap_or_default()) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "symbol is outside the AutoTrader allowlist".into(),
        ));
    }
    if serde_json::to_vec(&observation).map_or(true, |encoded| encoded.len() > 16 * 1024) {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "exit-timing observation exceeds the 16 KiB limit".into(),
        ));
    }
    let duplicate = persist_exit_timing_observation(&service.path, event_id, &observation)?;
    let mut state = service.state.lock().await;
    if !duplicate {
        state
            .exit_timing_observations
            .insert(event_id.to_owned(), observation);
        while state.exit_timing_observations.len() > 10_000 {
            if let Some(oldest) = state
                .exit_timing_observations
                .iter()
                .min_by_key(|(id, value)| {
                    (
                        value["observed_at_ms"].as_u64().unwrap_or_default(),
                        id.as_str(),
                    )
                })
                .map(|(id, _)| id.clone())
            {
                state.exit_timing_observations.remove(&oldest);
            }
        }
        if let Some(updates) = &state.state_updates {
            updates.notify_one();
        }
    }
    Ok(Json(
        serde_json::json!({"accepted": true, "duplicate": duplicate}),
    ))
}

fn persist_exit_timing_observation(
    path: &Path,
    event_id: &str,
    observation: &serde_json::Value,
) -> Result<bool, (StatusCode, String)> {
    let connection =
        open_state_db(path).map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let payload = serde_json::to_string(observation)
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    let inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO learning_exit_timing_observations (id, payload) VALUES (?1, ?2)",
            rusqlite::params![event_id, payload],
        )
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not durably accept exit-timing observation: {error}"),
            )
        })?;
    transaction.commit().map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not commit exit-timing observation: {error}"),
        )
    })?;
    Ok(inserted == 0)
}

async fn get_strategy_replays(State(service): State<ServiceState>) -> Json<Vec<StrategyReplayJob>> {
    let state = service.state.lock().await;
    let mut jobs = state
        .strategy_replay_jobs
        .values()
        .cloned()
        .collect::<Vec<_>>();
    jobs.sort_by_key(|job| std::cmp::Reverse(job.updated_at_ms));
    jobs.truncate(100);
    Json(jobs)
}

#[derive(Serialize)]
struct EvidenceRequestCoverageView {
    request: EvidenceRequest,
    as_of_ms: u64,
    coverage_scope: &'static str,
    coverage_status: Option<String>,
    gap_reason: Option<String>,
    sample_count: Option<usize>,
    effective_sample_size: Option<f64>,
    last_sample_timestamp_ms: Option<u64>,
    last_sample_age_ms: Option<u64>,
    regime_horizon_coverage: &'static str,
}

#[derive(Serialize)]
struct EvidenceRequestsResponse {
    as_of_ms: u64,
    total_count: usize,
    omitted_count: usize,
    requests: Vec<EvidenceRequestCoverageView>,
    total_coverage_cohorts: usize,
    omitted_coverage_cohorts: usize,
    coverage_cohorts: Vec<EvidenceCoverageCohortView>,
}

#[derive(Serialize)]
struct EvidenceCoverageCohortView {
    symbol: String,
    strategy: String,
    profile_id: String,
    regime: Option<String>,
    horizon: Option<String>,
    coverage_scope: &'static str,
    coverage_status: Option<String>,
    gap_reason: Option<String>,
    sample_count: Option<usize>,
    effective_sample_size: Option<f64>,
    last_sample_timestamp_ms: Option<u64>,
    last_sample_age_ms: Option<u64>,
    regime_horizon_coverage: &'static str,
}

async fn get_evidence_requests(
    State(service): State<ServiceState>,
) -> Json<EvidenceRequestsResponse> {
    const RESPONSE_LIMIT: usize = 100;
    const COVERAGE_COHORT_LIMIT: usize = 500;
    let as_of_ms = now_ms();
    let state = service.state.lock().await;
    let priority_snapshot = walk_forward_priority_snapshot(&state, as_of_ms);
    let mut requests = state
        .evidence_requests
        .values()
        .cloned()
        .map(|request| {
            let key = format!(
                "{}|{}|profile:{}|regime:{}|horizon:{}",
                request.symbol,
                request.strategy,
                request.profile_id,
                request.regime,
                request.horizon
            );
            let coverage = priority_snapshot.get(&key);
            let coverage_status = coverage
                .and_then(|coverage| coverage["freshnessStatus"].as_str())
                .map(str::to_owned)
                .or_else(|| Some("missing".to_owned()));
            let sample_count = coverage
                .and_then(|coverage| coverage["sampleCount"].as_u64())
                .and_then(|count| usize::try_from(count).ok())
                .or_else(|| Some(0));
            let effective_sample_size = coverage
                .and_then(|coverage| coverage["effectiveSampleSize"].as_f64())
                .or_else(|| Some(0.0));
            let last_sample_timestamp_ms = coverage
                .and_then(|coverage| coverage["lastSampleTimestampMs"].as_u64())
                .filter(|timestamp| *timestamp > 0);
            let gap_reason = match coverage_status.as_deref() {
                Some("missing") => Some("No eligible Scout outcomes are recorded.".to_owned()),
                Some("stale") => Some("The latest eligible Scout outcome is stale.".to_owned()),
                Some("fresh") => Some("Fresh aggregate Scout coverage is available.".to_owned()),
                _ => None,
            };
            EvidenceRequestCoverageView {
                request,
                as_of_ms,
                coverage_scope: "symbol_strategy_profile_regime_horizon",
                coverage_status,
                gap_reason,
                sample_count,
                effective_sample_size,
                last_sample_timestamp_ms,
                last_sample_age_ms: last_sample_timestamp_ms
                    .map(|timestamp| as_of_ms.saturating_sub(timestamp)),
                regime_horizon_coverage: "measured",
            }
        })
        .collect::<Vec<_>>();
    requests.sort_by_key(|view| std::cmp::Reverse(view.request.updated_at_ms));
    let total_count = requests.len();
    requests.truncate(RESPONSE_LIMIT);
    let mut coverage_cohorts = priority_snapshot
        .iter()
        .filter_map(|(key, coverage)| {
            let (symbol_strategy, context) = key.split_once("|profile:")?;
            let (symbol, strategy) = symbol_strategy.split_once('|')?;
            let mut segments = context.split('|');
            let profile_id = segments.next()?;
            let mut regime = None;
            let mut horizon = None;
            for segment in segments {
                if let Some(value) = segment.strip_prefix("regime:") {
                    regime = Some(value.to_owned());
                } else if let Some(value) = segment.strip_prefix("horizon:") {
                    horizon = Some(value.to_owned());
                }
            }
            let coverage_scope = match (regime.is_some(), horizon.is_some()) {
                (true, true) => "symbol_strategy_profile_regime_horizon",
                (false, false) => "symbol_strategy_profile_aggregate",
                _ => "symbol_strategy_profile_context",
            };
            let coverage_status = coverage
                .get("freshnessStatus")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let sample_count = coverage
                .get("sampleCount")
                .and_then(serde_json::Value::as_u64)
                .and_then(|count| usize::try_from(count).ok());
            let effective_sample_size = coverage
                .get("effectiveSampleSize")
                .and_then(serde_json::Value::as_f64);
            let last_sample_timestamp_ms = coverage
                .get("lastSampleTimestampMs")
                .and_then(serde_json::Value::as_u64)
                .filter(|timestamp| *timestamp > 0);
            let gap_reason = match coverage_status.as_deref() {
                Some("missing") => Some("No eligible Scout outcomes are recorded.".to_owned()),
                Some("stale") => Some("The latest eligible Scout outcome is stale.".to_owned()),
                Some("fresh") => Some("Fresh aggregate Scout coverage is available.".to_owned()),
                _ => None,
            };
            Some(EvidenceCoverageCohortView {
                symbol: symbol.to_owned(),
                strategy: strategy.to_owned(),
                profile_id: profile_id.to_owned(),
                regime,
                horizon,
                coverage_scope,
                coverage_status,
                gap_reason,
                sample_count,
                effective_sample_size,
                last_sample_timestamp_ms,
                last_sample_age_ms: last_sample_timestamp_ms
                    .map(|timestamp| as_of_ms.saturating_sub(timestamp)),
                regime_horizon_coverage: if coverage_scope
                    == "symbol_strategy_profile_regime_horizon"
                {
                    "measured"
                } else {
                    "unavailable"
                },
            })
        })
        .collect::<Vec<_>>();
    coverage_cohorts.sort_by(|left, right| {
        let freshness_priority = |status: Option<&str>| match status {
            Some("missing") => 0,
            Some("stale") => 1,
            Some("fresh") => 2,
            _ => 3,
        };
        freshness_priority(left.coverage_status.as_deref())
            .cmp(&freshness_priority(right.coverage_status.as_deref()))
            .then_with(|| {
                left.effective_sample_size
                    .unwrap_or(f64::INFINITY)
                    .total_cmp(&right.effective_sample_size.unwrap_or(f64::INFINITY))
            })
            .then_with(|| left.symbol.cmp(&right.symbol))
            .then_with(|| left.strategy.cmp(&right.strategy))
            .then_with(|| left.profile_id.cmp(&right.profile_id))
            .then_with(|| left.regime.cmp(&right.regime))
            .then_with(|| left.horizon.cmp(&right.horizon))
    });
    let total_coverage_cohorts = coverage_cohorts.len();
    coverage_cohorts.truncate(COVERAGE_COHORT_LIMIT);
    Json(EvidenceRequestsResponse {
        as_of_ms,
        total_count,
        omitted_count: total_count.saturating_sub(requests.len()),
        requests,
        total_coverage_cohorts,
        omitted_coverage_cohorts: total_coverage_cohorts.saturating_sub(coverage_cohorts.len()),
        coverage_cohorts,
    })
}

async fn get_strategy_replay(
    State(service): State<ServiceState>,
    AxumPath(event_id): AxumPath<String>,
) -> Result<Json<StrategyReplayJob>, StatusCode> {
    service
        .state
        .lock()
        .await
        .strategy_replay_jobs
        .get(&event_id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn retry_strategy_replay(
    State(service): State<ServiceState>,
    AxumPath(event_id): AxumPath<String>,
) -> Result<Json<StrategyReplayJob>, (StatusCode, String)> {
    let mut state = service.state.lock().await;
    let job = state.strategy_replay_jobs.get_mut(&event_id).ok_or((
        StatusCode::NOT_FOUND,
        "strategy replay job not found".into(),
    ))?;
    if job.status != StrategyReplayStatus::Failed || job.attempts >= 5 {
        return Err((
            StatusCode::CONFLICT,
            "only failed jobs with fewer than five attempts can be retried".into(),
        ));
    }
    job.status = StrategyReplayStatus::Pending;
    job.reason = None;
    job.updated_at_ms = now_ms();
    let result = job.clone();
    persist(&service.path, &state)?;
    Ok(Json(result))
}

fn load_state(path: &Path) -> Result<LearningState, String> {
    use rusqlite::OptionalExtension;

    let connection = open_state_db(path)?;
    if let Some(accepted_events) = connection
        .query_row(
            "SELECT accepted_events FROM learning_metadata WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("could not read learning metadata: {error}"))?
    {
        let mut state = LearningState {
            accepted_events: accepted_events.max(0) as u64,
            ..LearningState::default()
        };
        //TODO we need to remove direct DB queries in the code. Should be behind a set of helper functions. Dont like it in the code like this
        state.minimum_live_scout_parameter_samples = connection.query_row(
            "SELECT value FROM learning_config WHERE key = 'minimum_live_scout_parameter_samples'",
            [], |row| row.get::<_, i64>(0),
        ).optional().map_err(|error| format!("could not read learning configuration: {error}"))?
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| {
                (MINIMUM_SCOUT_PARAMETER_SAMPLES..=MAXIMUM_SCOUT_PARAMETER_SAMPLES)
                    .contains(value)
            })
            .unwrap_or_else(default_live_scout_parameter_samples);
        state.outcome_history_truncated = connection
            .query_row(
                "SELECT value FROM learning_config WHERE key = 'outcome_history_truncated'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("could not read Learning retention marker: {error}"))?
            .unwrap_or_default()
            != 0;
        state.seen_event_ids_overflowed = connection
            .query_row(
                "SELECT value FROM learning_config WHERE key = 'seen_event_ids_overflowed'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("could not read Learning event-ID retention marker: {error}"))?
            .unwrap_or_default()
            != 0;
        state.seen_execution_event_ids_overflowed = connection
            .query_row(
                "SELECT value FROM learning_config WHERE key = 'seen_execution_event_ids_overflowed'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("could not read Learning execution event-ID retention marker: {error}"))?
            .unwrap_or_default()
            != 0;
        let active_profile_rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM learning_active_profiles", [], |row| {
                row.get(0)
            })
            .map_err(|error| format!("could not count Learning active profiles: {error}"))?;
        if active_profile_rows as usize > retention::MAX_ACTIVE_PROFILES {
            return Err(format!(
                "Learning database has {active_profile_rows} active profiles; in-memory limit is {}",
                retention::MAX_ACTIVE_PROFILES
            ));
        }
        load_map(
            &connection,
            "learning_active_profiles",
            &mut state.active_profiles,
        )?;
        load_recent_map(
            &connection,
            "learning_context_stats",
            retention::MAX_CONTEXT_STATS,
            &mut state.context_learning,
        )?;
        load_recent_map(
            &connection,
            "learning_execution_stats",
            retention::MAX_EXECUTION_STATS,
            &mut state.execution_learning,
        )?;
        state.threshold_samples =
            load_recent_rows(&connection, "learning_threshold_samples", 100_000)?;
        state.seen_event_ids = load_ids(&connection, "learning_seen_event_ids")?;
        state.seen_execution_event_ids =
            load_ids(&connection, "learning_seen_execution_event_ids")?;
        state.seen_event_ids_overflowed |= connection
            .query_row("SELECT COUNT(*) FROM learning_seen_event_ids", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| format!("could not count Learning feedback IDs: {error}"))?
            as usize
            > 100_000;
        state.seen_execution_event_ids_overflowed |= connection
            .query_row(
                "SELECT COUNT(*) FROM learning_seen_execution_event_ids",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("could not count Learning execution IDs: {error}"))?
            as usize
            > 100_000;
        load_recent_map(
            &connection,
            "learning_exit_timing_observations",
            10_000,
            &mut state.exit_timing_observations,
        )?;
        load_map(
            &connection,
            "learning_strategy_replay_jobs",
            &mut state.strategy_replay_jobs,
        )?;
        load_map(
            &connection,
            "learning_scout_replay_runs",
            &mut state.scout_replay_runs,
        )?;
        load_map(
            &connection,
            "learning_missed_opportunity_jobs",
            &mut state.missed_opportunity_jobs,
        )?;
        load_map(
            &connection,
            "learning_evidence_requests",
            &mut state.evidence_requests,
        )?;
        state.missed_opportunity_bands =
            load_recent_rows(&connection, "learning_missed_opportunity_bands", 2_000)?;
        backfill_variant_recency(&connection)?;
        let mut variant_history_truncated = false;
        for id in state.active_profiles.values().map(variant_key) {
            variant_history_truncated |=
                load_one_variant_entry(&connection, &id, &mut state.variants)?;
        }
        variant_history_truncated |= load_recent_variants(
            &connection,
            retention::MAX_VARIANTS_IN_MEMORY,
            &mut state.variants,
        )?;
        variant_history_truncated |= retention::compact_variants(&mut state);
        let database_variant_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM learning_variants", [], |row| {
                row.get(0)
            })
            .map_err(|error| format!("could not count Learning variants: {error}"))?;
        state.outcome_history_truncated |=
            variant_history_truncated || database_variant_count as usize > state.variants.len();
        // Compact legacy high-volume outcome histories before they become the
        // live working set. Replay jobs and idempotency keys are intentionally
        // retained unchanged because they are durable workflow state.
        for variant in state.variants.values_mut() {
            state.outcome_history_truncated |=
                retention::compact_variant_outcomes(variant, now_ms());
        }
        return Ok(state);
    }
    Ok(LearningState::default())
}

fn load_map<T: for<'de> Deserialize<'de>>(
    connection: &rusqlite::Connection,
    table: &str,
    target: &mut BTreeMap<String, T>,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("SELECT id, payload FROM {table}"))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (id, payload) = row.map_err(|error| error.to_string())?;
        target.insert(
            id,
            serde_json::from_str(&payload).map_err(|error| error.to_string())?,
        );
    }
    Ok(())
}

fn load_ids(connection: &rusqlite::Connection, table: &str) -> Result<BTreeSet<String>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT id FROM {table} ORDER BY rowid DESC LIMIT 100000"
        ))
        .map_err(|error| error.to_string())?;
    statement
        .query_map([], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .map(|row| row.map_err(|error| error.to_string()))
        .collect()
}

pub(crate) fn persist_normalized(
    connection: &rusqlite::Connection,
    state: &LearningState,
) -> rusqlite::Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO learning_metadata (id, accepted_events) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET accepted_events = excluded.accepted_events
         WHERE accepted_events != excluded.accepted_events",
        [state.accepted_events.min(i64::MAX as u64) as i64],
    )?;
    transaction.execute(
        "INSERT INTO learning_config (key, value) VALUES ('minimum_live_scout_parameter_samples', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [state.minimum_live_scout_parameter_samples as i64],
    )?;
    transaction.execute(
        "INSERT INTO learning_config (key, value) VALUES ('outcome_history_truncated', ?1)
         ON CONFLICT(key) DO UPDATE SET value = MAX(value, excluded.value)",
        [if state.outcome_history_truncated {
            1_i64
        } else {
            0_i64
        }],
    )?;
    transaction.execute(
        "INSERT INTO learning_config (key, value) VALUES ('seen_event_ids_overflowed', ?1)
         ON CONFLICT(key) DO UPDATE SET value = MAX(value, excluded.value)",
        [if state.seen_event_ids_overflowed {
            1_i64
        } else {
            0_i64
        }],
    )?;
    transaction.execute(
        "INSERT INTO learning_config (key, value) VALUES ('seen_execution_event_ids_overflowed', ?1)
         ON CONFLICT(key) DO UPDATE SET value = MAX(value, excluded.value)",
        [if state.seen_execution_event_ids_overflowed { 1_i64 } else { 0_i64 }],
    )?;
    write_map_preserving_history(&transaction, "learning_variants", &state.variants)?;
    {
        let last_seen_ms = now_ms() as i64;
        let mut statement = transaction.prepare_cached(
            "INSERT INTO learning_variant_recency (id, last_seen_ms) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET last_seen_ms = MAX(last_seen_ms, excluded.last_seen_ms)",
        )?;
        for id in state.variants.keys() {
            statement.execute(rusqlite::params![id, last_seen_ms])?;
        }
    }
    write_map(
        &transaction,
        "learning_active_profiles",
        &state.active_profiles,
    )?;
    write_map_preserving_history(
        &transaction,
        "learning_context_stats",
        &state.context_learning,
    )?;
    write_map_preserving_history(
        &transaction,
        "learning_execution_stats",
        &state.execution_learning,
    )?;
    {
        let rows = state
            .threshold_samples
            .iter()
            .enumerate()
            .map(|(id, value)| (id as i64, value))
            .collect::<Vec<_>>();
        sync_rows(&transaction, "learning_threshold_samples", &rows)?;
    }
    sync_ids_preserving_history(
        &transaction,
        "learning_seen_event_ids",
        &state.seen_event_ids,
    )?;
    write_map_preserving_history(
        &transaction,
        "learning_exit_timing_observations",
        &state.exit_timing_observations,
    )?;
    write_map(
        &transaction,
        "learning_strategy_replay_jobs",
        &state.strategy_replay_jobs,
    )?;
    write_map(
        &transaction,
        "learning_scout_replay_runs",
        &state.scout_replay_runs,
    )?;
    write_map(
        &transaction,
        "learning_missed_opportunity_jobs",
        &state.missed_opportunity_jobs,
    )?;
    write_map(
        &transaction,
        "learning_evidence_requests",
        &state.evidence_requests,
    )?;
    let bands = state
        .missed_opportunity_bands
        .iter()
        .enumerate()
        .map(|(id, band)| (id as i64, band))
        .collect::<Vec<_>>();
    sync_rows(&transaction, "learning_missed_opportunity_bands", &bands)?;
    sync_ids_preserving_history(
        &transaction,
        "learning_seen_execution_event_ids",
        &state.seen_execution_event_ids,
    )?;
    transaction.commit()
}

fn persist_scout_outcome_delta(
    path: &Path,
    state: &LearningState,
    event: &FeedbackEvent,
) -> Result<(), (StatusCode, String)> {
    let FeedbackEvent::ScoutOutcome {
        event_id,
        symbol,
        strategy,
        profile_id,
        parameters,
        ..
    } = event
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            "incremental persistence requires a ScoutOutcome".into(),
        ));
    };
    let profile = StrategyProfile {
        profile_id: profile_id.clone(),
        version: 0,
        symbol: symbol.clone(),
        strategy: strategy.clone(),
        parameters: parameters.clone(),
        learning_samples: 0,
        positive_samples: 0,
        realized_pnl: 0.0,
    };
    let variant_id = variant_key(&profile);
    let variant = state.variants.get(&variant_id).ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Scout feedback variant {variant_id} is missing from Learning state"),
        )
    })?;

    let connection =
        open_state_db(path).map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    transaction
        .execute(
            "INSERT INTO learning_metadata (id, accepted_events) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET accepted_events = excluded.accepted_events
             WHERE accepted_events != excluded.accepted_events",
            [state.accepted_events.min(i64::MAX as u64) as i64],
        )
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if state.outcome_history_truncated {
        transaction
            .execute(
                "INSERT INTO learning_config (key, value) VALUES ('outcome_history_truncated', 1)
                 ON CONFLICT(key) DO UPDATE SET value = MAX(value, excluded.value)",
                [],
            )
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    if state.seen_event_ids_overflowed {
        transaction
            .execute(
                "INSERT INTO learning_config (key, value) VALUES ('seen_event_ids_overflowed', 1)
                 ON CONFLICT(key) DO UPDATE SET value = MAX(value, excluded.value)",
                [],
            )
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }

    write_map_preserving_history(
        &transaction,
        "learning_variants",
        &BTreeMap::from([(variant_id.clone(), variant.clone())]),
    )
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    transaction
        .execute(
            "INSERT INTO learning_variant_recency (id, last_seen_ms) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET last_seen_ms = MAX(last_seen_ms, excluded.last_seen_ms)",
            rusqlite::params![variant_id, now_ms() as i64],
        )
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let active_profile_key = key(symbol, strategy);
    if let Some(active_profile) = state.active_profiles.get(&active_profile_key) {
        write_map_preserving_history(
            &transaction,
            "learning_active_profiles",
            &BTreeMap::from([(active_profile_key, active_profile.clone())]),
        )
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    if let Some(context_key) = event_context(event) {
        if let Some(context_stat) = state.context_learning.get(&context_key) {
            write_map_preserving_history(
                &transaction,
                "learning_context_stats",
                &BTreeMap::from([(context_key, context_stat.clone())]),
            )
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        }
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO learning_seen_event_ids (id) VALUES (?1)",
            [event_id],
        )
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    transaction
        .commit()
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if let Some(updates) = &state.state_updates {
        updates.notify_one();
    }
    Ok(())
}

fn write_map<T: Serialize>(
    connection: &rusqlite::Connection,
    table: &str,
    values: &BTreeMap<String, T>,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare_cached(&format!(
        "INSERT INTO {table} (id, payload) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET payload = excluded.payload
         WHERE payload != excluded.payload"
    ))?;
    for (id, value) in values {
        statement.execute(rusqlite::params![
            id,
            serde_json::to_string(value)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
        ])?;
    }
    drop(statement);
    let existing = {
        let mut statement = connection.prepare_cached(&format!("SELECT id FROM {table}"))?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut delete = connection.prepare_cached(&format!("DELETE FROM {table} WHERE id = ?1"))?;
    for id in existing {
        if !values.contains_key(&id) {
            delete.execute([id])?;
        }
    }
    Ok(())
}

fn write_map_preserving_history<T: Serialize>(
    connection: &rusqlite::Connection,
    table: &str,
    values: &BTreeMap<String, T>,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare_cached(&format!(
        "INSERT INTO {table} (id, payload) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET payload = excluded.payload
         WHERE payload != excluded.payload"
    ))?;
    for (id, value) in values {
        statement.execute(rusqlite::params![
            id,
            serde_json::to_string(value).unwrap_or_default()
        ])?;
    }
    Ok(())
}

fn sync_rows<T: Serialize>(
    connection: &rusqlite::Connection,
    table: &str,
    values: &[(i64, &T)],
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare_cached(&format!(
        "INSERT INTO {table} (id, payload) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET payload = excluded.payload
         WHERE payload != excluded.payload"
    ))?;
    for (id, value) in values {
        statement.execute(rusqlite::params![
            id,
            serde_json::to_string(value)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
        ])?;
    }
    drop(statement);
    let expected_len = values.len() as i64;
    connection.execute(
        &format!("DELETE FROM {table} WHERE id >= ?1"),
        [expected_len],
    )?;
    Ok(())
}

fn sync_ids_preserving_history(
    connection: &rusqlite::Connection,
    table: &str,
    values: &BTreeSet<String>,
) -> rusqlite::Result<()> {
    let mut statement =
        connection.prepare_cached(&format!("INSERT OR IGNORE INTO {table} (id) VALUES (?1)"))?;
    for id in values {
        statement.execute([id])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[test]
    fn variant_loader_hydrates_only_recent_sqlite_rows() {
        let path = std::env::temp_dir().join(format!(
            "learning-recent-variant-loader-{}-{}.sqlite",
            std::process::id(),
            now_ms()
        ));
        let connection = open_state_db(&path).unwrap();
        for index in 0..3 {
            let profile = StrategyProfile {
                profile_id: format!("profile-{index}"),
                version: 0,
                symbol: "BTC-USDT".into(),
                strategy: "live-example".into(),
                parameters: BTreeMap::from([("x".into(), index as f64)]),
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            };
            let id = variant_key(&profile);
            let variant = RegisteredProfile {
                profile,
                specs: Vec::new(),
                samples: 0,
                positive: 0,
                realized_pnl: 0.0,
                outcomes: Vec::new(),
            };
            connection
                .execute(
                    "INSERT INTO learning_variants (id, payload) VALUES (?1, ?2)",
                    rusqlite::params![id, serde_json::to_string(&variant).unwrap()],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO learning_variant_recency (id, last_seen_ms) VALUES (?1, ?2)",
                    rusqlite::params![id, index],
                )
                .unwrap();
        }
        let mut loaded = BTreeMap::new();
        load_recent_variants(&connection, 2, &mut loaded).unwrap();
        let profile_ids = loaded
            .values()
            .map(|variant| variant.profile.profile_id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(loaded.len(), 2);
        assert!(profile_ids.contains("profile-1"));
        assert!(profile_ids.contains("profile-2"));
        drop(connection);
        let _ = std::fs::remove_file(path);
    }

    async fn json_request(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<Vec<u8>>,
    ) -> axum::response::Response {
        let mut builder = axum::http::Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        app.oneshot(builder.body(Body::from(body.unwrap_or_default())).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn exit_timing_ack_follows_durable_insert_and_retries_after_failure_and_restart() {
        let path = std::env::temp_dir().join(format!(
            "learning-exit-timing-durable-{}-{}.sqlite",
            std::process::id(),
            now_ms()
        ));
        let event_id = "exit-timing-durable-retry-event";
        let observation = serde_json::json!({
            "event_id": event_id,
            "symbol": "BTC-USDT",
            "strategy": "live-momentum",
            "exit_return_percent": 0.2,
            "post_exit_return_percent": 0.1,
            "missed_return_percent": 0.1,
            "observed_at_ms": now_ms()
        });
        let app = router(path.clone()).unwrap();
        let connection = open_state_db(&path).unwrap();
        connection
            .execute_batch(&format!(
                "CREATE TRIGGER reject_exit_timing_insert BEFORE INSERT ON learning_exit_timing_observations
                 WHEN NEW.id = '{event_id}' BEGIN SELECT RAISE(ABORT, 'injected persistence failure'); END;"
            ))
            .unwrap();
        drop(connection);

        let failed = json_request(
            app,
            "POST",
            "/exit-timing",
            Some(serde_json::to_vec(&observation).unwrap()),
        )
        .await;
        assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let connection = open_state_db(&path).unwrap();
        let missing_after_failure = connection
            .query_row(
                "SELECT COUNT(*) FROM learning_exit_timing_observations WHERE id=?1",
                [event_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(missing_after_failure, 0);
        connection
            .execute_batch("DROP TRIGGER reject_exit_timing_insert")
            .unwrap();

        let app = router(path.clone()).unwrap();
        let accepted = json_request(
            app,
            "POST",
            "/exit-timing",
            Some(serde_json::to_vec(&observation).unwrap()),
        )
        .await;
        assert_eq!(accepted.status(), StatusCode::OK);
        let accepted: serde_json::Value =
            serde_json::from_slice(&accepted.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);

        let old_id = "older-exit-timing-event-outside-working-set";
        let connection = open_state_db(&path).unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO learning_exit_timing_observations (id, payload) VALUES (?1, ?2)",
                rusqlite::params![
                    old_id,
                    serde_json::to_string(&serde_json::json!({
                        "event_id": old_id,
                        "symbol": "BTC-USDT",
                        "strategy": "live-momentum",
                        "exit_return_percent": 0.0,
                        "post_exit_return_percent": 0.0,
                        "missed_return_percent": 0.0,
                        "observed_at_ms": 1
                    }))
                    .unwrap()
                ],
            )
            .unwrap();
        for index in 2..=10_001 {
            let id = format!("exit-timing-history-{index:05}");
            let payload = serde_json::json!({
                "event_id": id,
                "symbol": "BTC-USDT",
                "strategy": "live-momentum",
                "exit_return_percent": 0.0,
                "post_exit_return_percent": 0.0,
                "missed_return_percent": 0.0,
                "observed_at_ms": index
            });
            transaction
                .execute(
                    "INSERT INTO learning_exit_timing_observations (id, payload) VALUES (?1, ?2)",
                    rusqlite::params![id, serde_json::to_string(&payload).unwrap()],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        let state = LearningState::default();
        persist_normalized(&connection, &state).unwrap();
        let retained = connection
            .query_row(
                "SELECT COUNT(*) FROM learning_exit_timing_observations WHERE id=?1",
                [old_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(retained, 1);
        drop(connection);

        let restarted_service = service_state(path.clone()).unwrap();
        assert!(
            !restarted_service
                .state
                .lock()
                .await
                .exit_timing_observations
                .contains_key(old_id)
        );
        let restarted = service_router(restarted_service);
        for duplicate_event in [
            observation,
            serde_json::json!({
                "event_id": old_id,
                "symbol": "BTC-USDT",
                "strategy": "live-momentum",
                "exit_return_percent": 0.0,
                "post_exit_return_percent": 0.0,
                "missed_return_percent": 0.0,
                "observed_at_ms": 1
            }),
        ] {
            let response = json_request(
                restarted.clone(),
                "POST",
                "/exit-timing",
                Some(serde_json::to_vec(&duplicate_event).unwrap()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let result: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(result["accepted"], true);
            assert_eq!(result["duplicate"], true);
        }

        drop(restarted);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn scout_outcome_delta_is_durable_and_deduplicated_after_restart() {
        let path = std::env::temp_dir().join(format!(
            "learning-scout-delta-restart-{}-{}.sqlite",
            std::process::id(),
            now_ms()
        ));
        let event = FeedbackEvent::ScoutOutcome {
            event_id: "scout-delta-restart-event".into(),
            observed_at_ms: now_ms(),
            learning_context: Some("long:trend:BTC-USDT:live-momentum".into()),
            market_regime: Some("trend".into()),
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "scout-delta-profile".into(),
            parameters: BTreeMap::from([("threshold".into(), 0.5)]),
            baseline_parameters: BTreeMap::new(),
            comparison: ScoutComparisonMetadata::Independent,
            specs: Vec::new(),
            net_return_percent: -0.25,
        };

        let first_app = router(path.clone()).unwrap();
        let first = json_request(
            first_app,
            "POST",
            "/feedback",
            Some(serde_json::to_vec(&event).unwrap()),
        )
        .await;
        assert_eq!(first.status(), StatusCode::OK);
        let first_result: FeedbackResult =
            serde_json::from_slice(&first.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert!(first_result.accepted);
        assert!(!first_result.duplicate);

        let app_after_restart = router(path.clone()).unwrap();
        let duplicate = json_request(
            app_after_restart,
            "POST",
            "/feedback",
            Some(serde_json::to_vec(&event).unwrap()),
        )
        .await;
        assert_eq!(duplicate.status(), StatusCode::OK);
        let duplicate_result: FeedbackResult =
            serde_json::from_slice(&duplicate.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(duplicate_result.accepted);
        assert!(duplicate_result.duplicate);

        let connection = open_state_db(&path).unwrap();
        let accepted_events = connection
            .query_row(
                "SELECT accepted_events FROM learning_metadata WHERE id=1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        let event_rows = connection
            .query_row(
                "SELECT COUNT(*) FROM learning_seen_event_ids WHERE id=?1",
                ["scout-delta-restart-event"],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(accepted_events, 1);
        assert_eq!(event_rows, 1);

        drop(connection);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn scout_outcome_delta_updates_large_history_without_losing_unchanged_records() {
        let suffix = format!("{}-{}", std::process::id(), now_ms());
        let delta_path = std::env::temp_dir().join(format!("learning-scout-delta-{suffix}.sqlite"));
        let mut baseline = LearningState {
            accepted_events: 100_000,
            seen_event_ids_overflowed: true,
            ..LearningState::default()
        };
        for index in 0..5_000 {
            let profile = StrategyProfile {
                profile_id: format!("historical-profile-{index}"),
                version: 0,
                symbol: format!("TEST-{index}-USDT"),
                strategy: "live-momentum".into(),
                parameters: BTreeMap::from([("threshold".into(), index as f64)]),
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            };
            let id = variant_key(&profile);
            baseline.variants.insert(
                id,
                RegisteredProfile {
                    profile,
                    specs: Vec::new(),
                    samples: 0,
                    positive: 0,
                    realized_pnl: 0.0,
                    outcomes: Vec::new(),
                },
            );
        }
        baseline.seen_event_ids = (0..100_000)
            .map(|index| format!("historical-event-{index:06}"))
            .collect();

        let context = "long:trend:BTC-USDT:live-momentum".to_owned();
        let profile = StrategyProfile {
            profile_id: "scout-delta-profile".into(),
            version: 0,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            parameters: BTreeMap::from([("threshold".into(), 0.5)]),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let variant_id = variant_key(&profile);
        baseline.variants.insert(
            variant_id.clone(),
            RegisteredProfile {
                profile: profile.clone(),
                specs: Vec::new(),
                samples: 0,
                positive: 0,
                realized_pnl: 0.0,
                outcomes: Vec::new(),
            },
        );
        baseline
            .active_profiles
            .insert(key("BTC-USDT", "live-momentum"), profile);

        let event = FeedbackEvent::ScoutOutcome {
            event_id: "scout-delta-large-state-event".into(),
            observed_at_ms: now_ms(),
            learning_context: Some(context.clone()),
            market_regime: Some("trend".into()),
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "scout-delta-profile".into(),
            parameters: BTreeMap::from([("threshold".into(), 0.5)]),
            baseline_parameters: BTreeMap::new(),
            comparison: ScoutComparisonMetadata::Independent,
            specs: Vec::new(),
            net_return_percent: -0.25,
        };
        let mut updated = baseline.clone();
        updated.accepted_events += 1;
        updated
            .seen_event_ids
            .insert("scout-delta-large-state-event".into());
        updated.context_learning.insert(
            context.clone(),
            ContextLearningStat {
                samples: 1,
                positive: 0,
                realized_pnl: -0.25,
                updated_at_ms: now_ms(),
            },
        );
        updated
            .variants
            .get_mut(&variant_id)
            .unwrap()
            .outcomes
            .push(LearningOutcome {
                observed_at_ms: event_observed_at(&event),
                net_return_percent: event_net_return_percent(&event),
                evidence_group_id: None,
                learning_context: None,
                market_regime: Some("trend".into()),
                horizon_seconds: Some(300),
                source: LearningOutcomeSource::Scout,
                realized_pnl_usd: None,
                paired_comparison: false,
                tested_parameter_keys: Vec::new(),
                baseline_parameters: BTreeMap::new(),
            });

        let delta_connection = open_state_db(&delta_path).unwrap();
        persist_normalized(&delta_connection, &baseline).unwrap();
        let baseline_variant_count: i64 = delta_connection
            .query_row("SELECT COUNT(*) FROM learning_variants", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(baseline_variant_count, 5_001);
        drop(delta_connection);
        persist_scout_outcome_delta(&delta_path, &updated, &event).unwrap();

        let delta_connection = open_state_db(&delta_path).unwrap();
        let delta_variant: String = delta_connection
            .query_row(
                "SELECT payload FROM learning_variants WHERE id=?1",
                [&variant_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            delta_variant,
            serde_json::to_string(updated.variants.get(&variant_id).unwrap()).unwrap()
        );
        let delta_context: String = delta_connection
            .query_row(
                "SELECT payload FROM learning_context_stats WHERE id=?1",
                [&context],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            delta_context,
            serde_json::to_string(updated.context_learning.get(&context).unwrap()).unwrap()
        );
        let variant_count: i64 = delta_connection
            .query_row("SELECT COUNT(*) FROM learning_variants", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(variant_count, 5_001);
        let event_count: i64 = delta_connection
            .query_row("SELECT COUNT(*) FROM learning_seen_event_ids", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(event_count, 100_001);
        let historical_profile = StrategyProfile {
            profile_id: "historical-profile-0".into(),
            version: 0,
            symbol: "TEST-0-USDT".into(),
            strategy: "live-momentum".into(),
            parameters: BTreeMap::from([("threshold".into(), 0.0)]),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let historical_id = variant_key(&historical_profile);
        let historical_payload: String = delta_connection
            .query_row(
                "SELECT payload FROM learning_variants WHERE id=?1",
                [&historical_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            historical_payload,
            serde_json::to_string(baseline.variants.get(&historical_id).unwrap()).unwrap()
        );
        let delta_event_exists: bool = delta_connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM learning_seen_event_ids WHERE id=?1)",
                ["scout-delta-large-state-event"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(delta_event_exists);

        drop(delta_connection);
        for path in [&delta_path] {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
            let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        }
    }

    #[tokio::test]
    async fn service_persists_profiles_deduplicates_feedback_and_publishes_bounded_updates() {
        let path =
            std::env::temp_dir().join(format!("learning-test-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let app = router(path.clone()).unwrap();
        let profile = StrategyProfile {
            profile_id: "profile-a".into(),
            version: 0,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            parameters: BTreeMap::from([
                ("threshold".into(), 0.5),
                ("take_profit_percent".into(), 0.8),
            ]),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let spec = ParameterSpec {
            key: "threshold".into(),
            label: "Threshold".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.1,
            unit: "ratio".into(),
            learning_direction: 1.0,
        };
        let unrelated_spec = ParameterSpec {
            key: "take_profit_percent".into(),
            label: "Take profit".into(),
            default: 0.8,
            min: 0.1,
            max: 1.5,
            step: 0.1,
            unit: "%".into(),
            learning_direction: 1.0,
        };
        let response = json_request(
            app.clone(),
            "POST",
            "/profiles",
            Some(
                serde_json::to_vec(&StrategyProfileRegistration {
                    profile: profile.clone(),
                    specs: vec![spec.clone(), unrelated_spec],
                })
                .unwrap(),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        for index in 0..5 {
            let event = FeedbackEvent::TradeOutcome {
                event_id: format!("trade-{index}"),
                schema_version: None,
                trade_id: None,
                opportunity_id: None,
                episode_id: None,
                profile_hash: None,
                profile_version: None,
                execution_mode: None,
                entry_policy: None,
                source: None,
                entry_profile_id: None,
                observed_at_ms: now_ms(),
                learning_context: None,
                symbol: profile.symbol.clone(),
                strategy: profile.strategy.clone(),
                profile_id: profile.profile_id.clone(),
                parameters: profile.parameters.clone(),
                specs: vec![spec.clone()],
                scout: false,
                realized_pnl_usd: 1.0,
                net_return_percent: 1.0,
                threshold_observations: Vec::new(),
                replay_context: None,
            };
            let response = json_request(
                app.clone(),
                "POST",
                "/feedback",
                Some(serde_json::to_vec(&event).unwrap()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }
        let duplicate = FeedbackEvent::TradeOutcome {
            event_id: "trade-4".into(),
            schema_version: None,
            trade_id: None,
            opportunity_id: None,
            episode_id: None,
            profile_hash: None,
            profile_version: None,
            execution_mode: None,
            entry_policy: None,
            source: None,
            entry_profile_id: None,
            observed_at_ms: now_ms(),
            learning_context: None,
            symbol: profile.symbol.clone(),
            strategy: profile.strategy.clone(),
            profile_id: profile.profile_id.clone(),
            parameters: profile.parameters.clone(),
            specs: vec![spec],
            scout: false,
            realized_pnl_usd: 1.0,
            net_return_percent: 1.0,
            threshold_observations: Vec::new(),
            replay_context: None,
        };
        let response = json_request(
            app.clone(),
            "POST",
            "/feedback",
            Some(serde_json::to_vec(&duplicate).unwrap()),
        )
        .await;
        let result: FeedbackResult =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(result.duplicate);

        let response = json_request(app.clone(), "GET", "/profiles", None).await;
        let profiles: Vec<StrategyProfile> =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].parameters["threshold"], 0.5);
        assert_eq!(profiles[0].parameters["take_profit_percent"], 0.8);
        assert_eq!(profiles[0].learning_samples, 5);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn paired_scout_promotion_requires_symbol_specific_evidence() {
        let path = std::env::temp_dir().join(format!(
            "learning-paired-test-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let app = router(path.clone()).unwrap();
        let spec = ParameterSpec {
            key: "threshold".into(),
            label: "Threshold".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.1,
            unit: "ratio".into(),
            learning_direction: 1.0,
        };
        for symbol in ["BTC-USDT", "ETH-USDT"] {
            let baseline = StrategyProfile {
                profile_id: format!("{symbol}-baseline"),
                version: 0,
                symbol: symbol.into(),
                strategy: "live-momentum".into(),
                parameters: BTreeMap::from([("threshold".into(), 0.5)]),
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            };
            let response = json_request(
                app.clone(),
                "POST",
                "/profiles",
                Some(
                    serde_json::to_vec(&StrategyProfileRegistration {
                        profile: baseline,
                        specs: vec![spec.clone()],
                    })
                    .unwrap(),
                ),
            )
            .await;
            assert_eq!(response.status(), StatusCode::ACCEPTED);
        }

        let response = json_request(
            app.clone(),
            "PUT",
            "/config/scout-parameter-samples",
            Some(br#"{"minimum_samples":9}"#.to_vec()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = json_request(
            app.clone(),
            "PUT",
            "/config/scout-parameter-samples",
            Some(br#"{"minimum_samples":10}"#.to_vec()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        for (symbol_index, symbol) in ["BTC-USDT", "ETH-USDT"].into_iter().enumerate() {
            let parameters = BTreeMap::from([("threshold".into(), 0.4)]);
            let profile_id = format!("{symbol}-candidate");
            for sample in 0..9 {
                for horizon in [60, 300, 900, 3_600] {
                    let event = FeedbackEvent::ScoutOutcome {
                        event_id: format!("fixture-event-{symbol}-{sample}-{horizon}"),
                        observed_at_ms: now_ms() + symbol_index as u64 * 10 + sample,
                        learning_context: None,
                        market_regime: Some(
                            if symbol_index == 0 { "trend" } else { "range" }.into(),
                        ),
                        symbol: symbol.into(),
                        strategy: "live-momentum".into(),
                        profile_id: profile_id.clone(),
                        parameters: parameters.clone(),
                        baseline_parameters: BTreeMap::from([("threshold".into(), 0.5)]),
                        comparison: ScoutComparisonMetadata::Paired {
                            group_id: format!("{symbol}-{sample}"),
                            horizon_seconds: Some(horizon),
                        },
                        specs: vec![spec.clone()],
                        net_return_percent: 0.02,
                    };
                    let response = json_request(
                        app.clone(),
                        "POST",
                        "/feedback",
                        Some(serde_json::to_vec(&event).unwrap()),
                    )
                    .await;
                    assert_eq!(response.status(), StatusCode::OK);
                }
                if symbol_index == 0 {
                    let response = json_request(app.clone(), "GET", "/profiles", None).await;
                    let profiles: Vec<StrategyProfile> = serde_json::from_slice(
                        &response.into_body().collect().await.unwrap().to_bytes(),
                    )
                    .unwrap();
                    assert!(
                        profiles
                            .iter()
                            .all(|profile| profile.parameters["threshold"] == 0.5)
                    );
                }
            }
        }

        let response = json_request(app.clone(), "GET", "/profiles", None).await;
        let profiles: Vec<StrategyProfile> =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(profiles.len(), 2);
        assert!(
            profiles
                .iter()
                .all(|profile| profile.parameters["threshold"] == 0.5)
        );

        // BTC now reaches the ten-sample minimum on its own. ETH's samples must
        // not contribute to BTC's promotion decision or be changed with it.
        for sample in 9..10 {
            let event = FeedbackEvent::ScoutOutcome {
                event_id: format!("fixture-event-BTC-USDT-extra-{sample}"),
                observed_at_ms: now_ms() + 100 + sample,
                learning_context: None,
                market_regime: Some("trend".into()),
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                profile_id: "BTC-USDT-candidate".into(),
                parameters: BTreeMap::from([("threshold".into(), 0.4)]),
                baseline_parameters: BTreeMap::from([("threshold".into(), 0.5)]),
                comparison: ScoutComparisonMetadata::Paired {
                    group_id: format!("BTC-USDT-extra-{sample}"),
                    horizon_seconds: None,
                },
                specs: vec![spec.clone()],
                net_return_percent: 0.02,
            };
            let response = json_request(
                app.clone(),
                "POST",
                "/feedback",
                Some(serde_json::to_vec(&event).unwrap()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }

        let response = json_request(app.clone(), "GET", "/profiles", None).await;
        let profiles: Vec<StrategyProfile> =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(profiles.len(), 2);
        assert!((profiles[0].parameters["threshold"] - 0.4).abs() < 1e-6);
        assert_eq!(profiles[1].parameters["threshold"], 0.5);

        let response = json_request(app.clone(), "GET", "/decision-snapshot", None).await;
        let snapshot: LearnedProfileSnapshot =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(
            snapshot
                .walk_forward_evidence
                .contains_key("BTC-USDT|live-momentum")
        );
        assert!(
            snapshot
                .walk_forward_evidence
                .contains_key("ETH-USDT|live-momentum")
        );
        assert!(!snapshot.walk_forward_evidence.contains_key("live-momentum"));
        assert!(snapshot.walk_forward_evidence["BTC-USDT|live-momentum"].sample_count > 0);
        assert!(snapshot.walk_forward_evidence["ETH-USDT|live-momentum"].sample_count > 0);
        assert!(
            snapshot
                .walk_forward_evidence
                .contains_key("BTC-USDT|live-momentum|regime:trend")
        );
        assert!(
            snapshot
                .walk_forward_evidence
                .contains_key("ETH-USDT|live-momentum|regime:range")
        );
        assert!(
            !snapshot
                .walk_forward_evidence
                .contains_key("BTC-USDT|live-momentum|regime:range")
        );
        assert!(
            snapshot
                .walk_forward_evidence
                .contains_key("BTC-USDT-candidate|regime:trend")
        );
        assert_eq!(snapshot.scout_parameter_variants.len(), 2);
        let btc = snapshot
            .scout_parameter_variants
            .iter()
            .find(|variant| variant.symbol == "BTC-USDT")
            .unwrap();
        assert_eq!(btc.sample_count, 10);
        assert!(btc.promoted);
        assert_eq!(btc.paired_baseline_decrease_samples, 10);
        assert_eq!(btc.paired_baseline_delta_samples, 10);
        assert!((btc.lower_90_one_sided_improvement_percent.unwrap() - 0.02).abs() < 1e-10);
        let eth = snapshot
            .scout_parameter_variants
            .iter()
            .find(|variant| variant.symbol == "ETH-USDT")
            .unwrap();
        assert_eq!(eth.sample_count, 9);
        assert!(!eth.promoted);
        assert_eq!(eth.paired_baseline_decrease_samples, 9);
        assert_eq!(eth.paired_baseline_delta_samples, 9);
        assert_eq!(eth.lower_90_one_sided_improvement_percent, None);
        for index in 0..5 {
            let event = FeedbackEvent::TradeOutcome {
                event_id: format!("old-profile-close-{index}"),
                schema_version: None,
                trade_id: None,
                opportunity_id: None,
                episode_id: None,
                profile_hash: None,
                profile_version: None,
                execution_mode: None,
                entry_policy: None,
                source: None,
                entry_profile_id: None,
                observed_at_ms: now_ms(),
                learning_context: None,
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                profile_id: "BTC-USDT-baseline".into(),
                parameters: BTreeMap::from([("threshold".into(), 0.5)]),
                specs: vec![],
                scout: false,
                realized_pnl_usd: -1.0,
                net_return_percent: -0.1,
                threshold_observations: vec![],
                replay_context: None,
            };
            let response = json_request(
                app.clone(),
                "POST",
                "/feedback",
                Some(serde_json::to_vec(&event).unwrap()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }
        let response = json_request(app.clone(), "GET", "/profiles", None).await;
        let profiles: Vec<StrategyProfile> =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(
            (profiles
                .iter()
                .find(|profile| profile.symbol == "BTC-USDT")
                .unwrap()
                .parameters["threshold"]
                - 0.4)
                .abs()
                < 1e-6
        );
        let response = json_request(
            app.clone(),
            "PUT",
            "/config/scout-parameter-samples",
            Some(br#"{"minimum_samples":10}"#.to_vec()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let restarted_app = router(path.clone()).unwrap();
        let response = json_request(restarted_app, "GET", "/decision-snapshot", None).await;
        let restarted_snapshot: LearnedProfileSnapshot =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let btc = restarted_snapshot
            .scout_parameter_variants
            .iter()
            .find(|variant| variant.symbol == "BTC-USDT")
            .unwrap();
        assert_eq!(btc.minimum_samples, 10);
        assert_eq!(btc.paired_baseline_decrease_samples, 10);
        let eth = restarted_snapshot
            .scout_parameter_variants
            .iter()
            .find(|variant| variant.symbol == "ETH-USDT")
            .unwrap();
        assert_eq!(eth.minimum_samples, 10);
        assert_eq!(eth.paired_baseline_decrease_samples, 9);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn trade_threshold_observations_train_and_validate_threshold_in_learning_service() {
        let path = std::env::temp_dir().join(format!(
            "learning-threshold-test-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let app = router(path.clone()).unwrap();
        let spec = ParameterSpec {
            key: "minimum_flow_imbalance".into(),
            label: "Minimum flow".into(),
            default: 0.02,
            min: 0.02,
            max: 0.8,
            step: 0.01,
            unit: "ratio".into(),
            learning_direction: -1.0,
        };
        let parameters = BTreeMap::from([(spec.key.clone(), 0.02)]);
        let profile = StrategyProfile {
            profile_id: "flow-profile".into(),
            version: 0,
            symbol: "BTC-USDT".into(),
            strategy: "live-trade-flow".into(),
            parameters: parameters.clone(),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let response = json_request(
            app.clone(),
            "POST",
            "/profiles",
            Some(
                serde_json::to_vec(&StrategyProfileRegistration {
                    profile,
                    specs: vec![spec.clone()],
                })
                .unwrap(),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        // Spread events over independent four-hour intervals: a burst of
        // trades in one interval is one effective sample window, not 150.
        let interval_ms = 4 * 60 * 60 * 1_000;
        let first_observed_at_ms = now_ms().saturating_sub(150 * interval_ms);
        for index in 0..150_u64 {
            let (feature_value, net_return_percent) = if index % 2 == 0 {
                (0.025, -0.2)
            } else {
                (0.075, 1.0)
            };
            let event = FeedbackEvent::TradeOutcome {
                event_id: format!("threshold-trade-{index}"),
                schema_version: None,
                trade_id: None,
                opportunity_id: None,
                episode_id: None,
                profile_hash: None,
                profile_version: None,
                execution_mode: None,
                entry_policy: None,
                source: None,
                entry_profile_id: None,
                observed_at_ms: first_observed_at_ms + index * interval_ms,
                learning_context: None,
                symbol: "BTC-USDT".into(),
                strategy: "live-trade-flow".into(),
                profile_id: "flow-profile".into(),
                parameters: parameters.clone(),
                specs: Vec::new(),
                scout: false,
                realized_pnl_usd: net_return_percent,
                net_return_percent,
                threshold_observations: vec![StrategyThresholdObservation {
                    parameter_key: spec.key.clone(),
                    threshold: 0.02,
                    feature_value,
                }],
                replay_context: None,
            };
            let response = json_request(
                app.clone(),
                "POST",
                "/feedback",
                Some(serde_json::to_vec(&event).unwrap()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }

        let response = json_request(app, "GET", "/profiles", None).await;
        let profiles: Vec<StrategyProfile> =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(profiles[0].parameters[&spec.key], 0.03);
        let _ = std::fs::remove_file(path);
    }

    fn profile() -> StrategyProfile {
        StrategyProfile {
            profile_id: "p1".into(),
            version: 3,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            parameters: BTreeMap::from([("threshold".into(), 0.9)]),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        }
    }

    #[test]
    fn feedback_updates_are_variant_scoped_and_bounded() {
        let profile = profile();
        let spec = ParameterSpec {
            key: "threshold".into(),
            label: "Threshold".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.2,
            unit: "ratio".into(),
            learning_direction: 1.0,
        };
        let event = FeedbackEvent::ScoutOutcome {
            event_id: "e1".into(),
            observed_at_ms: 1,
            learning_context: None,
            market_regime: None,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "p1".into(),
            parameters: profile.parameters.clone(),
            baseline_parameters: BTreeMap::new(),
            comparison: ScoutComparisonMetadata::Independent,
            specs: vec![spec.clone()],
            net_return_percent: 2.0,
        };
        let updated = update_profile(&profile, &[spec.clone()], &event).unwrap();
        assert_eq!(updated.parameters["threshold"], 0.9);
        assert_eq!(updated.version, 4);
        let FeedbackEvent::ScoutOutcome { parameters, .. } = event else {
            unreachable!()
        };
        let mut wrong = profile;
        wrong.parameters = parameters.into_iter().map(|(key, _)| (key, 0.1)).collect();
        assert!(
            update_profile(
                &wrong,
                &[],
                &FeedbackEvent::TradeOutcome {
                    event_id: "e2".into(),
                    schema_version: None,
                    trade_id: None,
                    opportunity_id: None,
                    episode_id: None,
                    profile_hash: None,
                    profile_version: None,
                    execution_mode: None,
                    entry_policy: None,
                    source: None,
                    entry_profile_id: None,
                    observed_at_ms: 2,
                    learning_context: None,
                    symbol: "BTC-USDT".into(),
                    strategy: "live-momentum".into(),
                    profile_id: "p1".into(),
                    parameters: BTreeMap::from([("threshold".into(), 0.9)]),
                    specs: vec![spec],
                    scout: false,
                    realized_pnl_usd: 1.0,
                    net_return_percent: 1.0,
                    threshold_observations: Vec::new(),
                    replay_context: None,
                }
            )
            .is_none()
        );
    }

    #[test]
    fn scout_feedback_keeps_the_exact_tested_candidate_value() {
        let mut profile = profile();
        profile.parameters.insert("take_profit_percent".into(), 0.8);
        let spec = ParameterSpec {
            key: "threshold".into(),
            label: "Threshold".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.2,
            unit: "ratio".into(),
            learning_direction: 1.0,
        };
        let event = FeedbackEvent::ScoutOutcome {
            event_id: "scout-probe-loss".into(),
            observed_at_ms: 1,
            learning_context: None,
            market_regime: None,
            symbol: profile.symbol.clone(),
            strategy: profile.strategy.clone(),
            profile_id: profile.profile_id.clone(),
            parameters: profile.parameters.clone(),
            baseline_parameters: BTreeMap::new(),
            comparison: ScoutComparisonMetadata::Independent,
            specs: vec![spec.clone()],
            net_return_percent: -1.0,
        };

        let updated = update_profile(&profile, &[spec], &event).unwrap();
        assert_eq!(updated.parameters["threshold"], 0.9);
        assert_eq!(updated.parameters["take_profit_percent"], 0.8);
    }

    #[test]
    fn trade_profile_statistics_publish_after_minimum_trade_samples_even_when_losing() {
        assert!(!should_publish_trade_profile(5, 2, f64::NAN));
        assert!(!should_publish_trade_profile(5, 6, -1.0));
        assert!(should_publish_trade_profile(5, 0, -1.0));
        assert!(should_publish_trade_profile(5, 3, 0.1));
        assert!(!should_publish_trade_profile(4, 4, 1.0));
    }

    #[test]
    fn profile_keys_include_identity_and_parameter_variant() {
        let first = profile();
        assert_eq!(key("BTC-USDT", "live-momentum"), "BTC-USDT:live-momentum");
        assert!(variant_key(&first).contains("BTC-USDT:live-momentum:p1:"));
        let mut second = first.clone();
        second.parameters.insert("threshold".into(), 0.8);
        assert_ne!(variant_key(&first), variant_key(&second));
    }

    #[test]
    fn event_parts_extract_the_shared_feedback_identity_for_both_event_kinds() {
        let scout = FeedbackEvent::ScoutOutcome {
            event_id: "scout-id".into(),
            observed_at_ms: 1,
            learning_context: None,
            market_regime: None,
            symbol: "BTC-USDT".into(),
            strategy: "momentum".into(),
            profile_id: "p1".into(),
            parameters: BTreeMap::from([("threshold".into(), 0.5)]),
            baseline_parameters: BTreeMap::new(),
            comparison: ScoutComparisonMetadata::Independent,
            specs: vec![],
            net_return_percent: 1.0,
        };
        let parts = event_parts(&scout);
        assert_eq!(
            (&parts.0, parts.1, parts.2, parts.4.as_str()),
            (&"scout-id", "BTC-USDT", "momentum", "p1")
        );
        assert_eq!(parts.3["threshold"], 0.5);
        let trade = FeedbackEvent::TradeOutcome {
            event_id: "trade-id".into(),
            schema_version: None,
            trade_id: None,
            opportunity_id: None,
            episode_id: None,
            profile_hash: None,
            profile_version: None,
            execution_mode: None,
            entry_policy: None,
            source: None,
            entry_profile_id: None,
            observed_at_ms: 2,
            learning_context: None,
            symbol: "ETH-USDT".into(),
            strategy: "carry".into(),
            profile_id: "p2".into(),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: false,
            realized_pnl_usd: 1.0,
            net_return_percent: 0.5,
            threshold_observations: Vec::new(),
            replay_context: None,
        };
        let parts = event_parts(&trade);
        assert_eq!(parts.0, "trade-id");
        assert_eq!(parts.1, "ETH-USDT");
        assert_eq!(parts.2, "carry");
        assert_eq!(parts.4, "p2");
    }

    #[tokio::test]
    async fn actual_trade_feedback_reaches_the_profile_context_snapshot() {
        let path = std::env::temp_dir().join(format!(
            "learning-risk-feedback-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let app = router(path.clone()).unwrap();
        let timestamp = now_ms().saturating_sub(100);
        for index in 0..3 {
            let event = FeedbackEvent::TradeOutcome {
                event_id: format!("risk-trade-{index}"),
                schema_version: None,
                trade_id: None,
                opportunity_id: None,
                episode_id: None,
                profile_hash: None,
                profile_version: None,
                execution_mode: None,
                entry_policy: None,
                source: None,
                entry_profile_id: None,
                observed_at_ms: timestamp + index,
                learning_context: Some("long:mixed:BTC-USDT:live-momentum:P0:depth:lt05".into()),
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                profile_id: "profile-risk".into(),
                parameters: BTreeMap::new(),
                specs: vec![],
                scout: false,
                realized_pnl_usd: -1.0,
                net_return_percent: -0.2,
                threshold_observations: vec![],
                replay_context: None,
            };
            let response = json_request(
                app.clone(),
                "POST",
                "/feedback",
                Some(serde_json::to_vec(&event).unwrap()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }
        let response = json_request(app, "GET", "/decision-snapshot", None).await;
        let snapshot: LearnedProfileSnapshot =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let values = &snapshot.recent_profile_context_returns["profile-risk|long:mixed:BTC-USDT:live-momentum"];
        assert_eq!(values.len(), 3);
        assert!(values.iter().all(|sample| sample.1 == -0.2));
        let _ = std::fs::remove_file(path);
    }
}
