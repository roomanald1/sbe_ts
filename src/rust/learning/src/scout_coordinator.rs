use anyhow::{Context, bail};
use axum::{Router, body::Body};
use futures_util::{Stream, stream};
use http_body_util::BodyExt;
use rsocket_rust::{prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::{WebsocketClientTransport, WebsocketServerTransport};
use std::{collections::BTreeMap, pin::Pin, sync::Arc, time::Duration};
use strategy::{
    MAX_REPLAY_SIGNALS, ScoutCandidateBatch, ScoutCandidateBatchResult, ScoutCoordinatorRequest,
    ScoutCoordinatorResponse, ScoutCoordinatorRouteCommand, ScoutCoordinatorRouteResult,
    ScoutFeedbackAcknowledgement, ScoutWorkAssignment, ScoutWorkDecision, ScoutWorkMode,
    ScoutWorkResult, ScoutWorkerHttpMethod, ScoutWorkerRequest, ScoutWorkerResponse,
    ScoutWorkerRouteCommand, ScoutWorkerRouteResult, Strategy, StrategyProfile,
    StrategyReplayPoint, StrategyReplayRequest, StrategyReplayResponse,
    request_scout_service_route,
};
use tokio::sync::Mutex as AsyncMutex;
use tower::ServiceExt;

use crate::ServiceState;

const DEFAULT_MAX_SCOUT_BATCH_SIGNALS: usize = 64;
const DEFAULT_MAX_SCOUT_ASSIGNMENTS: usize = 512;
const DEFAULT_MAX_SCOUT_BATCH_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_WORKER_TIMEOUT_MS: usize = 3_000;
const DEFAULT_REPLAY_WORKER_TIMEOUT_MS: usize = 10_000;
const MAX_REPLAY_TOTAL_SIGNALS: usize = 10_000;
const MAX_REPLAY_TOTAL_PAGES: usize = 128;
const MAX_REPLAY_TOTAL_RECORDS_SCANNED: usize = 524_288;
const MAX_REPLAY_TOTAL_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_REPLAY_SIGNAL_GAP_MS: u64 = 30_000;

fn strategy_replay_horizons_ms(strategy: &str) -> Option<Vec<u64>> {
    if !matches!(
        strategy,
        "live-momentum"
            | "live-trade-flow"
            | "live-order-book"
            | "live-vwap-dislocation"
            | "live-absorption-pressure"
    ) {
        return None;
    }
    let default = vec![5 * 60_000, 30 * 60_000, 2 * 60 * 60_000, 8 * 60 * 60_000];
    let environment_name = format!(
        "LEARNING_REPLAY_HORIZONS_{}",
        strategy.to_ascii_uppercase().replace('-', "_")
    );
    let Ok(value) = std::env::var(environment_name) else {
        return Some(default);
    };
    let horizons = value
        .split(',')
        .filter_map(|part| part.trim().parse::<u64>().ok())
        .collect::<Vec<_>>();
    (horizons.len() == 4
        && horizons
            .iter()
            .all(|value| *value > 0 && *value <= MAX_REPLAY_TOTAL_MS)
        && horizons.windows(2).all(|window| window[0] < window[1]))
    .then_some(horizons)
}

fn missed_opportunity_band(
    job: &crate::MissedOpportunityJob,
    horizon_ms: u64,
    result: &crate::MissedOpportunityHorizon,
    gate: &str,
    threshold: f64,
) -> Option<strategy::ContextMissedOpportunityBand> {
    if !result.complete || !threshold.is_finite() {
        return None;
    }
    let markout = result
        .gross_directional_return_percent
        .filter(|value| value.is_finite())?;
    Some(strategy::ContextMissedOpportunityBand {
        gate: gate.into(),
        threshold,
        horizon_seconds: horizon_ms / 1_000,
        sample_count: 1,
        average_net_return_percent: None,
        positive_net_rate: None,
        time_to_profit_sample_count: 0,
        time_to_profit_rate: None,
        average_time_to_profit_seconds: None,
        stop_target_sample_count: 0,
        stop_target_average_net_return_percent: None,
        stop_target_positive_net_rate: None,
        symbol: Some(job.symbol.clone()),
        direction: Some(job.direction.clone()),
        strategy: Some(job.strategy.clone()),
        profile_id: Some(job.profile_id.clone()),
        market_context: Some(job.learning_context.clone()),
        complete: true,
        gross_directional_markout_percent: Some(markout),
        event_id: Some(job.event_id.clone()),
        observed_at_ms: job.observed_at_ms,
    })
}

fn encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn gross_return_from_exit(context: &crate::TradeReplayContext, mark: f64) -> f64 {
    let direction = if context.side.eq_ignore_ascii_case("long") {
        1.0
    } else {
        -1.0
    };
    (mark / context.exit_price - 1.0) * direction * 100.0
}

fn classify_replay_exit(
    complete: bool,
    entry_found: bool,
    position_open: bool,
    counterfactual_exit_ms: Option<u64>,
    actual_exit_ms: u64,
) -> &'static str {
    if !complete || !entry_found {
        return "indeterminate";
    }
    if let Some(exit_ms) = counterfactual_exit_ms {
        let difference = exit_ms as i128 - actual_exit_ms as i128;
        if difference.abs() <= 20_000 {
            "on_time"
        } else if difference > 0 {
            "early"
        } else {
            "late"
        }
    } else if position_open {
        "early"
    } else {
        "indeterminate"
    }
}

#[derive(Clone)]
pub struct ScoutCoordinator {
    state: ServiceState,
    local_router: Router,
    worker_endpoints: BTreeMap<String, String>,
    max_batch_signals: usize,
    max_assignments: usize,
    max_batch_bytes: usize,
    worker_timeout: Duration,
    replay_worker_timeout: Duration,
    enabled: bool,
    insights_rsocket_url: String,
    replay_gate: Arc<AsyncMutex<()>>,
}

impl ScoutCoordinator {
    pub(crate) fn new(
        state: ServiceState,
        worker_endpoints: &str,
        local_router: Router,
    ) -> Result<Self, String> {
        let worker_endpoints = parse_worker_endpoints(worker_endpoints)?;
        Ok(Self {
            state,
            local_router,
            worker_endpoints,
            max_batch_signals: env_limit(
                "SCOUT_COORDINATOR_MAX_BATCH_SIGNALS",
                DEFAULT_MAX_SCOUT_BATCH_SIGNALS,
            )?,
            max_assignments: env_limit(
                "SCOUT_COORDINATOR_MAX_ASSIGNMENTS",
                DEFAULT_MAX_SCOUT_ASSIGNMENTS,
            )?,
            max_batch_bytes: env_limit(
                "SCOUT_COORDINATOR_MAX_BATCH_BYTES",
                DEFAULT_MAX_SCOUT_BATCH_BYTES,
            )?,
            worker_timeout: Duration::from_millis(env_limit(
                "SCOUT_COORDINATOR_WORKER_TIMEOUT_MS",
                DEFAULT_WORKER_TIMEOUT_MS,
            )? as u64),
            replay_worker_timeout: Duration::from_millis(env_limit(
                "LEARNING_REPLAY_WORKER_TIMEOUT_MS",
                DEFAULT_REPLAY_WORKER_TIMEOUT_MS,
            )? as u64),
            enabled: std::env::var("SCOUT_WORKERS_ENABLED")
                .map(|value| value.eq_ignore_ascii_case("true"))
                .unwrap_or(true),
            insights_rsocket_url: std::env::var("INSIGHTS_RPC_RSOCKET_URL")
                .unwrap_or_else(|_| "ws://insights:10007".into()),
            replay_gate: Arc::new(AsyncMutex::new(())),
        })
    }

