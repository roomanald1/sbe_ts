fn benchmark_signal(strategy: &str, candidate: bool) -> Signal {
    Signal {
        symbol: "BTC-USDT".into(),
        direction: "bullish".into(),
        strength: 1.0,
        timestamp: 1,
        stale: false,
        score: 1.0,
        ranging_probability: None,
        range_location: None,
        components: SignalComponents::default(),
        strategy_candidate: candidate,
        strategy: Some(strategy.into()),
        strategy_decision: Some(serde_json::json!({
            "observed_close": 120.0, "primary_level": 100.0, "secondary_level": 100.0,
            "btc_gate_required": true, "btc_close": 120.0, "btc_sma": 100.0,
            "primary_metric_value": 10.0, "strategy_order_size_multiplier": 0.5,
            "monthly_anchor_close_time_ms": 1, "cross_sectional_metric_value": 10.0,
            "cross_sectional_threshold": 5.0
        })),
        market_regime: None,
        market_state: None,
        news_attribution: None,
        entry_gate: None,
    }
}

fn service_endpoint_environment_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(feature = "benchmark-support")]
type BenchmarkOrderReadyObserver = Box<dyn FnOnce(&str, std::time::Instant) + Send + 'static>;

#[cfg(feature = "benchmark-support")]
fn benchmark_order_ready_observers()
-> &'static std::sync::Mutex<std::collections::HashMap<String, BenchmarkOrderReadyObserver>> {
    static OBSERVERS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, BenchmarkOrderReadyObserver>>,
    > = std::sync::OnceLock::new();
    OBSERVERS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Register a one-shot observer for a stable live-worker event ID. The hook
/// reports the exact AutoTrader decision ID once the prepared request is ready.
/// This
/// feature is enabled only as an Insights dev-dependency; normal service builds
/// do not include the observer registry or call site.
#[cfg(feature = "benchmark-support")]
pub fn observe_benchmark_order_ready(
    event_id: String,
    observer: impl FnOnce(&str, std::time::Instant) + Send + 'static,
) {
    let previous = benchmark_order_ready_observers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(event_id, Box::new(observer));
    assert!(previous.is_none(), "duplicate benchmark decision observer");
}

#[cfg(feature = "benchmark-support")]
pub(super) fn remove_benchmark_order_ready_observer(event_id: &str) {
    benchmark_order_ready_observers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(event_id);
}

#[cfg(feature = "benchmark-support")]
pub(super) fn notify_benchmark_order_ready(decision_id: &str, event_id: Option<&str>) {
    let Some(event_id) = event_id else {
        return;
    };
    let observer = benchmark_order_ready_observers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(event_id);
    if let Some(observer) = observer {
        observer(decision_id, std::time::Instant::now());
    }
}

#[cfg(feature = "benchmark-support")]
#[derive(Clone, Debug, serde::Serialize)]
pub struct CorrelatedMarketOrderReadySample {
    pub source_event_id: String,
    pub decision_id: Option<String>,
    pub strategy_decision: String,
    pub cycle_status: String,
    pub order_ready_ms: Option<f64>,
    pub durable_reservation_visible_at_hook: Option<bool>,
    pub transport_followed_order_ready: Option<bool>,
    pub client_order_identity_present_at_transport: Option<bool>,
    pub failure: Option<String>,
}

#[cfg(feature = "benchmark-support")]
pub struct CorrelatedMarketOrderReadySession {
    _environment_guard: std::sync::MutexGuard<'static, ()>,
    previous_pm_endpoint: Option<std::ffi::OsString>,
    server: tokio::task::JoinHandle<()>,
    pm_requests: Arc<std::sync::Mutex<Vec<(std::time::Instant, crate::service_rpc::RpcRequest)>>>,
    state: AutoTraderState,
    socket_api: AutoTraderSocketApi,
    settings_path: PathBuf,
}

#[cfg(feature = "benchmark-support")]
impl CorrelatedMarketOrderReadySession {
    pub async fn start() -> anyhow::Result<Self> {
        use rsocket_rust::prelude::*;
        use rsocket_rust_transport_websocket::WebsocketServerTransport;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone)]
        struct LocalPositionManager(
            Arc<std::sync::Mutex<Vec<(std::time::Instant, crate::service_rpc::RpcRequest)>>>,
        );
        #[rsocket_rust::async_trait]
        impl RSocket for LocalPositionManager {
            async fn metadata_push(&self, _: Payload) -> anyhow::Result<()> {
                anyhow::bail!("unsupported")
            }
            async fn fire_and_forget(&self, _: Payload) -> anyhow::Result<()> {
                anyhow::bail!("unsupported")
            }
            async fn request_response(&self, payload: Payload) -> anyhow::Result<Option<Payload>> {
                let request = serde_json::from_slice::<crate::service_rpc::RpcRequest>(
                    payload
                        .data()
                        .ok_or_else(|| anyhow::anyhow!("request data missing"))?
                        .as_ref(),
                )?;
                self.0
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push((std::time::Instant::now(), request.clone()));
                let (status, body) = match (request.method.as_str(), request.path.as_str()) {
                    ("POST", "/positions/orders") => (
                        200,
                        serde_json::json!({
                            "status": "accepted",
                            "filled_quantity": 0.0,
                            "average_execution_price": null,
                            "estimated_fee": 0.0,
                            "order": {}
                        }),
                    ),
                    ("DELETE", _) => (200, serde_json::json!({})),
                    _ => (
                        404,
                        serde_json::json!({"error":"unexpected benchmark route"}),
                    ),
                };
                let response = crate::service_rpc::RpcResponse {
                    status,
                    body,
                    headers: BTreeMap::from([(
                        "x-position-manager-handler-ms".into(),
                        "0.01".into(),
                    )]),
                };
                Ok(Some(
                    Payload::builder()
                        .set_data(serde_json::to_vec(&response)?)
                        .build(),
                ))
            }
            fn request_stream(&self, _: Payload) -> Flux<anyhow::Result<Payload>> {
                Box::pin(futures_util::stream::empty())
            }
            fn request_channel(
                &self,
                _: Flux<anyhow::Result<Payload>>,
            ) -> Flux<anyhow::Result<Payload>> {
                Box::pin(futures_util::stream::empty())
            }
        }

        static FIXTURE_INDEX: AtomicUsize = AtomicUsize::new(1);
        let fixture_index = FIXTURE_INDEX.fetch_add(1, Ordering::Relaxed);
        let environment_guard = service_endpoint_environment_lock();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        drop(listener);
        let endpoint = format!("ws://{address}");
        let pm_requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let service = LocalPositionManager(Arc::clone(&pm_requests));
        let server = tokio::spawn(async move {
            RSocketFactory::receive()
                .transport(WebsocketServerTransport::from(address.to_string()))
                .acceptor(Box::new(move |_, _| Ok(Box::new(service.clone()))))
                .serve()
                .await
                .expect("local correlated benchmark PM service must run");
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let previous_pm_endpoint = std::env::var_os("POSITION_MANAGEMENT_RSOCKET_URL");
        unsafe { std::env::set_var("POSITION_MANAGEMENT_RSOCKET_URL", &endpoint) };

        let strategy = Strategy::from_name("live-momentum")
            .expect("live-momentum strategy must be registered");
        let mut settings = Settings::default();
        settings.strategies = vec![strategy];
        settings.signal_strategies_enabled = true;
        settings.paper_learning_exploration_until_ms = current_time_ms().saturating_add(60_000);
        let state = benchmark_fixture_state(settings, "correlated-raw", fixture_index);
        let socket_api = AutoTraderSocketApi {
            state: state.clone(),
        };
        let settings_path = state.settings_path.clone();
        Ok(Self {
            _environment_guard: environment_guard,
            previous_pm_endpoint,
            server,
            pm_requests,
            state,
            socket_api,
            settings_path,
        })
    }

    /// Evaluate the actual emitted Insights signal with the registered
    /// Strategy evaluator, submit its versioned report through AutoTrader's
    /// production receiver, and drive the normal cycle. The ready sample ends
    /// at the feature-gated hook after durable reservation and exact request
    /// preparation, immediately before Position Management transport.
    pub async fn process_signal(
        &self,
        mut signal: strategy::StrategyMarketSignal,
        executable_market: serde_json::Value,
        raw_ingress_started: std::time::Instant,
    ) -> anyhow::Result<CorrelatedMarketOrderReadySample> {
        let strategy = Strategy::LiveMomentum;
        let source_event_id = signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("live_worker_event_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|event_id| !event_id.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Insights signal has no source event identity"))?
            .to_owned();
        let parameters = {
            let settings = self.state.settings.lock().await;
            learned_strategy_parameter_profile(&settings, &signal.symbol, strategy)
                .parameters
                .into_iter()
                .map(|parameter| (parameter.key, parameter.value))
                .collect::<BTreeMap<_, _>>()
        };
        let profile_id = strategy::profile_id(&signal.symbol, strategy.as_str(), &parameters);
        let profile_version = {
            let settings = self.state.settings.lock().await;
            live_profile_for_symbol(&settings, &signal.symbol, strategy).version
        };
        signal.strategy = Some(strategy.as_str().to_owned());
        signal.strategy_candidate = true;
        let input = strategy::StrategySignal::entry(&signal, &parameters);
        let evaluated = strategy::evaluate(strategy, &input);
        let (decision, reason) = match evaluated {
            Ok(strategy::StrategyDecision::Enter) => (strategy::LiveStrategyDecision::Enter, None),
            Ok(strategy::StrategyDecision::Hold) => (strategy::LiveStrategyDecision::Hold, None),
            Ok(strategy::StrategyDecision::Exit) => (strategy::LiveStrategyDecision::Exit, None),
            Ok(strategy::StrategyDecision::Rebalance) => {
                (strategy::LiveStrategyDecision::Rebalance, None)
            }
            Ok(strategy::StrategyDecision::Scout | strategy::StrategyDecision::Reject) => (
                strategy::LiveStrategyDecision::Reject,
                strategy.entry_rejection_reason(&input),
            ),
            Err(error) => (strategy::LiveStrategyDecision::Reject, Some(error)),
        };
        let identity = strategy::LiveStrategyEventIdentity::for_thesis(
            &source_event_id,
            strategy,
            &signal.symbol,
            &profile_id,
            &signal.direction,
            None,
        );
        let metadata = signal
            .strategy_decision
            .get_or_insert_with(|| serde_json::json!({}));
        let metadata = metadata
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Insights strategy metadata is not an object"))?;
        metadata.insert(
            "live_worker_event_id".into(),
            serde_json::Value::String(identity.event_id.clone()),
        );
        metadata.insert(
            "live_worker_event_identity_version".into(),
            serde_json::Value::from(identity.schema_version),
        );
        metadata.insert(
            "live_worker_episode_id".into(),
            serde_json::Value::String(identity.episode_id.clone()),
        );
        metadata.insert(
            "live_worker_episode_reset_generation".into(),
            serde_json::Value::from(identity.episode_reset_generation),
        );
        metadata.insert("strategy_parameters".into(), serde_json::json!(parameters));
        metadata.insert("learning_profile_id".into(), serde_json::json!(profile_id));
        metadata.insert(
            "learning_profile_symbol".into(),
            serde_json::json!(signal.symbol),
        );
        metadata.insert(
            "learning_profile_version".into(),
            serde_json::json!(profile_version),
        );
        metadata.insert(
            "live_worker_decision".into(),
            serde_json::to_value(decision)?
                .as_str()
                .map(str::to_owned)
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        );
        let report = strategy::LiveStrategyDecisionReport {
            contract_version: strategy::LIVE_EVALUATION_CONTRACT_VERSION,
            event_id: identity.event_id.clone(),
            event_identity_version: identity.schema_version,
            episode_id: identity.episode_id,
            strategy,
            symbol: signal.symbol.clone(),
            profile_id: profile_id.clone(),
            profile_version,
            parameter_hash: profile_id,
            event_time_ms: signal.timestamp,
            decision,
            reason,
            walk_forward_evidence: None,
            signal: signal.clone(),
            dated_carry_scan: None,
            dated_carry_candidate: None,
            dated_carry_settlement: None,
        };
        let accepted = self.socket_api.report_decision(report).await;
        anyhow::ensure!(
            accepted
                .as_ref()
                .is_ok_and(|status| (200..300).contains(status)),
            "AutoTrader live-decision receiver rejected the evaluated report: {accepted:?}"
        );

        let observer_event_id = signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("live_worker_event_id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("evaluated signal lost its stable worker event ID"))?
            .to_owned();
        let request_count_before = self
            .pm_requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        // Durable entry intents live in the dedicated intent store at
        // settings_path; the normalized Settings database is a sibling
        // settings_path.with_extension("sqlite") file.
        let database_path = self.settings_path.clone();
        observe_benchmark_order_ready(observer_event_id.clone(), move |decision_id, ready_at| {
            let observed_decision_id = decision_id.to_owned();
            let durable = rusqlite::Connection::open(database_path)
                .and_then(|connection| {
                    connection.query_row(
                        "SELECT EXISTS(SELECT 1 FROM durable_entry_reservations WHERE decision_id=?1)",
                        [&observed_decision_id],
                        |row| row.get::<_, bool>(0),
                    )
                })
                .unwrap_or(false);
            let _ = ready_tx.send((observed_decision_id, ready_at, durable));
        });
        let symbol = signal.symbol.clone();
        let dashboard = Dashboard {
            best: vec![signal],
            missed_opportunity_bands: Arc::new(Vec::new()),
            markets: Arc::new(BTreeMap::from([(symbol, executable_market)])),
            queued_at: None,
            queue_wait_ms: None,
        };
        let cycle = match run_cycle_inner(&self.state, dashboard).await {
            Ok(cycle) => cycle,
            Err((_, message)) => {
                remove_benchmark_order_ready_observer(&observer_event_id);
                return Ok(CorrelatedMarketOrderReadySample {
                    source_event_id,
                    decision_id: None,
                    strategy_decision: format!("{decision:?}"),
                    cycle_status: "cycle_error".into(),
                    order_ready_ms: None,
                    durable_reservation_visible_at_hook: None,
                    transport_followed_order_ready: None,
                    client_order_identity_present_at_transport: None,
                    failure: Some(message),
                });
            }
        };
        let cycle_status = cycle.status.to_owned();
        if cycle_status != "order_submitting" {
            remove_benchmark_order_ready_observer(&observer_event_id);
            return Ok(CorrelatedMarketOrderReadySample {
                source_event_id,
                decision_id: None,
                strategy_decision: format!("{decision:?}"),
                cycle_status,
                order_ready_ms: None,
                durable_reservation_visible_at_hook: None,
                transport_followed_order_ready: None,
                client_order_identity_present_at_transport: None,
                failure: cycle.reason,
            });
        }
        let observation = tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("order-ready hook was not reached within five seconds"))?
            .map_err(|_| anyhow::anyhow!("order-ready observer was dropped"))?;
        let transport = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let observed = self
                    .pm_requests
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(request_count_before)
                    .cloned();
                if let Some(observed) = observed {
                    return observed;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("local PM did not observe the prepared order"))?;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let completed = self
                    .state
                    .settings
                    .lock()
                    .await
                    .orders
                    .iter()
                    .find(|order| order.decision_id.as_deref() == Some(&observation.0))
                    .is_some_and(|order| order.status != "submitting");
                if completed {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("AutoTrader did not finish applying PM response"))?;
        Ok(CorrelatedMarketOrderReadySample {
            source_event_id,
            decision_id: Some(observation.0),
            strategy_decision: format!("{decision:?}"),
            cycle_status,
            order_ready_ms: Some(
                observation
                    .1
                    .saturating_duration_since(raw_ingress_started)
                    .as_secs_f64()
                    * 1_000.0,
            ),
            durable_reservation_visible_at_hook: Some(observation.2),
            transport_followed_order_ready: Some(transport.0 > observation.1),
            client_order_identity_present_at_transport: Some(
                transport
                    .1
                    .body
                    .as_ref()
                    .and_then(|body| body.get("client_order_id"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| !value.is_empty()),
            ),
            failure: None,
        })
    }
}

