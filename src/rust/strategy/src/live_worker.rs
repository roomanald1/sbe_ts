use anyhow::{Context, bail};
use futures_util::{Stream, StreamExt, stream};
use rsocket_rust::{prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::{WebsocketClientTransport, WebsocketServerTransport};
use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex, Notify, RwLock, Semaphore},
    task::JoinSet,
};

use crate::worker_metrics::{WorkerMetrics, serve_metrics};
use crate::{
    LIVE_EVALUATION_CONTRACT_VERSION, LiveEvaluationError, LiveEvaluationErrorCode,
    LiveEvaluationInput, LiveEvaluationRequest, LiveEvaluationResponse, LiveStrategyDecision,
    Strategy, StrategyDecision, StrategyInput,
};

fn live_worker_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn route_live_profiles(
    eligible_profiles: Vec<crate::LiveStrategyProfile>,
    market_state: Option<&crate::LiveMarketStateClassification>,
    symbol_has_open_position: bool,
) -> Vec<crate::LiveStrategyProfile> {
    if symbol_has_open_position {
        return eligible_profiles;
    }
    let exact = market_state
        .filter(|state| state.confidence >= LIVE_REGIME_PROFILE_MIN_CONFIDENCE)
        .and_then(|state| {
            eligible_profiles
                .iter()
                .filter(|profile| {
                    let key_match =
                        profile.market_state_key.as_deref() == Some(state.profile_key.as_str());
                    let legacy_match = profile.market_state_key.is_none()
                        && profile.market_regime.as_deref() == Some(state.evidence_regime.as_str());
                    (key_match || legacy_match)
                        && profile
                            .walk_forward_evidence_by_regime
                            .get(state.evidence_regime.as_str())
                            .is_some_and(|evidence| {
                                evidence.sample_count >= LIVE_REGIME_PROFILE_MIN_SAMPLES
                            })
                })
                .min_by_key(|profile| profile.profile_id.as_str())
                .cloned()
        });
    if let Some(profile) = exact {
        return vec![profile];
    }
    let fallback = eligible_profiles
        .iter()
        .filter(|profile| profile.market_regime.is_none() && profile.market_state_key.is_none())
        .min_by_key(|profile| profile.profile_id.as_str())
        .cloned();
    fallback.into_iter().collect()
}

const LIVE_REGIME_PROFILE_MIN_CONFIDENCE: f64 = 0.6;
const LIVE_REGIME_PROFILE_MIN_SAMPLES: usize = 8;
const LIVE_REGIME_HYSTERESIS_CONFIRMATIONS: u8 = 2;
const LIVE_REGIME_HYSTERESIS_MAX_GAP_MS: u64 = 30_000;

#[derive(Clone, Debug, Default)]
struct MarketStateTransition {
    active: Option<crate::LiveMarketStateClassification>,
    pending_key: Option<String>,
    pending_count: u8,
    pending_since_ms: u64,
    pending_last_ms: u64,
}

impl MarketStateTransition {
    fn observe(
        &mut self,
        candidate: Option<crate::LiveMarketStateClassification>,
    ) -> (Option<crate::LiveMarketStateClassification>, &'static str) {
        let Some(candidate) =
            candidate.filter(|state| state.confidence >= LIVE_REGIME_PROFILE_MIN_CONFIDENCE)
        else {
            return (None, "uncertain_fallback");
        };
        let Some(active) = self.active.clone() else {
            self.active = Some(candidate.clone());
            self.clear_pending();
            return (Some(candidate), "initialized");
        };
        if candidate.observed_at_ms < active.observed_at_ms {
            return (Some(active), "out_of_order_held");
        }
        if candidate.profile_key == active.profile_key {
            self.active = Some(candidate.clone());
            self.clear_pending();
            return (Some(candidate), "stable");
        }
        if self.pending_key.as_deref() == Some(candidate.profile_key.as_str())
            && candidate
                .observed_at_ms
                .saturating_sub(self.pending_since_ms)
                <= LIVE_REGIME_HYSTERESIS_MAX_GAP_MS
            && candidate.observed_at_ms > self.pending_last_ms
        {
            self.pending_count = self.pending_count.saturating_add(1);
            self.pending_last_ms = candidate.observed_at_ms;
        } else {
            self.pending_key = Some(candidate.profile_key.clone());
            self.pending_count = 1;
            self.pending_since_ms = candidate.observed_at_ms;
            self.pending_last_ms = candidate.observed_at_ms;
        }
        if self.pending_count >= LIVE_REGIME_HYSTERESIS_CONFIRMATIONS {
            self.active = Some(candidate.clone());
            self.clear_pending();
            return (Some(candidate), "transitioned");
        }
        (Some(active), "transition_pending")
    }

    fn clear_pending(&mut self) {
        self.pending_key = None;
        self.pending_count = 0;
        self.pending_since_ms = 0;
        self.pending_last_ms = 0;
    }
}

fn carry_placeholder_signal(
    symbol: &str,
    timestamp: u64,
    profile: Option<&crate::LiveStrategyProfile>,
) -> crate::StrategyMarketSignal {
    let mut signal = crate::StrategyMarketSignal {
        symbol: symbol.to_owned(),
        direction: "bullish".to_owned(),
        strength: 1.0,
        timestamp,
        stale: false,
        score: 1.0,
        strategy_candidate: true,
        strategy: Some(Strategy::DatedCarry.as_str().to_owned()),
        ..Default::default()
    };
    if let Some(profile) = profile {
        signal.strategy_decision = Some(serde_json::json!({
            "strategy": Strategy::DatedCarry.as_str(),
            "strategy_parameters": profile.parameters,
            "learning_profile_id": profile.profile_id,
            "learning_profile_symbol": profile.profile_symbol,
            "learning_profile_version": profile.version,
        }));
    }
    signal
}

#[derive(Clone, Debug)]
pub struct LiveWorkerConfig {
    pub strategy: Strategy,
    pub listen_addr: String,
    pub max_in_flight: usize,
    pub max_payload_bytes: usize,
    pub metrics_listen_addr: String,
    pub insights_stream_url: String,
    pub auto_trader_rsocket_url: String,
}

impl LiveWorkerConfig {
    pub fn from_env(strategy: Strategy) -> anyhow::Result<Self> {
        let config = Self {
            strategy,
            listen_addr: std::env::var("STRATEGY_LIVE_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:10201".into()),
            max_in_flight: env_usize("STRATEGY_LIVE_MAX_IN_FLIGHT", 32)?,
            max_payload_bytes: env_usize("STRATEGY_LIVE_MAX_PAYLOAD_BYTES", 262_144)?,
            metrics_listen_addr: std::env::var("STRATEGY_METRICS_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:10202".into()),
            insights_stream_url: std::env::var("INSIGHTS_STRATEGY_STREAM_URL")
                .unwrap_or_else(|_| "ws://insights:10004".into()),
            auto_trader_rsocket_url: std::env::var("AUTO_TRADER_RSOCKET_URL")
                .unwrap_or_else(|_| "ws://auto-trader:10006".into()),
        };
        if config.max_in_flight == 0 || config.max_payload_bytes == 0 {
            bail!("Live worker limits must be positive");
        }
        Ok(config)
    }
}

fn env_usize(name: &str, default: usize) -> anyhow::Result<usize> {
    match std::env::var(name) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("{name} must be a positive integer")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error).with_context(|| format!("could not read {name}")),
    }
}

struct LiveWorker {
    config: LiveWorkerConfig,
    in_flight: Arc<Semaphore>,
    entry_callbacks: Arc<Semaphore>,
    feedback_callbacks: Arc<Semaphore>,
    metrics: Arc<WorkerMetrics>,
    profiles: RwLock<Option<crate::LiveStrategyProfileSet>>,
    market_state_transitions: RwLock<BTreeMap<String, MarketStateTransition>>,
    last_insight_events: RwLock<BTreeMap<String, String>>,
    last_reported_decisions: RwLock<BTreeMap<(String, String), (crate::LiveStrategyDecision, u64)>>,
    decision_report_failures: RwLock<BTreeMap<(String, String), Instant>>,
    pending_insight_updates: Vec<Mutex<BTreeMap<String, crate::StrategyInsightUpdate>>>,
    pending_insight_notifies: Vec<Notify>,
    insight_update_slots: Arc<Semaphore>,
}

const DECISION_REPORT_FAILURE_RETRY_AFTER: Duration = Duration::from_secs(10);
const LIVE_WORKER_CACHE_CAPACITY: usize = 8_192;

impl LiveWorker {
    fn new(config: LiveWorkerConfig) -> Self {
        Self {
            in_flight: Arc::new(Semaphore::new(config.max_in_flight)),
            entry_callbacks: Arc::new(Semaphore::new(2)),
            // Match the four coalesced Insights update lanes so feedback callbacks
            // can make progress concurrently without dropping most lane results.
            feedback_callbacks: Arc::new(Semaphore::new(4)),
            metrics: WorkerMetrics::new("live", config.strategy.as_str(), None),
            config,
            profiles: RwLock::new(None),
            market_state_transitions: RwLock::new(BTreeMap::new()),
            last_insight_events: RwLock::new(BTreeMap::new()),
            last_reported_decisions: RwLock::new(BTreeMap::new()),
            decision_report_failures: RwLock::new(BTreeMap::new()),
            pending_insight_updates: (0..INSIGHTS_UPDATE_SHARD_COUNT)
                .map(|_| Mutex::new(BTreeMap::new()))
                .collect(),
            pending_insight_notifies: (0..INSIGHTS_UPDATE_SHARD_COUNT)
                .map(|_| Notify::new())
                .collect(),
            // Symbol snapshots are coalesced before evaluation. Four lanes
            // preserve parallel scans without flooding downstream route locks.
            insight_update_slots: Arc::new(Semaphore::new(4)),
        }
    }

    async fn apply_profile_set(&self, update: crate::LiveStrategyProfileSet) -> anyhow::Result<()> {
        anyhow::ensure!(
            update.contract_version == crate::LIVE_EVALUATION_CONTRACT_VERSION,
            "unsupported pushed live profile-set contract version {}",
            update.contract_version
        );
        anyhow::ensure!(
            update.strategy == self.config.strategy,
            "pushed profile-set strategy mismatch"
        );
        let mut profile_ids = std::collections::BTreeSet::new();
        for profile in &update.profiles {
            anyhow::ensure!(
                !profile.profile_symbol.trim().is_empty()
                    && profile_ids.insert(profile.profile_id.clone())
                    && crate::profile_id(
                        &profile.profile_symbol,
                        self.config.strategy.as_str(),
                        &profile.parameters,
                    ) == profile.profile_id,
                "pushed profile set contains an invalid or duplicate profile"
            );
            anyhow::ensure!(
                profile
                    .market_state_key
                    .as_deref()
                    .map_or(true, crate::LiveMarketStateClassification::key_is_supported),
                "pushed profile set contains an unsupported market-state key"
            );
        }
        anyhow::ensure!(
            update.open_positions.iter().all(|position| {
                profile_ids.contains(&position.profile_id) && !position.symbol.trim().is_empty()
            }),
            "pushed profile set contains an open position without its profile"
        );
        self.metrics.set_profile_set_version(update.set_version);
        self.metrics.record_profile_update(live_worker_now_ms());
        *self.profiles.write().await = Some(update);
        Ok(())
    }

