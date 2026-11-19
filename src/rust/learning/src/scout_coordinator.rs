use anyhow::{Context, bail};
use axum::{Router, body::Body};
use futures_util::{Stream, StreamExt, stream};
use http_body_util::BodyExt;
use rsocket_rust::{Client, prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::{WebsocketClientTransport, WebsocketServerTransport};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    pin::Pin,
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::Duration,
};
use strategy::{
    MAX_REPLAY_SIGNALS, ScoutCandidateBatch, ScoutCandidateBatchResult, ScoutCoordinatorRequest,
    ScoutCoordinatorResponse, ScoutCoordinatorRouteCommand, ScoutCoordinatorRouteResult,
    ScoutDataUpdate, ScoutFeedbackAcknowledgement, ScoutStreamEnvelope, ScoutStreamSubscription,
    ScoutWorkAssignment, ScoutWorkDecision, ScoutWorkMode, ScoutWorkResult, ScoutWorkerHttpMethod,
    ScoutWorkerRequest, ScoutWorkerResponse, ScoutWorkerRouteCommand, ScoutWorkerRouteResult,
    Strategy, StrategyProfile, StrategyReplayPoint, StrategyReplayRequest, StrategyReplayResponse,
    request_scout_service_route,
};
use tokio::sync::{Mutex as AsyncMutex, broadcast, oneshot};
use tower::ServiceExt;

use crate::ServiceState;

const DEFAULT_MAX_SCOUT_BATCH_SIGNALS: usize = 64;
const SCOUT_UPDATE_QUEUE_CAPACITY: usize = 16_384;
const DEFAULT_MAX_SCOUT_ASSIGNMENTS: usize = 512;
const DEFAULT_MAX_SCOUT_BATCH_BYTES: usize = 8 * 1024 * 1024;
const PROFILE_SNAPSHOT_PUBLISH_INTERVAL: Duration = Duration::from_secs(120);
const DEFAULT_WORKER_TIMEOUT_MS: usize = 3_000;
const DEFAULT_REPLAY_WORKER_TIMEOUT_MS: usize = 30_000;
const MAX_REPLAY_TOTAL_SIGNALS: usize = 10_000;
const MAX_REPLAY_TOTAL_PAGES: usize = 128;
const MAX_REPLAY_TOTAL_RECORDS_SCANNED: usize = 524_288;
const MAX_REPLAY_TOTAL_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_REPLAY_SIGNAL_GAP_MS: u64 = 30_000;
const MAX_WORKER_SCOUT_VARIANTS: usize = 2_000;
const MAX_AUTOTRADER_SCOUT_VARIANTS: usize = 5_000;

type ScoutWorkerClientPool = HashMap<String, Arc<Client>>;

fn scout_worker_client_pool() -> &'static StdMutex<ScoutWorkerClientPool> {
    static CLIENTS: OnceLock<StdMutex<ScoutWorkerClientPool>> = OnceLock::new();
    CLIENTS.get_or_init(|| StdMutex::new(HashMap::new()))
}

async fn pooled_scout_worker_client(endpoint: &str) -> anyhow::Result<Arc<Client>> {
    if let Some(client) = scout_worker_client_pool()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(endpoint)
        .cloned()
    {
        return Ok(client);
    }

    let connected = Arc::new(
        RSocketFactory::connect()
            .transport(WebsocketClientTransport::from(endpoint))
            .acceptor(Box::new(|| Box::new(EchoRSocket)))
            .start()
            .await
            .with_context(|| format!("could not connect to Scout worker at {endpoint}"))?,
    );
    let mut clients = scout_worker_client_pool()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(clients
        .entry(endpoint.to_owned())
        .or_insert(connected)
        .clone())
}

fn invalidate_scout_worker_client(endpoint: &str, failed_client: &Arc<Client>) {
    let mut clients = scout_worker_client_pool()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if clients
        .get(endpoint)
        .is_some_and(|client| Arc::ptr_eq(client, failed_client))
    {
        clients.remove(endpoint);
    }
}

struct ScoutUpdateHub {
    sender: broadcast::Sender<ScoutStreamEnvelope>,
    sequence: AtomicU64,
    latest: std::sync::RwLock<BTreeMap<(String, String), ScoutStreamEnvelope>>,
    assignments: AsyncMutex<BTreeMap<String, oneshot::Sender<Result<ScoutWorkResult, String>>>>,
    replays: AsyncMutex<BTreeMap<String, oneshot::Sender<Result<StrategyReplayResponse, String>>>>,
}

impl ScoutUpdateHub {
    fn new() -> Arc<Self> {
        let (sender, _) = broadcast::channel(SCOUT_UPDATE_QUEUE_CAPACITY);
        Arc::new(Self {
            sender,
            sequence: AtomicU64::new(0),
            latest: std::sync::RwLock::new(BTreeMap::new()),
            assignments: AsyncMutex::new(BTreeMap::new()),
            replays: AsyncMutex::new(BTreeMap::new()),
        })
    }

    fn publish(&self, strategy: Strategy, update: ScoutDataUpdate) -> u64 {
        let key = scout_cache_key(&update);
        let update = match update {
            ScoutDataUpdate::Profiles(snapshot) => {
                let previous = key.as_ref().and_then(|key| {
                    self.latest
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&(strategy.as_str().to_owned(), key.clone()))
                        .and_then(|envelope| match &envelope.update {
                            ScoutDataUpdate::Profiles(value) => Some(value.clone()),
                            _ => None,
                        })
                });
                ScoutDataUpdate::Profiles(merge_profile_snapshots(previous.as_ref(), snapshot))
            }
            update => update,
        };
        if let Some(key) = key.as_ref() {
            let latest = self.latest.read().unwrap_or_else(|e| e.into_inner());
            if latest
                .get(&(strategy.as_str().to_owned(), key.clone()))
                .is_some_and(|previous| previous.update == update)
            {
                return self.sequence.load(Ordering::Relaxed);
            }
        }
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let envelope = ScoutStreamEnvelope {
            strategy,
            sequence,
            replayed_from_cache: false,
            update,
        };
        if let Some(key) = key {
            self.latest
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert((strategy.as_str().to_owned(), key), envelope.clone());
        }
        let _ = self.sender.send(envelope);
        sequence
    }

    fn snapshot(&self, strategy: Strategy) -> Vec<ScoutStreamEnvelope> {
        self.latest
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|((name, _), _)| name == strategy.as_str())
            .map(|(_, update)| {
                let mut cached = update.clone();
                cached.replayed_from_cache = true;
                cached
            })
            .collect()
    }

    fn forget_assignment(&self, strategy: Strategy, assignment_id: &str) {
        self.latest
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(
                strategy.as_str().to_owned(),
                format!("assignment:{assignment_id}"),
            ));
    }

    fn has_cached(&self, strategy: Strategy, key: &str) -> bool {
        self.latest
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&(strategy.as_str().to_owned(), key.to_owned()))
    }
}