#[cfg(feature = "benchmark-support")]
impl Drop for CorrelatedMarketOrderReadySession {
    fn drop(&mut self) {
        self.server.abort();
        if let Some(endpoint) = self.previous_pm_endpoint.take() {
            unsafe { std::env::set_var("POSITION_MANAGEMENT_RSOCKET_URL", endpoint) };
        } else {
            unsafe { std::env::remove_var("POSITION_MANAGEMENT_RSOCKET_URL") };
        }
        let _ = std::fs::remove_file(&self.settings_path);
        let database_path = settings_database_path(&self.settings_path);
        let _ = std::fs::remove_file(&database_path);
        let _ = std::fs::remove_file(database_path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(database_path.with_extension("sqlite-shm"));
        let carry_path = self.settings_path.with_extension("dated-carry.json");
        let _ = std::fs::remove_file(carry_path);
    }
}

/// Per-event latency summary used by the end-to-end benchmark runner. Samples
/// are recorded before inspecting the operation result, so rejected/error
/// outcomes remain in the measured population.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BenchmarkLatencyDistribution {
    pub sample_count: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub p999_ms: f64,
}

impl BenchmarkLatencyDistribution {
    pub fn from_samples_ms(mut samples: Vec<f64>) -> Self {
        assert!(!samples.is_empty(), "benchmark needs at least one event");
        assert!(
            samples
                .iter()
                .all(|sample| sample.is_finite() && *sample >= 0.0)
        );
        samples.sort_by(f64::total_cmp);
        let percentile = |fraction: f64| {
            let index = ((samples.len() as f64 * fraction).ceil() as usize)
                .saturating_sub(1)
                .min(samples.len() - 1);
            samples[index]
        };
        Self {
            sample_count: samples.len(),
            p50_ms: percentile(0.50),
            p95_ms: percentile(0.95),
            p99_ms: percentile(0.99),
            p999_ms: percentile(0.999),
        }
    }
}

/// Measure every invocation, including invocations whose production path
/// returns an error or a rejected/skipped decision. The returned elapsed time
/// includes the complete operation boundary and is the denominator for the
/// sustained workload rate.
pub async fn measure_async_benchmark_events<T, E, F, Fut>(
    event_count: usize,
    mut operation: F,
) -> (BenchmarkLatencyDistribution, std::time::Duration, usize)
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    use std::time::Instant;
    assert!(event_count > 0, "benchmark event count must be positive");
    let mut samples_ms = Vec::with_capacity(event_count);
    let mut successful_events = 0;
    let workload_started = Instant::now();
    for index in 0..event_count {
        let event_started = Instant::now();
        let result = operation(index).await;
        samples_ms.push(event_started.elapsed().as_secs_f64() * 1_000.0);
        successful_events += usize::from(result.is_ok());
        let _ = std::hint::black_box(result);
    }
    let elapsed = workload_started.elapsed();
    (
        BenchmarkLatencyDistribution::from_samples_ms(samples_ms),
        elapsed,
        successful_events,
    )
}

/// Per-event production-path evidence. Timing distributions are optional when
/// no order reached that boundary; their sample counts must not be inferred
/// from the event count.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ProductionPathBenchmarkMeasurements {
    pub event_latency: BenchmarkLatencyDistribution,
    pub submit_ready_local: Option<BenchmarkLatencyDistribution>,
    pub position_management_rsocket: Option<BenchmarkLatencyDistribution>,
    pub event_count: usize,
    pub candidate_event_count: usize,
    pub eligible_event_count: usize,
    pub eligible_candidate_count: usize,
    pub successful_events: usize,
    pub submitted_order_events: usize,
    pub non_submitted_events: usize,
    pub response_timing_sample_count: usize,
    pub queue_wait: Option<BenchmarkLatencyDistribution>,
    pub cycle_lock_wait: Option<BenchmarkLatencyDistribution>,
    pub auxiliary_timings: BTreeMap<String, BenchmarkLatencyDistribution>,
    pub dropped_telemetry_events: usize,
    pub concurrent_events: Option<usize>,
    pub distinct_symbols: Option<usize>,
    pub max_in_flight: Option<usize>,
    pub dispatch_interval_ms: Option<u64>,
    pub burst_throughput_events_per_second: Option<f64>,
    pub elapsed_ms: f64,
    pub events_per_second: f64,
}

fn collect_shared_state_timings(
    state: &AutoTraderState,
    measurements: &mut ProductionPathBenchmarkMeasurements,
) {
    let mut add = |prefix: &str, groups: BTreeMap<String, Vec<f64>>| {
        for (name, samples) in groups {
            if samples
                .iter()
                .all(|sample| sample.is_finite() && *sample >= 0.0)
                && !samples.is_empty()
            {
                measurements.auxiliary_timings.insert(
                    format!("{prefix}.{name}"),
                    BenchmarkLatencyDistribution::from_samples_ms(samples),
                );
            }
        }
    };
    add(
        "cycle_lock_hold",
        state
            .cycle_lock_hold_ms
            .lock()
            .map(|groups| groups.clone())
            .unwrap_or_default(),
    );
    add(
        "cycle_lock_scope",
        state
            .cycle_lock_scope_ms
            .lock()
            .map(|groups| groups.clone())
            .unwrap_or_default(),
    );
    add(
        "settings_persistence",
        state
            .settings_persist_timings_ms
            .lock()
            .map(|groups| groups.clone())
            .unwrap_or_default(),
    );
}

async fn measure_production_path_events<T, E, F, Fut, X>(
    event_count: usize,
    mut operation: F,
    mut extract_timing: X,
) -> ProductionPathBenchmarkMeasurements
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    X: FnMut(&T) -> Option<(f64, f64)>,
{
    use std::time::Instant;
    assert!(event_count > 0, "benchmark event count must be positive");
    let mut event_samples_ms = Vec::with_capacity(event_count);
    let mut submit_ready_samples_ms = Vec::with_capacity(event_count);
    let mut transport_samples_ms = Vec::with_capacity(event_count);
    let mut successful_events = 0;
    let mut submitted_order_events = 0;
    let workload_started = Instant::now();
    for index in 0..event_count {
        let event_started = Instant::now();
        let result = operation(index).await;
        event_samples_ms.push(event_started.elapsed().as_secs_f64() * 1_000.0);
        if let Ok(output) = &result {
            successful_events += 1;
            if let Some((submit_ready_ms, transport_ms)) = extract_timing(output) {
                submitted_order_events += 1;
                if submit_ready_ms.is_finite() && submit_ready_ms >= 0.0 {
                    submit_ready_samples_ms.push(submit_ready_ms);
                }
                if transport_ms.is_finite() && transport_ms >= 0.0 {
                    transport_samples_ms.push(transport_ms);
                }
            }
        }
        let _ = std::hint::black_box(result);
    }
    let elapsed = workload_started.elapsed();
    let elapsed_seconds = elapsed.as_secs_f64();
    let response_timing_sample_count = submit_ready_samples_ms
        .len()
        .min(transport_samples_ms.len());
    ProductionPathBenchmarkMeasurements {
        event_latency: BenchmarkLatencyDistribution::from_samples_ms(event_samples_ms),
        submit_ready_local: (!submit_ready_samples_ms.is_empty())
            .then(|| BenchmarkLatencyDistribution::from_samples_ms(submit_ready_samples_ms)),
        position_management_rsocket: (!transport_samples_ms.is_empty())
            .then(|| BenchmarkLatencyDistribution::from_samples_ms(transport_samples_ms)),
        event_count,
        candidate_event_count: 0,
        eligible_event_count: 0,
        eligible_candidate_count: 0,
        successful_events,
        submitted_order_events,
        non_submitted_events: event_count.saturating_sub(submitted_order_events),
        response_timing_sample_count,
        queue_wait: None,
        cycle_lock_wait: None,
        auxiliary_timings: BTreeMap::new(),
        dropped_telemetry_events: 0,
        concurrent_events: None,
        distinct_symbols: None,
        max_in_flight: None,
        dispatch_interval_ms: None,
        burst_throughput_events_per_second: None,
        elapsed_ms: elapsed.as_secs_f64() * 1_000.0,
        events_per_second: if elapsed_seconds > 0.0 {
            event_count as f64 / elapsed_seconds
        } else {
            0.0
        },
    }
}