    async fn report_decision(
        &self,
        report: crate::LiveStrategyDecisionReport,
    ) -> anyhow::Result<()> {
        match crate::request_live_strategy_socket(
            &self.config.auto_trader_rsocket_url,
            &crate::LiveStrategySocketRequest::ReportDecision(report),
        )
        .await?
        {
            crate::LiveStrategySocketResponse::DecisionAccepted { status, .. }
                if (200..300).contains(&status) =>
            {
                Ok(())
            }
            crate::LiveStrategySocketResponse::Rejected { status, detail } => {
                anyhow::bail!("AutoTrader rejected live decision with status {status}: {detail}")
            }
            crate::LiveStrategySocketResponse::DecisionAccepted { status, detail } => {
                anyhow::bail!(
                    "AutoTrader returned non-success status {status} for live decision: {}",
                    detail.unwrap_or_else(|| "no detail".into())
                )
            }
            crate::LiveStrategySocketResponse::ProfilesApplied { .. } => {
                anyhow::bail!("AutoTrader returned a profile acknowledgement for a decision")
            }
            crate::LiveStrategySocketResponse::LearningSnapshotApplied => {
                anyhow::bail!("AutoTrader returned a Learning acknowledgement for a decision")
            }
        }
    }

    async fn report_worker_diagnostic(
        &self,
        update: &crate::StrategyInsightUpdate,
        profile: Option<&crate::LiveStrategyProfile>,
        reason: String,
    ) -> anyhow::Result<()> {
        let Some(signal) = update.signal.as_ref() else {
            return Ok(());
        };
        let diagnostic = crate::LiveStrategyWorkerDiagnostic {
            contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
            event_id: update.event_id.clone(),
            strategy: self.config.strategy,
            symbol: signal.symbol.clone(),
            profile_id: profile.map(|profile| profile.profile_id.clone()),
            event_time_ms: update.event_time_ms,
            reason,
        };
        let response = live_insight_step(
            "AutoTrader worker diagnostic callback",
            Duration::from_secs(3),
            crate::request_live_strategy_socket(
                &self.config.auto_trader_rsocket_url,
                &crate::LiveStrategySocketRequest::ReportDiagnostic(diagnostic),
            ),
        )
        .await?;
        match response {
            crate::LiveStrategySocketResponse::DecisionAccepted { status, .. }
                if (200..300).contains(&status) =>
            {
                Ok(())
            }
            crate::LiveStrategySocketResponse::Rejected { status, detail } => {
                anyhow::bail!(
                    "AutoTrader rejected live worker diagnostic with status {status}: {detail}"
                )
            }
            crate::LiveStrategySocketResponse::DecisionAccepted { status, detail } => {
                anyhow::bail!(
                    "AutoTrader returned non-success status {status} for worker diagnostic: {}",
                    detail.unwrap_or_else(|| "no detail".into())
                )
            }
            crate::LiveStrategySocketResponse::ProfilesApplied { .. } => {
                anyhow::bail!(
                    "AutoTrader returned a profile acknowledgement for a worker diagnostic"
                )
            }
            crate::LiveStrategySocketResponse::LearningSnapshotApplied => {
                anyhow::bail!(
                    "AutoTrader returned a Learning acknowledgement for a worker diagnostic"
                )
            }
        }
    }