    pub async fn serve(self, address: String) -> anyhow::Result<()> {
        let responder = CoordinatorResponder(self);
        let replay_coordinator = responder.0.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if let Err(error) = replay_coordinator.process_next_strategy_replay().await {
                    eprintln!("Learning strategy replay worker failed: {error:#}");
                }
                if let Err(error) = replay_coordinator.process_next_missed_opportunity().await {
                    eprintln!("Learning missed-opportunity worker failed: {error:#}");
                }
            }
        });
        eprintln!("Learning Scout coordinator listening on ws://{address}");
        RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address))
            .acceptor(Box::new(move |_setup, _socket| {
                Ok(Box::new(responder.clone()))
            }))
            .serve()
            .await
            .context("Learning Scout coordinator stopped")
    }

    async fn coordinate(
        &self,
        request: ScoutCandidateBatch,
    ) -> anyhow::Result<ScoutCandidateBatchResult> {
        if request.request_id.trim().is_empty() {
            bail!("Scout batch request ID is required");
        }
        if request.signals.len() > self.max_batch_signals {
            bail!("Scout batch exceeds the configured signal limit");
        }
        if !self.enabled {
            return Ok(ScoutCandidateBatchResult {
                request_id: request.request_id,
                assignments: Vec::new(),
            });
        }

        let state = self.state.state.lock().await;
        let mut assignments = Vec::new();
        let mut results = Vec::new();
        for (signal_index, signal) in request.signals.iter().enumerate() {
            let symbol = signal["symbol"]
                .as_str()
                .filter(|symbol| !symbol.is_empty())
                .context("Scout signal is missing its symbol")?;
            let mut profiles = selected_profiles(&state, symbol, signal)?;
            if request.mode == ScoutWorkMode::EvidenceOnly {
                let Some(parameters) = requested_profile_parameters(signal) else {
                    continue;
                };
                profiles.retain(|profile| profile.parameters == parameters);
            }
            for profile in profiles {
                let strategy = Strategy::from_name(&profile.strategy).with_context(|| {
                    format!("Learning profile has unknown strategy {}", profile.strategy)
                })?;
                let endpoint = self
                    .worker_endpoints
                    .get(strategy.as_str())
                    .with_context(|| {
                        format!("no RSocket worker configured for {}", strategy.as_str())
                    })?;
                let candidate = candidates_for_profile(signal, &profile)?;
                for candidate in candidate {
                    if assignments.len() >= self.max_assignments {
                        bail!("Scout batch exceeds the configured assignment limit");
                    }
                    let direction = candidate["direction"].as_str().unwrap_or("neutral");
                    let assignment_id = format!(
                        "{}:{}:{}:{}:{}:{}",
                        request.request_id,
                        signal_index,
                        symbol,
                        strategy.as_str(),
                        profile.profile_id,
                        direction
                    );
                    let evidence =
                        walk_forward_evidence(&state, &profile, signal_timestamp_ms(signal));
                    if request.mode == ScoutWorkMode::EvidenceOnly {
                        results.push(ScoutWorkResult {
                            assignment_id,
                            strategy,
                            symbol: symbol.to_owned(),
                            decision: ScoutWorkDecision::NotEvaluated,
                            candidate: compact_evidence_candidate(signal, &profile),
                            walk_forward_evidence: evidence,
                        });
                        continue;
                    }
                    assignments.push((
                        endpoint.clone(),
                        ScoutWorkAssignment {
                            assignment_id,
                            strategy,
                            mode: request.mode,
                            symbol: symbol.to_owned(),
                            parameters: profile.parameters.clone(),
                            candidate,
                        },
                        evidence,
                    ));
                }
            }
        }
        drop(state);

        results.reserve(assignments.len());
        for (endpoint, assignment, evidence) in assignments {
            let expected_id = assignment.assignment_id.clone();
            let expected_strategy = assignment.strategy;
            let mut result = tokio::time::timeout(
                self.worker_timeout,
                dispatch_assignment(&endpoint, &assignment),
            )
            .await
            .with_context(|| format!("Scout worker {} timed out", expected_strategy.as_str()))?
            .with_context(|| format!("Scout worker {} failed", expected_strategy.as_str()))?;
            if result.assignment_id != expected_id || result.strategy != expected_strategy {
                bail!("Scout worker returned a result for a different assignment");
            }
            result.walk_forward_evidence = evidence;
            results.push(result);
        }
        Ok(ScoutCandidateBatchResult {
            request_id: request.request_id,
            assignments: results,
        })
    }

    async fn replay_strategy(
        &self,
        request: StrategyReplayRequest,
    ) -> anyhow::Result<StrategyReplayResponse> {
        if request.job_id.trim().is_empty()
            || request.points.is_empty()
            || request.points.len() > MAX_REPLAY_SIGNALS
        {
            bail!("strategy replay request is outside its configured bounds");
        }
        let endpoint = self
            .worker_endpoints
            .get(request.strategy.as_str())
            .with_context(|| {
                format!(
                    "no RSocket worker configured for {}",
                    request.strategy.as_str()
                )
            })?;
        let expected_job_id = request.job_id.clone();
        let expected_strategy = request.strategy;
        let expected_symbol = request.symbol.clone();
        let expected_profile_id = request.profile_id.clone();
        let response = tokio::time::timeout(
            self.replay_worker_timeout,
            dispatch_strategy_replay(endpoint, &request),
        )
        .await
        .context("strategy replay worker timed out")??;
        if response.job_id != expected_job_id
            || response.strategy != expected_strategy
            || response.symbol != expected_symbol
            || response.profile_id != expected_profile_id
            || response.evaluated_count != response.decisions.len()
        {
            bail!("strategy replay worker returned a mismatched result");
        }
        Ok(response)
    }

    async fn process_next_strategy_replay(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let Ok(_permit) = self.replay_gate.try_lock() else {
            return Ok(());
        };
        let mut job = {
            let mut state = self.state.state.lock().await;
            let Some(event_id) = state
                .strategy_replay_jobs
                .values()
                .filter(|job| job.status == crate::StrategyReplayStatus::Pending)
                .min_by_key(|job| (job.created_at_ms, job.event_id.clone()))
                .map(|job| job.event_id.clone())
            else {
                return Ok(());
            };
            let job = state
                .strategy_replay_jobs
                .get_mut(&event_id)
                .expect("selected replay job exists");
            job.status = crate::StrategyReplayStatus::Running;
            job.attempts = job.attempts.saturating_add(1);
            job.updated_at_ms = crate::now_ms();
            let job = job.clone();
            crate::persist(&self.state.path, &state).map_err(|error| anyhow::anyhow!(error.1))?;
            job
        };
        match self.run_strategy_replay(&mut job).await {
            Ok((status, result, reason)) => {
                job.status = status;
                job.result = result;
                job.reason = reason;
            }
            Err(error) => {
                job.status = crate::StrategyReplayStatus::Failed;
                job.reason = Some(error.to_string());
            }
        }
        job.updated_at_ms = crate::now_ms();
        self.save_strategy_replay_job(&job).await?;
        Ok(())
    }

    async fn process_next_missed_opportunity(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let mut job = {
            let mut state = self.state.state.lock().await;
            let now = crate::now_ms();
            let Some(event_id) = state
                .missed_opportunity_jobs
                .values()
                .filter(|job| {
                    job.status == crate::StrategyReplayStatus::Pending
                        && strategy_replay_horizons_ms(&job.strategy).is_some_and(|horizons| {
                            horizons
                                .last()
                                .is_some_and(|last| now >= job.observed_at_ms.saturating_add(*last))
                        })
                })
                .min_by_key(|job| (job.observed_at_ms, job.event_id.clone()))
                .map(|job| job.event_id.clone())
            else {
                return Ok(());
            };
            let job = state
                .missed_opportunity_jobs
                .get_mut(&event_id)
                .expect("selected missed-opportunity job exists");
            job.status = crate::StrategyReplayStatus::Running;
            let job = job.clone();
            crate::persist(&self.state.path, &state).map_err(|error| anyhow::anyhow!(error.1))?;
            job
        };
        let horizons = strategy_replay_horizons_ms(&job.strategy)
            .context("strategy has no configured missed-opportunity horizons")?;
        let result = self
            .run_missed_opportunity_history(&mut job, &horizons)
            .await;
        match result {
            Ok(()) => {
                job.status = crate::StrategyReplayStatus::Completed;
                job.reason = None;
            }
            Err(error) => {
                job.status = crate::StrategyReplayStatus::Unsupported;
                job.reason = Some(error.to_string());
            }
        }
        let mut state = self.state.state.lock().await;
        if job.status == crate::StrategyReplayStatus::Completed {
            for (&horizon_ms, result) in &job.horizons {
                for (gate, threshold) in [("score", job.score.abs()), ("strength", job.strength)] {
                    let Some(band) =
                        missed_opportunity_band(&job, horizon_ms, result, gate, threshold)
                    else {
                        continue;
                    };
                    state.missed_opportunity_bands.retain(|existing| {
                        !(existing.event_id == band.event_id
                            && existing.gate == band.gate
                            && existing.horizon_seconds == band.horizon_seconds)
                    });
                    state.missed_opportunity_bands.push(band);
                }
            }
            if state.missed_opportunity_bands.len() > 2_000 {
                let excess = state.missed_opportunity_bands.len() - 2_000;
                state.missed_opportunity_bands.drain(..excess);
            }
        }
        state
            .missed_opportunity_jobs
            .insert(job.event_id.clone(), job);
        crate::persist(&self.state.path, &state).map_err(|error| anyhow::anyhow!(error.1))?;
        Ok(())
    }

    async fn run_missed_opportunity_history(
        &self,
        job: &mut crate::MissedOpportunityJob,
        horizons: &[u64],
    ) -> anyhow::Result<()> {
        let through_ms = job
            .observed_at_ms
            .saturating_add(*horizons.last().context("horizons are empty")?);
        let mut more = true;
        let mut last_signal = job.coverage_through_ms;
        while more {
            if job.pages_scanned >= 20 {
                bail!("missed-opportunity history exceeded its page cap");
            }
            let cursor = job
                .history_cursor
                .as_ref()
                .map(|cursor| format!("&cursor={cursor}"))
                .unwrap_or_default();
            let path = format!(
                "/internal/strategy-snapshot-history?symbol={}&fromMs={}&throughMs={}&limit=512{}",
                encode_query_component(&job.symbol),
                job.observed_at_ms,
                through_ms,
                cursor
            );
            let route = ScoutCoordinatorRouteCommand {
                request_id: format!("{}:missed:{}", job.event_id, job.pages_scanned),
                strategy: None,
                method: ScoutWorkerHttpMethod::Get,
                path,
                payload: serde_json::Value::Null,
            };
            let page = request_scout_service_route(&self.insights_rsocket_url, &route).await?;
            if page.status != 200 {
                bail!(
                    "Insights missed-opportunity history returned HTTP {}",
                    page.status
                );
            }
            let records = page.body["records"]
                .as_array()
                .context("history response omitted records")?;
            for record in records {
                let timestamp = record["snapshot"]["timestamp"]
                    .as_u64()
                    .context("history record omitted timestamp")?;
                if let Some(previous) = last_signal {
                    if timestamp <= previous {
                        bail!("history timestamps are not strictly ordered");
                    }
                    if timestamp.saturating_sub(previous) > MAX_REPLAY_SIGNAL_GAP_MS {
                        job.cadence_gap_detected = true;
                        job.cadence_gap_at_ms.get_or_insert(previous);
                    }
                }
                last_signal = Some(timestamp);
                job.coverage_start_ms.get_or_insert(timestamp);
                job.coverage_through_ms = Some(timestamp);
                let Some(bid) = record["topBid"]
                    .as_f64()
                    .filter(|price| price.is_finite() && *price > 0.0)
                else {
                    continue;
                };
                let Some(ask) = record["topAsk"]
                    .as_f64()
                    .filter(|price| price.is_finite() && *price >= bid)
                else {
                    continue;
                };
                let mark = (bid + ask) / 2.0;
                if job.reference_price.is_none()
                    && timestamp.abs_diff(job.observed_at_ms) <= MAX_REPLAY_SIGNAL_GAP_MS
                {
                    job.reference_price = Some(if job.direction == "bullish" { ask } else { bid });
                }
                for horizon in horizons {
                    let target = job.observed_at_ms.saturating_add(*horizon);
                    if timestamp.abs_diff(target) <= MAX_REPLAY_SIGNAL_GAP_MS {
                        let replace = job
                            .horizons
                            .get(horizon)
                            .and_then(|sample| sample.timestamp_ms)
                            .is_none_or(|old| old.abs_diff(target) > timestamp.abs_diff(target));
                        if replace {
                            let direction = if job.direction == "bullish" {
                                1.0
                            } else {
                                -1.0
                            };
                            job.horizons.insert(
                                *horizon,
                                crate::MissedOpportunityHorizon {
                                    mark_price: Some(mark),
                                    timestamp_ms: Some(timestamp),
                                    gross_directional_return_percent: job.reference_price.map(
                                        |reference| (mark / reference - 1.0) * direction * 100.0,
                                    ),
                                    complete: false,
                                },
                            );
                        }
                    }
                }
            }
            job.corrupt_records = job
                .corrupt_records
                .saturating_add(page.body["corruptRecords"].as_u64().unwrap_or_default() as usize);
            job.archived_days_unscanned = job.archived_days_unscanned.max(
                page.body["archivedDaysUnscanned"]
                    .as_u64()
                    .unwrap_or_default() as usize,
            );
            job.archived_days_unavailable = job.archived_days_unavailable.max(
                page.body["archivedDaysUnavailable"]
                    .as_u64()
                    .unwrap_or_default() as usize,
            );
            more = page.body["moreAvailable"].as_bool().unwrap_or(false);
            job.history_cursor = page.body["nextCursor"].as_str().map(str::to_owned);
            job.pages_scanned += 1;
            job.reason = Some("history scan in progress".into());
            let mut state = self.state.state.lock().await;
            state
                .missed_opportunity_jobs
                .insert(job.event_id.clone(), job.clone());
            crate::persist(&self.state.path, &state).map_err(|error| anyhow::anyhow!(error.1))?;
        }
        let coverage_complete = job.reference_price.is_some()
            && job.coverage_start_ms.is_some_and(|start| {
                start <= job.observed_at_ms.saturating_add(MAX_REPLAY_SIGNAL_GAP_MS)
            })
            && job
                .coverage_through_ms
                .is_some_and(|end| end.saturating_add(MAX_REPLAY_SIGNAL_GAP_MS) >= through_ms)
            && job.corrupt_records == 0
            && job.archived_days_unscanned == 0
            && job.archived_days_unavailable == 0;
        for (horizon, sample) in &mut job.horizons {
            let target = job.observed_at_ms.saturating_add(*horizon);
            sample.complete = coverage_complete
                && sample.timestamp_ms.is_some_and(|timestamp| {
                    timestamp.abs_diff(target) <= MAX_REPLAY_SIGNAL_GAP_MS
                })
                && job.cadence_gap_at_ms.is_none_or(|gap| gap > target);
        }
        if job.horizons.values().all(|sample| !sample.complete) {
            job.reason =
                Some("history or book-mark coverage was incomplete; no band was learned".into());
        }
        Ok(())
    }

    async fn save_strategy_replay_job(&self, job: &crate::StrategyReplayJob) -> anyhow::Result<()> {
        let mut state = self.state.state.lock().await;
        state
            .strategy_replay_jobs
            .insert(job.event_id.clone(), job.clone());
        crate::persist(&self.state.path, &state).map_err(|error| anyhow::anyhow!(error.1))
    }

    async fn run_strategy_replay(
        &self,
        job: &mut crate::StrategyReplayJob,
    ) -> anyhow::Result<(
        crate::StrategyReplayStatus,
        Option<crate::StrategyReplayResult>,
        Option<String>,
    )> {
        let context = job.context.as_ref().context("replay context is missing")?;
        let strategy = Strategy::from_name(&job.strategy).context("unknown replay strategy")?;
        if !job.strategy.starts_with("live-") {
            return Ok((
                crate::StrategyReplayStatus::Unsupported,
                None,
                Some("exact daily strategy features are not persisted in replay history".into()),
            ));
        }
        if job.horizons_ms.is_empty() {
            job.horizons_ms = strategy_replay_horizons_ms(&job.strategy)
                .context("strategy has no configured replay horizons")?;
            self.save_strategy_replay_job(job).await?;
        }
        let horizons = job.horizons_ms.clone();
        let observation_through_ms = context
            .exit_timestamp_ms
            .saturating_add(*horizons.last().context("replay horizon list is empty")?);
        let requested_from_ms = context.entry_timestamp_ms;
        if observation_through_ms.saturating_sub(requested_from_ms) > MAX_REPLAY_TOTAL_MS {
            return Ok((
                crate::StrategyReplayStatus::Unsupported,
                None,
                Some("trade replay range exceeds the bounded 24-hour signal cap".into()),
            ));
        }
        let mut more_available = true;
        let mut virtual_open = job.counterfactual_position_open;
        let mut entry_found = job.counterfactual_entry_found;
        while more_available {
            if job.signals_scanned >= MAX_REPLAY_TOTAL_SIGNALS
                || job.pages_scanned >= MAX_REPLAY_TOTAL_PAGES
                || job.records_scanned >= MAX_REPLAY_TOTAL_RECORDS_SCANNED
            {
                return Ok((
                    crate::StrategyReplayStatus::Unsupported,
                    None,
                    Some("trade replay exceeded the bounded snapshot cap".into()),
                ));
            }
            let cursor_query = job
                .history_cursor
                .as_ref()
                .map(|cursor| format!("&cursor={cursor}"))
                .unwrap_or_default();
            let path = format!(
                "/internal/strategy-snapshot-history?symbol={}&fromMs={}&throughMs={}&limit={}{cursor_query}",
                encode_query_component(&context.market_symbol),
                requested_from_ms,
                observation_through_ms,
                MAX_REPLAY_SIGNALS.min(256),
            );
            let route = ScoutCoordinatorRouteCommand {
                request_id: format!("{}:history:{}", job.event_id, job.pages_scanned),
                strategy: None,
                method: ScoutWorkerHttpMethod::Get,
                path,
                payload: serde_json::Value::Null,
            };
            let page = request_scout_service_route(&self.insights_rsocket_url, &route).await?;
            if page.status != 200 {
                anyhow::bail!("Insights snapshot history returned HTTP {}", page.status);
            }
            let records = page.body["records"]
                .as_array()
                .context("Insights snapshot history omitted records")?;
            if records.is_empty() && page.body["moreAvailable"].as_bool().unwrap_or(false) {
                anyhow::bail!("Insights snapshot history returned an empty page with more data");
            }
            let cursor = page.body["nextCursor"].as_str().map(str::to_owned);
            let more = page.body["moreAvailable"].as_bool().unwrap_or(false);
            job.records_scanned = job
                .records_scanned
                .saturating_add(page.body["scannedRecords"].as_u64().unwrap_or_default() as usize);
            let mut points = Vec::with_capacity(records.len());
            for record in records {
                let snapshot = record
                    .get("snapshot")
                    .context("snapshot history record omitted snapshot")?;
                let timestamp_ms = snapshot["timestamp"]
                    .as_u64()
                    .context("snapshot history record omitted timestamp")?;
                let symbol_value = snapshot["symbol"]
                    .as_str()
                    .context("snapshot history record omitted symbol")?;
                if symbol_value != context.market_symbol {
                    continue;
                }
                if let Some(previous) = job.last_signal_timestamp_ms {
                    if timestamp_ms <= previous {
                        anyhow::bail!("Insights replay snapshots are not strictly ordered");
                    }
                    if timestamp_ms.saturating_sub(previous) > MAX_REPLAY_SIGNAL_GAP_MS {
                        job.cadence_gap_detected = true;
                        job.cadence_gap_at_ms.get_or_insert(previous);
                    }
                }
                job.coverage_start_ms.get_or_insert(timestamp_ms);
                job.coverage_through_ms = Some(timestamp_ms);
                job.last_signal_timestamp_ms = Some(timestamp_ms);
                let mut signal_json = snapshot.clone();
                signal_json["strategy"] = serde_json::json!(job.strategy);
                signal_json["strategy_candidate"] = serde_json::json!(true);
                signal_json["strategy_decision"] = context
                    .entry_walk_forward_evidence
                    .map(|evidence| serde_json::json!({"strategy_walk_forward": evidence}))
                    .unwrap_or(serde_json::Value::Null);
                signal_json["market_regime"] = serde_json::Value::Null;
                signal_json["entry_gate"] = serde_json::Value::Null;
                let signal: strategy::StrategyMarketSignal = serde_json::from_value(signal_json)
                    .context("persisted signal snapshot cannot reconstruct StrategyMarketSignal")?;
                points.push(StrategyReplayPoint {
                    timestamp_ms,
                    signal,
                    top_bid: record["topBid"].as_f64(),
                    top_ask: record["topAsk"].as_f64(),
                });
            }
            job.archived_days_unscanned = job.archived_days_unscanned.max(
                page.body["archivedDaysUnscanned"]
                    .as_u64()
                    .unwrap_or_default() as usize,
            );
            job.archived_days_unavailable = job.archived_days_unavailable.max(
                page.body["archivedDaysUnavailable"]
                    .as_u64()
                    .unwrap_or_default() as usize,
            );
            job.corrupt_history_records = job
                .corrupt_history_records
                .saturating_add(page.body["corruptRecords"].as_u64().unwrap_or_default() as usize);
            let request = StrategyReplayRequest {
                job_id: job.event_id.clone(),
                strategy,
                symbol: context.market_symbol.clone(),
                profile_symbol: job.symbol.clone(),
                profile_id: job.profile_id.clone(),
                parameters: job.parameters.clone(),
                entry_timestamp_ms: context.entry_timestamp_ms,
                actual_exit_timestamp_ms: context.exit_timestamp_ms,
                observation_through_timestamp_ms: observation_through_ms,
                position_side: context.side.clone(),
                counterfactual_position_open: virtual_open,
                counterfactual_entry_found: entry_found,
                points,
            };
            if !request.points.is_empty() {
                let response = self.replay_strategy(request).await?;
                if response.input_count != response.evaluated_count {
                    anyhow::bail!("Scout replay worker omitted decisions");
                }
                for point in &response.decisions {
                    if let Some(mark) = point.mark_price {
                        if point.timestamp_ms.abs_diff(context.exit_timestamp_ms)
                            <= MAX_REPLAY_SIGNAL_GAP_MS
                        {
                            job.mark_at_actual_exit = Some(mark);
                        }
                        if point.timestamp_ms.abs_diff(observation_through_ms)
                            <= MAX_REPLAY_SIGNAL_GAP_MS
                        {
                            job.mark_at_observation_end = Some(mark);
                        }
                        for horizon_ms in &horizons {
                            let target = context.exit_timestamp_ms.saturating_add(*horizon_ms);
                            if point.timestamp_ms.abs_diff(target) <= MAX_REPLAY_SIGNAL_GAP_MS {
                                let prior = job.horizon_mark_timestamps.get(horizon_ms).copied();
                                if prior.is_none_or(|timestamp| {
                                    timestamp.abs_diff(target) > point.timestamp_ms.abs_diff(target)
                                }) {
                                    job.horizon_mark_timestamps
                                        .insert(*horizon_ms, point.timestamp_ms);
                                    job.horizon_mark_prices.insert(*horizon_ms, mark);
                                }
                            }
                        }
                    }
                }
                if job.counterfactual_exit_timestamp_ms.is_none() {
                    job.counterfactual_exit_timestamp_ms =
                        response.counterfactual_exit_timestamp_ms;
                    job.counterfactual_exit_mark_price = response.counterfactual_exit_mark_price;
                }
                virtual_open = response.counterfactual_position_open;
                entry_found = response.counterfactual_entry_found;
                job.counterfactual_position_open = virtual_open;
                job.counterfactual_entry_found = entry_found;
                job.signals_scanned = job.signals_scanned.saturating_add(response.evaluated_count);
            }
            job.pages_scanned = job.pages_scanned.saturating_add(1);
            job.history_cursor = cursor;
            job.updated_at_ms = crate::now_ms();
            self.save_strategy_replay_job(job).await?;
            more_available = more;
        }
        let observation_end = job.coverage_through_ms.unwrap_or_default();
        let base_coverage_complete = job.coverage_start_ms.is_some_and(|start| {
            start <= requested_from_ms.saturating_add(MAX_REPLAY_SIGNAL_GAP_MS)
        }) && job.archived_days_unscanned == 0
            && job.archived_days_unavailable == 0
            && job.corrupt_history_records == 0;
        let mut horizon_results = Vec::with_capacity(horizons.len());
        for horizon_ms in &horizons {
            let target = context.exit_timestamp_ms.saturating_add(*horizon_ms);
            let complete = base_coverage_complete
                && observation_end.saturating_add(MAX_REPLAY_SIGNAL_GAP_MS) >= target
                && job.cadence_gap_at_ms.is_none_or(|gap_at| gap_at > target);
            let classification = classify_replay_exit(
                complete,
                job.counterfactual_entry_found,
                job.counterfactual_position_open,
                job.counterfactual_exit_timestamp_ms,
                context.exit_timestamp_ms,
            );
            let estimated_mark_return_percent = job
                .horizon_mark_prices
                .get(horizon_ms)
                .copied()
                .map(|mark| gross_return_from_exit(context, mark));
            horizon_results.push(crate::StrategyReplayHorizonResult {
                horizon_ms: *horizon_ms,
                classification: classification.into(),
                counterfactual_exit_timestamp_ms: job.counterfactual_exit_timestamp_ms,
                estimated_mark_return_percent,
                complete,
            });
        }
        let any_complete_horizon = horizon_results.iter().any(|horizon| horizon.complete);
        let classification = horizon_results
            .iter()
            .rev()
            .find(|horizon| horizon.complete)
            .map(|horizon| horizon.classification.clone())
            .unwrap_or_else(|| "indeterminate".into());
        let estimated_mark_return_percent = horizon_results
            .iter()
            .rev()
            .find_map(|horizon| horizon.estimated_mark_return_percent);
        let reason = if !any_complete_horizon {
            "snapshot coverage does not reach any complete replay checkpoint".to_owned()
        } else if !job.counterfactual_entry_found {
            "the replay profile did not produce a counterfactual entry".to_owned()
        } else if estimated_mark_return_percent.is_some() {
            "10-second book markouts are indicative; they are not executable fills or slippage estimates".to_owned()
        } else {
            "no top-of-book marks were available at the replay checkpoints".to_owned()
        };
        Ok((
            if any_complete_horizon {
                crate::StrategyReplayStatus::Completed
            } else {
                crate::StrategyReplayStatus::Unsupported
            },
            Some(crate::StrategyReplayResult {
                classification,
                input_count: job.signals_scanned,
                evaluated_count: job.signals_scanned,
                first_exit_timestamp_ms: job.counterfactual_exit_timestamp_ms,
                estimated_mark_return_percent,
                execution_costs_complete: context.execution_costs_complete,
                reason: Some(reason),
                horizons: horizon_results,
            }),
            if any_complete_horizon {
                None
            } else {
                Some(
                    "snapshot history is incomplete or has gaps across all replay checkpoints"
                        .into(),
                )
            },
        ))
    }

    async fn route_worker_command(
        &self,
        command: ScoutCoordinatorRouteCommand,
    ) -> anyhow::Result<ScoutCoordinatorRouteResult> {
        if !self.enabled {
            bail!("Scout execution is disabled");
        }
        let targets = self.route_targets(&command).await?;
        if targets.is_empty() {
            return Ok(ScoutCoordinatorRouteResult {
                request_id: command.request_id,
                status: 204,
                body: serde_json::Value::Null,
            });
        }
        let mut results = Vec::with_capacity(targets.len());
        for strategy in targets {
            let endpoint = self
                .worker_endpoints
                .get(strategy.as_str())
                .with_context(|| {
                    format!("no RSocket worker configured for {}", strategy.as_str())
                })?;
            let mut payload = command.payload.clone();
            if command.path == "/configuration" && command.method == ScoutWorkerHttpMethod::Post {
                payload["strategies"] = serde_json::json!([strategy.as_str()]);
            }
            let worker_path = if command.path.split('?').next() == Some("/outcomes") {
                replace_query_parameter(&command.path, "page", "0", "page_size", "100")
            } else {
                command.path.clone()
            };
            let worker_command = ScoutWorkerRouteCommand {
                request_id: format!("{}:{}", command.request_id, strategy.as_str()),
                strategy,
                method: command.method,
                path: worker_path,
                payload,
            };
            results.push(self.dispatch_route(endpoint, worker_command).await?);
        }
        Ok(aggregate_route_results(command, results))
    }

    async fn accept_scout_feedback(
        &self,
        event: strategy::FeedbackEvent,
    ) -> anyhow::Result<ScoutFeedbackAcknowledgement> {
        let event_id = crate::event_parts(&event).0.to_owned();
        match crate::accept_feedback(axum::extract::State(self.state.clone()), axum::Json(event))
            .await
        {
            Ok(result) => Ok(ScoutFeedbackAcknowledgement {
                event_id,
                accepted: result.0.accepted,
                duplicate: result.0.duplicate,
                status: 200,
                detail: None,
            }),
            Err((status, message)) => Ok(ScoutFeedbackAcknowledgement {
                event_id,
                accepted: false,
                duplicate: false,
                status: status.as_u16(),
                detail: Some(message),
            }),
        }
    }

    async fn profile_snapshot(&self) -> anyhow::Result<serde_json::Value> {
        let state = self.state.state.lock().await;
        let profiles = state
            .active_profiles
            .values()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(serde_json::json!({"profiles": profiles}))
    }

    async fn dispatch_route(
        &self,
        endpoint: &str,
        command: ScoutWorkerRouteCommand,
    ) -> anyhow::Result<ScoutWorkerRouteResult> {
        let client = RSocketFactory::connect()
            .transport(WebsocketClientTransport::from(endpoint))
            .acceptor(Box::new(|| Box::new(EchoRSocket)))
            .start()
            .await
            .with_context(|| format!("could not connect to Scout worker at {endpoint}"))?;
        let response = tokio::time::timeout(
            self.worker_timeout,
            client.request_response(
                Payload::builder()
                    .set_data(serde_json::to_vec(&ScoutWorkerRequest::Route(
                        command.clone(),
                    ))?)
                    .build(),
            ),
        )
        .await
        .context("Scout worker route request timed out")??
        .context("Scout worker route returned an empty response")?;
        let response: ScoutWorkerResponse = serde_json::from_slice(
            response
                .data()
                .context("Scout worker route response had no data")?,
        )
        .context("Scout worker route response was invalid")?;
        let ScoutWorkerResponse::Route(result) = response else {
            bail!("Scout worker returned the wrong operation response");
        };
        if result.request_id != command.request_id || result.strategy != command.strategy {
            bail!("Scout worker returned a result for a different lifecycle command");
        }
        Ok(result)
    }

    async fn route_targets(
        &self,
        command: &ScoutCoordinatorRouteCommand,
    ) -> anyhow::Result<Vec<Strategy>> {
        if let Some(strategy) = command.strategy {
            return Ok(self
                .worker_endpoints
                .contains_key(strategy.as_str())
                .then_some(strategy)
                .into_iter()
                .collect());
        }
        let mut names = route_strategy_names(command);
        if names.is_empty() {
            let state = self.state.state.lock().await;
            names = state
                .active_profiles
                .values()
                .map(|profile| profile.strategy.clone())
                .collect();
        }
        if names.is_empty() {
            names.extend(Strategy::live_strategies().map(|strategy| strategy.as_str().to_owned()));
        }
        let mut strategies = names
            .into_iter()
            .filter_map(|name| Strategy::from_name(&name))
            .filter(|strategy| self.worker_endpoints.contains_key(strategy.as_str()))
            .collect::<Vec<_>>();
        let path = command.path.split('?').next().unwrap_or(&command.path);
        if path == "/outcomes" || path == "/status" || path == "/configuration" {
            strategies = self
                .worker_endpoints
                .keys()
                .filter_map(|name| Strategy::from_name(name))
                .collect();
        }
        strategies.sort_by_key(|strategy| strategy.as_str());
        strategies.dedup();
        Ok(strategies)
    }

    async fn local_route(
        &self,
        command: ScoutCoordinatorRouteCommand,
    ) -> anyhow::Result<ScoutCoordinatorRouteResult> {
        let path = command.path.split('?').next().unwrap_or(&command.path);
        if !learning_route_allowed(path, command.method) {
            bail!("unsupported Learning RSocket route: {path}");
        }
        let method = match command.method {
            ScoutWorkerHttpMethod::Get => http::Method::GET,
            ScoutWorkerHttpMethod::Post => http::Method::POST,
            ScoutWorkerHttpMethod::Put => http::Method::PUT,
            ScoutWorkerHttpMethod::Delete => http::Method::DELETE,
        };
        let body = if method == http::Method::GET || method == http::Method::DELETE {
            Vec::new()
        } else {
            serde_json::to_vec(&command.payload)?
        };
        let response = self
            .local_router
            .clone()
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(&command.path)
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))?,
            )
            .await?;
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await?.to_bytes();
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&bytes).into()));
        Ok(ScoutCoordinatorRouteResult {
            request_id: command.request_id,
            status,
            body,
        })
    }
}