/// Machine-readable evidence shape accepted by scripts/local/performance_gate.py.
/// The caller supplies the measured production path distributions and metadata;
/// this type deliberately cannot manufacture values from the proxy benchmarks.
#[derive(serde::Serialize)]
pub struct AutoTraderPerformanceEvidence {
    pub hardware: String,
    pub git_head: String,
    pub build_mode: String,
    pub measurement_scope: String,
    pub workload_id: String,
    pub event_latency_boundary: String,
    pub measurement_definition: String,
    pub benchmark_config_sha256: String,
    pub sampling_method: String,
    pub execution_runtime_flavor: String,
    /// Release-build measurements remain observational until independently
    /// reviewed against the applicable performance acceptance gates.
    pub acceptance_state: String,
    pub event_count: usize,
    pub candidate_event_count: usize,
    pub eligible_event_count: usize,
    pub eligible_candidate_count: usize,
    pub successful_events: usize,
    pub submitted_order_events: usize,
    pub non_submitted_events: usize,
    pub response_timing_sample_count: usize,
    pub events_per_second: f64,
    /// Eligible precomputed signal candidates per second, as reported by each
    /// production cycle's eligibility diagnostics. This is the rate relevant
    /// to the v35 eligible-ingress threshold; it can differ from submissions
    /// when downstream capital or position gates reject an entry.
    pub eligible_signals_per_second: f64,
    /// Actual order-submitting events per second. Kept separately from eligible
    /// signal rate so utilization/risk gates do not redefine ingress throughput.
    pub submitted_order_rate_per_second: f64,
    /// Compatibility alias for actual submitted-order rate.
    pub signals_per_second: f64,
    pub dropped_telemetry_events: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrent_events: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distinct_symbols: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_in_flight: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_interval_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub burst_throughput_events_per_second: Option<f64>,
    pub measurements: std::collections::BTreeMap<String, BenchmarkLatencyDistribution>,
    /// Boundaries that this AutoTrader executable cannot measure. In
    /// particular, raw market ingress and its upstream feature evaluation
    /// belong to Insights, not this process.
    pub unsupported_boundaries: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exchange_network_latency: Option<BenchmarkLatencyDistribution>,
}

pub const AUTOTRADER_BENCHMARK_ACCEPTANCE_STATE: &str =
    "unaccepted_release_execution_and_independent_review_required";
pub const AUTOTRADER_SUBMIT_TIMING_BOUNDARIES: &[&str] = &[
    "order_payload_ready_after_local_construction",
    "position_management_rsocket_request_response",
];

fn response_timing_boundaries(response: &serde_json::Value) -> Option<(f64, f64)> {
    let timings = response.get("performance_timing_boundaries")?;
    if timings.get("submit_ready_boundary")?.as_str()? != AUTOTRADER_SUBMIT_TIMING_BOUNDARIES[0]
        || timings.get("rsocket_boundary")?.as_str()? != AUTOTRADER_SUBMIT_TIMING_BOUNDARIES[1]
        || timings.get("acceptance_state")?.as_str()? != BENCHMARK_ACCEPTANCE_STATE
    {
        return None;
    }
    Some((
        timings.get("request_prepare_local_ms")?.as_f64()?,
        timings.get("rsocket_request_response_ms")?.as_f64()?,
    ))
}

fn cycle_submission_decision_id(cycle: &CycleResult) -> Option<String> {
    cycle
        .entry_diagnostic
        .as_ref()?
        .candidate_decision_ids
        .first()
        .cloned()
}

fn cycle_eligible_candidate_count(cycle: &CycleResult) -> usize {
    cycle
        .entry_diagnostic
        .as_ref()
        .map_or(0, |diagnostic| diagnostic.eligible_candidate_count)
}

async fn wait_for_shared_state_order_timings(
    state: &AutoTraderState,
    expected_count: usize,
) -> Vec<(f64, f64)> {
    if expected_count == 0 {
        return Vec::new();
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let timings = {
            let settings = state.settings.lock().await;
            settings
                .orders
                .iter()
                .filter_map(|order| {
                    Some((
                        *order.stage_timings_ms.get("order_request_prepare_local")?,
                        *order.stage_timings_ms.get("client_request_to_headers")?,
                    ))
                })
                .collect::<Vec<_>>()
        };
        if timings.len() >= expected_count {
            return timings.into_iter().take(expected_count).collect();
        }
        tokio::task::yield_now().await;
    }
    let settings = state.settings.lock().await;
    settings
        .orders
        .iter()
        .filter_map(|order| {
            Some((
                *order.stage_timings_ms.get("order_request_prepare_local")?,
                *order.stage_timings_ms.get("client_request_to_headers")?,
            ))
        })
        .take(expected_count)
        .collect()
}

fn production_path_measurements(
    event_samples_ms: Vec<f64>,
    submit_ready_samples_ms: Vec<f64>,
    transport_samples_ms: Vec<f64>,
    queue_wait_samples_ms: Vec<f64>,
    cycle_lock_wait_samples_ms: Vec<f64>,
    successful_events: usize,
    submitted_order_events: usize,
    candidate_event_count: usize,
    eligible_event_count: usize,
    eligible_candidate_count: usize,
    dropped_telemetry_events: usize,
    measured_elapsed: std::time::Duration,
    concurrent_events: Option<usize>,
    distinct_symbols: Option<usize>,
    max_in_flight: Option<usize>,
    dispatch_interval_ms: Option<u64>,
) -> ProductionPathBenchmarkMeasurements {
    let event_count = event_samples_ms.len();
    let response_timing_sample_count = submit_ready_samples_ms
        .len()
        .min(transport_samples_ms.len());
    let elapsed_seconds = measured_elapsed.as_secs_f64();
    ProductionPathBenchmarkMeasurements {
        event_latency: BenchmarkLatencyDistribution::from_samples_ms(event_samples_ms),
        submit_ready_local: (!submit_ready_samples_ms.is_empty())
            .then(|| BenchmarkLatencyDistribution::from_samples_ms(submit_ready_samples_ms)),
        position_management_rsocket: (!transport_samples_ms.is_empty())
            .then(|| BenchmarkLatencyDistribution::from_samples_ms(transport_samples_ms)),
        event_count,
        candidate_event_count,
        eligible_event_count,
        eligible_candidate_count,
        successful_events,
        submitted_order_events,
        non_submitted_events: event_count.saturating_sub(submitted_order_events),
        response_timing_sample_count,
        queue_wait: (!queue_wait_samples_ms.is_empty())
            .then(|| BenchmarkLatencyDistribution::from_samples_ms(queue_wait_samples_ms)),
        cycle_lock_wait: (!cycle_lock_wait_samples_ms.is_empty())
            .then(|| BenchmarkLatencyDistribution::from_samples_ms(cycle_lock_wait_samples_ms)),
        auxiliary_timings: BTreeMap::new(),
        dropped_telemetry_events,
        concurrent_events,
        distinct_symbols,
        max_in_flight,
        dispatch_interval_ms,
        burst_throughput_events_per_second: concurrent_events.map(|_| {
            if elapsed_seconds > 0.0 {
                event_count as f64 / elapsed_seconds
            } else {
                0.0
            }
        }),
        elapsed_ms: measured_elapsed.as_secs_f64() * 1_000.0,
        events_per_second: if elapsed_seconds > 0.0 {
            event_count as f64 / elapsed_seconds
        } else {
            0.0
        },
    }
}

/// Invoke actual precomputed-signal ingress for a sequence of deterministic
/// Dashboard events on one shared AutoTraderState/settings path. The cycle-
/// return distribution stops after in-memory reservation. Since the
/// production submit task runs asynchronously, this function then reads the
/// resulting per-order local request-preparation and RSocket timings from the
/// same state; the wait for those timings is excluded from event latency.
async fn benchmark_precomputed_signal_ingress<F>(
    state: AutoTraderState,
    event_count: usize,
    mut fixture: F,
) -> ProductionPathBenchmarkMeasurements
where
    F: FnMut(usize) -> Dashboard,
{
    use std::time::Instant;
    assert!(event_count > 0, "benchmark event count must be positive");
    let state = Arc::new(state);
    let mut event_samples_ms = Vec::with_capacity(event_count);
    let mut successful_events = 0;
    let mut eligible_event_count = 0;
    let mut eligible_candidate_count = 0;
    let mut pending_decision_ids = Vec::with_capacity(event_count);
    let mut queue_wait_samples_ms = Vec::with_capacity(event_count);
    let mut dropped_telemetry_events = 0_usize;
    let mut submit_ready_samples_ms = Vec::with_capacity(event_count);
    let mut transport_samples_ms = Vec::with_capacity(event_count);
    let workload_started = Instant::now();
    for index in 0..event_count {
        let mut dashboard = fixture(index);
        if let Some(queue_wait_ms) = dashboard.queue_wait_ms {
            queue_wait_samples_ms.push(queue_wait_ms);
        }
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        for signal in &mut dashboard.best {
            signal.timestamp = timestamp_ms;
        }
        let dropped_before = state.audit_tx.dropped.load(Ordering::Relaxed)
            + state.execution_audit_tx.dropped.load(Ordering::Relaxed);
        let started = Instant::now();
        let result = run_cycle_inner(&state, dashboard).await;
        event_samples_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
        successful_events += usize::from(result.is_ok());
        let eligible_candidates = result
            .as_ref()
            .map(cycle_eligible_candidate_count)
            .unwrap_or_default();
        eligible_event_count += usize::from(eligible_candidates > 0);
        eligible_candidate_count += eligible_candidates;
        let decision_id = match &result {
            Ok(cycle) if cycle.status == "order_submitting" => cycle_submission_decision_id(cycle),
            _ => None,
        };
        if let Some(decision_id) = decision_id {
            pending_decision_ids.push(decision_id);
        }
        let dropped_after = state.audit_tx.dropped.load(Ordering::Relaxed)
            + state.execution_audit_tx.dropped.load(Ordering::Relaxed);
        dropped_telemetry_events = dropped_telemetry_events
            .saturating_add(dropped_after.saturating_sub(dropped_before) as usize);
        let _ = std::hint::black_box(result);
    }
    let measured_elapsed = workload_started.elapsed();
    let submitted_order_events = pending_decision_ids.len();
    for (local_ms, transport_ms) in
        wait_for_shared_state_order_timings(&state, pending_decision_ids.len()).await
    {
        submit_ready_samples_ms.push(local_ms);
        transport_samples_ms.push(transport_ms);
    }
    let cycle_lock_wait_samples_ms = state
        .cycle_lock_wait_ms
        .lock()
        .await
        .values()
        .flatten()
        .copied()
        .collect();
    let mut measurements = production_path_measurements(
        event_samples_ms,
        submit_ready_samples_ms,
        transport_samples_ms,
        queue_wait_samples_ms,
        cycle_lock_wait_samples_ms,
        successful_events,
        submitted_order_events,
        event_count,
        eligible_event_count,
        eligible_candidate_count,
        dropped_telemetry_events,
        measured_elapsed,
        None,
        None,
        None,
        None,
    );
    collect_shared_state_timings(&state, &mut measurements);
    measurements
}