    async fn evaluate_insight_update(
        &self,
        update: crate::StrategyInsightUpdate,
    ) -> anyhow::Result<()> {
        if update.dated_carry_scan.is_some()
            || update.dated_carry_candidate.is_some()
            || update.dated_carry_settlement.is_some()
        {
            return self.evaluate_dated_carry_update(update).await;
        }
        if let Some(snapshot) = update.daily_snapshot.clone() {
            if !self.remember_insight_event("daily", &update.event_id).await
                || update.event_time_ms != snapshot.as_of_ms
                || live_worker_now_ms().saturating_sub(update.event_time_ms) > 10 * 60_000
            {
                return Ok(());
            }
            return self.evaluate_daily_snapshot(update, snapshot).await;
        }
        let Some(input_signal) = update.signal.clone() else {
            return Ok(());
        };
        if update.contract_version != crate::STRATEGY_INSIGHT_UPDATE_VERSION
            || update.event_time_ms != input_signal.timestamp
            || input_signal.stale
            || live_worker_now_ms().saturating_sub(update.event_time_ms) > 5_000
            || !self
                .remember_insight_event(&input_signal.symbol, &update.event_id)
                .await
        {
            return Ok(());
        }
        self.metrics.record_insight_update(update.event_time_ms);
        let profile_set = self.profiles.read().await.clone();
        let Some(profile_set) = profile_set else {
            self.report_worker_diagnostic(
                &update,
                None,
                "AutoTrader has not installed a live profile set on this worker".into(),
            )
            .await?;
            return Ok(());
        };
        if !profile_set.enabled {
            self.report_worker_diagnostic(
                &update,
                None,
                "AutoTrader has disabled live evaluation for this strategy".into(),
            )
            .await?;
            return Ok(());
        }
        let open_profile_ids = profile_set
            .open_positions
            .iter()
            .filter(|position| position.symbol == input_signal.symbol)
            .map(|position| position.profile_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let symbol_has_open_position = !open_profile_ids.is_empty();
        let candidate_market_state = crate::classify_live_market_state(&input_signal);
        let (market_state, market_state_transition) = self
            .market_state_transitions
            .write()
            .await
            .entry(input_signal.symbol.clone())
            .or_default()
            .observe(candidate_market_state.clone());
        let matching_profiles = profile_set
            .profiles
            .into_iter()
            .filter(|profile| profile.profile_symbol == input_signal.symbol)
            .collect::<Vec<_>>();
        if matching_profiles.is_empty() {
            self.report_worker_diagnostic(
                &update,
                None,
                "No active live profile is installed for this symbol".into(),
            )
            .await?;
            return Ok(());
        }
        let eligible_profiles = matching_profiles
            .into_iter()
            .filter(|profile| {
                !symbol_has_open_position || open_profile_ids.contains(profile.profile_id.as_str())
            })
            .collect::<Vec<_>>();
        if eligible_profiles.is_empty() {
            self.report_worker_diagnostic(
                &update,
                None,
                "An open position is assigned to a different profile; this profile was skipped"
                    .into(),
            )
            .await?;
            return Ok(());
        }
        // Open positions stay pinned to their opening profile. New entries
        // require a confident exact state and enough regime-scoped evidence;
        // otherwise only an unconditioned fallback is safe.
        let routed_profiles = route_live_profiles(
            eligible_profiles,
            market_state.as_ref(),
            symbol_has_open_position,
        );
        if routed_profiles.is_empty() {
            self.report_worker_diagnostic(
                &update,
                None,
                if candidate_market_state.is_none() {
                    "Market state is unavailable and no unconditioned profile is installed".into()
                } else if market_state.is_none() {
                    "Market state is uncertain and no unconditioned profile is installed".into()
                } else {
                    "Regime profile is missing or under-sampled and no unconditioned profile is installed".into()
                },
            )
            .await?;
            return Ok(());
        }
        for profile in routed_profiles {
            let has_open_position = open_profile_ids.contains(profile.profile_id.as_str());
            let mut signal = match crate::prepare_live_strategy_signal(
                &input_signal,
                self.config.strategy,
                &profile.parameters,
            ) {
                Ok(signal) => signal,
                Err(error) => {
                    self.report_worker_diagnostic(
                        &update,
                        Some(&profile),
                        format!("Not evaluated: {error}"),
                    )
                    .await?;
                    continue;
                }
            };
            signal.strategy_candidate = true;
            if let Some(state) = market_state.as_ref() {
                signal.market_regime = Some(state.evidence_regime.to_owned());
            }
            signal.market_state = market_state.clone();
            signal.strategy_decision = Some(serde_json::json!({
                "strategy": self.config.strategy.as_str(),
                "strategy_parameters": profile.parameters,
                "learning_profile_id": profile.profile_id,
                "learning_profile_symbol": profile.profile_symbol,
                "learning_profile_version": profile.version,
                "strategy_walk_forward": null,
                "direction_source": "insights_subscription",
                "market_state_candidate": candidate_market_state,
                "market_state": market_state,
                "market_state_transition": market_state_transition,
                "profile_regime": profile.market_regime,
                "profile_state_key": profile.market_state_key,
                "profile_fallback_reason": if profile.market_regime.is_none() && profile.market_state_key.is_none() && !symbol_has_open_position { Some(if market_state.is_none() { "market_state_uncertain" } else { "matching_regime_profile_missing_or_under_sampled" }) } else { None },
            }));
            let make_entry_request = |walk_forward_evidence| crate::LiveEvaluationRequest {
                contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
                request_id: format!(
                    "{}:{}:{}",
                    update.event_id,
                    self.config.strategy.as_str(),
                    profile.profile_id
                ),
                strategy: self.config.strategy,
                symbol: signal.symbol.clone(),
                profile: profile.clone(),
                input: crate::LiveEvaluationInput::Entry {
                    signal: signal.clone(),
                    daily_features: None,
                    walk_forward_evidence,
                },
            };
            // Learning evidence arrives with the pushed profile set. The live
            // signal path never calls back into Learning while evaluating.
            let evaluation = live_insight_step(
                "local live strategy evaluation",
                Duration::from_secs(1),
                self.evaluate(&serde_json::to_vec(&make_entry_request(None))?),
            )
            .await;
            let mut response = match evaluation {
                Ok(response) => response,
                Err(error) => {
                    self.report_worker_diagnostic(
                        &update,
                        Some(&profile),
                        format!("Strategy evaluation failed: {error:#}"),
                    )
                    .await?;
                    return Err(error);
                }
            };
            if let Some(error) = response.error.as_ref() {
                self.report_worker_diagnostic(
                    &update,
                    Some(&profile),
                    format!(
                        "Strategy evaluation failed [{:?}]: {}",
                        error.code, error.message
                    ),
                )
                .await?;
                continue;
            }
            let walk_forward_evidence =
                needs_walk_forward_evidence(has_open_position, response.decision)
                    .then(|| walk_forward_evidence_for_signal(&profile, &signal))
                    .flatten();
            if walk_forward_evidence.is_some() {
                let evaluation = live_insight_step(
                    "local live strategy evidence evaluation",
                    Duration::from_secs(1),
                    self.evaluate(&serde_json::to_vec(&make_entry_request(
                        walk_forward_evidence,
                    ))?),
                )
                .await;
                response = match evaluation {
                    Ok(response) => response,
                    Err(error) => {
                        self.report_worker_diagnostic(
                            &update,
                            Some(&profile),
                            format!("Strategy evidence evaluation failed: {error:#}"),
                        )
                        .await?;
                        return Err(error);
                    }
                };
                if let Some(error) = response.error.as_ref() {
                    self.report_worker_diagnostic(
                        &update,
                        Some(&profile),
                        format!(
                            "Strategy evidence evaluation failed [{:?}]: {}",
                            error.code, error.message
                        ),
                    )
                    .await?;
                    continue;
                }
            }
            signal.strategy_decision = Some(serde_json::json!({
                "strategy": self.config.strategy.as_str(),
                "strategy_parameters": profile.parameters,
                "learning_profile_id": profile.profile_id,
                "learning_profile_symbol": profile.profile_symbol,
                "learning_profile_version": profile.version,
                "strategy_walk_forward": walk_forward_evidence,
                "direction_source": "insights_subscription",
            }));
            let missing_entry_decision = !has_open_position && response.decision.is_none();
            let decision = if has_open_position {
                let position_request = crate::LiveEvaluationRequest {
                    contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
                    request_id: format!(
                        "{}:{}:{}:position",
                        update.event_id,
                        self.config.strategy.as_str(),
                        profile.profile_id
                    ),
                    strategy: self.config.strategy,
                    symbol: signal.symbol.clone(),
                    profile: profile.clone(),
                    input: crate::LiveEvaluationInput::OpenPosition {
                        position: crate::LivePositionContext {
                            is_open: true,
                            strategy_enabled: true,
                            signal_active: Some(
                                response.decision == Some(crate::LiveStrategyDecision::Enter),
                            ),
                            rebalance_required: response.decision
                                == Some(crate::LiveStrategyDecision::Rebalance),
                            daily_features: None,
                        },
                    },
                };
                let position_response = live_insight_step(
                    "local open-position strategy evaluation",
                    Duration::from_secs(1),
                    self.evaluate(&serde_json::to_vec(&position_request)?),
                )
                .await;
                match position_response {
                    Ok(response) => {
                        if let Some(error) = response.error.as_ref() {
                            self.report_worker_diagnostic(
                                &update,
                                Some(&profile),
                                format!(
                                    "Open-position evaluation failed [{:?}]: {}",
                                    error.code, error.message
                                ),
                            )
                            .await?;
                            continue;
                        }
                        // A missing evaluator result must not silently drop an
                        // open-position update. Hold is the safe explicit outcome.
                        response
                            .decision
                            .unwrap_or(crate::LiveStrategyDecision::Hold)
                    }
                    Err(error) => {
                        self.report_worker_diagnostic(
                            &update,
                            Some(&profile),
                            format!("Open-position evaluation failed: {error:#}"),
                        )
                        .await?;
                        return Err(error);
                    }
                }
            } else {
                // Preserve the evaluation in the decision stream even if an
                // older or malformed responder returns an empty result.
                response
                    .decision
                    .unwrap_or(crate::LiveStrategyDecision::Reject)
            };
            if (has_open_position
                && !matches!(
                    decision,
                    crate::LiveStrategyDecision::Exit
                        | crate::LiveStrategyDecision::Hold
                        | crate::LiveStrategyDecision::Rebalance
                ))
                || !self
                    .should_report_decision(
                        &signal.symbol,
                        &profile.profile_id,
                        decision,
                        update.event_time_ms,
                    )
                    .await
            {
                continue;
            }
            let priority_callback = matches!(
                decision,
                crate::LiveStrategyDecision::Enter
                    | crate::LiveStrategyDecision::Exit
                    | crate::LiveStrategyDecision::Rebalance
            );
            let _callback_slot = if priority_callback {
                Some(
                    Arc::clone(&self.entry_callbacks)
                        .acquire_owned()
                        .await
                        .context("entry callback semaphore closed")?,
                )
            } else {
                match Arc::clone(&self.feedback_callbacks).try_acquire_owned() {
                    Ok(slot) => Some(slot),
                    Err(_) => {
                        self.report_worker_diagnostic(
                            &update,
                            Some(&profile),
                            "Decision callback capacity is full; worker skipped this update".into(),
                        )
                        .await?;
                        continue;
                    }
                }
            };
            let report = crate::LiveStrategyDecisionReport {
                contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
                event_id: update.event_id.clone(),
                strategy: self.config.strategy,
                symbol: signal.symbol.clone(),
                profile_id: profile.profile_id.clone(),
                profile_version: profile.version,
                parameter_hash: profile.profile_id.clone(),
                event_time_ms: update.event_time_ms,
                decision,
                reason: (decision == crate::LiveStrategyDecision::Reject).then(|| {
                    if missing_entry_decision {
                        return "Entry evaluator returned neither a decision nor an error".into();
                    }
                    let mut input = crate::StrategySignal::entry(&signal, &profile.parameters);
                    input.walk_forward_evidence = walk_forward_evidence;
                    self.config
                        .strategy
                        .entry_rejection_reason(&input)
                        .unwrap_or_else(|| {
                            format!(
                                "{} rejected entry: no rule explanation was produced",
                                self.config.strategy.as_str()
                            )
                        })
                }),
                walk_forward_evidence,
                signal,
                dated_carry_scan: None,
                dated_carry_candidate: None,
                dated_carry_settlement: None,
            };
            let callback_stage = if priority_callback {
                "AutoTrader entry/position callback"
            } else {
                "AutoTrader rejection/hold callback"
            };
            let report_symbol = report.symbol.clone();
            let report_profile_id = report.profile_id.clone();
            let callback = live_insight_step(
                callback_stage,
                Duration::from_secs(3),
                self.report_decision(report),
            )
            .await;
            match callback {
                Ok(()) => {}
                Err(error) => {
                    self.record_decision_report_failure(&report_symbol, &report_profile_id)
                        .await;
                    self.metrics
                        .record_decision_report(update.event_time_ms, false);
                    if let Err(diagnostic_error) = self
                        .report_worker_diagnostic(
                            &update,
                            Some(&profile),
                            format!("AutoTrader rejected or did not receive the decision report: {error:#}"),
                        )
                        .await
                    {
                        eprintln!(
                            "WARN {} could not report decision callback failure: {diagnostic_error:#}",
                            self.config.strategy.as_str()
                        );
                    }
                    return Err(error);
                }
            }
            self.record_reported_decision(
                &report_symbol,
                &report_profile_id,
                decision,
                update.event_time_ms,
            )
            .await;
            self.decision_report_failures
                .write()
                .await
                .remove(&(report_symbol.clone(), report_profile_id.clone()));
            self.metrics
                .record_decision_report(update.event_time_ms, true);
        }
        Ok(())
    }

    async fn evaluate_dated_carry_update(
        &self,
        update: crate::StrategyInsightUpdate,
    ) -> anyhow::Result<()> {
        if self.config.strategy != Strategy::DatedCarry
            || live_worker_now_ms().saturating_sub(update.event_time_ms) > 120_000
            || !self
                .remember_insight_event(
                    &format!("dated-carry:{}", update.event_id),
                    &update.event_id,
                )
                .await
        {
            return Ok(());
        }
        if let Some(scan) = update.dated_carry_scan {
            let signal = carry_placeholder_signal("BTCUSDT", update.event_time_ms, None);
            let report = crate::LiveStrategyDecisionReport {
                contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
                event_id: update.event_id,
                strategy: Strategy::DatedCarry,
                symbol: signal.symbol.clone(),
                profile_id: String::new(),
                profile_version: 0,
                parameter_hash: String::new(),
                event_time_ms: scan.emitted_at_ms,
                decision: crate::LiveStrategyDecision::Hold,
                reason: scan.last_error.clone(),
                walk_forward_evidence: None,
                signal,
                dated_carry_scan: Some(scan),
                dated_carry_candidate: None,
                dated_carry_settlement: None,
            };
            return self.report_dated_carry_decision(report).await;
        }
        if let Some(candidate) = update.dated_carry_candidate {
            let profile_symbol = candidate.spot_symbol.replace("USDT", "-USDT");
            let profile_set = self.profiles.read().await.clone();
            let Some(profile_set) = profile_set.filter(|set| set.enabled) else {
                return Ok(());
            };
            let Some(profile) = profile_set
                .profiles
                .into_iter()
                .find(|profile| profile.profile_symbol == profile_symbol)
            else {
                return Ok(());
            };
            let request = crate::LiveEvaluationRequest {
                contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
                request_id: format!("{}:{}:entry", update.event_id, profile.profile_id),
                strategy: Strategy::DatedCarry,
                symbol: candidate.spot_symbol.clone(),
                profile: profile.clone(),
                input: crate::LiveEvaluationInput::DatedCarryEntry {
                    emitted_at_ms: update.event_time_ms,
                    evaluated_at_ms: live_worker_now_ms(),
                    candidate: candidate.clone(),
                },
            };
            let decision = self
                .evaluate(&serde_json::to_vec(&request)?)
                .await?
                .decision
                .unwrap_or(crate::LiveStrategyDecision::Reject);
            let signal = carry_placeholder_signal(
                &candidate.spot_symbol,
                update.event_time_ms,
                Some(&profile),
            );
            let report = crate::LiveStrategyDecisionReport {
                contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
                event_id: update.event_id,
                strategy: Strategy::DatedCarry,
                symbol: candidate.spot_symbol.clone(),
                profile_id: profile.profile_id.clone(),
                profile_version: profile.version,
                parameter_hash: profile.profile_id.clone(),
                event_time_ms: update.event_time_ms,
                decision,
                reason: None,
                walk_forward_evidence: None,
                signal,
                dated_carry_scan: None,
                dated_carry_candidate: Some(candidate),
                dated_carry_settlement: None,
            };
            return self.report_dated_carry_decision(report).await;
        }
        if let Some(settlement) = update.dated_carry_settlement {
            let profile_symbol = settlement.spot_symbol.replace("USDT", "-USDT");
            let profile_set = self.profiles.read().await.clone();
            let Some(profile_set) = profile_set else {
                return Ok(());
            };
            let Some(profile) = profile_set
                .profiles
                .into_iter()
                .find(|profile| profile.profile_symbol == profile_symbol)
            else {
                return Ok(());
            };
            let request = crate::LiveEvaluationRequest {
                contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
                request_id: format!("{}:{}:settlement", update.event_id, profile.profile_id),
                strategy: Strategy::DatedCarry,
                symbol: settlement.spot_symbol.clone(),
                profile: profile.clone(),
                input: crate::LiveEvaluationInput::Settlement {
                    emitted_at_ms: settlement.emitted_at_ms,
                    evaluated_at_ms: live_worker_now_ms(),
                    spot_symbol: settlement.spot_symbol.clone(),
                    future_symbol: settlement.future_symbol.clone(),
                    expiry_ms: settlement.expiry_ms,
                    spot_exit_twap: settlement.spot_exit_twap,
                    settlement_price: settlement.settlement_price,
                    settlement_source: Some(settlement.settlement_source),
                },
            };
            let decision = self
                .evaluate(&serde_json::to_vec(&request)?)
                .await?
                .decision
                .unwrap_or(crate::LiveStrategyDecision::Hold);
            if decision != crate::LiveStrategyDecision::Exit {
                return Ok(());
            }
            let signal = carry_placeholder_signal(
                &settlement.spot_symbol,
                update.event_time_ms,
                Some(&profile),
            );
            let report = crate::LiveStrategyDecisionReport {
                contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
                event_id: update.event_id,
                strategy: Strategy::DatedCarry,
                symbol: settlement.spot_symbol.clone(),
                profile_id: profile.profile_id.clone(),
                profile_version: profile.version,
                parameter_hash: profile.profile_id.clone(),
                event_time_ms: update.event_time_ms,
                decision,
                reason: None,
                walk_forward_evidence: None,
                signal,
                dated_carry_scan: None,
                dated_carry_candidate: None,
                dated_carry_settlement: Some(settlement),
            };
            return self.report_dated_carry_decision(report).await;
        }
        Ok(())
    }

    async fn report_dated_carry_decision(
        &self,
        report: crate::LiveStrategyDecisionReport,
    ) -> anyhow::Result<()> {
        let event_time_ms = report.event_time_ms;
        let symbol = report.symbol.clone();
        match self.report_decision(report).await {
            Ok(()) => self.metrics.record_decision_report(event_time_ms, true),
            Err(error) => {
                self.metrics.record_decision_report(event_time_ms, false);
                return Err(error).context(format!(
                    "AutoTrader dated-carry socket report failed for {symbol}"
                ));
            }
        }
        Ok(())
    }

    async fn evaluate_daily_snapshot(
        &self,
        update: crate::StrategyInsightUpdate,
        snapshot: crate::daily::DailyStrategySnapshot,
    ) -> anyhow::Result<()> {
        let Some(profile_set) = self.profiles.read().await.clone().filter(|set| set.enabled) else {
            return Ok(());
        };
        let open_positions = profile_set.open_positions;
        let signals = snapshot
            .generated_signals
            .iter()
            .filter(|signal| signal.strategy.as_deref() == Some(self.config.strategy.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let mut symbols = signals
            .iter()
            .map(|signal| signal.symbol.clone())
            .collect::<std::collections::BTreeSet<_>>();
        symbols.extend(
            open_positions
                .iter()
                .map(|position| position.symbol.clone()),
        );
        for symbol in symbols {
            let open_position = open_positions
                .iter()
                .find(|position| position.symbol == symbol);
            let source_signal = signals
                .iter()
                .find(|signal| signal.symbol == symbol)
                .cloned();
            let profile_symbol = open_position
                .map(|position| position.profile_symbol.clone())
                .or_else(|| {
                    source_signal.as_ref().and_then(|signal| {
                        signal
                            .strategy_decision
                            .as_ref()
                            .and_then(|decision| {
                                decision
                                    .get("learning_profile_symbol")
                                    .or_else(|| decision.get("scout_parameter_profile_symbol"))
                            })
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    })
                })
                .unwrap_or_else(|| symbol.clone());
            let mut profile_candidates = self
                .profiles
                .read()
                .await
                .clone()
                .filter(|set| set.enabled)
                .map(|set| {
                    set.profiles
                        .into_iter()
                        .filter(|profile| profile.profile_symbol == profile_symbol)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if let Some(open_position) = open_position {
                profile_candidates.retain(|profile| profile.profile_id == open_position.profile_id);
            } else if let Some(parameters) = source_signal
                .as_ref()
                .and_then(|signal| signal.strategy_decision.as_ref())
                .and_then(|decision| decision.get("strategy_parameters"))
                .cloned()
                .and_then(|value| serde_json::from_value::<BTreeMap<String, f64>>(value).ok())
            {
                profile_candidates.retain(|profile| profile.parameters == parameters);
            }
            let signal_template = source_signal.or_else(|| {
                open_position.map(|_| crate::StrategyMarketSignal {
                    symbol: symbol.clone(),
                    timestamp: snapshot.as_of_ms,
                    stale: false,
                    strategy: Some(self.config.strategy.as_str().to_owned()),
                    ..Default::default()
                })
            });
            let Some(signal_template) = signal_template else {
                continue;
            };
            for profile in profile_candidates {
                let mut signal = signal_template.clone();
                let is_open = open_positions.iter().any(|position| {
                    position.symbol == symbol && position.profile_id == profile.profile_id
                });
                let daily_features = signal
                    .strategy_decision
                    .as_ref()
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok());
                let walk_forward_evidence = if is_open {
                    None
                } else {
                    walk_forward_evidence_for_signal(&profile, &signal)
                };
                signal.timestamp = update.event_time_ms;
                signal.strategy = Some(self.config.strategy.as_str().to_owned());
                signal.strategy_candidate = true;
                let mut decision_data = signal
                    .strategy_decision
                    .take()
                    .unwrap_or_else(|| serde_json::json!({}));
                if !decision_data.is_object() {
                    decision_data = serde_json::json!({});
                }
                decision_data["strategy"] = serde_json::json!(self.config.strategy.as_str());
                decision_data["strategy_parameters"] = serde_json::to_value(&profile.parameters)?;
                decision_data["learning_profile_id"] = serde_json::json!(profile.profile_id);
                decision_data["learning_profile_symbol"] =
                    serde_json::json!(profile.profile_symbol);
                decision_data["learning_profile_version"] = serde_json::json!(profile.version);
                if let Some(evidence) = walk_forward_evidence {
                    decision_data["strategy_walk_forward"] = serde_json::to_value(evidence)?;
                }
                signal.strategy_decision = Some(decision_data);
                let input = if is_open {
                    crate::LiveEvaluationInput::OpenPosition {
                        position: crate::LivePositionContext {
                            is_open: true,
                            strategy_enabled: true,
                            signal_active: snapshot
                                .signal_active(self.config.strategy.as_str(), &symbol),
                            rebalance_required: snapshot.strategy_rebalance_required(
                                self.config.strategy.as_str(),
                                &symbol,
                                signal.strategy_decision.as_ref(),
                            ),
                            daily_features,
                        },
                    }
                } else {
                    crate::LiveEvaluationInput::Entry {
                        signal: signal.clone(),
                        daily_features,
                        walk_forward_evidence,
                    }
                };
                let request = crate::LiveEvaluationRequest {
                    contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
                    request_id: format!(
                        "{}:{}:{}:{}",
                        update.event_id,
                        self.config.strategy.as_str(),
                        profile.profile_id,
                        if is_open { "position" } else { "entry" }
                    ),
                    strategy: self.config.strategy,
                    symbol: symbol.clone(),
                    profile: profile.clone(),
                    input,
                };
                let decision = self
                    .evaluate(&serde_json::to_vec(&request)?)
                    .await?
                    .decision
                    .unwrap_or(crate::LiveStrategyDecision::Hold);
                if (is_open
                    && !matches!(
                        decision,
                        crate::LiveStrategyDecision::Exit
                            | crate::LiveStrategyDecision::Hold
                            | crate::LiveStrategyDecision::Rebalance
                    ))
                    || !self
                        .should_report_decision(
                            &symbol,
                            &profile.profile_id,
                            decision,
                            update.event_time_ms,
                        )
                        .await
                {
                    continue;
                }
                let report = crate::LiveStrategyDecisionReport {
                    contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
                    event_id: update.event_id.clone(),
                    strategy: self.config.strategy,
                    symbol: symbol.clone(),
                    profile_id: profile.profile_id.clone(),
                    profile_version: profile.version,
                    parameter_hash: profile.profile_id.clone(),
                    event_time_ms: update.event_time_ms,
                    decision,
                    reason: None,
                    walk_forward_evidence,
                    signal,
                    dated_carry_scan: None,
                    dated_carry_candidate: None,
                    dated_carry_settlement: None,
                };
                let report_time_ms = report.event_time_ms;
                match self.report_decision(report).await {
                    Ok(()) => self.metrics.record_decision_report(report_time_ms, true),
                    Err(error) => {
                        self.metrics.record_decision_report(report_time_ms, false);
                        return Err(error).with_context(|| {
                            format!("AutoTrader daily decision socket report failed for {symbol}")
                        });
                    }
                }
            }
        }
        Ok(())
    }

    async fn remember_insight_event(&self, key: &str, event_id: &str) -> bool {
        let mut seen = self.last_insight_events.write().await;
        if seen.get(key).is_some_and(|previous| previous == event_id) {
            return false;
        }
        if seen.len() >= LIVE_WORKER_CACHE_CAPACITY && !seen.contains_key(key) {
            if let Some(oldest) = seen.keys().next().cloned() {
                seen.remove(&oldest);
            }
        }
        seen.insert(key.to_owned(), event_id.to_owned());
        true
    }

    async fn should_report_decision(
        &self,
        symbol: &str,
        profile_id: &str,
        decision: crate::LiveStrategyDecision,
        timestamp_ms: u64,
    ) -> bool {
        let key = (symbol.to_owned(), profile_id.to_owned());
        {
            let mut failures = self.decision_report_failures.write().await;
            failures.retain(|_, failed_at| failed_at.elapsed() < Duration::from_secs(10 * 60));
            if failures
                .get(&key)
                .is_some_and(|failed_at| failed_at.elapsed() < DECISION_REPORT_FAILURE_RETRY_AFTER)
            {
                return false;
            }
            failures.remove(&key);
        }
        let mut cache = self.last_reported_decisions.write().await;
        cache.retain(|_, (_, timestamp)| timestamp_ms.saturating_sub(*timestamp) <= 10 * 60_000);
        let previous = cache.get(&key).copied();
        let interval_ms = match decision {
            crate::LiveStrategyDecision::Enter
            | crate::LiveStrategyDecision::Exit
            | crate::LiveStrategyDecision::Rebalance => 5_000,
            // Refresh every rejection so the dashboard reflects each fresh
            // evaluation instead of displaying a stale blocker for five minutes.
            crate::LiveStrategyDecision::Reject => 0,
            crate::LiveStrategyDecision::Hold => 30_000,
        };
        let should_report = previous.is_none_or(|(previous_decision, previous_at)| {
            previous_decision != decision || timestamp_ms.saturating_sub(previous_at) >= interval_ms
        });
        should_report
    }

    async fn record_reported_decision(
        &self,
        symbol: &str,
        profile_id: &str,
        decision: crate::LiveStrategyDecision,
        timestamp_ms: u64,
    ) {
        let key = (symbol.to_owned(), profile_id.to_owned());
        let mut cache = self.last_reported_decisions.write().await;
        cache.retain(|_, (_, timestamp)| timestamp_ms.saturating_sub(*timestamp) <= 10 * 60_000);
        if cache.len() >= LIVE_WORKER_CACHE_CAPACITY && !cache.contains_key(&key) {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, (_, timestamp))| *timestamp)
                .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(key, (decision, timestamp_ms));
    }

    async fn record_decision_report_failure(&self, symbol: &str, profile_id: &str) {
        let key = (symbol.to_owned(), profile_id.to_owned());
        let mut failures = self.decision_report_failures.write().await;
        if failures.len() >= LIVE_WORKER_CACHE_CAPACITY && !failures.contains_key(&key) {
            if let Some(oldest) = failures
                .iter()
                .min_by_key(|(_, failed_at)| **failed_at)
                .map(|(key, _)| key.clone())
            {
                failures.remove(&oldest);
            }
        }
        failures.insert(key, Instant::now());
    }

    async fn evaluate(&self, bytes: &[u8]) -> anyhow::Result<LiveEvaluationResponse> {
        self.metrics.begin();
        let started = std::time::Instant::now();
        let result = self.evaluate_inner(bytes).await;
        self.metrics.finish(
            started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            result
                .as_ref()
                .err()
                .map(|error| WorkerMetrics::classify_error(&error.to_string())),
        );
        result
    }

    async fn evaluate_inner(&self, bytes: &[u8]) -> anyhow::Result<LiveEvaluationResponse> {
        if bytes.len() > self.config.max_payload_bytes {
            bail!("live evaluation exceeds configured payload limit");
        }
        let _permit = Arc::clone(&self.in_flight)
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("live worker is at its configured in-flight limit"))?;
        let request: LiveEvaluationRequest =
            serde_json::from_slice(bytes).context("invalid live evaluation request")?;
        let mut response = LiveEvaluationResponse {
            contract_version: LIVE_EVALUATION_CONTRACT_VERSION,
            request_id: request.request_id.clone(),
            strategy: request.strategy,
            symbol: request.symbol.clone(),
            profile_id: request.profile.profile_id.clone(),
            profile_symbol: request.profile.profile_symbol.clone(),
            profile_version: request.profile.version,
            decision: None,
            error: None,
        };
        match evaluate_request(&request, self.config.strategy) {
            Ok(decision) => response.decision = Some(decision),
            Err(error) => response.error = Some(error),
        }
        Ok(response)
    }
}

fn invalid(code: LiveEvaluationErrorCode, message: impl Into<String>) -> LiveEvaluationError {
    LiveEvaluationError {
        code,
        message: message.into(),
    }
}

fn evaluate_request(
    request: &LiveEvaluationRequest,
    worker_strategy: Strategy,
) -> Result<LiveStrategyDecision, LiveEvaluationError> {
    if request.contract_version != LIVE_EVALUATION_CONTRACT_VERSION {
        return Err(invalid(
            LiveEvaluationErrorCode::UnsupportedVersion,
            format!("unsupported contract version {}", request.contract_version),
        ));
    }
    if request.request_id.trim().is_empty() {
        return Err(invalid(
            LiveEvaluationErrorCode::InvalidRequest,
            "request ID is required",
        ));
    }
    if request.strategy != worker_strategy {
        return Err(invalid(
            LiveEvaluationErrorCode::WrongStrategy,
            format!(
                "request for {} routed to {} worker",
                request.strategy.as_str(),
                worker_strategy.as_str()
            ),
        ));
    }
    if request.symbol.trim().is_empty() {
        return Err(invalid(
            LiveEvaluationErrorCode::InvalidSymbol,
            "symbol is required",
        ));
    }
    if request.profile.profile_id.trim().is_empty()
        || request.profile.profile_symbol.trim().is_empty()
    {
        return Err(invalid(
            LiveEvaluationErrorCode::InvalidProfile,
            "profile ID and profile symbol are required",
        ));
    }
    let specs = crate::parameter_specs(worker_strategy);
    for (key, value) in &request.profile.parameters {
        let Some(spec) = specs.iter().find(|spec| spec.key.as_str() == key) else {
            return Err(invalid(
                LiveEvaluationErrorCode::InvalidParameters,
                format!("unknown parameter {key}"),
            ));
        };
        let step_position = (*value - spec.min) / spec.step;
        if !value.is_finite()
            || *value < spec.min
            || *value > spec.max
            || spec.step <= 0.0
            || (step_position - step_position.round()).abs() > 1e-6
        {
            return Err(invalid(
                LiveEvaluationErrorCode::InvalidParameters,
                format!("parameter {key} is outside its bounds"),
            ));
        }
    }
    let expected_profile_id = crate::profile_id(
        &request.profile.profile_symbol,
        request.strategy.as_str(),
        &request.profile.parameters,
    );
    if request.profile.profile_id != expected_profile_id {
        return Err(invalid(
            LiveEvaluationErrorCode::InvalidProfile,
            "profile ID does not match profile symbol, strategy, and parameters",
        ));
    }

    let parameters = &request.profile.parameters;
    let input = match &request.input {
        LiveEvaluationInput::Entry {
            signal,
            daily_features,
            walk_forward_evidence,
        } => {
            if signal.symbol != request.symbol {
                return Err(invalid(
                    LiveEvaluationErrorCode::InvalidSymbol,
                    "signal symbol does not match request symbol",
                ));
            }
            if signal.strategy.as_deref().and_then(Strategy::from_name) != Some(request.strategy) {
                return Err(invalid(
                    LiveEvaluationErrorCode::WrongStrategy,
                    "signal strategy does not match request strategy",
                ));
            }
            let mut input = StrategyInput::entry(signal, parameters);
            input.daily_features = *daily_features;
            input.walk_forward_evidence = *walk_forward_evidence;
            input
        }
        LiveEvaluationInput::OpenPosition { position } => {
            if !position.is_open {
                return Err(invalid(
                    LiveEvaluationErrorCode::InvalidRequest,
                    "open-position evaluation requires an open position",
                ));
            }
            let mut input = StrategyInput::position(
                parameters,
                position.strategy_enabled,
                position.signal_active,
                position.rebalance_required,
            );
            input.daily_features = position.daily_features;
            input
        }
        LiveEvaluationInput::DatedCarryEntry {
            emitted_at_ms,
            evaluated_at_ms,
            candidate,
        } => {
            if request.strategy != Strategy::DatedCarry || candidate.spot_symbol != request.symbol {
                return Err(invalid(
                    LiveEvaluationErrorCode::InvalidSymbol,
                    "dated-carry candidate does not match strategy or symbol",
                ));
            }
            StrategyInput::dated_carry_entry(
                candidate,
                *emitted_at_ms,
                *evaluated_at_ms,
                parameters,
            )
        }
        LiveEvaluationInput::Settlement {
            emitted_at_ms,
            evaluated_at_ms,
            spot_symbol,
            future_symbol,
            expiry_ms,
            spot_exit_twap,
            settlement_price,
            settlement_source,
        } => {
            if request.strategy != Strategy::DatedCarry
                || spot_symbol != &request.symbol
                || future_symbol.trim().is_empty()
            {
                return Err(invalid(
                    LiveEvaluationErrorCode::InvalidSymbol,
                    "settlement does not match dated-carry strategy or request symbol",
                ));
            }
            StrategyInput::dated_carry_settlement(
                *emitted_at_ms,
                *evaluated_at_ms,
                spot_symbol,
                future_symbol,
                *expiry_ms,
                *spot_exit_twap,
                *settlement_price,
                *settlement_source,
                parameters,
            )
        }
    };
    let evaluated = worker_strategy
        .evaluate(&input)
        .map_err(|message| invalid(LiveEvaluationErrorCode::EvaluationFailed, message))?;
    Ok(match evaluated {
        StrategyDecision::Enter => LiveStrategyDecision::Enter,
        StrategyDecision::Hold => LiveStrategyDecision::Hold,
        StrategyDecision::Exit => LiveStrategyDecision::Exit,
        StrategyDecision::Rebalance => LiveStrategyDecision::Rebalance,
        // Scout is a strategy-internal research result, never a live action.
        StrategyDecision::Scout | StrategyDecision::Reject => LiveStrategyDecision::Reject,
    })
}

#[derive(Clone)]
struct Responder(Arc<LiveWorker>);

#[rsocket_rust::async_trait]
impl RSocket for Responder {
    async fn metadata_push(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("live evaluation requires request-response")
    }
    async fn fire_and_forget(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("live evaluation requires correlated request-response")
    }
    async fn request_response(&self, req: Payload) -> anyhow::Result<Option<Payload>> {
        let bytes = req.data().context("live evaluation payload is empty")?;
        if let Ok(socket_request) =
            serde_json::from_slice::<crate::LiveStrategySocketRequest>(bytes.as_ref())
        {
            let socket_response = match socket_request {
                crate::LiveStrategySocketRequest::UpdateProfiles(profile_set) => {
                    let strategy = profile_set.strategy;
                    let set_version = profile_set.set_version;
                    match self.0.apply_profile_set(profile_set).await {
                        Ok(()) => crate::LiveStrategySocketResponse::ProfilesApplied {
                            strategy,
                            set_version,
                        },
                        Err(error) => {
                            self.0.metrics.record_profile_update_failure();
                            crate::LiveStrategySocketResponse::Rejected {
                                status: 422,
                                detail: format!("{error:#}"),
                            }
                        }
                    }
                }
                crate::LiveStrategySocketRequest::UpdateLearningSnapshot(_) => {
                    crate::LiveStrategySocketResponse::Rejected {
                        status: 400,
                        detail: "live workers accept profile-set updates, not Learning snapshots"
                            .into(),
                    }
                }
                crate::LiveStrategySocketRequest::ReportDecision(_) => {
                    crate::LiveStrategySocketResponse::Rejected {
                        status: 400,
                        detail: "live workers accept profile updates, not decision reports".into(),
                    }
                }
                crate::LiveStrategySocketRequest::ReportDiagnostic(_) => {
                    crate::LiveStrategySocketResponse::Rejected {
                        status: 400,
                        detail: "live workers cannot accept worker diagnostics".into(),
                    }
                }
            };
            return Ok(Some(
                Payload::builder()
                    .set_data(serde_json::to_vec(&socket_response)?)
                    .build(),
            ));
        }
        let response = self.0.evaluate(bytes).await?;
        Ok(Some(
            Payload::builder()
                .set_data(serde_json::to_vec(&response)?)
                .build(),
        ))
    }
    fn request_stream(
        &self,
        _req: Payload,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
        Box::pin(stream::once(async {
            Err(anyhow::anyhow!("live worker does not stream work"))
        }))
    }
    fn request_channel(
        &self,
        _reqs: Flux<anyhow::Result<Payload>>,
    ) -> Flux<anyhow::Result<Payload>> {
        Box::pin(stream::once(async {
            Err(anyhow::anyhow!("live worker accepts request-response only"))
        }))
    }
}

pub async fn serve_live_worker(config: LiveWorkerConfig) -> anyhow::Result<()> {
    let worker = Arc::new(LiveWorker::new(config.clone()));
    let metrics_server = serve_metrics(config.metrics_listen_addr, Arc::clone(&worker.metrics));
    let insights_worker = Arc::clone(&worker);
    let insights_subscriber = run_insights_subscription(insights_worker);
    eprintln!(
        "Strategy live worker for {} listening on ws://{} (max in-flight {}, max payload {} bytes)",
        config.strategy.as_str(),
        config.listen_addr,
        config.max_in_flight,
        config.max_payload_bytes
    );
    let rsocket_server = RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(config.listen_addr))
        .acceptor(Box::new(move |_setup, _socket| {
            Ok(Box::new(Responder(Arc::clone(&worker))))
        }))
        .serve();
    tokio::try_join!(
        async {
            rsocket_server
                .await
                .context("Strategy live RSocket worker stopped")
        },
        metrics_server,
        insights_subscriber,
    )?;
    Ok(())
}

async fn connect_insights_strategy_stream(
    url: &str,
    strategy: Strategy,
) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<crate::StrategyInsightUpdate>> + Send>>>
{
    let client = RSocketFactory::connect()
        .transport(WebsocketClientTransport::from(url))
        .acceptor(Box::new(|| Box::new(EchoRSocket)))
        .start()
        .await
        .context("Insights strategy-stream connection failed")?;
    let source = client.request_stream(
        Payload::builder()
            .set_data_utf8(if strategy.is_live_insight() {
                "strategy-signals:live"
            } else {
                "strategy-signals:maintenance"
            })
            .build(),
    );
    Ok(Box::pin(source.map(move |item| {
        let _client = &client;
        let payload = item.map_err(anyhow::Error::from)?;
        let data = payload
            .data()
            .context("Insights returned an empty strategy signal")?;
        serde_json::from_slice(data).context("invalid Insights strategy update")
    })))
}

const INSIGHTS_UPDATE_SHARD_COUNT: usize = 32;
// Insights publishes live snapshots continuously. A quiet stream for this long
// almost always means the RSocket connection survived an Insights restart but
// no longer has a live publisher behind it. Break the stream so the outer loop
// reconnects and receives the server's latest-signal replay.
const INSIGHTS_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

fn needs_walk_forward_evidence(
    has_open_position: bool,
    decision: Option<crate::LiveStrategyDecision>,
) -> bool {
    !has_open_position && decision == Some(crate::LiveStrategyDecision::Enter)
}

fn walk_forward_evidence_for_signal(
    profile: &crate::LiveStrategyProfile,
    signal: &crate::StrategyMarketSignal,
) -> Option<crate::StrategyWalkForwardEvidence> {
    crate::market_regime(signal)
        .and_then(|regime| profile.walk_forward_evidence_by_regime.get(regime).copied())
        .or(profile.walk_forward_evidence)
}

fn insight_update_key(update: &crate::StrategyInsightUpdate) -> String {
    if let Some(signal) = update.signal.as_ref() {
        return format!("signal:{}", signal.symbol);
    }
    if update.daily_snapshot.is_some() {
        return "daily-snapshot".to_owned();
    }
    update.event_id.clone()
}

fn store_latest_insight_update(
    pending: &mut BTreeMap<String, crate::StrategyInsightUpdate>,
    update: crate::StrategyInsightUpdate,
) {
    let key = insight_update_key(&update);
    if pending
        .get(&key)
        .is_none_or(|current| update.event_time_ms > current.event_time_ms)
    {
        pending.insert(key, update);
    }
}

fn insights_update_shard(update: &crate::StrategyInsightUpdate) -> usize {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    update
        .signal
        .as_ref()
        .map(|signal| signal.symbol.as_str())
        .unwrap_or(&update.event_id)
        .hash(&mut hasher);
    hasher.finish() as usize % INSIGHTS_UPDATE_SHARD_COUNT
}

async fn process_one_insight_update(
    worker: Arc<LiveWorker>,
    update: crate::StrategyInsightUpdate,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let is_stale = |now_ms: u64| {
        update.signal.is_some() && now_ms.saturating_sub(update.event_time_ms) > 5_000
    };
    if is_stale(live_worker_now_ms()) {
        return;
    }
    worker.metrics.record_insight_update(update.event_time_ms);
    match tokio::time::timeout(
        Duration::from_secs(5),
        worker.evaluate_insight_update(update),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            worker.metrics.record_insight_update_failure(false);
            eprintln!(
                "ERROR {} failed to evaluate/report Insights update: {error:#}",
                worker.config.strategy.as_str()
            );
        }
        Err(_) => {
            worker.metrics.record_insight_update_failure(true);
            eprintln!(
                "ERROR {} timed out evaluating/reporting an Insights update after 5 seconds",
                worker.config.strategy.as_str()
            );
        }
    }
}