fn strategy_profile_snapshot(
    snapshot: &serde_json::Value,
    strategy: Strategy,
) -> serde_json::Value {
    let name = strategy.as_str();
    let mut variants = snapshot["scout_parameter_variants"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|variant| variant["strategy"].as_str() == Some(name))
        .cloned()
        .collect::<Vec<_>>();
    variants.sort_by(|left, right| {
        right["promoted"]
            .as_bool()
            .cmp(&left["promoted"].as_bool())
            .then_with(|| right["rejected"].as_bool().cmp(&left["rejected"].as_bool()))
            .then_with(|| {
                right["sample_count"]
                    .as_u64()
                    .cmp(&left["sample_count"].as_u64())
            })
            .then_with(|| left["symbol"].as_str().cmp(&right["symbol"].as_str()))
            .then_with(|| left["parameter"].as_str().cmp(&right["parameter"].as_str()))
    });
    variants.truncate(MAX_WORKER_SCOUT_VARIANTS);
    let walk_forward_evidence = snapshot["walk_forward_evidence"]
        .as_object()
        .into_iter()
        .flat_map(|evidence| evidence.iter())
        .filter(|(key, _)| {
            key.rsplit_once('|')
                .is_some_and(|(_, strategy_name)| strategy_name == name)
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    serde_json::json!({
        "profiles": snapshot["profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|profile| profile["strategy"].as_str() == Some(name))
            .cloned()
            .collect::<Vec<_>>(),
        "scout_parameter_variants": variants,
        "walk_forward_evidence": walk_forward_evidence,
    })
}

fn trim_autotrader_variant_snapshot(snapshot: &mut strategy::LearnedProfileSnapshot) {
    snapshot
        .scout_parameter_variants
        .retain(|variant| strategy::is_autotrader_symbol_allowed(&variant.symbol));
    snapshot.scout_parameter_variants.sort_by(|left, right| {
        right
            .promoted
            .cmp(&left.promoted)
            .then_with(|| right.rejected.cmp(&left.rejected))
            .then_with(|| right.sample_count.cmp(&left.sample_count))
            .then_with(|| left.symbol.cmp(&right.symbol))
            .then_with(|| left.strategy.cmp(&right.strategy))
            .then_with(|| left.parameter.cmp(&right.parameter))
            .then_with(|| left.candidate_value.total_cmp(&right.candidate_value))
    });
    snapshot
        .scout_parameter_variants
        .truncate(MAX_AUTOTRADER_SCOUT_VARIANTS);
}

fn trim_autotrader_snapshot(
    snapshot: &mut strategy::LearnedProfileSnapshot,
    allowed_profile_ids: &std::collections::BTreeSet<String>,
) {
    snapshot
        .profiles
        .retain(|profile| strategy::is_autotrader_symbol_allowed(&profile.symbol));
    snapshot.registered_profile_ids = allowed_profile_ids.clone();
    snapshot.walk_forward_evidence.retain(|key, _| {
        let identity = key.split('|').next().unwrap_or_default();
        strategy::is_autotrader_symbol_allowed(identity) || allowed_profile_ids.contains(identity)
    });
    snapshot.recent_profile_context_returns.retain(|key, _| {
        key.split('|')
            .next()
            .is_some_and(|profile_id| allowed_profile_ids.contains(profile_id))
    });
    snapshot.missed_opportunity_bands.retain(|band| {
        band.symbol
            .as_deref()
            .is_none_or(strategy::is_autotrader_symbol_allowed)
    });
    trim_autotrader_variant_snapshot(snapshot);
}

fn snapshot_section_sizes(snapshot: &strategy::LearnedProfileSnapshot) -> String {
    let sizes = [
        (
            "profiles",
            serde_json::to_vec(&snapshot.profiles).map_or(0, |v| v.len()),
        ),
        (
            "registered_profile_ids",
            serde_json::to_vec(&snapshot.registered_profile_ids).map_or(0, |v| v.len()),
        ),
        (
            "walk_forward_evidence",
            serde_json::to_vec(&snapshot.walk_forward_evidence).map_or(0, |v| v.len()),
        ),
        (
            "recent_profile_context_returns",
            serde_json::to_vec(&snapshot.recent_profile_context_returns).map_or(0, |v| v.len()),
        ),
        (
            "context_learning",
            serde_json::to_vec(&snapshot.context_learning).map_or(0, |v| v.len()),
        ),
        (
            "context_thresholds",
            serde_json::to_vec(&snapshot.context_thresholds).map_or(0, |v| v.len()),
        ),
        (
            "missed_opportunity_bands",
            serde_json::to_vec(&snapshot.missed_opportunity_bands).map_or(0, |v| v.len()),
        ),
        (
            "execution_learning",
            serde_json::to_vec(&snapshot.execution_learning).map_or(0, |v| v.len()),
        ),
        (
            "scout_parameter_variants",
            serde_json::to_vec(&snapshot.scout_parameter_variants).map_or(0, |v| v.len()),
        ),
    ];
    sizes
        .into_iter()
        .map(|(name, bytes)| format!("{name}={bytes}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn merge_profile_snapshots(
    previous: Option<&serde_json::Value>,
    incoming: serde_json::Value,
) -> serde_json::Value {
    let mut profiles = BTreeMap::<String, serde_json::Value>::new();
    let mut variants = BTreeMap::<String, serde_json::Value>::new();
    for snapshot in previous.into_iter().chain(std::iter::once(&incoming)) {
        for profile in snapshot["profiles"].as_array().into_iter().flatten() {
            let key = format!(
                "{}:{}:{}",
                profile["symbol"].as_str().unwrap_or_default(),
                profile["strategy"].as_str().unwrap_or_default(),
                profile["profile_id"].as_str().unwrap_or_default()
            );
            profiles.insert(key, profile.clone());
        }
        for variant in snapshot["scout_parameter_variants"]
            .as_array()
            .into_iter()
            .flatten()
        {
            variants.insert(
                serde_json::to_string(variant).unwrap_or_default(),
                variant.clone(),
            );
        }
    }
    serde_json::json!({
        "profiles": profiles.into_values().collect::<Vec<_>>(),
        "scout_parameter_variants": variants.into_values().collect::<Vec<_>>(),
    })
}

fn scout_cache_key(update: &ScoutDataUpdate) -> Option<String> {
    match update {
        ScoutDataUpdate::Profiles(_) => Some("profiles".into()),
        ScoutDataUpdate::Feedback(_) => Some("feedback".into()),
        ScoutDataUpdate::Configuration(_) => Some("configuration".into()),
        ScoutDataUpdate::HistoricalReplay(_) => Some("historical_replay".into()),
        ScoutDataUpdate::Assignment(assignment) => {
            Some(format!("assignment:{}", assignment.assignment_id))
        }
    }
}

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
    auto_trader_rsocket_url: String,
    replay_gate: Arc<AsyncMutex<()>>,
    scout_updates: Arc<ScoutUpdateHub>,
    recent_scout_activity: Arc<AsyncMutex<Vec<serde_json::Value>>>,
}

impl ScoutCoordinator {
    pub fn scout_update_queue_sender(&self) -> broadcast::Sender<ScoutStreamEnvelope> {
        self.scout_updates.sender.clone()
    }

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
            auto_trader_rsocket_url: std::env::var("AUTO_TRADER_RSOCKET_URL")
                .unwrap_or_else(|_| "ws://auto-trader:10006".into()),
            replay_gate: Arc::new(AsyncMutex::new(())),
            scout_updates: ScoutUpdateHub::new(),
            recent_scout_activity: Arc::new(AsyncMutex::new(Vec::new())),
        })
    }

    pub async fn serve(self, address: String) -> anyhow::Result<()> {
        let responder = CoordinatorResponder(self);
        let replay_coordinator = responder.0.clone();
        let snapshot_publisher = responder.0.clone();
        tokio::spawn(async move {
            let mut retry = tokio::time::interval(PROFILE_SNAPSHOT_PUBLISH_INTERVAL);
            retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut last_publish_at: Option<tokio::time::Instant> = None;
            let mut last_trader_snapshot_hash = None;
            let mut last_trader_sync_at: Option<tokio::time::Instant> = None;
            loop {
                tokio::select! {
                    _ = snapshot_publisher.state.state_updates.notified() => {
                        // Coalesce bursts of outcomes and profile writes into
                        // one state push instead of serializing a snapshot per event.
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    _ = retry.tick() => {}
                }
                if last_publish_at
                    .is_some_and(|last| last.elapsed() < PROFILE_SNAPSHOT_PUBLISH_INTERVAL)
                {
                    continue;
                }
                let mut snapshot = crate::profiles::get_decision_snapshot(axum::extract::State(
                    snapshot_publisher.state.clone(),
                ))
                .await
                .0;
                let scout_snapshot =
                    serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null);
                for strategy in Strategy::all() {
                    snapshot_publisher.scout_updates.publish(
                        strategy,
                        ScoutDataUpdate::Profiles(strategy_profile_snapshot(
                            &scout_snapshot,
                            strategy,
                        )),
                    );
                }
                let unfiltered_bytes = serde_json::to_vec(&snapshot).map_or(0, |bytes| bytes.len());
                let allowed_profile_ids = {
                    let state = snapshot_publisher.state.state.lock().await;
                    state
                        .variants
                        .values()
                        .filter(|variant| {
                            strategy::is_autotrader_symbol_allowed(&variant.profile.symbol)
                        })
                        .map(|variant| variant.profile.profile_id.clone())
                        .collect::<std::collections::BTreeSet<_>>()
                };
                trim_autotrader_snapshot(&mut snapshot, &allowed_profile_ids);
                let encoded = serde_json::to_vec(&snapshot).unwrap_or_default();
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(&encoded, &mut hasher);
                let snapshot_hash = std::hash::Hasher::finish(&hasher);
                let trader_sync_due = last_trader_sync_at
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(5 * 60));
                if last_trader_snapshot_hash != Some(snapshot_hash) || trader_sync_due {
                    eprintln!(
                        "Publishing Learning decision snapshot to AutoTrader ({} bytes after symbol filtering, {} before; {})",
                        encoded.len(),
                        unfiltered_bytes,
                        snapshot_section_sizes(&snapshot)
                    );
                    match strategy::request_learning_snapshot_push(
                        &snapshot_publisher.auto_trader_rsocket_url,
                        &snapshot,
                    )
                    .await
                    {
                        Ok(()) => {
                            last_trader_snapshot_hash = Some(snapshot_hash);
                            last_trader_sync_at = Some(tokio::time::Instant::now());
                        }
                        Err(error) => {
                            eprintln!("Learning snapshot push to AutoTrader failed: {error:#}")
                        }
                    }
                }
                last_publish_at = Some(tokio::time::Instant::now());
            }
        });
        let configuration_publisher = responder.0.clone();
        tokio::spawn(async move {
            loop {
                let pending = configuration_publisher
                    .worker_endpoints
                    .iter()
                    .filter_map(|(name, endpoint)| {
                        let strategy = Strategy::from_name(name)?;
                        (!configuration_publisher
                            .scout_updates
                            .has_cached(strategy, "configuration"))
                        .then(|| (strategy, endpoint.clone()))
                    })
                    .collect::<Vec<_>>();
                if pending.is_empty() {
                    break;
                }
                let responses = futures_util::future::join_all(pending.into_iter().map(
                    |(strategy, endpoint)| {
                        let coordinator = configuration_publisher.clone();
                        async move {
                            let command = ScoutWorkerRouteCommand {
                                request_id: format!(
                                    "learning-config-bootstrap:{}:{}",
                                    strategy.as_str(),
                                    crate::now_ms()
                                ),
                                strategy,
                                method: ScoutWorkerHttpMethod::Get,
                                path: "/configuration".into(),
                                payload: serde_json::Value::Null,
                            };
                            (
                                strategy,
                                coordinator.dispatch_route(&endpoint, command).await,
                            )
                        }
                    },
                ))
                .await;
                for (strategy, result) in responses {
                    if let Ok(result) = result {
                        if (200..300).contains(&result.status) {
                            configuration_publisher
                                .scout_updates
                                .publish(strategy, ScoutDataUpdate::Configuration(result.body));
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
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
            if !strategy::is_autotrader_symbol_allowed(symbol) {
                continue;
            }
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
                self.worker_endpoints
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
        for (assignment, evidence) in assignments {
            let expected_id = assignment.assignment_id.clone();
            let expected_strategy = assignment.strategy;
            let (sender, receiver) = oneshot::channel();
            self.scout_updates
                .assignments
                .lock()
                .await
                .insert(expected_id.clone(), sender);
            let sequence = self
                .scout_updates
                .publish(expected_strategy, ScoutDataUpdate::Assignment(assignment));
            if self.scout_updates.sender.receiver_count() == 0 {
                self.scout_updates
                    .assignments
                    .lock()
                    .await
                    .remove(&expected_id);
                bail!(
                    "no Scout stream is subscribed for {}",
                    expected_strategy.as_str()
                );
            }
            let result = tokio::time::timeout(self.worker_timeout, receiver)
                .await
                .with_context(|| {
                    format!(
                        "Scout worker {} assignment stream timed out (sequence {sequence})",
                        expected_strategy.as_str()
                    )
                })?
                .with_context(|| {
                    format!(
                        "Scout worker {} assignment result channel closed",
                        expected_strategy.as_str()
                    )
                })?;
            let mut result = result.map_err(|error| {
                anyhow::anyhow!(
                    "Scout worker {} assignment failed: {error}",
                    expected_strategy.as_str()
                )
            })?;
            if result.assignment_id != expected_id || result.strategy != expected_strategy {
                bail!("Scout worker returned a result for a different assignment");
            }
            result.walk_forward_evidence = evidence;
            results.push(result);
        }
        let completed_at_ms = crate::now_ms();
        let mut activity = self.recent_scout_activity.lock().await;
        for result in &results {
            let mut row = serde_json::to_value(result)?;
            if let Some(object) = row.as_object_mut() {
                object.insert(
                    "request_id".into(),
                    serde_json::json!(request.request_id.clone()),
                );
                object.insert("evaluated_at_ms".into(), serde_json::json!(completed_at_ms));
                object.insert("mode".into(), serde_json::to_value(request.mode)?);
                let evidence_count = result
                    .walk_forward_evidence
                    .as_ref()
                    .map_or(0, |e| e.sample_count);
                object.insert(
                    "rank_basis".into(),
                    serde_json::json!("decision_then_prior_scout_samples"),
                );
                object.insert(
                    "evidence_sample_count".into(),
                    serde_json::json!(evidence_count),
                );
            }
            activity.push(row);
        }
        activity.sort_by(|left, right| {
            right["evaluated_at_ms"]
                .as_u64()
                .cmp(&left["evaluated_at_ms"].as_u64())
        });
        activity.truncate(500);
        // Rank within each request: actionable decisions first, then profiles
        // with more prior Scout evidence. This is a transparency ordering only.
        let request_ids = results
            .iter()
            .map(|_| request.request_id.as_str())
            .collect::<BTreeSet<_>>();
        for request_id in request_ids {
            let mut batch = activity
                .iter_mut()
                .filter(|row| row["request_id"].as_str() == Some(request_id))
                .collect::<Vec<_>>();
            batch.sort_by(|left, right| {
                scout_decision_rank(&left["decision"])
                    .cmp(&scout_decision_rank(&right["decision"]))
                    .then_with(|| {
                        right["evidence_sample_count"]
                            .as_u64()
                            .cmp(&left["evidence_sample_count"].as_u64())
                    })
                    .then_with(|| {
                        left["assignment_id"]
                            .as_str()
                            .cmp(&right["assignment_id"].as_str())
                    })
            });
            for (index, row) in batch.into_iter().enumerate() {
                row["rank"] = serde_json::json!(index + 1);
            }
        }
        drop(activity);
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
        anyhow::ensure!(
            self.worker_endpoints
                .contains_key(request.strategy.as_str()),
            "no RSocket worker configured for {}",
            request.strategy.as_str()
        );
        let expected_job_id = request.job_id.clone();
        let expected_strategy = request.strategy;
        let expected_symbol = request.symbol.clone();
        let expected_profile_id = request.profile_id.clone();
        let (sender, receiver) = oneshot::channel();
        self.scout_updates
            .replays
            .lock()
            .await
            .insert(expected_job_id.clone(), sender);
        self.scout_updates.publish(
            expected_strategy,
            ScoutDataUpdate::HistoricalReplay(request.clone()),
        );
        let response = tokio::time::timeout(self.replay_worker_timeout, receiver).await;
        self.scout_updates
            .replays
            .lock()
            .await
            .remove(&expected_job_id);
        let response = response
            .context("strategy replay worker timed out")?
            .context("strategy replay result channel closed")?
            .map_err(|error| anyhow::anyhow!("strategy replay worker failed: {error}"))?;
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
        // Insights only needs the coordinator's configured strategy names.
        // Fetching /configuration from every worker serially made this
        // lightweight discovery request fail whenever any one worker was
        // slow or restarting.
        if command.strategy.is_none()
            && command.method == ScoutWorkerHttpMethod::Get
            && command.path.split('?').next() == Some("/configuration")
        {
            let strategies = self.worker_endpoints.keys().collect::<Vec<_>>();
            return Ok(ScoutCoordinatorRouteResult {
                request_id: command.request_id,
                status: 200,
                body: serde_json::json!({"strategies": strategies}),
            });
        }
        let targets = self.route_targets(&command).await?;
        if targets.is_empty() {
            return Ok(ScoutCoordinatorRouteResult {
                request_id: command.request_id,
                status: 204,
                body: serde_json::Value::Null,
            });
        }
        let replay_route = matches!(
            command.path.split('?').next(),
            Some("/replay/outcomes" | "/replay/searches")
        );
        if replay_route {
            let coordinator = self.clone();
            let results = futures_util::future::join_all(targets.into_iter().map(|strategy| {
                let coordinator = coordinator.clone();
                let endpoint = self.worker_endpoints.get(strategy.as_str()).cloned();
                let payload = command.payload.clone();
                let path = command.path.clone();
                let method = command.method;
                let request_id = command.request_id.clone();
                async move {
                    let worker_request_id = format!("{request_id}:{}", strategy.as_str());
                    let result = match endpoint {
                        Some(endpoint) => {
                            coordinator
                                .dispatch_route_with_timeout(
                                    &endpoint,
                                    ScoutWorkerRouteCommand {
                                        request_id: worker_request_id.clone(),
                                        strategy,
                                        method,
                                        path,
                                        payload,
                                    },
                                    coordinator.replay_worker_timeout,
                                )
                                .await
                        }
                        None => Err(anyhow::anyhow!(
                            "no RSocket worker configured for {}",
                            strategy.as_str()
                        )),
                    };
                    result.unwrap_or_else(|error| ScoutWorkerRouteResult {
                        request_id: worker_request_id,
                        strategy,
                        status: 503,
                        body: serde_json::json!({"error": error.to_string()}),
                    })
                }
            }))
            .await;
            return Ok(aggregate_route_results(command, results));
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
            let result = self.dispatch_route(endpoint, worker_command).await?;
            if command.path == "/configuration" {
                self.scout_updates.publish(
                    strategy,
                    ScoutDataUpdate::Configuration(result.body.clone()),
                );
            }
            results.push(result);
        }
        Ok(aggregate_route_results(command, results))
    }

    async fn accept_scout_feedback(
        &self,
        event: strategy::FeedbackEvent,
    ) -> anyhow::Result<ScoutFeedbackAcknowledgement> {
        let event_id = crate::event_parts(&event).0.to_owned();
        match crate::accept_feedback(
            axum::extract::State(self.state.clone()),
            axum::Json(event.clone()),
        )
        .await
        {
            Ok(result) => {
                if let Some(strategy) = Strategy::from_name(crate::event_parts(&event).2) {
                    self.scout_updates
                        .publish(strategy, ScoutDataUpdate::Feedback(event));
                }
                Ok(ScoutFeedbackAcknowledgement {
                    event_id,
                    accepted: result.0.accepted,
                    duplicate: result.0.duplicate,
                    status: 200,
                    detail: None,
                })
            }
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
            .filter(|profile| strategy::is_autotrader_symbol_allowed(&profile.symbol))
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(serde_json::json!({"profiles": profiles}))
    }

    async fn profile_snapshot_for(
        &self,
        symbols: Vec<String>,
        strategies: Vec<String>,
    ) -> anyhow::Result<serde_json::Value> {
        if !self.enabled {
            return Ok(serde_json::json!({"profiles": [], "scout_parameter_variants": []}));
        }
        anyhow::ensure!(
            symbols.len() <= 512 && strategies.len() <= Strategy::all().count(),
            "Learning profile snapshot request exceeds its bounds"
        );
        let state = self.state.state.lock().await;
        let mut profiles = Vec::new();
        let mut profile_symbols = std::collections::BTreeSet::new();
        for symbol in symbols
            .into_iter()
            .filter(|symbol| strategy::is_autotrader_symbol_allowed(symbol))
        {
            profile_symbols.insert(symbol.clone());
            for strategy_name in &strategies {
                let Some(strategy) = Strategy::from_name(strategy_name) else {
                    continue;
                };
                let request = serde_json::json!({
                    "symbol": symbol,
                    "strategy": strategy.as_str(),
                    "strategy_candidate": false,
                });
                profiles.extend(
                    selected_profiles(&state, &symbol, &request)?
                        .into_iter()
                        .map(serde_json::to_value)
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
        }
        let scout_parameter_variants = crate::profiles::scout_parameter_variant_snapshots(&state)
            .into_iter()
            .filter(|variant| {
                profile_symbols.contains(&variant.symbol)
                    && strategies
                        .iter()
                        .any(|strategy| strategy == &variant.strategy)
            })
            .collect::<Vec<_>>();
        let walk_forward_evidence =
            crate::profiles::walk_forward_priority_snapshot(&state, crate::now_ms())
                .into_iter()
                .filter(|(key, _)| {
                    profile_symbols.iter().any(|symbol| {
                        strategies
                            .iter()
                            .any(|strategy| key == &format!("{symbol}|{strategy}"))
                    })
                })
                .collect::<BTreeMap<_, _>>();
        Ok(
            serde_json::json!({"profiles": profiles, "scout_parameter_variants": scout_parameter_variants, "walk_forward_evidence": walk_forward_evidence}),
        )
    }

    async fn dispatch_route(
        &self,
        endpoint: &str,
        command: ScoutWorkerRouteCommand,
    ) -> anyhow::Result<ScoutWorkerRouteResult> {
        self.dispatch_route_with_timeout(endpoint, command, self.worker_timeout)
            .await
    }

    async fn dispatch_route_with_timeout(
        &self,
        endpoint: &str,
        command: ScoutWorkerRouteCommand,
        timeout: Duration,
    ) -> anyhow::Result<ScoutWorkerRouteResult> {
        let client = tokio::time::timeout(timeout, pooled_scout_worker_client(endpoint))
            .await
            .map_err(|_| anyhow::anyhow!("Scout worker connection timed out"))??;
        let response = match tokio::time::timeout(
            timeout,
            client.request_response(
                Payload::builder()
                    .set_data(serde_json::to_vec(&ScoutWorkerRequest::Route(
                        command.clone(),
                    ))?)
                    .build(),
            ),
        )
        .await
        {
            Ok(Ok(Some(response))) => response,
            Ok(Ok(None)) => {
                invalidate_scout_worker_client(endpoint, &client);
                bail!("Scout worker route returned an empty response");
            }
            Ok(Err(error)) => {
                invalidate_scout_worker_client(endpoint, &client);
                return Err(error).context("Scout worker route request failed");
            }
            Err(_) => {
                invalidate_scout_worker_client(endpoint, &client);
                bail!("Scout worker route request timed out");
            }
        };
        let response: ScoutWorkerResponse = serde_json::from_slice(
            response
                .data()
                .context("Scout worker route response had no data")?,
        )
        .context("Scout worker route response was invalid")?;
        let ScoutWorkerResponse::Route(result) = response;
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
        if path == "/outcomes"
            || path == "/replay/outcomes"
            || path == "/replay/searches"
            || path == "/status"
            || path == "/configuration"
        {
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
        if path == "/scout-activity" && command.method == ScoutWorkerHttpMethod::Get {
            return Ok(ScoutCoordinatorRouteResult {
                request_id: command.request_id,
                status: 200,
                body: serde_json::json!({"items": self.recent_scout_activity.lock().await.clone()}),
            });
        }
        if path == "/scout-replay/paired-returns" && command.method == ScoutWorkerHttpMethod::Get {
            return self.scout_paired_return_stream(&command.request_id).await;
        }
        if path == "/scout-replay/runs" && command.method == ScoutWorkerHttpMethod::Get {
            let state = self.state.state.lock().await;
            let mut runs = state
                .scout_replay_runs
                .values()
                .cloned()
                .collect::<Vec<_>>();
            runs.sort_by_key(|run| run["createdAtMs"].as_u64().unwrap_or_default());
            return Ok(ScoutCoordinatorRouteResult {
                request_id: command.request_id,
                status: 200,
                body: serde_json::json!({"runs": runs}),
            });
        }
        let feedback = (path == "/feedback")
            .then(|| {
                serde_json::from_value::<strategy::FeedbackEvent>(command.payload.clone()).ok()
            })
            .flatten();
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
        if (200..300).contains(&status) {
            if let Some(event) = feedback {
                if let Some(strategy) = Strategy::from_name(crate::event_parts(&event).2) {
                    self.scout_updates
                        .publish(strategy, ScoutDataUpdate::Feedback(event));
                }
            }
        }
        Ok(ScoutCoordinatorRouteResult {
            request_id: command.request_id,
            status,
            body,
        })
    }

    async fn scout_paired_return_stream(
        &self,
        request_id: &str,
    ) -> anyhow::Result<ScoutCoordinatorRouteResult> {
        let outcomes = self
            .route_worker_command(ScoutCoordinatorRouteCommand {
                request_id: format!("{request_id}:outcomes"),
                strategy: None,
                method: ScoutWorkerHttpMethod::Get,
                path: "/replay/outcomes".into(),
                payload: serde_json::Value::Null,
            })
            .await?;
        if !(200..300).contains(&outcomes.status) {
            return Ok(ScoutCoordinatorRouteResult {
                request_id: request_id.to_owned(),
                status: outcomes.status,
                body: outcomes.body,
            });
        }
        let searches = self
            .route_worker_command(ScoutCoordinatorRouteCommand {
                request_id: format!("{request_id}:searches"),
                strategy: None,
                method: ScoutWorkerHttpMethod::Get,
                path: "/replay/searches".into(),
                payload: serde_json::Value::Null,
            })
            .await?;
        if !(200..300).contains(&searches.status) {
            return Ok(ScoutCoordinatorRouteResult {
                request_id: request_id.to_owned(),
                status: searches.status,
                body: searches.body,
            });
        }
        let replay =
            crate::scout_replay::build_paired_return_stream(&outcomes.body, &searches.body);
        self.persist_scout_replay_run(&replay).await?;
        Ok(ScoutCoordinatorRouteResult {
            request_id: request_id.to_owned(),
            status: 200,
            body: replay,
        })
    }

    async fn persist_scout_replay_run(&self, replay: &serde_json::Value) -> anyhow::Result<()> {
        let Some(run_id) = replay["runId"].as_str() else {
            anyhow::bail!("Scout replay response has no stable run ID");
        };
        let mut state = self.state.state.lock().await;
        if state.scout_replay_runs.contains_key(run_id) {
            return Ok(());
        }
        let created_at_ms = crate::now_ms();
        state.scout_replay_runs.insert(
            run_id.to_owned(),
            serde_json::json!({
                "runId": run_id,
                "datasetHash": replay["datasetHash"],
                "configVersion": replay["configVersion"],
                "configHash": replay["configHash"],
                "createdAtMs": created_at_ms,
                "status": replay["status"],
                "coverage": replay["coverage"],
                "dependenceAwareInference": replay["dependenceAwareInference"],
                "rollingOrigin": replay["rollingOrigin"],
            }),
        );
        const MAX_PERSISTED_SCOUT_REPLAY_RUNS: usize = 32;
        while state.scout_replay_runs.len() > MAX_PERSISTED_SCOUT_REPLAY_RUNS {
            let Some(oldest) = state
                .scout_replay_runs
                .iter()
                .min_by_key(|(_, run)| run["createdAtMs"].as_u64().unwrap_or_default())
                .map(|(run_id, _)| run_id.clone())
            else {
                break;
            };
            state.scout_replay_runs.remove(&oldest);
        }
        crate::persist(&self.state.path, &state).map_err(|error| anyhow::anyhow!(error.1))?;
        Ok(())
    }
}

fn learning_route_allowed(path: &str, method: ScoutWorkerHttpMethod) -> bool {
    matches!(
        (method, path),
        (ScoutWorkerHttpMethod::Get, "/profiles")
            | (ScoutWorkerHttpMethod::Get, "/scout-activity")
            | (ScoutWorkerHttpMethod::Get, "/decision-snapshot")
            | (ScoutWorkerHttpMethod::Get, "/summary")
            | (ScoutWorkerHttpMethod::Get, "/scout-replay/paired-returns")
            | (ScoutWorkerHttpMethod::Get, "/scout-replay/runs")
            | (ScoutWorkerHttpMethod::Post, "/profiles")
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
    let path = command.path.split('?').next().unwrap_or(&command.path);
    if matches!(path, "/replay/outcomes" | "/replay/searches") {
        let errors = results
            .iter()
            .filter(|result| result.status >= 400)
            .map(|result| {
                serde_json::json!({
                    "strategy": result.strategy.as_str(),
                    "status": result.status,
                    "error": result.body["error"].as_str().unwrap_or("Scout worker route failed"),
                })
            })
            .collect::<Vec<_>>();
        let successful = results
            .into_iter()
            .filter(|result| result.status < 400)
            .collect::<Vec<_>>();
        if successful.is_empty() {
            return ScoutCoordinatorRouteResult {
                request_id: command.request_id,
                status: 503,
                body: serde_json::json!({"error": "No Scout workers returned replay history", "workerErrors": errors}),
            };
        }
        let mut aggregated = aggregate_route_results_without_replay_failures(command, successful);
        if !errors.is_empty() {
            aggregated.body["coverage"]["complete"] = serde_json::Value::Bool(false);
            aggregated.body["coverage"]["status"] = serde_json::json!("incomplete");
            aggregated.body["workerErrors"] = serde_json::Value::Array(errors);
        }
        return aggregated;
    }
    aggregate_route_results_without_replay_failures(command, results)
}

fn aggregate_route_results_without_replay_failures(
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
        "/replay/outcomes" => aggregate_replay_outcomes(&bodies),
        "/replay/searches" => aggregate_replay_searches(&bodies),
        "/status" => aggregate_status(&bodies),
        _ => serde_json::json!({"by_strategy": bodies}),
    };
    ScoutCoordinatorRouteResult {
        request_id: command.request_id,
        status: 200,
        body,
    }
}

fn aggregate_replay_outcomes(bodies: &BTreeMap<String, serde_json::Value>) -> serde_json::Value {
    let mut outcomes = Vec::new();
    let mut total_durable_records = 0_u64;
    let mut returned_records = 0_u64;
    let mut malformed_records = 0_u64;
    let mut runtime_hydration_limit = 0_u64;
    let mut completed_before_origin_records = 0_u64;
    let mut at_or_after_origin_records = 0_u64;
    let mut missing_regime_records = 0_u64;
    let mut fee_rate_and_net_return_reported_records = 0_u64;
    let mut explicit_cost_breakdown_records = 0_u64;
    let mut explicit_cost_breakdown = true;
    let mut open_scout_instances = 0_u64;
    let mut expired_instances_without_outcome = 0_u64;
    let mut has_instance_coverage = false;
    let mut origin_at_ms = None;
    let mut origin_was_requested = false;
    let mut complete = true;
    let mut has_more = false;
    let mut page = 0_u64;
    let mut page_size = 0_u64;
    let mut first_timestamp_ms: Option<u64> = None;
    let mut last_timestamp_ms: Option<u64> = None;
    for (strategy, body) in bodies {
        let coverage = &body["coverage"];
        total_durable_records += coverage["totalDurableRecords"].as_u64().unwrap_or_default();
        returned_records += coverage["returnedRecords"].as_u64().unwrap_or_default();
        malformed_records += coverage["malformedRecords"].as_u64().unwrap_or_default();
        runtime_hydration_limit = runtime_hydration_limit.max(
            coverage["runtimeHydrationLimit"]
                .as_u64()
                .unwrap_or_default(),
        );
        origin_was_requested |= coverage["originAtMs"].is_u64();
        origin_at_ms = origin_at_ms.or_else(|| coverage["originAtMs"].as_u64());
        completed_before_origin_records += coverage["completedBeforeOriginRecords"]
            .as_u64()
            .unwrap_or_default();
        at_or_after_origin_records += coverage["atOrAfterOriginRecords"]
            .as_u64()
            .unwrap_or_default();
        missing_regime_records += coverage["missingRegimeRecords"]
            .as_u64()
            .unwrap_or_default();
        fee_rate_and_net_return_reported_records += coverage["feeRateAndNetReturnReportedRecords"]
            .as_u64()
            .unwrap_or_default();
        explicit_cost_breakdown_records += coverage["explicitCostBreakdownRecords"]
            .as_u64()
            .unwrap_or_default();
        explicit_cost_breakdown &= coverage["explicitCostBreakdown"].as_bool().unwrap_or(false);
        if let Some(count) = coverage["openScoutInstances"].as_u64() {
            open_scout_instances += count;
            has_instance_coverage = true;
        }
        if let Some(count) = coverage["expiredInstancesWithoutOutcome"].as_u64() {
            expired_instances_without_outcome += count;
            has_instance_coverage = true;
        }
        complete &= coverage["complete"].as_bool().unwrap_or(false);
        has_more |= coverage["hasMore"].as_bool().unwrap_or(false);
        page = coverage["page"].as_u64().unwrap_or(page);
        page_size = coverage["pageSize"].as_u64().unwrap_or(page_size);
        if let Some(timestamp) = coverage["firstEvaluatedTimestampMs"].as_u64() {
            first_timestamp_ms =
                Some(first_timestamp_ms.map_or(timestamp, |value| value.min(timestamp)));
        }
        if let Some(timestamp) = coverage["lastEvaluatedTimestampMs"].as_u64() {
            last_timestamp_ms =
                Some(last_timestamp_ms.map_or(timestamp, |value| value.max(timestamp)));
        }
        if let Some(rows) = body["outcomes"].as_array() {
            outcomes.extend(rows.iter().cloned().map(|mut row| {
                if let Some(outcome_id) = row["outcome_id"].as_str() {
                    row["outcome_id"] = serde_json::json!(format!("{strategy}|{outcome_id}"));
                }
                (strategy, row)
            }));
        }
    }
    outcomes.sort_by(|(left_strategy, left), (right_strategy, right)| {
        left["evaluated_timestamp_ms"]
            .as_u64()
            .unwrap_or_default()
            .cmp(&right["evaluated_timestamp_ms"].as_u64().unwrap_or_default())
            .then_with(|| left_strategy.cmp(right_strategy))
            .then_with(|| {
                left["decision_id"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["decision_id"].as_str().unwrap_or_default())
            })
    });
    let outcomes = outcomes
        .into_iter()
        .map(|(_, outcome)| outcome)
        .collect::<Vec<_>>();
    let mut coverage = serde_json::json!({
        "status": if complete { "complete" } else { "incomplete" },
        "complete": complete,
        "totalDurableRecords": total_durable_records,
        "returnedRecords": returned_records,
        "page": page,
        "pageSize": page_size,
        "hasMore": has_more,
        "malformedRecords": malformed_records,
        "firstEvaluatedTimestampMs": first_timestamp_ms,
        "lastEvaluatedTimestampMs": last_timestamp_ms,
        "runtimeHydrationLimit": runtime_hydration_limit,
        "originAtMs": if origin_was_requested { serde_json::json!(origin_at_ms) } else { serde_json::Value::Null },
        "completedBeforeOriginRecords": if origin_was_requested { serde_json::json!(completed_before_origin_records) } else { serde_json::Value::Null },
        "atOrAfterOriginRecords": if origin_was_requested { serde_json::json!(at_or_after_origin_records) } else { serde_json::Value::Null },
        "missingRegimeRecords": missing_regime_records,
        "feeRateAndNetReturnReportedRecords": fee_rate_and_net_return_reported_records,
        "explicitCostBreakdownRecords": explicit_cost_breakdown_records,
        "explicitCostBreakdown": explicit_cost_breakdown,
    });
    if has_instance_coverage {
        coverage["openScoutInstances"] = serde_json::json!(open_scout_instances);
        coverage["expiredInstancesWithoutOutcome"] =
            serde_json::json!(expired_instances_without_outcome);
    }
    serde_json::json!({
        "datasetVersion": 2,
        "source": "scout_records.outcomes",
        "ordering": "evaluated_timestamp_ms_asc_then_strategy_then_decision_id",
        "coverage": coverage,
        "outcomes": outcomes,
    })
}

fn aggregate_replay_searches(bodies: &BTreeMap<String, serde_json::Value>) -> serde_json::Value {
    let mut searches = Vec::new();
    let mut total_durable_records = 0_u64;
    let mut returned_records = 0_u64;
    let mut malformed_records = 0_u64;
    let mut runtime_cache_limit = 0_u64;
    let mut complete = true;
    for (strategy, body) in bodies {
        let coverage = &body["coverage"];
        total_durable_records += coverage["totalDurableRecords"].as_u64().unwrap_or_default();
        returned_records += coverage["returnedRecords"].as_u64().unwrap_or_default();
        malformed_records += coverage["malformedRecords"].as_u64().unwrap_or_default();
        runtime_cache_limit =
            runtime_cache_limit.max(coverage["runtimeCacheLimit"].as_u64().unwrap_or_default());
        complete &= coverage["complete"].as_bool().unwrap_or(false);
        if let Some(rows) = body["searches"].as_array() {
            searches.extend(rows.iter().cloned().map(|mut row| {
                if let Some(search_id) = row["search_id"].as_str() {
                    row["search_id"] = serde_json::json!(format!("{strategy}|{search_id}"));
                }
                (strategy, row)
            }));
        }
    }
    searches.sort_by(|(left_strategy, left), (right_strategy, right)| {
        left["as_of_timestamp_ms"]
            .as_u64()
            .unwrap_or_default()
            .cmp(&right["as_of_timestamp_ms"].as_u64().unwrap_or_default())
            .then_with(|| left_strategy.cmp(right_strategy))
            .then_with(|| {
                left["search_id"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["search_id"].as_str().unwrap_or_default())
            })
    });
    let first_as_of_timestamp_ms = searches
        .first()
        .and_then(|(_, search)| search["as_of_timestamp_ms"].as_u64());
    let last_as_of_timestamp_ms = searches
        .last()
        .and_then(|(_, search)| search["as_of_timestamp_ms"].as_u64());
    let searches = searches
        .into_iter()
        .map(|(_, search)| search)
        .collect::<Vec<_>>();
    serde_json::json!({
        "datasetVersion": 1,
        "source": "scout_records.scout_search_ledger",
        "ordering": "as_of_timestamp_ms_asc_then_strategy_then_search_id",
        "coverage": {
            "status": if complete { "complete" } else { "incomplete" },
            "complete": complete,
            "totalDurableRecords": total_durable_records,
            "returnedRecords": returned_records,
            "malformedRecords": malformed_records,
            "firstAsOfTimestampMs": first_as_of_timestamp_ms,
            "lastAsOfTimestampMs": last_as_of_timestamp_ms,
            "runtimeCacheLimit": runtime_cache_limit,
        },
        "searches": searches,
    })
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

fn scout_decision_rank(decision: &serde_json::Value) -> u8 {
    match decision.as_str().unwrap_or_default() {
        "enter" => 0,
        "rebalance" => 1,
        "scout" => 2,
        "hold" => 3,
        "exit" => 4,
        "not_evaluated" => 5,
        _ => 6,
    }
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
    let mut seen_groups = BTreeSet::new();
    let mut samples = registered
        .outcomes
        .iter()
        .filter(|outcome| {
            outcome.observed_at_ms < signal_timestamp_ms
                && outcome.net_return_percent.is_finite()
                && matches!(outcome.source, crate::LearningOutcomeSource::Scout)
                && outcome
                    .evidence_group_id
                    .as_deref()
                    .is_none_or(|group| seen_groups.insert(group.to_owned()))
        })
        .map(
            |outcome| crate::walk_forward_statistics::WalkForwardSample {
                observed_at_ms: outcome.observed_at_ms,
                net_return_percent: outcome.net_return_percent,
                weight: 1.0,
            },
        )
        .collect::<Vec<_>>();
    if samples.is_empty() {
        return None;
    }
    samples.sort_by_key(|outcome| outcome.observed_at_ms);
    let last_sample_timestamp_ms = samples.last()?.observed_at_ms;
    let statistics = crate::walk_forward_statistics::summarize_walk_forward_samples(samples);
    Some(strategy::StrategyWalkForwardEvidence {
        evidence_contract_version: 2,
        estimator: strategy::StrategyWalkForwardEstimator::CompletedOutcomeClusters,
        history_coverage: strategy::StrategyWalkForwardCoverage::Partial,
        cost_coverage: strategy::StrategyWalkForwardCoverage::Unknown,
        search_family_coverage: strategy::StrategyWalkForwardCoverage::Unknown,
        point_in_time_cutoff_timestamp_ms: Some(signal_timestamp_ms),
        final_holdout_status: strategy::StrategyWalkForwardValidationStatus::NotValidated,
        oos_fold_count: 0,
        spa_p_value: None,
        sample_count: statistics.sample_count,
        positive_net_rate: Some(statistics.positive_net_rate),
        average_net_return_percent: Some(statistics.average_net_return_percent),
        last_sample_timestamp_ms,
        expectancy_ci_low_percent: statistics.expectancy_ci_low_percent,
        expectancy_ci_high_percent: statistics.expectancy_ci_high_percent,
        effective_sample_size: Some(statistics.effective_sample_size),
        window_count: statistics.window_count,
        max_drawdown_percent: statistics.max_drawdown_percent,
        downside_loss_percent: statistics.downside_loss_percent,
        evidence_scope: strategy::StrategyWalkForwardEvidenceScope::ExactProfileUnconditioned,
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

#[derive(Clone)]
struct CoordinatorResponder(ScoutCoordinator);

#[rsocket_rust::async_trait]
impl RSocket for CoordinatorResponder {
    async fn metadata_push(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("Learning Scout coordinator accepts request-response and subscribed streams")
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
            ScoutCoordinatorRequest::ProfileSnapshotFor {
                symbols,
                strategies,
            } => {
                let snapshot = self
                    .0
                    .profile_snapshot_for(symbols, strategies.clone())
                    .await?;
                for strategy in strategies
                    .iter()
                    .filter_map(|name| Strategy::from_name(name))
                {
                    self.0.scout_updates.publish(
                        strategy,
                        ScoutDataUpdate::Profiles(strategy_profile_snapshot(&snapshot, strategy)),
                    );
                }
                ScoutCoordinatorResponse::ProfileSnapshot(snapshot)
            }
            ScoutCoordinatorRequest::AssignmentResult(result) => {
                let assignment_id = result.assignment_id.clone();
                self.0
                    .scout_updates
                    .forget_assignment(result.strategy, &assignment_id);
                if let Some(waiter) = self
                    .0
                    .scout_updates
                    .assignments
                    .lock()
                    .await
                    .remove(&assignment_id)
                {
                    let _ = waiter.send(Ok(result));
                }
                ScoutCoordinatorResponse::AssignmentAcknowledged { assignment_id }
            }
            ScoutCoordinatorRequest::AssignmentFailed {
                assignment_id,
                strategy,
                detail,
            } => {
                self.0
                    .scout_updates
                    .forget_assignment(strategy, &assignment_id);
                if let Some(waiter) = self
                    .0
                    .scout_updates
                    .assignments
                    .lock()
                    .await
                    .remove(&assignment_id)
                {
                    let _ = waiter.send(Err(detail));
                }
                ScoutCoordinatorResponse::AssignmentAcknowledged { assignment_id }
            }
            ScoutCoordinatorRequest::ReplayResult(result) => {
                let job_id = result.job_id.clone();
                if let Some(waiter) = self.0.scout_updates.replays.lock().await.remove(&job_id) {
                    let _ = waiter.send(Ok(result));
                }
                ScoutCoordinatorResponse::ReplayAcknowledged { job_id }
            }
            ScoutCoordinatorRequest::ReplayFailed {
                job_id,
                strategy: _,
                detail,
            } => {
                if let Some(waiter) = self.0.scout_updates.replays.lock().await.remove(&job_id) {
                    let _ = waiter.send(Err(detail));
                }
                ScoutCoordinatorResponse::ReplayAcknowledged { job_id }
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
        req: Payload,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
        let Some(data) = req.data() else {
            return Box::pin(stream::once(async {
                Err(anyhow::anyhow!("Scout subscription is empty"))
            }));
        };
        let Ok(subscription) = serde_json::from_slice::<ScoutStreamSubscription>(data) else {
            return Box::pin(stream::once(async {
                Err(anyhow::anyhow!("invalid Scout stream subscription"))
            }));
        };
        let strategy = subscription.strategy;
        let hub = Arc::clone(&self.0.scout_updates);
        let receiver = hub.sender.subscribe();
        let initial = hub.snapshot(strategy);
        let cached = stream::iter(initial.into_iter().map(|item| {
            serde_json::to_vec(&item)
                .map(|bytes| Payload::builder().set_data(bytes).build())
                .map_err(anyhow::Error::from)
        }));
        let live = stream::unfold(
            (receiver, strategy),
            |(mut receiver, strategy)| async move {
                loop {
                    match receiver.recv().await {
                        Ok(item) if item.strategy == strategy => {
                            let payload = serde_json::to_vec(&item)
                                .map(|bytes| Payload::builder().set_data(bytes).build())
                                .map_err(anyhow::Error::from);
                            return Some((payload, (receiver, strategy)));
                        }
                        Ok(_) => continue,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            return Some((
                                Err(anyhow::anyhow!(
                                    "Scout stream lagged by {skipped} updates; reconnect to replay cached state"
                                )),
                                (receiver, strategy),
                            ));
                        }
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            },
        );
        Box::pin(cached.chain(live))
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
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex as StdMutex};
    use strategy::StrategyProfile;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn paired_replay_config_and_fold_assignment_are_persisted_by_run_id() {
        let path = std::env::temp_dir().join(format!(
            "learning-scout-replay-run-{}-{}.json",
            std::process::id(),
            crate::now_ms()
        ));
        let service = ServiceState {
            state: Arc::new(Mutex::new(LearningState::default())),
            path: path.clone(),
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let coordinator =
            ScoutCoordinator::new(service, "live-momentum=ws://127.0.0.1:1", Router::new())
                .unwrap();
        let replay = serde_json::json!({
            "runId":"scout-replay-v1-fnv1a64:test",
            "datasetHash":"fnv1a64:test",
            "configVersion":1,
            "configHash":"fnv1a64:config",
            "status":"incomplete",
            "coverage":{"matchedPairs":2},
            "rollingOrigin":{"folds":[{"foldIndex":0,"evaluationPairIds":["pair-1"]}]}
        });
        coordinator.persist_scout_replay_run(&replay).await.unwrap();
        let state = crate::load_state(&path).unwrap();
        let run = &state.scout_replay_runs["scout-replay-v1-fnv1a64:test"];
        assert_eq!(run["configHash"], "fnv1a64:config");
        assert_eq!(run["datasetHash"], "fnv1a64:test");
        assert_eq!(
            run["rollingOrigin"]["folds"][0]["evaluationPairIds"][0],
            "pair-1"
        );
        let listed = coordinator
            .local_route(ScoutCoordinatorRouteCommand {
                request_id: "list-replay-runs".into(),
                strategy: None,
                method: ScoutWorkerHttpMethod::Get,
                path: "/scout-replay/runs".into(),
                payload: serde_json::Value::Null,
            })
            .await
            .unwrap();
        assert_eq!(listed.body["runs"].as_array().unwrap().len(), 1);
        assert_eq!(
            listed.body["runs"][0]["runId"],
            "scout-replay-v1-fnv1a64:test"
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn replay_outcomes_route_targets_all_configured_workers() {
        let service = ServiceState {
            state: Arc::new(Mutex::new(LearningState::default())),
            path: PathBuf::new(),
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let coordinator = ScoutCoordinator::new(
            service,
            "live-momentum=ws://momentum:10200;dated-futures-carry=ws://carry:10200",
            Router::new(),
        )
        .unwrap();
        let command = ScoutCoordinatorRouteCommand {
            request_id: "replay-route-test".into(),
            strategy: None,
            method: ScoutWorkerHttpMethod::Get,
            path: "/replay/outcomes".into(),
            payload: serde_json::Value::Null,
        };
        let targets = coordinator.route_targets(&command).await.unwrap();
        assert_eq!(targets, vec![Strategy::DatedCarry, Strategy::LiveMomentum]);
        let search_command = ScoutCoordinatorRouteCommand {
            path: "/replay/searches".into(),
            ..command
        };
        let search_targets = coordinator.route_targets(&search_command).await.unwrap();
        assert_eq!(
            search_targets,
            vec![Strategy::DatedCarry, Strategy::LiveMomentum]
        );
    }

    #[test]
    fn paired_return_stream_is_an_allowed_learning_route() {
        assert!(learning_route_allowed(
            "/scout-replay/paired-returns",
            ScoutWorkerHttpMethod::Get
        ));
        assert!(learning_route_allowed(
            "/scout-replay/runs",
            ScoutWorkerHttpMethod::Get
        ));
    }

    #[test]
    fn replay_outcomes_aggregate_coverage_and_sort_across_workers() {
        let body = |decision_id: &str, timestamp: u64| {
            serde_json::json!({
                "datasetVersion": 1,
                "source": "scout_records.outcomes",
                "ordering": "evaluated_timestamp_ms_asc_then_record_key_asc",
                "coverage": {
                    "complete": true,
                    "totalDurableRecords": 1,
                    "returnedRecords": 1,
                    "malformedRecords": 0,
                    "runtimeHydrationLimit": 4096,
                    "originAtMs": 150,
                    "completedBeforeOriginRecords": if timestamp < 150 { 1 } else { 0 },
                    "atOrAfterOriginRecords": if timestamp < 150 { 0 } else { 1 },
                    "missingRegimeRecords": 0,
                    "feeRateAndNetReturnReportedRecords": 1,
                    "explicitCostBreakdownRecords": 1,
                    "explicitCostBreakdown": false,
                    "openScoutInstances": 2,
                    "expiredInstancesWithoutOutcome": 1
                },
                "outcomes": [{"decision_id": decision_id, "outcome_id": decision_id, "evaluated_timestamp_ms": timestamp}]
            })
        };
        let bodies = BTreeMap::from([
            ("live-momentum".to_owned(), body("later", 200)),
            ("dated-futures-carry".to_owned(), body("earlier", 100)),
        ]);
        let aggregate = aggregate_replay_outcomes(&bodies);
        assert_eq!(aggregate["datasetVersion"], 2);
        assert_eq!(aggregate["coverage"]["complete"], true);
        assert_eq!(aggregate["coverage"]["totalDurableRecords"], 2);
        assert_eq!(aggregate["coverage"]["returnedRecords"], 2);
        assert_eq!(aggregate["coverage"]["firstEvaluatedTimestampMs"], 100);
        assert_eq!(aggregate["coverage"]["lastEvaluatedTimestampMs"], 200);
        assert_eq!(aggregate["coverage"]["originAtMs"], 150);
        assert_eq!(aggregate["coverage"]["completedBeforeOriginRecords"], 1);
        assert_eq!(aggregate["coverage"]["atOrAfterOriginRecords"], 1);
        assert_eq!(aggregate["coverage"]["explicitCostBreakdownRecords"], 2);
        assert_eq!(aggregate["coverage"]["explicitCostBreakdown"], false);
        assert_eq!(aggregate["coverage"]["openScoutInstances"], 4);
        assert_eq!(aggregate["coverage"]["expiredInstancesWithoutOutcome"], 2);
        assert_eq!(aggregate["outcomes"][0]["decision_id"], "earlier");
        assert_eq!(aggregate["outcomes"][1]["decision_id"], "later");
        assert_eq!(
            aggregate["outcomes"][0]["outcome_id"],
            "dated-futures-carry|earlier"
        );
    }

    #[test]
    fn replay_searches_aggregate_complete_history_and_qualify_ids() {
        let body = |search_id: &str, as_of_timestamp_ms: u64| {
            serde_json::json!({
                "datasetVersion": 1,
                "coverage": {
                    "complete": true,
                    "totalDurableRecords": 1,
                    "returnedRecords": 1,
                    "malformedRecords": 0,
                    "runtimeCacheLimit": 32
                },
                "searches": [{"search_id": search_id, "as_of_timestamp_ms": as_of_timestamp_ms}]
            })
        };
        let bodies = BTreeMap::from([
            ("live-momentum".to_owned(), body("search-later", 200)),
            (
                "dated-futures-carry".to_owned(),
                body("search-earlier", 100),
            ),
        ]);
        let aggregate = aggregate_replay_searches(&bodies);
        assert_eq!(aggregate["datasetVersion"], 1);
        assert_eq!(aggregate["coverage"]["complete"], true);
        assert_eq!(aggregate["coverage"]["totalDurableRecords"], 2);
        assert_eq!(aggregate["coverage"]["returnedRecords"], 2);
        assert_eq!(aggregate["coverage"]["firstAsOfTimestampMs"], 100);
        assert_eq!(aggregate["coverage"]["lastAsOfTimestampMs"], 200);
        assert_eq!(
            aggregate["searches"][0]["search_id"],
            "dated-futures-carry|search-earlier"
        );
        assert_eq!(
            aggregate["searches"][1]["search_id"],
            "live-momentum|search-later"
        );
    }

    #[test]
    fn autotrader_snapshot_drops_unallowed_symbol_data_before_transport() {
        let allowed_id = "allowed-profile".to_owned();
        let excluded_id = "excluded-profile".to_owned();
        let mut snapshot = strategy::LearnedProfileSnapshot {
            profiles: vec![
                StrategyProfile {
                    profile_id: allowed_id.clone(),
                    version: 1,
                    symbol: "BTC-USDT".into(),
                    strategy: "example".into(),
                    parameters: BTreeMap::new(),
                    learning_samples: 0,
                    positive_samples: 0,
                    realized_pnl: 0.0,
                },
                StrategyProfile {
                    profile_id: excluded_id.clone(),
                    version: 1,
                    symbol: "XMSTR-USDT".into(),
                    strategy: "example".into(),
                    parameters: BTreeMap::new(),
                    learning_samples: 0,
                    positive_samples: 0,
                    realized_pnl: 0.0,
                },
            ],
            walk_forward_evidence: BTreeMap::from([
                ("BTC-USDT|example".into(), Default::default()),
                (format!("{excluded_id}|regime:trend"), Default::default()),
            ]),
            registered_profile_ids: [allowed_id.clone(), excluded_id].into(),
            strategy_size_multipliers: BTreeMap::new(),
            recent_profile_context_returns: BTreeMap::from([
                (format!("{allowed_id}|context"), Vec::new()),
                ("excluded-profile|context".into(), Vec::new()),
            ]),
            contextual_stop_loss_percents: BTreeMap::new(),
            context_learning: BTreeMap::new(),
            context_thresholds: BTreeMap::new(),
            missed_opportunity_bands: Vec::new(),
            execution_learning: BTreeMap::new(),
            scout_parameter_variants: Vec::new(),
        };

        trim_autotrader_snapshot(&mut snapshot, &[allowed_id.clone()].into());

        assert_eq!(snapshot.profiles.len(), 1);
        assert_eq!(snapshot.registered_profile_ids, [allowed_id.clone()].into());
        assert_eq!(snapshot.walk_forward_evidence.len(), 1);
        assert_eq!(snapshot.recent_profile_context_returns.len(), 1);
    }

    #[test]
    fn rsocket_allows_profile_registration_but_rejects_other_mutations() {
        assert!(learning_route_allowed(
            "/profiles",
            ScoutWorkerHttpMethod::Get,
        ));
        assert!(learning_route_allowed(
            "/profiles",
            ScoutWorkerHttpMethod::Post,
        ));
        assert!(!learning_route_allowed(
            "/profiles",
            ScoutWorkerHttpMethod::Delete,
        ));
        assert!(!learning_route_allowed(
            "/unlisted",
            ScoutWorkerHttpMethod::Post,
        ));
    }

    #[test]
    fn scout_markouts_route_to_their_attributed_strategy_worker() {
        let command = ScoutCoordinatorRouteCommand {
            request_id: "markout-1".into(),
            strategy: None,
            method: ScoutWorkerHttpMethod::Post,
            path: "/scout-markout".into(),
            payload: serde_json::json!({
                "decision_id": "sample-1",
                "strategy": "live-trade-flow",
            }),
        };

        assert_eq!(route_strategy_names(&command), vec!["live-trade-flow"]);
    }

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

    #[tokio::test]
    async fn symbol_scoped_profile_snapshot_is_learning_owned_and_supplies_default_profiles() {
        let service = ServiceState {
            state: Arc::new(Mutex::new(LearningState::default())),
            path: PathBuf::new(),
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let coordinator =
            ScoutCoordinator::new(service, "live-momentum=ws://127.0.0.1:1", Router::new())
                .unwrap();
        let snapshot = coordinator
            .profile_snapshot_for(
                vec!["DOGE-USDT".into(), "XMSTR-USDT".into()],
                vec!["live-momentum".into()],
            )
            .await
            .unwrap();
        let profiles = snapshot["profiles"].as_array().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0]["symbol"], "DOGE-USDT");
        assert_eq!(profiles[0]["strategy"], "live-momentum");
        assert_eq!(profiles[0]["version"], 0);
        assert!(profiles[0]["parameters"].is_object());
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
            state_updates: Arc::new(tokio::sync::Notify::new()),
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
    async fn learning_stream_delivers_typed_assignment_and_correlates_worker_result() {
        let assignments = Arc::new(StdMutex::new(Vec::new()));
        let worker_assignments = Arc::clone(&assignments);

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
            state_updates: Arc::new(tokio::sync::Notify::new()),
        };
        let endpoint = "live-momentum=ws://127.0.0.1:1";
        let coordinator = ScoutCoordinator::new(service, &endpoint, Router::new()).unwrap();
        let hub = Arc::clone(&coordinator.scout_updates);
        let mut receiver = hub.sender.subscribe();
        let result_task = tokio::spawn(async move {
            let envelope = receiver.recv().await.unwrap();
            let ScoutDataUpdate::Assignment(assignment) = envelope.update else {
                panic!("expected assignment")
            };
            worker_assignments.lock().unwrap().push(assignment.clone());
            let result = ScoutWorkResult {
                assignment_id: assignment.assignment_id.clone(),
                strategy: assignment.strategy,
                symbol: assignment.symbol,
                decision: ScoutWorkDecision::Enter,
                candidate: assignment.candidate,
                walk_forward_evidence: None,
            };
            if let Some(waiter) = hub.assignments.lock().await.remove(&result.assignment_id) {
                let _ = waiter.send(Ok(result));
            }
        });
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

        result_task.await.unwrap();
    }

    #[test]
    fn scout_stream_replays_latest_state_and_pending_assignments() {
        let hub = ScoutUpdateHub::new();
        let strategy = Strategy::LiveMomentum;
        hub.publish(
            strategy,
            ScoutDataUpdate::Profiles(serde_json::json!({
                "profiles":[{"symbol":"BTC-USDT","strategy":"live-momentum","profile_id":"btc-v1"}],
                "scout_parameter_variants":[]
            })),
        );
        hub.publish(
            strategy,
            ScoutDataUpdate::Profiles(serde_json::json!({
                "profiles":[{"symbol":"ETH-USDT","strategy":"live-momentum","profile_id":"eth-v1"}],
                "scout_parameter_variants":[]
            })),
        );
        let assignment = ScoutWorkAssignment {
            assignment_id: "assignment-1".into(),
            strategy,
            mode: ScoutWorkMode::Evaluate,
            symbol: "BTC-USDT".into(),
            parameters: BTreeMap::new(),
            candidate: serde_json::json!({"direction":"bullish"}),
        };
        hub.publish(strategy, ScoutDataUpdate::Assignment(assignment));

        let snapshot = hub.snapshot(strategy);
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.iter().any(|update| matches!(
            &update.update,
            ScoutDataUpdate::Profiles(value) if value["profiles"].as_array().is_some_and(|profiles| profiles.len() == 2)
        )));
        assert!(snapshot.iter().any(|update| matches!(
            &update.update,
            ScoutDataUpdate::Assignment(value) if value.assignment_id == "assignment-1"
        )));
        hub.forget_assignment(strategy, "assignment-1");
        assert_eq!(hub.snapshot(strategy).len(), 1);
    }
}