/// Run a simultaneous cross-symbol burst through the production precomputed
/// signal cycle. Event latency begins when each item is dispatched into the
/// task set and includes task scheduling/queue wait plus every production
/// decision gate until local reservation or rejection. Async Position
/// Management response timing is read afterward from each event's order
/// stage record and remains a separate distribution.
async fn benchmark_precomputed_signal_ingress_burst_with_symbols<F>(
    event_count: usize,
    workload: &str,
    max_concurrency: Option<usize>,
    dispatch_interval: Option<std::time::Duration>,
    mut symbol_for_event: F,
) -> ProductionPathBenchmarkMeasurements
where
    F: FnMut(usize) -> String,
{
    use std::time::Instant;
    assert!(
        event_count >= 128,
        "cross-symbol burst requires at least 128 events"
    );
    let state = Arc::new(benchmark_precomputed_ingress_state(workload));
    let symbols = (0..event_count)
        .map(&mut symbol_for_event)
        .collect::<Vec<_>>();
    let distinct_symbols = symbols
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    let dashboards = (0..event_count)
        .map(|index| benchmark_precomputed_ingress_dashboard(index, &symbols[index]))
        .collect::<Vec<_>>();
    let concurrency_limit =
        max_concurrency.map(|limit| Arc::new(tokio::sync::Semaphore::new(limit)));
    let mut tasks = JoinSet::new();
    let workload_started = Instant::now();
    for (index, mut dashboard) in dashboards.into_iter().enumerate() {
        let state = Arc::clone(&state);
        let concurrency_limit = concurrency_limit.clone();
        let enqueued_at = Instant::now();
        dashboard.queued_at = Some(enqueued_at);
        tasks.spawn(async move {
            let _permit = match concurrency_limit {
                Some(limit) => Some(
                    limit
                        .acquire_owned()
                        .await
                        .expect("benchmark concurrency semaphore remains open"),
                ),
                None => None,
            };
            let queue_wait_ms = enqueued_at.elapsed().as_secs_f64() * 1_000.0;
            dashboard.queue_wait_ms = Some(queue_wait_ms);
            let result = run_cycle_inner(&state, dashboard).await;
            let event_latency_ms = enqueued_at.elapsed().as_secs_f64() * 1_000.0;
            let decision_id = match &result {
                Ok(cycle) if cycle.status == "order_submitting" => {
                    cycle_submission_decision_id(cycle)
                }
                _ => None,
            };
            (
                event_latency_ms,
                queue_wait_ms,
                result.is_ok(),
                result
                    .as_ref()
                    .map(cycle_eligible_candidate_count)
                    .unwrap_or_default(),
                decision_id,
            )
        });
        if index + 1 < event_count {
            if let Some(interval) = dispatch_interval {
                tokio::time::sleep(interval).await;
            }
        }
    }

    let mut event_samples_ms = Vec::with_capacity(event_count);
    let mut queue_wait_samples_ms = Vec::with_capacity(event_count);
    let mut successful_events = 0;
    let mut eligible_event_count = 0;
    let mut eligible_candidate_count = 0;
    let mut submitted_order_events = 0;
    let mut pending_decision_ids = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((latency_ms, queue_wait_ms, successful, eligible_candidates, decision_id)) => {
                event_samples_ms.push(latency_ms);
                queue_wait_samples_ms.push(queue_wait_ms);
                successful_events += usize::from(successful);
                eligible_event_count += usize::from(eligible_candidates > 0);
                eligible_candidate_count += eligible_candidates;
                if let Some(decision_id) = decision_id {
                    submitted_order_events += 1;
                    pending_decision_ids.push(decision_id);
                }
            }
            Err(_) => {
                // A worker panic still contributes an event sample and reduces
                // the success count; the enclosing burst keeps every outcome.
                event_samples_ms.push(workload_started.elapsed().as_secs_f64() * 1_000.0);
            }
        }
    }
    let measured_elapsed = workload_started.elapsed();

    let mut submit_ready_samples_ms = Vec::with_capacity(pending_decision_ids.len());
    let mut transport_samples_ms = Vec::with_capacity(pending_decision_ids.len());
    for (local_ms, transport_ms) in
        wait_for_shared_state_order_timings(&state, pending_decision_ids.len()).await
    {
        submit_ready_samples_ms.push(local_ms);
        transport_samples_ms.push(transport_ms);
    }
    let dropped_telemetry_events = (state.audit_tx.dropped.load(Ordering::Relaxed)
        + state.execution_audit_tx.dropped.load(Ordering::Relaxed))
        as usize;
    let cycle_lock_wait_samples_ms = state
        .cycle_lock_wait_ms
        .lock()
        .await
        .values()
        .flatten()
        .copied()
        .collect();
    let mut measurements = production_path_measurements(
        event_samples_ms,
        submit_ready_samples_ms,
        transport_samples_ms,
        queue_wait_samples_ms,
        cycle_lock_wait_samples_ms,
        successful_events,
        submitted_order_events,
        event_count,
        eligible_event_count,
        eligible_candidate_count,
        dropped_telemetry_events,
        measured_elapsed,
        Some(event_count),
        Some(distinct_symbols),
        max_concurrency,
        dispatch_interval.map(|interval| interval.as_millis() as u64),
    );
    collect_shared_state_timings(&state, &mut measurements);
    measurements
}

async fn benchmark_precomputed_signal_ingress_burst(
    event_count: usize,
) -> ProductionPathBenchmarkMeasurements {
    const SYMBOLS: [&str; 4] = ["BTC-USDT", "ETH-USDT", "SOL-USDT", "XRP-USDT"];
    benchmark_precomputed_signal_ingress_burst_with_symbols(
        event_count,
        "ingress-burst",
        None,
        None,
        |index| SYMBOLS[index % SYMBOLS.len()].to_owned(),
    )
    .await
}

/// Saturate a multi-thread runtime with independent entry candidates on one
/// shared state. Symbols are selected only from the authoritative AutoTrader
/// allowlist; the list is intentionally cycled because it is finite. Cycle
/// diagnostics count eligible candidates separately from submissions.
async fn benchmark_precomputed_signal_ingress_multithread(
    event_count: usize,
) -> ProductionPathBenchmarkMeasurements {
    let allowed_symbols = strategy::autotrader_allowed_symbols();
    benchmark_precomputed_signal_ingress_burst_with_symbols(
        event_count,
        "ingress-multithread-allowlist-burst",
        None,
        None,
        |index| allowed_symbols[index % allowed_symbols.len()].to_owned(),
    )
    .await
}

async fn benchmark_precomputed_signal_ingress_multithread_sustained(
    event_count: usize,
) -> ProductionPathBenchmarkMeasurements {
    let allowed_symbols = strategy::autotrader_allowed_symbols();
    benchmark_precomputed_signal_ingress_burst_with_symbols(
        event_count,
        "ingress-multithread-sustained",
        Some(16),
        Some(std::time::Duration::from_millis(8)),
        |index| allowed_symbols[index % allowed_symbols.len()].to_owned(),
    )
    .await
}

/// Invoke the production exit decision and IOC submission path repeatedly on
/// one shared AutoTraderState/settings path. Response timing is split into
/// local request preparation and Position Management RSocket round trip.
async fn benchmark_emergency_ioc_exit_path(
    state: AutoTraderState,
    symbol: String,
    strategy_key: String,
    position: TraderPosition,
    market: serde_json::Value,
    event_count: usize,
) -> ProductionPathBenchmarkMeasurements {
    let state = Arc::new(state);
    let mut measurements = measure_production_path_events(
        event_count,
        |_| {
            let state = Arc::clone(&state);
            let symbol = symbol.clone();
            let strategy_key = strategy_key.clone();
            let position = position.clone();
            let market = market.clone();
            let mut inputs = BTreeMap::from([(symbol.clone(), market)]);
            async move {
                evaluate_position_exit(&state, symbol, strategy_key, position, &mut inputs).await
            }
        },
        |result| {
            result
                .as_ref()
                .and_then(|cycle| cycle.order_response.as_ref())
                .and_then(response_timing_boundaries)
        },
    )
    .await;
    collect_shared_state_timings(&state, &mut measurements);
    measurements
}

fn benchmark_fixture_state(settings: Settings, workload: &str, index: usize) -> AutoTraderState {
    let mut state = benchmark_snapshot_state(settings);
    // Production-path fixtures must preserve the actual audit availability
    // gate. The generic snapshot microbenchmark uses one-byte queues because
    // it never enqueues production events; that capacity rejects every audit
    // and correctly pauses real signal entries.
    state.audit_tx = AuditQueue::new(
        DECISION_AUDIT_QUEUE_CAPACITY,
        DECISION_AUDIT_QUEUE_MAX_BYTES,
    );
    state.execution_audit_tx = AuditQueue::new(
        EXECUTION_AUDIT_QUEUE_CAPACITY,
        EXECUTION_AUDIT_QUEUE_MAX_BYTES,
    );
    let fixture_id = format!("{}-{workload}-{index}", std::process::id());
    state.settings_path = std::env::temp_dir().join(format!("auto-trader-bench-{fixture_id}.json"));
    state.dated_carry_path =
        std::env::temp_dir().join(format!("auto-trader-bench-{fixture_id}.dated-carry.json"));
    state
}

fn cleanup_benchmark_fixture_files(workload: &str, fixture_instances: usize) {
    for index in 0..fixture_instances {
        let fixture_id = format!("{}-{workload}-{index}", std::process::id());
        let settings_path =
            std::env::temp_dir().join(format!("auto-trader-bench-{fixture_id}.json"));
        let carry_path =
            std::env::temp_dir().join(format!("auto-trader-bench-{fixture_id}.dated-carry.json"));
        let _ = std::fs::remove_file(settings_path);
        let _ = std::fs::remove_file(carry_path);
    }
}

fn benchmark_local_market(updated_at_ms: u64) -> serde_json::Value {
    serde_json::json!({
        "updated_at_ms": updated_at_ms,
        "bids": [{"price": 99.99, "size": 100.0}],
        "asks": [{"price": 100.01, "size": 100.0}],
        "bid": 99.99,
        "ask": 100.01
    })
}

fn benchmark_precomputed_ingress_state(workload: &str) -> AutoTraderState {
    let strategy =
        Strategy::from_name("live-momentum").expect("benchmark live strategy must be registered");
    let mut settings = Settings::default();
    settings.strategies = vec![strategy];
    settings.signal_strategies_enabled = true;
    let state = benchmark_fixture_state(settings.clone(), workload, 0);
    persist_settings(&state.settings_path, &settings)
        .expect("benchmark startup must durably initialize Settings and intent schema");
    let connection = rusqlite::Connection::open(&state.settings_path)
        .expect("benchmark startup database must open after initialization");
    let reservation_schema_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='durable_entry_reservations')",
            [],
            |row| row.get(0),
        )
        .expect("benchmark startup schema lookup must succeed");
    assert!(
        reservation_schema_exists,
        "durable intent schema must exist before the first measured reservation"
    );
    state
}