fn learning_route_allowed(path: &str, method: ScoutWorkerHttpMethod) -> bool {
    matches!(
        (method, path),
        (ScoutWorkerHttpMethod::Get, "/profiles")
            | (ScoutWorkerHttpMethod::Get, "/decision-snapshot")
            | (ScoutWorkerHttpMethod::Get, "/summary")
            | (ScoutWorkerHttpMethod::Post, "/feedback")
            | (ScoutWorkerHttpMethod::Post, "/exit-timing")
            | (ScoutWorkerHttpMethod::Post, "/context-thresholds")
            | (ScoutWorkerHttpMethod::Post, "/execution-decision")
            | (ScoutWorkerHttpMethod::Post, "/execution-outcomes")
    )
}

fn route_strategy_names(command: &ScoutCoordinatorRouteCommand) -> Vec<String> {
    fn collect(value: &serde_json::Value, names: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(object) => {
                for (key, value) in object {
                    if matches!(key.as_str(), "strategy" | "strategy_name") {
                        if let Some(name) = value.as_str() {
                            if Strategy::from_name(name).is_some() {
                                names.insert(name.to_owned());
                            }
                        }
                    }
                    collect(value, names);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    collect(value, names);
                }
            }
            _ => {}
        }
    }
    let mut names = std::collections::BTreeSet::new();
    collect(&command.payload, &mut names);
    if command.path == "/live-inputs" {
        return Strategy::live_strategies()
            .map(|strategy| strategy.as_str().to_owned())
            .collect();
    }
    if command.path == "/dated-carry-inputs" {
        return vec![Strategy::DatedCarry.as_str().to_owned()];
    }
    if command.path == "/variants" || command.path == "/variants/batch" {
        return Vec::new();
    }
    names.into_iter().collect()
}