async fn process_insights_update_shard(worker: Arc<LiveWorker>, shard: usize) {
    loop {
        if worker.pending_insight_updates[shard]
            .lock()
            .await
            .is_empty()
        {
            worker.pending_insight_notifies[shard].notified().await;
            continue;
        }
        // Acquire capacity before dequeueing. While waiting, updates continue
        // to overwrite per-symbol slots instead of piling up behind this task.
        let permit = match Arc::clone(&worker.insight_update_slots)
            .acquire_owned()
            .await
        {
            Ok(permit) => permit,
            Err(_) => return,
        };
        let update = {
            let mut pending = worker.pending_insight_updates[shard].lock().await;
            let newest_key = pending
                .iter()
                .max_by_key(|(_, update)| update.event_time_ms)
                .map(|(key, _)| key.clone());
            newest_key.and_then(|key| pending.remove(&key))
        };
        if let Some(update) = update {
            process_one_insight_update(Arc::clone(&worker), update, permit).await;
        }
    }
}

async fn live_insight_step<T>(
    name: &'static str,
    timeout: Duration,
    future: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    tokio::time::timeout(timeout, future)
        .await
        .with_context(|| format!("{name} timed out"))?
        .with_context(|| format!("{name} failed"))
}

async fn run_insights_subscription(worker: Arc<LiveWorker>) -> anyhow::Result<()> {
    let mut retry_delay = Duration::from_millis(500);
    loop {
        match connect_insights_strategy_stream(
            &worker.config.insights_stream_url,
            worker.config.strategy,
        )
        .await
        {
            Ok(mut stream) => {
                worker.metrics.set_insights_connected(true);
                eprintln!(
                    "{} subscribed to Insights strategy updates",
                    worker.config.strategy.as_str()
                );
                retry_delay = Duration::from_millis(500);
                let mut update_tasks = JoinSet::new();
                for shard in 0..INSIGHTS_UPDATE_SHARD_COUNT {
                    update_tasks.spawn(process_insights_update_shard(Arc::clone(&worker), shard));
                    // A prior stream task may have been aborted with queued
                    // snapshots still in its latest-value map.
                    worker.pending_insight_notifies[shard].notify_one();
                }
                loop {
                    let next_update =
                        match tokio::time::timeout(INSIGHTS_STREAM_IDLE_TIMEOUT, stream.next())
                            .await
                        {
                            Ok(next_update) => next_update,
                            Err(_) => {
                                worker.metrics.set_insights_connected(false);
                                eprintln!(
                                    "WARN {} Insights strategy stream idle for {:?}; reconnecting",
                                    worker.config.strategy.as_str(),
                                    INSIGHTS_STREAM_IDLE_TIMEOUT
                                );
                                break;
                            }
                        };
                    let update = match next_update {
                        Some(Ok(update))
                            if update.contract_version
                                == crate::STRATEGY_INSIGHT_UPDATE_VERSION =>
                        {
                            update
                        }
                        Some(Ok(update)) => {
                            worker.metrics.set_insights_connected(false);
                            eprintln!(
                                "WARN {} Insights update version mismatch: {}",
                                worker.config.strategy.as_str(),
                                update.contract_version
                            );
                            break;
                        }
                        Some(Err(error)) => {
                            worker.metrics.set_insights_connected(false);
                            eprintln!(
                                "WARN {} Insights stream needs resubscribe after update gap: {error}",
                                worker.config.strategy.as_str()
                            );
                            break;
                        }
                        None => break,
                    };
                    let shard = insights_update_shard(&update);
                    {
                        let mut pending = worker.pending_insight_updates[shard].lock().await;
                        store_latest_insight_update(&mut pending, update);
                    }
                    worker.pending_insight_notifies[shard].notify_one();
                }
                worker.metrics.set_insights_connected(false);
                update_tasks.abort_all();
                while update_tasks.join_next().await.is_some() {}
                eprintln!(
                    "WARN {} Insights strategy stream closed; reconnecting",
                    worker.config.strategy.as_str()
                );
            }
            Err(error) => {
                worker.metrics.set_insights_connected(false);
                eprintln!(
                    "WARN {} could not subscribe to Insights: {error}",
                    worker.config.strategy.as_str()
                );
            }
        }
        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_secs(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SignalComponents, StrategyMarketSignal};
    use rsocket_rust::utils::EchoRSocket;
    use rsocket_rust_transport_websocket::WebsocketClientTransport;
    use std::collections::BTreeMap;
    use tokio::time::sleep;

    fn test_live_worker() -> LiveWorker {
        LiveWorker::new(LiveWorkerConfig {
            strategy: Strategy::LiveMomentum,
            listen_addr: "127.0.0.1:0".into(),
            max_in_flight: 1,
            max_payload_bytes: 64 * 1024,
            metrics_listen_addr: "127.0.0.1:0".into(),
            insights_stream_url: "ws://127.0.0.1:1".into(),
            auto_trader_rsocket_url: "ws://127.0.0.1:1".into(),
        })
    }

    fn pushed_profile_set(set_version: u64, symbol: &str) -> crate::LiveStrategyProfileSet {
        let parameters = Strategy::LiveMomentum
            .parameter_specs()
            .into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let profile = crate::LiveStrategyProfile {
            profile_id: crate::profile_id(symbol, Strategy::LiveMomentum.as_str(), &parameters),
            profile_symbol: symbol.to_owned(),
            version: set_version,
            parameters,
            walk_forward_evidence: None,
            market_regime: None,
            market_state_key: None,
            walk_forward_evidence_by_regime: Default::default(),
        };
        crate::LiveStrategyProfileSet {
            contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
            set_version,
            strategy: Strategy::LiveMomentum,
            enabled: true,
            profiles: vec![profile],
            open_positions: Vec::new(),
        }
    }

    struct DecisionCapture(Arc<Mutex<Vec<crate::LiveStrategyDecisionReport>>>);

    #[rsocket_rust::async_trait]
    impl RSocket for DecisionCapture {
        async fn metadata_push(&self, _request: Payload) -> anyhow::Result<()> {
            bail!("unsupported")
        }

        async fn fire_and_forget(&self, _request: Payload) -> anyhow::Result<()> {
            bail!("unsupported")
        }

        async fn request_response(&self, request: Payload) -> anyhow::Result<Option<Payload>> {
            let request: crate::LiveStrategySocketRequest =
                serde_json::from_slice(request.data().context("empty test request")?)?;
            let response = match request {
                crate::LiveStrategySocketRequest::ReportDecision(report) => {
                    self.0.lock().await.push(report);
                    crate::LiveStrategySocketResponse::DecisionAccepted {
                        status: 204,
                        detail: None,
                    }
                }
                crate::LiveStrategySocketRequest::ReportDiagnostic(_) => {
                    crate::LiveStrategySocketResponse::DecisionAccepted {
                        status: 204,
                        detail: None,
                    }
                }
                _ => crate::LiveStrategySocketResponse::Rejected {
                    status: 400,
                    detail: "unexpected request in routing integration test".into(),
                },
            };
            Ok(Some(
                Payload::builder()
                    .set_data(serde_json::to_vec(&response)?)
                    .build(),
            ))
        }

        fn request_stream(&self, _request: Payload) -> Flux<rsocket_rust::Result<Payload>> {
            Box::pin(futures_util::stream::empty())
        }

        fn request_channel(
            &self,
            requests: Flux<rsocket_rust::Result<Payload>>,
        ) -> Flux<rsocket_rust::Result<Payload>> {
            requests
        }
    }

    #[tokio::test]
    async fn insight_update_reports_the_matching_regime_profile() {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let reports_for_server = Arc::clone(&reports);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_setup, _socket| {
                Ok(Box::new(DecisionCapture(Arc::clone(&reports_for_server))))
            }))
            .serve();
        let server_task = tokio::spawn(server);
        sleep(std::time::Duration::from_millis(20)).await;

        let worker_config = LiveWorkerConfig {
            strategy: Strategy::LiveMomentum,
            listen_addr: "127.0.0.1:0".into(),
            max_in_flight: 1,
            max_payload_bytes: 64 * 1024,
            metrics_listen_addr: "127.0.0.1:0".into(),
            insights_stream_url: "ws://127.0.0.1:1".into(),
            auto_trader_rsocket_url: format!("ws://{address}"),
        };
        let worker = LiveWorker::new(worker_config.clone());

        let mut fallback = pushed_profile_set(1, "BTC-USDT").profiles.remove(0);
        let parameter_spec = Strategy::LiveMomentum.parameter_specs()[0].clone();
        let mut regime_profile = fallback.clone();
        let selected_value = if parameter_spec.default + parameter_spec.step <= parameter_spec.max {
            parameter_spec.default + parameter_spec.step
        } else {
            parameter_spec.default - parameter_spec.step
        };
        regime_profile
            .parameters
            .insert(parameter_spec.key.as_str().into(), selected_value);
        regime_profile.profile_id = crate::profile_id(
            "BTC-USDT",
            Strategy::LiveMomentum.as_str(),
            &regime_profile.parameters,
        );
        regime_profile.market_state_key = Some("v2:range:unknown".into());
        regime_profile.walk_forward_evidence_by_regime.insert(
            "range".into(),
            crate::StrategyWalkForwardEvidence {
                sample_count: LIVE_REGIME_PROFILE_MIN_SAMPLES,
                ..Default::default()
            },
        );
        fallback.market_regime = None;
        fallback.market_state_key = None;
        let selected_profile_id = regime_profile.profile_id.clone();
        let profile_set = crate::LiveStrategyProfileSet {
            contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
            set_version: 5,
            strategy: Strategy::LiveMomentum,
            enabled: true,
            profiles: vec![fallback, regime_profile],
            open_positions: Vec::new(),
        };
        worker.apply_profile_set(profile_set.clone()).await.unwrap();
        let mut invalid_replacement = profile_set;
        invalid_replacement.set_version = 6;
        invalid_replacement.profiles[0].market_state_key = Some("v3:range:unknown".into());
        assert!(worker.apply_profile_set(invalid_replacement).await.is_err());
        assert_eq!(
            worker.profiles.read().await.as_ref().unwrap().set_version,
            5
        );

        let now = live_worker_now_ms();
        let update = crate::StrategyInsightUpdate {
            contract_version: crate::STRATEGY_INSIGHT_UPDATE_VERSION,
            event_id: "regime-route-integration".into(),
            event_time_ms: now,
            signal: Some(StrategyMarketSignal {
                symbol: "BTC-USDT".into(),
                direction: "bullish".into(),
                timestamp: now,
                ranging_probability: Some(0.9),
                ..Default::default()
            }),
            market: None,
            taker_fee_rate: None,
            daily_snapshot: None,
            dated_carry_scan: None,
            dated_carry_candidate: None,
            dated_carry_settlement: None,
        };
        worker
            .evaluate_insight_update(update.clone())
            .await
            .unwrap();

        let mut stale_update = update;
        stale_update.event_id = "regime-route-stale-input".into();
        let stale_signal = stale_update.signal.as_mut().unwrap();
        stale_signal.stale = true;
        stale_signal.components.trend_5m_volatility_units = Some(2.0);
        stale_signal.components.volatility_1m_percent = Some(1.2);
        worker.evaluate_insight_update(stale_update).await.unwrap();

        let reports = reports.lock().await;
        assert_eq!(reports.len(), 1, "only one profile should be evaluated");
        assert_eq!(reports[0].profile_id, selected_profile_id);
        assert_eq!(reports[0].signal.market_regime.as_deref(), Some("range"));
        assert_eq!(
            worker.market_state_transitions.read().await["BTC-USDT"]
                .active
                .as_ref()
                .unwrap()
                .profile_key,
            "v2:range:unknown",
            "a stale trend update must not alter the active regime"
        );
        server_task.abort();
    }

    #[test]
    fn new_entries_select_exact_regime_profile_and_uncertain_states_use_fallback() {
        let fallback = pushed_profile_set(1, "BTC-USDT").profiles.remove(0);
        let mut range = fallback.clone();
        range.profile_id.push_str("-range");
        range.market_state_key = Some("v2:range:unknown".into());
        range.walk_forward_evidence_by_regime.insert(
            "range".into(),
            crate::StrategyWalkForwardEvidence {
                sample_count: 8,
                ..Default::default()
            },
        );
        let mut trend = fallback.clone();
        trend.profile_id.push_str("-trend");
        trend.market_state_key = Some("v2:trend_up:unknown".into());
        trend.walk_forward_evidence_by_regime.insert(
            "trend".into(),
            crate::StrategyWalkForwardEvidence {
                sample_count: 8,
                ..Default::default()
            },
        );
        let state = crate::classify_live_market_state(&StrategyMarketSignal {
            ranging_probability: Some(0.9),
            ..Default::default()
        })
        .unwrap();

        let routed = route_live_profiles(
            vec![fallback.clone(), trend, range.clone()],
            Some(&state),
            false,
        );
        assert_eq!(routed.len(), 1);
        assert_eq!(
            routed[0].market_state_key.as_deref(),
            Some("v2:range:unknown")
        );

        let routed = route_live_profiles(vec![range.clone(), fallback.clone()], None, false);
        assert_eq!(routed.len(), 1);
        assert_eq!(routed[0].profile_id, fallback.profile_id);

        range.walk_forward_evidence_by_regime.insert(
            "range".into(),
            crate::StrategyWalkForwardEvidence {
                sample_count: 2,
                ..Default::default()
            },
        );
        let routed = route_live_profiles(vec![range], Some(&state), false);
        assert!(routed.is_empty(), "under-sampled regime must not route");
    }

    #[test]
    fn open_positions_keep_their_pinned_profiles_during_regime_changes() {
        let mut pinned = pushed_profile_set(1, "BTC-USDT").profiles.remove(0);
        pinned.market_state_key = Some("v2:range:unknown".into());
        let state = crate::classify_live_market_state(&StrategyMarketSignal {
            components: SignalComponents {
                trend_5m_volatility_units: Some(2.0),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        let routed = route_live_profiles(vec![pinned.clone()], Some(&state), true);
        assert_eq!(routed[0].profile_id, pinned.profile_id);
    }

    #[test]
    fn market_state_hysteresis_transitions_after_two_fresh_confirmations() {
        let classify = |timestamp, trend| {
            crate::classify_live_market_state(&StrategyMarketSignal {
                timestamp,
                components: SignalComponents {
                    trend_5m_volatility_units: Some(trend),
                    volatility_1m_percent: Some(1.2),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap()
        };
        let mut state = MarketStateTransition::default();
        let (active, transition) = state.observe(Some(classify(1_000, -2.0)));
        assert_eq!(transition, "initialized");
        assert_eq!(active.unwrap().profile_key, "v2:trend_down:high");

        let (active, transition) = state.observe(Some(classify(2_000, 2.0)));
        assert_eq!(transition, "transition_pending");
        assert_eq!(active.unwrap().profile_key, "v2:trend_down:high");

        let (active, transition) = state.observe(Some(classify(2_000, 2.0)));
        assert_eq!(transition, "transition_pending");
        assert_eq!(active.unwrap().profile_key, "v2:trend_down:high");

        let (active, transition) = state.observe(Some(classify(3_000, 2.0)));
        assert_eq!(transition, "transitioned");
        assert_eq!(active.unwrap().profile_key, "v2:trend_up:high");

        let (active, transition) = state.observe(None);
        assert_eq!(transition, "uncertain_fallback");
        assert!(active.is_none());
        assert_eq!(
            state.active.as_ref().unwrap().profile_key,
            "v2:trend_up:high"
        );
    }

    #[test]
    fn market_state_replay_resets_pending_transition_after_a_large_gap() {
        let classify = |timestamp, trend| {
            crate::classify_live_market_state(&StrategyMarketSignal {
                timestamp,
                components: SignalComponents {
                    trend_5m_volatility_units: Some(trend),
                    volatility_1m_percent: Some(0.4),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap()
        };
        let mut state = MarketStateTransition::default();
        state.observe(Some(classify(1_000, -2.0)));
        let (_, transition) = state.observe(Some(classify(2_000, 2.0)));
        assert_eq!(transition, "transition_pending");
        let (active, transition) = state.observe(Some(classify(60_000, 2.0)));
        assert_eq!(transition, "transition_pending");
        assert_eq!(active.unwrap().structure, "trend_down");
    }

    #[tokio::test]
    async fn pushed_profile_set_replaces_the_worker_cache_atomically() {
        let worker = test_live_worker();
        worker
            .apply_profile_set(pushed_profile_set(1, "BTC-USDT"))
            .await
            .unwrap();
        worker
            .apply_profile_set(pushed_profile_set(2, "ETH-USDT"))
            .await
            .unwrap();

        let current = worker.profiles.read().await.clone().unwrap();
        assert_eq!(current.set_version, 2);
        assert_eq!(current.profiles.len(), 1);
        assert_eq!(current.profiles[0].profile_symbol, "ETH-USDT");
    }

    #[tokio::test]
    async fn pushed_profile_set_rejects_a_mismatched_strategy_without_replacing_cache() {
        let worker = test_live_worker();
        worker
            .apply_profile_set(pushed_profile_set(1, "BTC-USDT"))
            .await
            .unwrap();
        let mut mismatch = pushed_profile_set(2, "ETH-USDT");
        mismatch.strategy = Strategy::LiveTradeFlow;

        assert!(worker.apply_profile_set(mismatch).await.is_err());
        assert_eq!(
            worker.profiles.read().await.as_ref().unwrap().set_version,
            1
        );
    }

    #[test]
    fn walk_forward_evidence_is_loaded_only_for_flat_possible_entries() {
        assert!(!needs_walk_forward_evidence(
            false,
            Some(LiveStrategyDecision::Reject)
        ));
        assert!(!needs_walk_forward_evidence(
            false,
            Some(LiveStrategyDecision::Hold)
        ));
        assert!(needs_walk_forward_evidence(
            false,
            Some(LiveStrategyDecision::Enter)
        ));
        assert!(!needs_walk_forward_evidence(
            true,
            Some(LiveStrategyDecision::Enter)
        ));
    }

    #[test]
    fn profile_evidence_prefers_the_signal_regime_then_uses_unconditioned_fallback() {
        let mut profile = pushed_profile_set(1, "BTC-USDT").profiles.remove(0);
        profile.walk_forward_evidence = Some(crate::StrategyWalkForwardEvidence {
            sample_count: 3,
            positive_net_rate: Some(0.4),
            average_net_return_percent: Some(-0.1),
            last_sample_timestamp_ms: 1,
            ..Default::default()
        });
        profile.walk_forward_evidence_by_regime.insert(
            "trend".into(),
            crate::StrategyWalkForwardEvidence {
                sample_count: 8,
                positive_net_rate: Some(0.7),
                average_net_return_percent: Some(0.2),
                last_sample_timestamp_ms: 2,
                ..Default::default()
            },
        );
        let signal = crate::StrategyMarketSignal {
            market_regime: Some("trend".into()),
            ..Default::default()
        };
        assert_eq!(
            walk_forward_evidence_for_signal(&profile, &signal)
                .unwrap()
                .sample_count,
            8
        );
        let unknown = crate::StrategyMarketSignal::default();
        assert_eq!(
            walk_forward_evidence_for_signal(&profile, &unknown)
                .unwrap()
                .sample_count,
            3
        );
    }

    #[test]
    fn pending_insight_updates_keep_only_the_latest_snapshot_per_symbol() {
        let make_update = |symbol: &str, event_id: &str, timestamp: u64| {
            let LiveEvaluationInput::Entry { mut signal, .. } = request().input else {
                unreachable!();
            };
            signal.symbol = symbol.to_owned();
            signal.timestamp = timestamp;
            crate::StrategyInsightUpdate {
                contract_version: crate::STRATEGY_INSIGHT_UPDATE_VERSION,
                event_id: event_id.to_owned(),
                event_time_ms: timestamp,
                signal: Some(signal),
                market: None,
                taker_fee_rate: None,
                daily_snapshot: None,
                dated_carry_scan: None,
                dated_carry_candidate: None,
                dated_carry_settlement: None,
            }
        };
        let mut pending = BTreeMap::new();
        store_latest_insight_update(&mut pending, make_update("BTC-USDT", "btc-1", 1_000));
        store_latest_insight_update(&mut pending, make_update("ETH-USDT", "eth-1", 1_500));
        store_latest_insight_update(&mut pending, make_update("BTC-USDT", "btc-2", 2_000));

        assert_eq!(pending.len(), 2);
        assert!(pending.values().any(|update| {
            update
                .signal
                .as_ref()
                .is_some_and(|signal| signal.symbol == "BTC-USDT" && signal.timestamp == 2_000)
        }));
    }

    fn request() -> LiveEvaluationRequest {
        let parameters = BTreeMap::from([
            ("minimum_momentum".into(), 0.005),
            ("minimum_signal_strength".into(), 0.0),
        ]);
        LiveEvaluationRequest {
            contract_version: LIVE_EVALUATION_CONTRACT_VERSION,
            request_id: "r-1".into(),
            strategy: Strategy::LiveMomentum,
            symbol: "BTC-USDT".into(),
            profile: crate::LiveStrategyProfile {
                profile_id: crate::profile_id(
                    "BTC-USDT",
                    Strategy::LiveMomentum.as_str(),
                    &parameters,
                ),
                profile_symbol: "BTC-USDT".into(),
                version: 0,
                parameters,
                walk_forward_evidence: None,
                market_regime: None,
                market_state_key: None,
                walk_forward_evidence_by_regime: Default::default(),
            },
            input: LiveEvaluationInput::Entry {
                signal: StrategyMarketSignal {
                    symbol: "BTC-USDT".into(),
                    strategy: Some("live-momentum".into()),
                    direction: "bullish".into(),
                    components: SignalComponents {
                        momentum_10s_percent: Some(0.01),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                daily_features: None,
                walk_forward_evidence: None,
            },
        }
    }

    #[test]
    fn validates_strategy_profile_and_correlates_result() {
        let req = request();
        let response = evaluate_request(&req, Strategy::LiveMomentum).unwrap();
        assert_eq!(response, LiveStrategyDecision::Enter);
        let mut wrong = req.clone();
        wrong.strategy = Strategy::DatedCarry;
        assert_eq!(
            evaluate_request(&wrong, Strategy::LiveMomentum)
                .unwrap_err()
                .code,
            LiveEvaluationErrorCode::WrongStrategy
        );
        let mut invalid = req.clone();
        invalid
            .profile
            .parameters
            .insert("minimum_momentum".into(), 1e20);
        assert_eq!(
            evaluate_request(&invalid, Strategy::LiveMomentum)
                .unwrap_err()
                .code,
            LiveEvaluationErrorCode::InvalidParameters
        );
        let mut invalid_step = req.clone();
        invalid_step
            .profile
            .parameters
            .insert("minimum_momentum".into(), 0.0055);
        assert_eq!(
            evaluate_request(&invalid_step, Strategy::LiveMomentum)
                .unwrap_err()
                .code,
            LiveEvaluationErrorCode::InvalidParameters
        );
        let mut invalid_profile = req.clone();
        invalid_profile.profile.profile_symbol = "ETH-USDT".into();
        assert_eq!(
            evaluate_request(&invalid_profile, Strategy::LiveMomentum)
                .unwrap_err()
                .code,
            LiveEvaluationErrorCode::InvalidProfile
        );
    }

    #[tokio::test]
    async fn live_role_returns_profile_attributed_correlated_rsocket_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let config = LiveWorkerConfig {
            strategy: Strategy::LiveMomentum,
            listen_addr: address.to_string(),
            max_in_flight: 1,
            max_payload_bytes: 64 * 1024,
            metrics_listen_addr: "127.0.0.1:0".into(),
            insights_stream_url: "ws://127.0.0.1:1".into(),
            auto_trader_rsocket_url: "ws://127.0.0.1:1".into(),
        };
        let server = tokio::spawn(async move { serve_live_worker(config).await });
        let url = format!("ws://{address}");
        let mut client = None;
        for _ in 0..25 {
            match RSocketFactory::connect()
                .transport(WebsocketClientTransport::from(url.as_str()))
                .acceptor(Box::new(|| Box::new(EchoRSocket)))
                .start()
                .await
            {
                Ok(connected) => {
                    client = Some(connected);
                    break;
                }
                Err(_) => sleep(std::time::Duration::from_millis(20)).await,
            }
        }
        let client = client.expect("live worker should accept an RSocket connection");
        let response = client
            .request_response(
                Payload::builder()
                    .set_data(serde_json::to_vec(&request()).unwrap())
                    .build(),
            )
            .await
            .unwrap()
            .expect("live worker should return request-response result");
        let result: LiveEvaluationResponse =
            serde_json::from_slice(response.data().unwrap()).unwrap();
        assert_eq!(result.request_id, "r-1");
        assert_eq!(result.strategy, Strategy::LiveMomentum);
        assert_eq!(result.profile_id, request().profile.profile_id);
        assert_eq!(result.profile_symbol, "BTC-USDT");
        assert_eq!(result.profile_version, 0);
        assert_eq!(result.decision, Some(LiveStrategyDecision::Enter));
        assert_eq!(result.error, None);

        let request = request();
        let pushed_set = crate::LiveStrategyProfileSet {
            contract_version: crate::LIVE_EVALUATION_CONTRACT_VERSION,
            set_version: 11,
            strategy: Strategy::LiveMomentum,
            enabled: true,
            profiles: vec![request.profile.clone()],
            open_positions: Vec::new(),
        };
        let response = client
            .request_response(
                Payload::builder()
                    .set_data(
                        serde_json::to_vec(&crate::LiveStrategySocketRequest::UpdateProfiles(
                            pushed_set.clone(),
                        ))
                        .unwrap(),
                    )
                    .build(),
            )
            .await
            .unwrap()
            .expect("live worker should acknowledge pushed profiles");
        let acknowledgement: crate::LiveStrategySocketResponse =
            serde_json::from_slice(response.data().unwrap()).unwrap();
        assert_eq!(
            acknowledgement,
            crate::LiveStrategySocketResponse::ProfilesApplied {
                strategy: Strategy::LiveMomentum,
                set_version: 11,
            }
        );
        server.abort();
        drop(client);
    }
}