fn benchmark_precomputed_ingress_dashboard(index: usize, symbol: &str) -> Dashboard {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let strategy =
        Strategy::from_name("live-momentum").expect("benchmark live strategy must be registered");
    let mut signal = benchmark_signal("live-momentum", true);
    signal.symbol = symbol.to_owned();
    signal.timestamp = timestamp_ms;
    signal.strategy_decision = Some(serde_json::json!({
        "live_worker_decision": "enter",
        "live_worker_event_id": format!("benchmark-live-event-{index}"),
        "live_worker_decision_id": format!("benchmark-live-decision-{index}"),
        "score": 1.0,
        "strength": 1.0,
        "observed_close": 100.0,
        "primary_level": 99.0,
        "secondary_level": 99.0,
        "primary_metric_value": 1.0,
        "strategy_order_size_multiplier": 0.5
    }));
    let parameters = strategy
        .parameter_specs()
        .into_iter()
        .map(|spec| (spec.key.as_str().to_owned(), spec.default))
        .collect::<BTreeMap<_, _>>();
    signal.strategy_decision.as_mut().unwrap()["strategy_parameters"] =
        serde_json::json!(parameters);
    let market = benchmark_local_market(timestamp_ms);
    Dashboard {
        best: vec![signal],
        missed_opportunity_bands: Arc::new(Vec::new()),
        markets: Arc::new(BTreeMap::from([(symbol.to_owned(), market)])),
        queued_at: None,
        queue_wait_ms: None,
    }
}

fn benchmark_emergency_exit_fixture(
    index: usize,
) -> (
    AutoTraderState,
    String,
    String,
    TraderPosition,
    serde_json::Value,
) {
    let symbol = "BTC-USDT".to_owned();
    let strategy = "live-vwap-dislocation".to_owned();
    let position = TraderPosition {
        quantity: 1.0,
        average_entry_price: 100.0,
        entry_decision_id: Some(format!("benchmark-exit-entry-{index}")),
        strategy: Some(strategy.clone()),
        stop_loss_percent: Some(1.0),
        ..TraderPosition::default()
    };
    let mut settings = Settings::default();
    settings.stop_loss_percent = 1.0;
    settings.take_profit_percent = 2.0;
    settings.positions.insert(symbol.clone(), position.clone());
    settings.strategy_positions.insert(
        symbol.clone(),
        BTreeMap::from([(strategy.clone(), position.clone())]),
    );
    let state = benchmark_fixture_state(settings, "emergency", index);
    let market = serde_json::json!({"bid": 98.5, "ask": 99.5});
    (state, symbol, strategy, position, market)
}

/// Run actual production ingress and emergency-stop wrappers against an
/// ephemeral local RSocket service double, and return separate machine-readable
/// evidence records. The service responds deterministically with zero fills;
/// the resulting records remain unaccepted and cannot satisfy release gates.
pub async fn run_local_production_path_benchmarks(
    event_count: usize,
    hardware: String,
    git_head: String,
    build_mode: String,
) -> anyhow::Result<Vec<AutoTraderPerformanceEvidence>> {
    use rsocket_rust::prelude::*;
    use rsocket_rust_transport_websocket::WebsocketServerTransport;
    use std::sync::{
        Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    };

    #[derive(Clone)]
    struct ServiceDouble(Arc<StdMutex<Vec<crate::service_rpc::RpcRequest>>>);

    #[rsocket_rust::async_trait]
    impl RSocket for ServiceDouble {
        async fn metadata_push(&self, _: Payload) -> anyhow::Result<()> {
            anyhow::bail!("unsupported")
        }
        async fn fire_and_forget(&self, _: Payload) -> anyhow::Result<()> {
            anyhow::bail!("unsupported")
        }
        async fn request_response(&self, payload: Payload) -> anyhow::Result<Option<Payload>> {
            let request = serde_json::from_slice::<crate::service_rpc::RpcRequest>(
                payload
                    .data()
                    .ok_or_else(|| anyhow::anyhow!("request data missing"))?
                    .as_ref(),
            )?;
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request.clone());
            let (status, body) = match (request.method.as_str(), request.path.as_str()) {
                ("POST", "/positions/orders") => (
                    200,
                    serde_json::json!({
                        "status": "accepted",
                        "filled_quantity": 0.0,
                        "average_execution_price": null,
                        "estimated_fee": 0.0,
                        "order": {}
                    }),
                ),
                ("DELETE", _) => (200, serde_json::json!({})),
                _ => (
                    404,
                    serde_json::json!({"error": "unexpected benchmark route"}),
                ),
            };
            let response = crate::service_rpc::RpcResponse {
                status,
                body,
                headers: BTreeMap::from([("x-position-manager-handler-ms".into(), "0.01".into())]),
            };
            Ok(Some(
                Payload::builder()
                    .set_data(serde_json::to_vec(&response)?)
                    .build(),
            ))
        }
        fn request_stream(&self, _: Payload) -> Flux<anyhow::Result<Payload>> {
            Box::pin(futures_util::stream::empty())
        }
        fn request_channel(
            &self,
            _: Flux<anyhow::Result<Payload>>,
        ) -> Flux<anyhow::Result<Payload>> {
            Box::pin(futures_util::stream::empty())
        }
    }

    static RUN_ID: AtomicU64 = AtomicU64::new(1);
    anyhow::ensure!(event_count > 0, "benchmark event count must be positive");
    let _env_guard = service_endpoint_environment_lock();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    drop(listener);
    let endpoint = format!("ws://{address}");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let service = ServiceDouble(Arc::clone(&requests));
    let server = tokio::spawn(async move {
        RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_, _| Ok(Box::new(service.clone()))))
            .serve()
            .await
            .expect("local benchmark RSocket service must run");
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let previous_endpoint = std::env::var_os("POSITION_MANAGEMENT_RSOCKET_URL");
    unsafe { std::env::set_var("POSITION_MANAGEMENT_RSOCKET_URL", &endpoint) };
    let ingress_state = benchmark_precomputed_ingress_state("ingress");
    let ingress = benchmark_precomputed_signal_ingress(ingress_state, event_count, |index| {
        benchmark_precomputed_ingress_dashboard(index, "BTC-USDT")
    })
    .await;
    let (emergency_state, symbol, strategy, position, market) = benchmark_emergency_exit_fixture(0);
    let emergency = benchmark_emergency_ioc_exit_path(
        emergency_state,
        symbol,
        strategy,
        position,
        market,
        event_count,
    )
    .await;
    let burst_event_count = event_count.max(128);
    let burst = benchmark_precomputed_signal_ingress_burst(burst_event_count).await;
    let multithread = benchmark_precomputed_signal_ingress_multithread(burst_event_count).await;
    let multithread_sustained =
        benchmark_precomputed_signal_ingress_multithread_sustained(burst_event_count).await;
    if let Some(previous) = previous_endpoint {
        unsafe { std::env::set_var("POSITION_MANAGEMENT_RSOCKET_URL", previous) };
    } else {
        unsafe { std::env::remove_var("POSITION_MANAGEMENT_RSOCKET_URL") };
    }
    server.abort();
    cleanup_benchmark_fixture_files("ingress", 1);
    cleanup_benchmark_fixture_files("emergency", 1);
    cleanup_benchmark_fixture_files("ingress-burst", 1);
    cleanup_benchmark_fixture_files("ingress-multithread-allowlist-burst", 1);
    cleanup_benchmark_fixture_files("ingress-multithread-sustained", 1);

    let seen_requests = requests
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    anyhow::ensure!(
        ingress.event_latency.sample_count == event_count
            && ingress.response_timing_sample_count <= ingress.submitted_order_events,
        "precomputed ingress event/submit/response samples: {}/{}/{} for {event_count} events",
        ingress.event_latency.sample_count,
        ingress.submitted_order_events,
        ingress.response_timing_sample_count,
    );
    anyhow::ensure!(
        emergency.event_latency.sample_count == event_count
            && emergency.submitted_order_events == event_count
            && emergency.response_timing_sample_count == event_count,
        "emergency exit event/submit/response samples: {}/{}/{} for {event_count} events",
        emergency.event_latency.sample_count,
        emergency.submitted_order_events,
        emergency.response_timing_sample_count,
    );
    anyhow::ensure!(
        burst.event_count == burst_event_count
            && burst.concurrent_events == Some(burst_event_count)
            && burst.distinct_symbols == Some(4)
            && burst.event_latency.sample_count == burst_event_count
            && burst.submitted_order_events <= 4
            && burst.response_timing_sample_count <= burst.submitted_order_events,
        "cross-symbol burst evidence incomplete: {} event samples for {} events, {} submits, {} responses, {:?} concurrent, {:?} symbols",
        burst.event_latency.sample_count,
        burst.event_count,
        burst.submitted_order_events,
        burst.response_timing_sample_count,
        burst.concurrent_events,
        burst.distinct_symbols,
    );
    anyhow::ensure!(
        multithread.event_count == burst_event_count
            && multithread.candidate_event_count == burst_event_count
            && multithread.eligible_event_count <= burst_event_count
            && multithread.eligible_candidate_count <= burst_event_count
            && multithread.concurrent_events == Some(burst_event_count)
            && multithread.distinct_symbols == Some(strategy::autotrader_allowed_symbols().len())
            && multithread.event_latency.sample_count == burst_event_count
            && multithread.submitted_order_events <= multithread.eligible_candidate_count
            && multithread.submitted_order_events > 0
            && multithread.response_timing_sample_count <= multithread.submitted_order_events,
        "multi-thread allowlisted ingress evidence incomplete: {} candidate events, {} eligible events, {} eligible candidates, {} symbols, {} submits, {} responses",
        multithread.event_count,
        multithread.eligible_event_count,
        multithread.eligible_candidate_count,
        multithread.distinct_symbols.unwrap_or_default(),
        multithread.submitted_order_events,
        multithread.response_timing_sample_count,
    );
    anyhow::ensure!(
        multithread_sustained.event_count == burst_event_count
            && multithread_sustained.candidate_event_count == burst_event_count
            && multithread_sustained.eligible_event_count <= burst_event_count
            && multithread_sustained.eligible_candidate_count <= burst_event_count
            && multithread_sustained.concurrent_events == Some(burst_event_count)
            && multithread_sustained.distinct_symbols
                == Some(strategy::autotrader_allowed_symbols().len())
            && multithread_sustained.event_latency.sample_count == burst_event_count
            && multithread_sustained.submitted_order_events
                <= multithread_sustained.eligible_candidate_count
            && multithread_sustained.submitted_order_events > 0
            && multithread_sustained.response_timing_sample_count
                <= multithread_sustained.submitted_order_events,
        "bounded multi-thread ingress evidence incomplete: {} candidate events, {} eligible events, {} eligible candidates, {} symbols, {} submits, {} responses",
        multithread_sustained.event_count,
        multithread_sustained.eligible_event_count,
        multithread_sustained.eligible_candidate_count,
        multithread_sustained.distinct_symbols.unwrap_or_default(),
        multithread_sustained.submitted_order_events,
        multithread_sustained.response_timing_sample_count,
    );
    let received_market_orders = seen_requests
        .iter()
        .filter(|request| request.method == "POST" && request.path == "/positions/orders")
        .count();
    let expected_market_orders = emergency.submitted_order_events
        + ingress.submitted_order_events
        + burst.submitted_order_events
        + multithread.submitted_order_events
        + multithread_sustained.submitted_order_events;
    anyhow::ensure!(
        received_market_orders >= expected_market_orders,
        "local service double received {received_market_orders} market orders for {expected_market_orders} reported submissions"
    );
    anyhow::ensure!(
        seen_requests
            .iter()
            .filter(|request| {
                request.method == "POST"
                    && request.path == "/positions/orders"
                    && request
                        .body
                        .as_ref()
                        .and_then(|body| body.get("order_type"))
                        .and_then(serde_json::Value::as_str)
                        == Some("ioc")
                    && request
                        .body
                        .as_ref()
                        .and_then(|body| body.get("reduce_only"))
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
            })
            .count()
            >= emergency.submitted_order_events,
        "local service double did not observe the emergency IOC reduce-only submissions"
    );
    drop(seen_requests);

    let config = serde_json::json!({
        "fixture_version": 2,
        "event_count": event_count,
        "burst_event_count": burst_event_count,
        "burst_symbols": ["BTC-USDT", "ETH-USDT", "SOL-USDT", "XRP-USDT"],
        "multithread_symbol_source": "strategy::autotrader_allowed_symbols-cycled",
        "multithread_runtime": "tokio-multi-thread-shared-state",
        "sustained_batch_max_in_flight": 16,
        "sustained_batch_offered_rate_hz": 125,
        "sustained_batch_dispatch_interval_ms": 8,
        "sustained_batch_definition": "paced bounded-concurrency event batch; this is not a duration soak",
        "state_model": "one-shared-autotrader-state-and-settings-path-per-workload",
        "position_management": "local-rsocket-zero-fill-double",
        "ingress": "live-momentum-precomputed-enter",
        "emergency_exit": "one-percent-stop-loss-ioc"
    });
    use sha2::Digest as _;
    let config_hash = format!(
        "sha256:v1:{:x}",
        sha2::Sha256::digest(serde_json::to_vec(&config)?)
    );
    let run_id = RUN_ID.fetch_add(1, Ordering::Relaxed);
    let build_mode = if cfg!(debug_assertions) {
        "debug".to_owned()
    } else {
        build_mode
    };
    let records = vec![
        AutoTraderPerformanceEvidence::from_production_measurements(
            &format!("autotrader-precomputed-ingress-{run_id}"),
            &ingress,
            hardware.clone(),
            git_head.clone(),
            build_mode.clone(),
            config_hash.clone(),
        ),
        AutoTraderPerformanceEvidence::from_production_measurements(
            &format!("autotrader-emergency-ioc-exit-{run_id}"),
            &emergency,
            hardware.clone(),
            git_head.clone(),
            build_mode.clone(),
            config_hash.clone(),
        ),
        AutoTraderPerformanceEvidence::from_production_measurements(
            &format!("autotrader-precomputed-ingress-burst-{run_id}"),
            &burst,
            hardware.clone(),
            git_head.clone(),
            build_mode.clone(),
            config_hash.clone(),
        ),
        AutoTraderPerformanceEvidence::from_production_measurements(
            &format!("autotrader-precomputed-ingress-multithread-eligible-{run_id}"),
            &multithread,
            hardware.clone(),
            git_head.clone(),
            build_mode.clone(),
            config_hash.clone(),
        ),
        AutoTraderPerformanceEvidence::from_production_measurements(
            &format!("autotrader-precomputed-ingress-multithread-sustained-{run_id}"),
            &multithread_sustained,
            hardware,
            git_head,
            build_mode,
            config_hash,
        ),
    ];
    Ok(records)
}