fn replace_query_parameter(
    path: &str,
    first_key: &str,
    first_value: &str,
    second_key: &str,
    second_value: &str,
) -> String {
    let (base, query) = path.split_once('?').unwrap_or((path, ""));
    let mut parameters = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .collect::<BTreeMap<_, _>>();
    parameters.insert(first_key, first_value);
    parameters.insert(second_key, second_value);
    let query = parameters
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{query}")
}

fn aggregate_route_results(
    command: ScoutCoordinatorRouteCommand,
    results: Vec<ScoutWorkerRouteResult>,
) -> ScoutCoordinatorRouteResult {
    if results.len() == 1 && command.path.split('?').next() != Some("/outcomes") {
        let result = results.into_iter().next().unwrap();
        return ScoutCoordinatorRouteResult {
            request_id: command.request_id,
            status: result.status,
            body: result.body,
        };
    }
    if let Some(error) = results.iter().find(|result| result.status >= 400) {
        return ScoutCoordinatorRouteResult {
            request_id: command.request_id,
            status: error.status,
            body: error.body.clone(),
        };
    }
    let path = command.path.split('?').next().unwrap_or(&command.path);
    let bodies = results
        .into_iter()
        .map(|result| (result.strategy.as_str().to_owned(), result.body))
        .collect::<BTreeMap<_, _>>();
    let body = match path {
        "/live-inputs" | "/daily-inputs" | "/dated-carry-inputs" | "/scout-markout" => {
            serde_json::Value::Null
        }
        "/live-candidates" => serde_json::Value::Array(
            bodies
                .values()
                .flat_map(|body| body.as_array().cloned().unwrap_or_default())
                .collect(),
        ),
        "/configuration" => {
            let mut configuration = bodies.values().next().cloned().unwrap_or_default();
            configuration["strategies"] = serde_json::json!(bodies.keys().collect::<Vec<_>>());
            configuration
        }
        "/outcomes" => aggregate_outcomes(&bodies, &command.path),
        "/status" => aggregate_status(&bodies),
        _ => serde_json::json!({"by_strategy": bodies}),
    };
    ScoutCoordinatorRouteResult {
        request_id: command.request_id,
        status: 200,
        body,
    }
}