/// The AutoTrader executable receives already-computed strategy signals.
/// There is no supported raw-market-to-feature-evaluation boundary in this
/// crate, so performance evidence emitted here must preserve this limitation.
pub const AUTOTRADER_UNSUPPORTED_PERFORMANCE_BOUNDARIES: &[&str] = &[
    "raw market event ingress and upstream Insights feature evaluation",
    "exchange transmission, acknowledgement, and fill latency (unless supplied as separate transport evidence)",
];

impl AutoTraderPerformanceEvidence {
    /// Persist input for performance_gate.py. The writer does not run the gate
    /// or imply that measurements passed acceptance.
    pub fn write_json(&self, path: &std::path::Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, bytes)
    }

    pub fn from_production_measurements(
        workload_id: &str,
        measurements: &ProductionPathBenchmarkMeasurements,
        hardware: String,
        git_head: String,
        build_mode: String,
        benchmark_config_sha256: String,
    ) -> Self {
        let mut distributions = BTreeMap::from([(
            format!("{workload_id}.event_latency"),
            measurements.event_latency.clone(),
        )]);
        if let Some(distribution) = &measurements.queue_wait {
            distributions.insert(format!("{workload_id}.queue_wait"), distribution.clone());
        }
        if let Some(distribution) = &measurements.cycle_lock_wait {
            distributions.insert(
                format!("{workload_id}.cycle_lock_wait"),
                distribution.clone(),
            );
        }
        if let Some(distribution) = &measurements.submit_ready_local {
            distributions.insert(
                format!("{workload_id}.submit_ready_local"),
                distribution.clone(),
            );
        }
        if let Some(distribution) = &measurements.position_management_rsocket {
            distributions.insert(
                format!("{workload_id}.position_management_rsocket_request_response"),
                distribution.clone(),
            );
        }
        for (name, distribution) in &measurements.auxiliary_timings {
            distributions.insert(format!("{workload_id}.{name}"), distribution.clone());
        }
        Self {
            hardware,
            git_head,
            build_mode,
            measurement_scope: "AutoTrader production orchestration using one shared AutoTraderState and persisted settings path per workload with deterministic local Position Management RSocket double".into(),
            workload_id: workload_id.to_owned(),
            event_latency_boundary: if workload_id.contains("precomputed-ingress") {
                "run_cycle_inner return after local order reservation; asynchronous submit-ready preparation and RSocket response are separate distributions".into()
            } else {
                "evaluate_position_exit return after production IOC submission response".into()
            },
            measurement_definition: format!(
                "event latency measured at the documented production wrapper boundary; {} total events, {} input candidate events, {} eligible events with {} eligible candidates from cycle diagnostics, {} completed operations, {} order-submitting events, {} rejected/skipped events, {:.3} dispatched events/sec, {:.3} eligible candidate signals/sec, {:.3} submitted-order events/sec; submit-ready and RSocket distributions use {} response timing samples; {} audit telemetry events dropped; concurrency={:?}; distinct_symbols={:?}; max_in_flight={:?}; dispatch_interval_ms={:?}; burst_throughput_events_per_second={:?}; cycle_lock_wait_samples={}",
                measurements.event_count,
                measurements.candidate_event_count,
                measurements.eligible_event_count,
                measurements.eligible_candidate_count,
                measurements.successful_events,
                measurements.submitted_order_events,
                measurements.non_submitted_events,
                measurements.events_per_second,
                if measurements.elapsed_ms > 0.0 {
                    measurements.eligible_candidate_count as f64 * 1_000.0
                        / measurements.elapsed_ms
                } else {
                    0.0
                },
                if measurements.elapsed_ms > 0.0 {
                    measurements.submitted_order_events as f64 * 1_000.0
                        / measurements.elapsed_ms
                } else {
                    0.0
                },
                measurements.response_timing_sample_count,
                measurements.dropped_telemetry_events,
                measurements.concurrent_events,
                measurements.distinct_symbols,
                measurements.max_in_flight,
                measurements.dispatch_interval_ms,
                measurements.burst_throughput_events_per_second,
                measurements.cycle_lock_wait.as_ref().map_or(0, |distribution| distribution.sample_count),
            ),
            benchmark_config_sha256,
            sampling_method: if measurements.concurrent_events.is_some() {
                "concurrent shared-state batch; every dispatched event is timed including rejected/errors; candidate and accepted submission counts are separate; response distributions include events with production timing metadata; bounded-concurrency workload is not a duration soak".into()
            } else {
                "sequential events on one shared AutoTraderState/settings path; every event is timed, response distributions include events with production timing metadata".into()
            },
            execution_runtime_flavor: tokio::runtime::Handle::try_current()
                .map(|handle| format!("{:?}", handle.runtime_flavor()))
                .unwrap_or_else(|_| "unavailable".to_owned()),
            acceptance_state: AUTOTRADER_BENCHMARK_ACCEPTANCE_STATE.to_owned(),
            event_count: measurements.event_count,
            candidate_event_count: measurements.candidate_event_count,
            eligible_event_count: measurements.eligible_event_count,
            eligible_candidate_count: measurements.eligible_candidate_count,
            successful_events: measurements.successful_events,
            submitted_order_events: measurements.submitted_order_events,
            non_submitted_events: measurements.non_submitted_events,
            response_timing_sample_count: measurements.response_timing_sample_count,
            events_per_second: measurements.events_per_second,
            eligible_signals_per_second: if measurements.elapsed_ms > 0.0 {
                measurements.eligible_candidate_count as f64 * 1_000.0
                    / measurements.elapsed_ms
            } else {
                0.0
            },
            submitted_order_rate_per_second: if measurements.elapsed_ms > 0.0 {
                measurements.submitted_order_events as f64 * 1_000.0
                    / measurements.elapsed_ms
            } else {
                0.0
            },
            dropped_telemetry_events: measurements.dropped_telemetry_events,
            concurrent_events: measurements.concurrent_events,
            distinct_symbols: measurements.distinct_symbols,
            max_in_flight: measurements.max_in_flight,
            dispatch_interval_ms: measurements.dispatch_interval_ms,
            burst_throughput_events_per_second: measurements.burst_throughput_events_per_second,
            // The v35 eligible-ingress performance threshold must not count
            // candidates that were rejected/skipped by production gates.
            signals_per_second: if measurements.elapsed_ms > 0.0 {
                measurements.submitted_order_events as f64 * 1_000.0
                    / measurements.elapsed_ms
            } else {
                0.0
            },
            measurements: distributions,
            unsupported_boundaries: AUTOTRADER_UNSUPPORTED_PERFORMANCE_BOUNDARIES
                .iter()
                .map(|boundary| (*boundary).to_owned())
                .collect(),
            exchange_network_latency: None,
        }
    }
}

#[cfg(test)]
mod benchmark_measurement_tests {
    use super::*;

    #[test]
    fn measured_ingress_fixture_preinitializes_intent_schema_before_first_order() {
        let state = benchmark_precomputed_ingress_state("startup-schema-test");
        let connection = rusqlite::Connection::open(&state.settings_path)
            .expect("benchmark settings store must open after production startup persistence");
        let reservation_schema_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='durable_entry_reservations')",
                [],
                |row| row.get(0),
            )
            .expect("durable reservation table lookup must succeed");
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("WAL journal mode must be readable");
        assert!(reservation_schema_exists);
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        drop(connection);
        cleanup_benchmark_fixture_files("startup-schema-test", 1);
    }

    #[test]
    fn percentile_summary_uses_nearest_rank_and_preserves_every_sample() {
        let distribution = BenchmarkLatencyDistribution::from_samples_ms(
            (1..=1000).rev().map(f64::from).collect(),
        );
        assert_eq!(distribution.sample_count, 1000);
        assert_eq!(distribution.p50_ms, 500.0);
        assert_eq!(distribution.p95_ms, 950.0);
        assert_eq!(distribution.p99_ms, 990.0);
        assert_eq!(distribution.p999_ms, 999.0);
    }

    #[test]
    fn production_benchmark_metadata_keeps_submit_boundaries_and_limits_explicit() {
        assert_eq!(AUTOTRADER_SUBMIT_TIMING_BOUNDARIES.len(), 2);
        assert_ne!(
            AUTOTRADER_SUBMIT_TIMING_BOUNDARIES[0],
            AUTOTRADER_SUBMIT_TIMING_BOUNDARIES[1]
        );
        assert!(AUTOTRADER_BENCHMARK_ACCEPTANCE_STATE.starts_with("unaccepted_"));
        assert!(
            AUTOTRADER_UNSUPPORTED_PERFORMANCE_BOUNDARIES
                .iter()
                .any(|boundary| boundary.contains("raw market event ingress"))
        );
    }

    #[test]
    fn response_timing_parser_keeps_local_and_rsocket_samples_at_distinct_boundaries() {
        let response = serde_json::json!({
            "performance_timing_boundaries": {
                "submit_ready_boundary": AUTOTRADER_SUBMIT_TIMING_BOUNDARIES[0],
                "request_prepare_local_ms": 1.25,
                "rsocket_boundary": AUTOTRADER_SUBMIT_TIMING_BOUNDARIES[1],
                "rsocket_request_response_ms": 7.5,
                "acceptance_state": BENCHMARK_ACCEPTANCE_STATE,
            }
        });
        assert_eq!(response_timing_boundaries(&response), Some((1.25, 7.5)));

        let mut mismatched = response;
        mismatched["performance_timing_boundaries"]["rsocket_boundary"] =
            serde_json::json!("exchange_acknowledgement");
        assert_eq!(response_timing_boundaries(&mismatched), None);
    }

    #[tokio::test]
    async fn async_measurement_counts_failed_events_in_the_sample_population() {
        let (distribution, elapsed, successful) =
            measure_async_benchmark_events(4, |index| async move {
                if index == 1 {
                    Err("deterministic rejection")
                } else {
                    Ok(index)
                }
            })
            .await;
        assert_eq!(distribution.sample_count, 4);
        assert_eq!(successful, 3);
        assert!(!elapsed.is_zero());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn local_rsocket_double_runs_production_paths_and_emits_unaccepted_evidence() {
        const EVENT_COUNT: usize = 128;
        let git_head_output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git rev-parse HEAD must run to produce benchmark evidence metadata");
        assert!(
            git_head_output.status.success(),
            "git rev-parse HEAD failed: {}",
            String::from_utf8_lossy(&git_head_output.stderr)
        );
        let git_head = String::from_utf8_lossy(&git_head_output.stdout)
            .trim()
            .to_owned();
        assert!(
            !git_head.is_empty(),
            "git rev-parse HEAD returned an empty hash"
        );
        let source_status = std::process::Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=normal"])
            .output()
            .expect("git status must run to determine the benchmark source-tree state");
        assert!(
            source_status.status.success(),
            "git status failed: {}",
            String::from_utf8_lossy(&source_status.stderr)
        );
        let source_tree_dirty = !source_status.stdout.is_empty();
        let mut records = run_local_production_path_benchmarks(
            EVENT_COUNT,
            format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
            git_head,
            "release".into(),
        )
        .await
        .expect("deterministic local fixture should run both production paths");
        assert_eq!(records.len(), 5);
        for record in &mut records {
            record
                .sampling_method
                .push_str(&format!("; source_tree_dirty={source_tree_dirty}"));
        }
        println!(
            "AUTOTRADER_PERFORMANCE_EVIDENCE_JSON={}",
            serde_json::to_string(&records).expect("evidence JSON must serialize")
        );
        for record in records {
            assert_eq!(
                record.acceptance_state,
                AUTOTRADER_BENCHMARK_ACCEPTANCE_STATE
            );
            assert!(record.execution_runtime_flavor.contains("MultiThread"));
            assert!(
                record
                    .measurement_scope
                    .contains("local Position Management RSocket double")
            );
            assert!(
                record
                    .measurement_scope
                    .contains("one shared AutoTraderState")
            );
            assert!(
                record
                    .unsupported_boundaries
                    .iter()
                    .any(|boundary| { boundary.contains("raw market event ingress") })
            );
            assert_eq!(record.event_count, EVENT_COUNT);
            assert!(record.successful_events <= EVENT_COUNT);
            assert!(record.submitted_order_events <= EVENT_COUNT);
            assert!(record.candidate_event_count <= EVENT_COUNT);
            assert!(record.eligible_event_count <= record.candidate_event_count);
            assert!(record.eligible_candidate_count <= record.candidate_event_count);
            if record.workload_id.contains("precomputed-ingress") {
                assert_eq!(record.candidate_event_count, EVENT_COUNT);
                assert!(record.submitted_order_events <= record.eligible_candidate_count);
            } else {
                assert_eq!(record.candidate_event_count, 0);
                assert_eq!(record.eligible_candidate_count, 0);
            }
            assert_eq!(
                record.non_submitted_events,
                EVENT_COUNT - record.submitted_order_events
            );
            assert!(record.events_per_second.is_finite() && record.events_per_second > 0.0);
            assert!(record.signals_per_second.is_finite() && record.signals_per_second >= 0.0);
            assert!(
                record.signals_per_second <= record.events_per_second + 1e-9,
                "submitted-order throughput must not exceed event throughput"
            );
            assert!(
                (record.signals_per_second
                    - record.events_per_second * record.submitted_order_events as f64
                        / record.event_count as f64)
                    .abs()
                    < 1e-6
            );
            assert!(record.eligible_signals_per_second.is_finite());
            assert_eq!(
                record.submitted_order_rate_per_second, record.signals_per_second,
                "submitted-order rate has an explicit name while retaining the old alias"
            );
            assert!(
                (record.eligible_signals_per_second
                    - record.events_per_second * record.eligible_candidate_count as f64
                        / record.event_count as f64)
                    .abs()
                    < 1e-9
            );
            assert!(record.response_timing_sample_count <= EVENT_COUNT);
            assert!(record.response_timing_sample_count <= record.submitted_order_events);
            let burst = record.concurrent_events.is_some();
            for (name, distribution) in &record.measurements {
                if name.ends_with(".event_latency") || name.ends_with(".queue_wait") {
                    assert_eq!(distribution.sample_count, EVENT_COUNT);
                } else if name.ends_with(".cycle_lock_wait")
                    || name.contains(".cycle_lock_hold.")
                    || name.contains(".cycle_lock_scope.")
                    || name.contains(".settings_persistence.")
                {
                    assert!(distribution.sample_count > 0);
                } else {
                    assert_eq!(
                        distribution.sample_count,
                        record.response_timing_sample_count
                    );
                }
            }
            if record.workload_id.contains("multithread-sustained") {
                // Persistence sub-stages must retain their own sample counts;
                // none may be synthesized from the order/event population.
                for stage in [
                    "entry_intent_batch_size",
                    "entry_intent_writer_lock_wait",
                    "entry_intent_serialization",
                    "entry_intent_directory_prepare",
                    "entry_intent_connection_open",
                    "entry_intent_schema_setup",
                    "entry_intent_sqlite_lock_wait",
                    "entry_intent_sql_prepare",
                    "entry_intent_sql_execute",
                    "entry_intent_durable_commit",
                    "entry_intent_sqlite_write_total",
                ] {
                    let distribution = record
                        .measurements
                        .iter()
                        .find(|(name, _)| name.ends_with(&format!(".settings_persistence.{stage}")))
                        .unwrap_or_else(|| panic!("missing persistence timing stage {stage}"));
                    assert!(distribution.1.sample_count > 0, "{stage}");
                }
            }
            if burst {
                assert_eq!(record.concurrent_events, Some(EVENT_COUNT));
                if record.workload_id.contains("ingress-burst")
                    && !record.workload_id.contains("multithread")
                {
                    assert_eq!(record.distinct_symbols, Some(4));
                    assert!(record.submitted_order_events <= 4);
                } else {
                    assert_eq!(
                        record.distinct_symbols,
                        Some(strategy::autotrader_allowed_symbols().len())
                    );
                    assert!(record.eligible_event_count > 0);
                    assert!(record.eligible_candidate_count > 0);
                    assert!(record.submitted_order_events > 0);
                    assert!(record.submitted_order_events <= record.eligible_candidate_count);
                }
                assert!(record.response_timing_sample_count <= record.submitted_order_events);
                assert!(
                    record
                        .burst_throughput_events_per_second
                        .unwrap_or_default()
                        > 0.0
                );
                let queue = record
                    .measurements
                    .iter()
                    .find(|(name, _)| name.ends_with(".queue_wait"))
                    .expect("burst queue wait distribution is required");
                assert_eq!(queue.1.sample_count, EVENT_COUNT);
                if record.workload_id.contains("multithread-sustained") {
                    assert!(record.sampling_method.contains("not a duration soak"));
                    assert_eq!(record.max_in_flight, Some(16));
                    assert_eq!(record.dispatch_interval_ms, Some(8));
                }
            } else if record.workload_id.contains("emergency-ioc-exit") {
                assert_eq!(record.response_timing_sample_count, EVENT_COUNT);
            } else {
                assert!(record.response_timing_sample_count <= EVENT_COUNT);
            }
            assert!(
                record
                    .measurements
                    .contains_key(&format!("{}.event_latency", record.workload_id))
            );
            let encoded = serde_json::to_vec(&record).expect("evidence must be machine-readable");
            let decoded: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(
                decoded["acceptance_state"],
                AUTOTRADER_BENCHMARK_ACCEPTANCE_STATE
            );
            assert_eq!(
                decoded["unsupported_boundaries"][0],
                AUTOTRADER_UNSUPPORTED_PERFORMANCE_BOUNDARIES[0]
            );
        }
    }
}

/// Benchmark AutoTrader-only paths that are outside the reusable strategy crate.
pub fn benchmark_autotrader_paths(iterations: usize) -> Vec<(String, f64)> {
    use std::{hint::black_box, time::Instant};
    assert!(iterations > 0, "benchmark iterations must be positive");
    let signal = benchmark_signal("donchian-55-btc200", true);
    let parameters = ExecutionParameters::default();
    let signal_payload = serde_json::json!({ "symbol": signal.symbol, "direction": signal.direction,
        "strength": signal.strength, "timestamp": signal.timestamp, "strategy": signal.strategy,
        "strategy_decision": signal.strategy_decision });
    let market = serde_json::json!({
        "bids": (0..20).map(|i| serde_json::json!({"price": 100.0 - i as f64 * 0.01, "size": 1.0 + i as f64})).collect::<Vec<_>>(),
        "asks": (0..20).map(|i| serde_json::json!({"price": 100.02 + i as f64 * 0.01, "size": 1.5 + i as f64})).collect::<Vec<_>>()
    });
    let mut results = Vec::new();
    let started = Instant::now();
    for _ in 0..iterations {
        black_box(serde_json::to_vec(black_box(&signal_payload)).unwrap());
    }
    results.push((
        "signal serialization".into(),
        started.elapsed().as_nanos() as f64 / iterations as f64,
    ));
    let started = Instant::now();
    for _ in 0..iterations {
        black_box(assess_deep_book(
            black_box(&market),
            "BTC-USDT",
            true,
            2_000.0,
            10.0,
        ));
    }
    results.push((
        "market depth assessment".into(),
        started.elapsed().as_nanos() as f64 / iterations as f64,
    ));
    let started = Instant::now();
    for index in 0..iterations {
        black_box(build_order_request(
            "BTC-USDT",
            "buy",
            0.25,
            100.0,
            100.02,
            AutoOrderType::Limit,
            parameters,
            "bench-order",
            index as u64,
            false,
            None,
        ));
    }
    results.push((
        "limit order request construction".into(),
        started.elapsed().as_nanos() as f64 / iterations as f64,
    ));
    let overrides = BTreeMap::new();
    let started = Instant::now();
    for _ in 0..iterations {
        black_box(validate_strategy_parameter_overrides(black_box(&overrides)).unwrap());
    }
    results.push((
        "strategy parameter validation".into(),
        started.elapsed().as_nanos() as f64 / iterations as f64,
    ));
    results
}