fn aggregate_outcomes(
    bodies: &BTreeMap<String, serde_json::Value>,
    path: &str,
) -> serde_json::Value {
    let mut rows = Vec::new();
    let mut learning_rows = Vec::new();
    let mut learning_outcomes = Vec::new();
    let mut pending_probes = Vec::new();
    let mut symbols = std::collections::BTreeSet::new();
    let mut summaries = serde_json::json!({"all":{"total":0,"open":0,"resolved":0,"wins":0,"losses":0,"flats":0,"unavailable":0},"live":{"total":0,"open":0,"resolved":0,"wins":0,"losses":0,"flats":0,"unavailable":0}});
    for body in bodies.values() {
        rows.extend(body["rows"].as_array().cloned().unwrap_or_default());
        learning_rows.extend(body["learningRows"].as_array().cloned().unwrap_or_default());
        learning_outcomes.extend(
            body["learningOutcomes"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        );
        pending_probes.extend(
            body["pendingParameterProbes"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        );
        symbols.extend(
            body["symbols"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().map(str::to_owned)),
        );
        for group in ["all", "live"] {
            for key in [
                "total",
                "open",
                "resolved",
                "wins",
                "losses",
                "flats",
                "unavailable",
            ] {
                summaries[group][key] = serde_json::json!(
                    summaries[group][key]
                        .as_u64()
                        .unwrap_or_default()
                        .saturating_add(body["summary"][group][key].as_u64().unwrap_or_default())
                );
            }
        }
    }
    let params = path
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default()
        .split('&')
        .filter_map(|part| part.split_once('='))
        .collect::<BTreeMap<_, _>>();
    let page = params
        .get("page")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_default();
    let page_size = params
        .get("page_size")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(50)
        .clamp(1, 100);
    let sort_by = params.get("sort_by").copied().unwrap_or("openedAt");
    let descending = params.get("sort_direction").copied() != Some("asc");
    rows.sort_by(|left, right| {
        let key = match sort_by {
            "symbol" => "symbol",
            "strategy" => "strategy",
            "status" => "status",
            "exitScenario" => "exitScenario",
            "scoutLevelPercent" => "scoutLevelPercent",
            "durationLabel" => "durationMs",
            "entryPrice" => "entryPrice",
            "markPrice" => "markPrice",
            "netReturnLabel" | "displayNetReturnPercent" => "displayNetReturnPercent",
            "pairedDeltaPercent" => "pairedDeltaPercent",
            "strength" => "strength",
            _ => "openedAt",
        };
        let ordering = if let (Some(a), Some(b)) = (left[key].as_f64(), right[key].as_f64()) {
            a.total_cmp(&b)
        } else if let (Some(a), Some(b)) = (left[key].as_str(), right[key].as_str()) {
            a.cmp(b)
        } else {
            left["openedAt"].as_u64().cmp(&right["openedAt"].as_u64())
        };
        let ordering = if descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| {
            left["decisionId"]
                .as_str()
                .cmp(&right["decisionId"].as_str())
        })
    });
    let total_rows = rows.len();
    let page_count = total_rows.div_ceil(page_size);
    let page = page.min(page_count.saturating_sub(1));
    let page_rows = rows
        .drain(page.saturating_mul(page_size)..)
        .take(page_size)
        .collect::<Vec<_>>();
    serde_json::json!({
        "rows": page_rows, "summary": summaries, "learningRows": learning_rows,
        "learningOutcomes": learning_outcomes, "pendingParameterProbes": pending_probes,
        "symbols": symbols, "learningSymbol": params.get("learning_symbol").filter(|symbol| **symbol != "all").copied(), "updatedAt": crate::now_ms(),
        "source": params.get("source").copied().unwrap_or("live"),
        "status": params.get("status").copied().unwrap_or("all"),
        "sortBy": sort_by,
        "sortDirection": if descending { "desc" } else { "asc" },
        "page": page, "pageSize": page_size, "pageCount": page_count,
        "totalRows": total_rows, "pageNumber": page + 1,
        "hasPrevious": page > 0, "hasNext": page + 1 < page_count,
    })
}

fn aggregate_status(bodies: &BTreeMap<String, serde_json::Value>) -> serde_json::Value {
    let total = |key: &str| {
        bodies
            .values()
            .map(|body| body[key].as_u64().unwrap_or_default())
            .sum::<u64>()
    };
    serde_json::json!({
        "active_samples": total("active_samples"),
        "learning_samples": total("learning_samples"),
        "positive_samples": total("positive_samples"),
        "realized_pnl_usd": bodies.values().map(|body| body["realized_pnl_usd"].as_f64().unwrap_or_default()).sum::<f64>(),
    })
}

fn signal_timestamp_ms(signal: &serde_json::Value) -> u64 {
    signal["book_updated_at_ms"]
        .as_u64()
        .or_else(|| signal["timestamp"].as_u64())
        .unwrap_or_default()
}

fn selected_profiles(
    state: &crate::LearningState,
    symbol: &str,
    signal: &serde_json::Value,
) -> anyhow::Result<Vec<StrategyProfile>> {
    if signal["strategy_candidate"].as_bool() == Some(true) {
        return Ok(Vec::new());
    }
    let strategy_names = match signal["strategy"].as_str() {
        Some(name) => {
            if Strategy::from_name(name).is_none() {
                bail!("Scout signal names unknown strategy {name}");
            }
            vec![name.to_owned()]
        }
        None => Strategy::live_strategies()
            .map(|strategy| strategy.as_str().to_owned())
            .collect(),
    };
    let mut profiles = Vec::new();
    for strategy_name in strategy_names {
        let Some(strategy) = Strategy::from_name(&strategy_name) else {
            continue;
        };
        let key = format!("{symbol}:{strategy_name}");
        let baseline = state.active_profiles.get(&key).cloned().unwrap_or_else(|| {
            let parameters = strategy
                .parameter_specs()
                .into_iter()
                .map(|spec| (spec.key.as_str().to_owned(), spec.default))
                .collect::<BTreeMap<_, _>>();
            StrategyProfile {
                profile_id: strategy::profile_id(symbol, &strategy_name, &parameters),
                version: 0,
                symbol: symbol.to_owned(),
                strategy: strategy_name.clone(),
                parameters,
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            }
        });
        profiles.push(baseline.clone());
        let mut variants = state
            .variants
            .values()
            .filter(|registered| {
                registered.profile.symbol == symbol
                    && registered.profile.strategy == strategy_name
                    && registered.profile.parameters != baseline.parameters
            })
            .map(|registered| registered.profile.clone())
            .collect::<Vec<_>>();
        variants.sort_by(|left, right| left.profile_id.cmp(&right.profile_id));
        profiles.extend(variants);
    }
    Ok(profiles)
}

fn candidates_for_profile(
    signal: &serde_json::Value,
    profile: &StrategyProfile,
) -> anyhow::Result<Vec<serde_json::Value>> {
    if signal["symbol"].as_str() != Some(profile.symbol.as_str()) {
        bail!("Scout signal symbol does not match its selected Learning profile");
    }
    let strategy = Strategy::from_name(&profile.strategy)
        .with_context(|| format!("unknown strategy {}", profile.strategy))?;
    let mut candidate = signal.clone();
    candidate["strategy"] = serde_json::json!(profile.strategy);
    let mut direction = signal["direction"].as_str().unwrap_or_default().to_owned();
    if !matches!(direction.as_str(), "bullish" | "bearish") {
        let components = &signal["components"];
        let values = match strategy {
            Strategy::LiveMomentum => vec![
                components["momentum_10s_percent"].as_f64(),
                components["trend_5m_volatility_units"].as_f64(),
            ],
            Strategy::LiveTradeFlow => vec![components["trade_flow_imbalance"].as_f64()],
            Strategy::LiveOrderBook => vec![components["book_imbalance"].as_f64()],
            Strategy::LiveVwapDislocation => vec![components["mispricing_vs_vwap_bps"].as_f64()],
            Strategy::LiveAbsorptionPressure => vec![
                components["pressure_without_price_move"]
                    .as_f64()
                    .map(|value| -value),
            ],
            _ => Vec::new(),
        };
        let directions = values
            .into_iter()
            .flatten()
            .filter(|value| value.is_finite() && *value != 0.0)
            .map(|value| if value > 0.0 { "bullish" } else { "bearish" })
            .collect::<std::collections::BTreeSet<_>>();
        if directions.len() != 1 {
            return Ok(Vec::new());
        }
        direction = directions.into_iter().next().unwrap_or_default().to_owned();
    }
    if direction.is_empty() {
        return Ok(Vec::new());
    }
    let side = if direction == "bullish" { 1.0 } else { -1.0 };
    candidate["direction"] = serde_json::json!(direction);
    candidate["score"] =
        serde_json::json!(signal["score"].as_f64().unwrap_or_default().abs() * side);
    candidate["strength"] = signal["strength"].clone();
    let mut decision = candidate["strategy_decision"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    decision.insert("strategy".into(), serde_json::json!(profile.strategy));
    decision.insert(
        "strategy_parameters".into(),
        serde_json::to_value(&profile.parameters)?,
    );
    decision.insert("direction".into(), serde_json::json!(direction));
    decision.insert(
        "direction_source".into(),
        serde_json::json!("strategy_insight"),
    );
    candidate["strategy_decision"] = serde_json::Value::Object(decision);
    Ok(vec![candidate])
}

fn walk_forward_evidence(
    state: &crate::LearningState,
    profile: &StrategyProfile,
    signal_timestamp_ms: u64,
) -> Option<strategy::StrategyWalkForwardEvidence> {
    let registered = state.variants.values().find(|registered| {
        registered.profile.symbol == profile.symbol
            && registered.profile.strategy == profile.strategy
            && registered.profile.parameters == profile.parameters
    })?;
    let samples = registered
        .outcomes
        .iter()
        .filter(|outcome| {
            outcome.observed_at_ms < signal_timestamp_ms
                && outcome.net_return_percent.is_finite()
                && matches!(outcome.source, crate::LearningOutcomeSource::Scout)
        })
        .collect::<Vec<_>>();
    if samples.is_empty() {
        return None;
    }
    let sample_count = samples.len();
    let positive_rate = samples
        .iter()
        .filter(|outcome| outcome.net_return_percent > 0.0)
        .count() as f64
        / sample_count as f64;
    let average_return = samples
        .iter()
        .map(|outcome| outcome.net_return_percent)
        .sum::<f64>()
        / sample_count as f64;
    Some(strategy::StrategyWalkForwardEvidence {
        sample_count,
        positive_net_rate: Some(positive_rate),
        average_net_return_percent: Some(average_return),
        last_sample_timestamp_ms: samples.last()?.observed_at_ms,
    })
}

fn requested_profile_parameters(signal: &serde_json::Value) -> Option<BTreeMap<String, f64>> {
    serde_json::from_value(signal["strategy_decision"]["strategy_parameters"].clone()).ok()
}

fn compact_evidence_candidate(
    signal: &serde_json::Value,
    profile: &StrategyProfile,
) -> serde_json::Value {
    serde_json::json!({
        "strategy": profile.strategy,
        "symbol": profile.symbol,
        "timestamp": signal["timestamp"],
        "strategy_decision": {
            "strategy_parameters": profile.parameters,
        },
    })
}

fn parse_worker_endpoints(input: &str) -> Result<BTreeMap<String, String>, String> {
    let mut endpoints = BTreeMap::new();
    if input.trim().is_empty() {
        return Ok(endpoints);
    }
    for entry in input.split(';') {
        let (strategy_name, endpoint) = entry.split_once('=').ok_or_else(|| {
            "Scout worker endpoints must use strategy=ws://host:port entries".to_owned()
        })?;
        let strategy = Strategy::from_name(strategy_name)
            .ok_or_else(|| format!("unknown Scout worker strategy {strategy_name}"))?;
        if !(endpoint.starts_with("ws://") || endpoint.starts_with("wss://")) {
            return Err(format!(
                "Scout worker {} endpoint must use RSocket WebSocket",
                strategy.as_str()
            ));
        }
        if endpoints
            .insert(strategy.as_str().to_owned(), endpoint.to_owned())
            .is_some()
        {
            return Err(format!(
                "duplicate Scout worker endpoint for {}",
                strategy.as_str()
            ));
        }
    }
    Ok(endpoints)
}

fn env_limit(name: &str, default: usize) -> Result<usize, String> {
    match std::env::var(name) {
        Ok(value) => {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| format!("{name} must be a positive integer"))?;
            if parsed == 0 {
                return Err(format!("{name} must be a positive integer"));
            }
            Ok(parsed)
        }
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(format!("could not read {name}: {error}")),
    }
}

async fn dispatch_assignment(
    endpoint: &str,
    assignment: &ScoutWorkAssignment,
) -> anyhow::Result<ScoutWorkResult> {
    let client = RSocketFactory::connect()
        .transport(WebsocketClientTransport::from(endpoint))
        .acceptor(Box::new(|| Box::new(EchoRSocket)))
        .start()
        .await
        .with_context(|| format!("could not connect to Scout worker at {endpoint}"))?;
    let response = client
        .request_response(
            Payload::builder()
                .set_data(serde_json::to_vec(&ScoutWorkerRequest::Evaluate(
                    assignment.clone(),
                ))?)
                .build(),
        )
        .await?
        .context("Scout worker returned an empty response")?;
    let response: ScoutWorkerResponse = serde_json::from_slice(
        response
            .data()
            .context("Scout worker response had no data")?,
    )
    .context("Scout worker response was not a valid result")?;
    let ScoutWorkerResponse::Evaluation(result) = response else {
        bail!("Scout worker returned the wrong operation response");
    };
    Ok(result)
}

async fn dispatch_strategy_replay(
    endpoint: &str,
    request: &StrategyReplayRequest,
) -> anyhow::Result<StrategyReplayResponse> {
    let client = RSocketFactory::connect()
        .transport(WebsocketClientTransport::from(endpoint))
        .acceptor(Box::new(|| Box::new(EchoRSocket)))
        .start()
        .await
        .with_context(|| format!("could not connect to strategy worker at {endpoint}"))?;
    let response = client
        .request_response(
            Payload::builder()
                .set_data(serde_json::to_vec(&ScoutWorkerRequest::StrategyReplay(
                    request.clone(),
                ))?)
                .build(),
        )
        .await?
        .context("strategy worker returned an empty replay response")?;
    let response: ScoutWorkerResponse = serde_json::from_slice(
        response
            .data()
            .context("strategy worker response had no data")?,
    )
    .context("invalid strategy worker response")?;
    let ScoutWorkerResponse::StrategyReplay(result) = response else {
        bail!("strategy worker returned the wrong replay operation");
    };
    Ok(result)
}

#[derive(Clone)]
struct CoordinatorResponder(ScoutCoordinator);

#[rsocket_rust::async_trait]
impl RSocket for CoordinatorResponder {
    async fn metadata_push(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("Learning Scout coordinator accepts request-response only")
    }

    async fn fire_and_forget(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("Learning Scout coordination requires request-response results")
    }

    async fn request_response(&self, req: Payload) -> anyhow::Result<Option<Payload>> {
        let data = req.data().context("Scout candidate batch is empty")?;
        if data.len() > self.0.max_batch_bytes {
            bail!("Scout candidate batch exceeds the configured payload limit");
        }
        let request: ScoutCoordinatorRequest =
            serde_json::from_slice(data).context("invalid Scout coordinator request")?;
        let result = match request {
            ScoutCoordinatorRequest::CandidateBatch(request) => {
                ScoutCoordinatorResponse::CandidateBatch(self.0.coordinate(request).await?)
            }
            ScoutCoordinatorRequest::StrategyReplay(request) => {
                ScoutCoordinatorResponse::StrategyReplay(self.0.replay_strategy(request).await?)
            }
            ScoutCoordinatorRequest::WorkerRoute(command) => {
                ScoutCoordinatorResponse::WorkerRoute(self.0.route_worker_command(command).await?)
            }
            ScoutCoordinatorRequest::Feedback(event) => {
                ScoutCoordinatorResponse::Feedback(self.0.accept_scout_feedback(event).await?)
            }
            ScoutCoordinatorRequest::LearningRoute(command) => {
                ScoutCoordinatorResponse::LearningRoute(self.0.local_route(command).await?)
            }
            ScoutCoordinatorRequest::ProfileSnapshot => {
                ScoutCoordinatorResponse::ProfileSnapshot(self.0.profile_snapshot().await?)
            }
        };
        Ok(Some(
            Payload::builder()
                .set_data(serde_json::to_vec(&result)?)
                .build(),
        ))
    }

    fn request_stream(
        &self,
        _req: Payload,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
        Box::pin(stream::once(async {
            Err(anyhow::anyhow!(
                "Learning Scout coordinator does not stream work"
            ))
        }))
    }

    fn request_channel(
        &self,
        _reqs: Flux<anyhow::Result<Payload>>,
    ) -> Flux<anyhow::Result<Payload>> {
        Box::pin(stream::once(async {
            Err(anyhow::anyhow!(
                "Learning Scout coordinator does not accept request channels"
            ))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LearningState, RegisteredProfile, variant_key};
    use rsocket_rust_transport_websocket::WebsocketServerTransport;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex as StdMutex};
    use strategy::StrategyProfile;
    use tokio::sync::Mutex;

    #[test]
    fn replay_classification_requires_complete_attributed_evidence() {
        assert_eq!(
            classify_replay_exit(true, true, true, Some(100_010), 100_000),
            "on_time"
        );
        assert_eq!(
            classify_replay_exit(true, true, false, Some(70_000), 100_000),
            "late"
        );
        assert_eq!(
            classify_replay_exit(true, true, true, Some(130_000), 100_000),
            "early"
        );
        assert_eq!(
            classify_replay_exit(true, true, true, None, 100_000),
            "early"
        );
        assert_eq!(
            classify_replay_exit(false, true, true, Some(70_000), 100_000),
            "indeterminate"
        );
        assert_eq!(
            classify_replay_exit(true, false, false, None, 100_000),
            "indeterminate"
        );
    }

    #[test]
    fn default_live_replay_uses_four_increasing_checkpoints() {
        assert_eq!(
            strategy_replay_horizons_ms("live-momentum"),
            Some(vec![300_000, 1_800_000, 7_200_000, 28_800_000])
        );
        assert!(strategy_replay_horizons_ms("donchian-20-btc200").is_none());
    }

    #[test]
    fn missed_opportunity_bands_require_complete_history_and_keep_gross_markouts_out_of_net_fields()
    {
        let job = crate::MissedOpportunityJob {
            event_id: "missed-1".into(),
            observed_at_ms: 1_000,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "profile-1".into(),
            parameters: BTreeMap::new(),
            direction: "bullish".into(),
            strength: 0.6,
            score: 0.3,
            learning_context: "trend:btc".into(),
            status: crate::StrategyReplayStatus::Completed,
            reason: None,
            horizons: BTreeMap::new(),
            history_cursor: None,
            pages_scanned: 1,
            coverage_start_ms: Some(1_000),
            coverage_through_ms: Some(30_000_000),
            corrupt_records: 0,
            archived_days_unscanned: 0,
            archived_days_unavailable: 0,
            cadence_gap_detected: false,
            cadence_gap_at_ms: None,
            reference_price: None,
        };
        let incomplete = crate::MissedOpportunityHorizon {
            mark_price: Some(101.0),
            timestamp_ms: Some(301_000),
            gross_directional_return_percent: Some(1.0),
            complete: false,
        };
        assert!(missed_opportunity_band(&job, 300_000, &incomplete, "score", 0.3).is_none());
        let complete = crate::MissedOpportunityHorizon {
            complete: true,
            ..incomplete
        };
        let band = missed_opportunity_band(&job, 300_000, &complete, "score", 0.3).unwrap();
        assert!(band.complete);
        assert_eq!(band.gross_directional_markout_percent, Some(1.0));
        assert_eq!(band.average_net_return_percent, None);
        assert_eq!(band.stop_target_average_net_return_percent, None);
        assert_eq!(band.profile_id.as_deref(), Some("profile-1"));
    }

    struct AssignmentResponder(Arc<StdMutex<Vec<ScoutWorkAssignment>>>);

    #[rsocket_rust::async_trait]
    impl RSocket for AssignmentResponder {
        async fn metadata_push(&self, _req: Payload) -> anyhow::Result<()> {
            bail!("unsupported")
        }
        async fn fire_and_forget(&self, _req: Payload) -> anyhow::Result<()> {
            bail!("unsupported")
        }
        async fn request_response(&self, req: Payload) -> anyhow::Result<Option<Payload>> {
            let request: ScoutWorkerRequest =
                serde_json::from_slice(req.data().context("missing test assignment")?)?;
            let ScoutWorkerRequest::Evaluate(assignment) = request else {
                bail!("expected evaluation assignment")
            };
            self.0.lock().unwrap().push(assignment.clone());
            let result = ScoutWorkResult {
                assignment_id: assignment.assignment_id,
                strategy: assignment.strategy,
                symbol: assignment.symbol,
                decision: strategy::ScoutWorkDecision::Enter,
                candidate: assignment.candidate,
                walk_forward_evidence: None,
            };
            Ok(Some(
                Payload::builder()
                    .set_data(serde_json::to_vec(&ScoutWorkerResponse::Evaluation(
                        result,
                    ))?)
                    .build(),
            ))
        }
        fn request_stream(
            &self,
            _req: Payload,
        ) -> Pin<Box<dyn Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
            Box::pin(stream::empty())
        }
        fn request_channel(
            &self,
            reqs: Flux<anyhow::Result<Payload>>,
        ) -> Flux<anyhow::Result<Payload>> {
            reqs
        }
    }

    #[test]
    fn worker_endpoint_configuration_accepts_only_known_strategy_rsocket_addresses() {
        let parsed = parse_worker_endpoints(
            "live-momentum=ws://scout-live-momentum:10200;dated-futures-carry=wss://carry:10200",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert!(parse_worker_endpoints("not-a-strategy=ws://worker:10200").is_err());
        assert!(parse_worker_endpoints("live-momentum=http://worker:10200").is_err());
        assert!(parse_worker_endpoints("live-momentum=ws://a:1;live-momentum=ws://b:2").is_err());
    }

    #[test]
    fn outcome_aggregation_merges_worker_pages_and_preserves_global_pagination() {
        let bodies = BTreeMap::from([
            (
                "live-momentum".to_owned(),
                serde_json::json!({
                    "rows":[{"decisionId":"b","openedAt":20,"strategy":"live-momentum"}],
                    "summary":{"all":{"total":1,"wins":1},"live":{"total":1,"wins":1}},
                    "learningRows":[],"learningOutcomes":[],"pendingParameterProbes":[],"symbols":["BTC-USDT"]
                }),
            ),
            (
                "live-trade-flow".to_owned(),
                serde_json::json!({
                    "rows":[{"decisionId":"a","openedAt":10,"strategy":"live-trade-flow"}],
                    "summary":{"all":{"total":1,"wins":0},"live":{"total":1,"wins":0}},
                    "learningRows":[],"learningOutcomes":[],"pendingParameterProbes":[],"symbols":["ETH-USDT"]
                }),
            ),
        ]);
        let response = aggregate_outcomes(&bodies, "/outcomes?source=all&page=1&page_size=1");
        assert_eq!(response["totalRows"], 2);
        assert_eq!(response["pageCount"], 2);
        assert_eq!(response["rows"][0]["decisionId"], "a");
        assert_eq!(response["summary"]["all"]["total"], 2);
        assert_eq!(response["summary"]["all"]["wins"], 1);
        assert_eq!(response["symbols"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn evidence_only_uses_the_requested_profile_without_worker_or_full_signal_copy() {
        let baseline = StrategyProfile {
            profile_id: "baseline".into(),
            version: 1,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            parameters: BTreeMap::from([("minimum_momentum".into(), 0.45)]),
            learning_samples: 1,
            positive_samples: 1,
            realized_pnl: 1.0,
        };
        let requested = StrategyProfile {
            profile_id: "requested".into(),
            version: 0,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            parameters: BTreeMap::from([("minimum_momentum".into(), 0.6)]),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let mut state = LearningState::default();
        state
            .active_profiles
            .insert("BTC-USDT:live-momentum".into(), baseline);
        state.variants.insert(
            variant_key(&requested),
            RegisteredProfile {
                profile: requested,
                specs: Vec::new(),
                samples: 0,
                positive: 0,
                realized_pnl: 0.0,
                outcomes: Vec::new(),
            },
        );
        let service = ServiceState {
            state: Arc::new(Mutex::new(state)),
            path: PathBuf::new(),
        };
        let coordinator =
            ScoutCoordinator::new(service, "live-momentum=ws://127.0.0.1:1", Router::new())
                .unwrap();

        let response = coordinator
            .coordinate(ScoutCandidateBatch {
                request_id: "evidence-only".into(),
                mode: ScoutWorkMode::EvidenceOnly,
                signals: vec![serde_json::json!({
                    "symbol":"BTC-USDT",
                    "strategy":"live-momentum",
                    "direction":"bullish",
                    "timestamp":123,
                    "strategy_decision":{"strategy_parameters":{"minimum_momentum":0.6}},
                    "components":{"large_market_payload":"discarded from the compact response"}
                })],
            })
            .await
            .unwrap();

        assert_eq!(response.assignments.len(), 1);
        assert_eq!(
            response.assignments[0].decision,
            ScoutWorkDecision::NotEvaluated
        );
        assert_eq!(
            response.assignments[0].candidate["strategy"],
            "live-momentum"
        );
        assert_eq!(response.assignments[0].candidate["timestamp"], 123);
        assert_eq!(
            response.assignments[0].candidate["strategy_decision"]["strategy_parameters"]["minimum_momentum"],
            0.6
        );
        assert!(
            response.assignments[0]
                .candidate
                .get("components")
                .is_none()
        );
    }

    #[tokio::test]
    async fn learning_selects_active_profile_and_dispatches_typed_assignment_over_rsocket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let assignments = Arc::new(StdMutex::new(Vec::new()));
        let worker_assignments = Arc::clone(&assignments);
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_setup, _socket| {
                Ok(Box::new(AssignmentResponder(Arc::clone(
                    &worker_assignments,
                ))))
            }))
            .serve();
        let server_task = tokio::spawn(async move { server.await });

        let mut state = LearningState::default();
        state.active_profiles.insert(
            "BTC-USDT:live-momentum".into(),
            StrategyProfile {
                profile_id: "profile-v2".into(),
                version: 2,
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                parameters: BTreeMap::from([("minimum_momentum".into(), 0.45)]),
                learning_samples: 12,
                positive_samples: 9,
                realized_pnl: 2.3,
            },
        );
        let service = ServiceState {
            state: Arc::new(Mutex::new(state)),
            path: PathBuf::new(),
        };
        let endpoint = format!("live-momentum=ws://{address}");
        let coordinator = ScoutCoordinator::new(service, &endpoint, Router::new()).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let response = coordinator
            .coordinate(ScoutCandidateBatch {
                request_id: "signal-batch-1".into(),
                mode: strategy::ScoutWorkMode::Evaluate,
                signals: vec![serde_json::json!({
                    "symbol":"BTC-USDT", "strategy":"live-momentum", "direction":"bullish",
                    "components":{"momentum_10s_percent":0.8}
                })],
            })
            .await
            .unwrap();
        assert_eq!(response.request_id, "signal-batch-1");
        assert_eq!(response.assignments.len(), 1);
        assert_eq!(
            response.assignments[0].decision,
            strategy::ScoutWorkDecision::Enter
        );
        let received = assignments.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].strategy, Strategy::LiveMomentum);
        assert_eq!(received[0].parameters["minimum_momentum"], 0.45);
        assert_eq!(received[0].symbol, "BTC-USDT");

        server_task.abort();
    }
}