/// Measure the production signal settings lock/gate/profile/snapshot path with
/// deterministic retained-order fixtures, including an empty and full ledger.
pub fn benchmark_signal_settings_snapshot_path(iterations: usize) -> Vec<(String, f64)> {
    use std::{hint::black_box, time::Instant};

    assert!(iterations > 0, "benchmark iterations must be positive");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build benchmark runtime");
    let signal = benchmark_signal("live-momentum", false);
    let mut results = Vec::new();

    for retained_orders in [0, 256, MAX_RETAINED_AUTO_TRADER_ORDERS] {
        let mut settings = Settings::default();
        settings.orders = (0..retained_orders)
            .map(|index| {
                serde_json::from_value(serde_json::json!({
                    "timestamp": index,
                    "symbol": format!("BENCH-{index}-USDT"),
                    "side": "buy",
                    "order_type": "Limit",
                    "purpose": "signal",
                    "strategy": "live-momentum",
                    "quantity": 1.0,
                    "filled_quantity": 1.0,
                    "average_price": 100.0,
                    "status": "filled",
                    "decision_id": format!("benchmark-decision-{index}"),
                    "processed_fill_trade_ids": [format!("benchmark-fill-{index}")],
                    "realized_pnl": 0.0
                }))
                .expect("deserialize benchmark order")
            })
            .collect();
        let state = benchmark_snapshot_state(settings);
        let us_per_operation = runtime.block_on(async {
            snapshot_signal_settings(&state, &signal, u64::MAX - 1, false)
                .await
                .expect("benchmark warmup signal must pass position gates");
            let started = Instant::now();
            for _ in 0..iterations {
                black_box(
                    snapshot_signal_settings(&state, &signal, u64::MAX - 1, false)
                        .await
                        .expect("benchmark signal must pass position gates"),
                );
            }
            started.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64
        });
        results.push((
            format!("signal settings snapshot ({retained_orders} retained orders)"),
            us_per_operation,
        ));
    }
    results
}

/// Benchmark the decision-parameters route's learning-state snapshot, profile
/// construction, walk-forward evidence lookup, and JSON encoding.
pub fn benchmark_decision_parameters_path(
    iterations: usize,
    retained_variants: usize,
) -> (f64, f64, f64, usize, usize) {
    use std::{hint::black_box, time::Instant};

    assert!(iterations > 0, "benchmark iterations must be positive");
    assert!(
        retained_variants <= 20_000,
        "fixture supports at most 20,000 variants"
    );
    let mut settings = Settings::default();
    // Model a persisted learning state with many historical evidence entries;
    // only a small subset belongs to the profiles returned by this endpoint.
    for index in 0..20_000 {
        let symbol = format!("BENCH-{}-USDT", index % 16);
        settings.learned_walk_forward_evidence.insert(
            format!("historical-{index}|live-momentum"),
            strategy::StrategyWalkForwardEvidence {
                evidence_contract_version: 2,
                estimator: strategy::StrategyWalkForwardEstimator::CompletedOutcomeClusters,
                history_coverage: strategy::StrategyWalkForwardCoverage::Unknown,
                cost_coverage: strategy::StrategyWalkForwardCoverage::Unknown,
                search_family_coverage: strategy::StrategyWalkForwardCoverage::Unknown,
                point_in_time_cutoff_timestamp_ms: None,
                final_holdout_status: strategy::StrategyWalkForwardValidationStatus::NotValidated,
                oos_fold_count: 0,
                spa_p_value: None,
                sample_count: index,
                positive_net_rate: Some(0.51),
                average_net_return_percent: Some(0.12),
                last_sample_timestamp_ms: index as u64,
                expectancy_ci_low_percent: None,
                expectancy_ci_high_percent: None,
                effective_sample_size: Some(index as f64),
                window_count: 0,
                max_drawdown_percent: None,
                downside_loss_percent: None,
                evidence_scope: strategy::StrategyWalkForwardEvidenceScope::Unknown,
            },
        );
        if index < retained_variants {
            settings
                .learned_profiles
                .entry(format!("{symbol}:live-momentum"))
                .or_insert_with(|| strategy::StrategyProfile {
                    profile_id: format!("{symbol}:live-momentum"),
                    version: 1,
                    symbol: symbol.clone(),
                    strategy: "live-momentum".into(),
                    parameters: BTreeMap::new(),
                    learning_samples: 0,
                    positive_samples: 0,
                    realized_pnl: 0.0,
                });
            settings
                .learned_scout_parameter_variants
                .push(strategy::ScoutParameterVariant {
                    symbol,
                    strategy: "live-momentum".into(),
                    market_regime: None,
                    baseline_profile_id: String::new(),
                    parameter: "minimum_momentum".into(),
                    candidate_value: 0.01,
                    sample_count: index,
                    minimum_samples: 10,
                    average_net_improvement_percent: 0.02,
                    lower_90_one_sided_improvement_percent: Some(0.01),
                    upper_90_one_sided_improvement_percent: Some(0.03),
                    paired_baseline_increase_samples: 5,
                    paired_baseline_decrease_samples: 4,
                    paired_baseline_same_samples: 1,
                    paired_baseline_unknown_samples: 0,
                    paired_baseline_delta_mean: Some(0.02),
                    paired_baseline_delta_samples: 10,
                    promoted: false,
                    rejected: false,
                });
        }
    }
    let symbols = strategy_profile_symbols(&settings);
    let strategies = Strategy::all()
        .filter(|strategy| !strategy.is_legacy_composite())
        .collect::<Vec<_>>();
    let mut output_bytes = 0;
    let mut compact_bytes = 0;
    let mut output_variants = 0;
    let snapshot_time = std::time::Duration::ZERO;
    let mut profile_time = std::time::Duration::ZERO;
    let mut json_time = std::time::Duration::ZERO;
    for _ in 0..iterations {
        let started = Instant::now();
        let mut profiles = strategy_parameter_profiles_for(
            black_box(&settings),
            symbols.clone(),
            strategies.clone(),
        );
        for profile in &mut profiles {
            let profile_strategy = Strategy::from_name(&profile.strategy).unwrap_or_default();
            profile.walk_forward_evidence = walk_forward_evidence_for_profile(
                &settings,
                &profile.symbol,
                &profile.symbol,
                profile_strategy,
            );
            profile.walk_forward_evidence_by_regime = walk_forward_evidence_by_regime_for_profile(
                &settings,
                &profile.symbol,
                &profile.symbol,
                profile_strategy,
            );
        }
        output_variants = profiles
            .iter()
            .flat_map(|profile| &profile.parameters)
            .map(|parameter| parameter.scout_variants.len())
            .sum();
        profile_time += started.elapsed();

        let started = Instant::now();
        let encoded = serde_json::to_vec(black_box(&profiles)).unwrap();
        output_bytes = encoded.len();
        black_box(encoded);
        json_time += started.elapsed();

        let compact_started = Instant::now();
        let catalog = strategies
            .iter()
            .map(|strategy| {
                (
                    strategy.as_str().to_owned(),
                    strategy
                        .parameter_specs()
                        .iter()
                        .map(|spec| {
                            (
                                spec.key.as_str(),
                                spec.label,
                                spec.default,
                                spec.min,
                                spec.max,
                                spec.step,
                                spec.unit,
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let values = profiles
            .iter()
            .map(|profile| {
                (
                    &profile.symbol,
                    &profile.strategy,
                    profile.learning_samples,
                    profile.positive_rate,
                    profile.realized_pnl,
                    profile.walk_forward_evidence.as_ref(),
                    &profile.walk_forward_evidence_by_regime,
                    profile
                        .parameters
                        .iter()
                        .map(|parameter| {
                            (
                                &parameter.key,
                                parameter.value,
                                parameter.learned_value,
                                parameter.configured_value,
                                parameter.overridden,
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let compact = serde_json::to_vec(black_box(&(catalog, values))).unwrap();
        compact_bytes = compact.len();
        black_box(compact);
        black_box(compact_started.elapsed());
    }
    let to_us =
        |elapsed: std::time::Duration| elapsed.as_secs_f64() * 1_000_000.0 / iterations as f64;
    (
        to_us(snapshot_time),
        to_us(profile_time),
        to_us(json_time),
        output_bytes.saturating_sub(compact_bytes),
        output_variants,
    )
}

fn benchmark_snapshot_state(settings: Settings) -> AutoTraderState {
    let (state_updates, _) = tokio::sync::broadcast::channel(1);
    let path = std::env::temp_dir().join(format!(
        "auto-trader-lock-bench-{}.json",
        std::process::id()
    ));
    AutoTraderState {
        latest_live_strategy_signals: Arc::new(Mutex::new(BTreeMap::new())),
        strategy_diagnostic: Arc::new(std::sync::Mutex::new(Vec::new())),
        latest_strategy_blockers: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        latest_worker_decisions: Arc::new(Mutex::new(BTreeMap::new())),
        latest_worker_diagnostics: Arc::new(Mutex::new(BTreeMap::new())),
        seen_worker_decision_events: Arc::new(Mutex::new(BTreeMap::new())),
        enabled: Arc::new(AtomicBool::new(true)),
        settings: Arc::new(Mutex::new(settings)),
        activity_snapshot: Arc::new(Mutex::new(None)),
        settings_persist_notify: Arc::new(Notify::new()),
        settings_persist_write_lock: Arc::new(Mutex::new(())),
        settings_persist_revision: Arc::new(AtomicU64::new(0)),
        settings_persist_durable_revision: Arc::new(AtomicU64::new(0)),
        settings_persist_durable_waiters: Arc::new(AtomicU64::new(0)),
        settings_persist_batch_notify: Arc::new(Notify::new()),
        durable_entry_reservation_batch: Arc::new(Mutex::new(Vec::new())),
        durable_entry_reservation_batch_scheduled: Arc::new(AtomicBool::new(false)),
        durable_feedback_event_ids: Arc::new(Mutex::new(BTreeSet::new())),
        price_path_last_checkpoint_requested_at: Arc::new(std::sync::Mutex::new(None)),
        state_updates,
        live_profile_updates: tokio::sync::broadcast::channel(1).0,
        settings_persist_timings_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        latest_missed_opportunity_bands: Arc::new(Mutex::new(Arc::new(Vec::new()))),
        latest_execution_parameters: Arc::new(Mutex::new(None)),
        cycle_lock: Arc::new(Mutex::new(())),
        pending_daily_signals: Arc::new(Mutex::new(BTreeMap::new())),
        daily_signal_notify: Arc::new(Notify::new()),
        pending_live_signals: Arc::new(Mutex::new(BTreeMap::new())),
        live_signal_notify: Arc::new(Notify::new()),
        audit_tx: AuditQueue::new(1, 1),
        execution_audit_tx: AuditQueue::new(1, 1),
        audit_health: Arc::new(AtomicU64::new(3)),
        audit_failure_latched: Arc::new(AtomicBool::new(false)),
        gap_summary: Arc::new(std::sync::Mutex::new(GapSummary::default())),
        stale_signal_candidates_total: Arc::new(AtomicU64::new(0)),
        stale_signal_cycles_total: Arc::new(AtomicU64::new(0)),
        insight_processing_ms: Arc::new(Mutex::new(Vec::new())),
        cycle_lock_wait_ms: Arc::new(Mutex::new(BTreeMap::new())),
        cycle_lock_hold_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        cycle_lock_scope_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        settings_path: path.clone(),
        dated_carry: Arc::new(Mutex::new(DatedCarryBook::default())),
        dated_carry_work_lock: Arc::new(Mutex::new(())),
        dated_carry_path: path.with_extension("dated_carry.json"),
    }
}
