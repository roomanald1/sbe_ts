//! Cross-boundary lifecycle fixture for AutoTrader's real selected-entry and
//! exit paths. The local RSocket responder models service transport contracts;
//! the Position Management idempotency check verifies the stable key AutoTrader
//! supplies, not Position Management's own SQLite implementation.

use super::*;
use http_body_util::BodyExt;
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use std::{collections::BTreeMap, pin::Pin, sync::Mutex as StdMutex};
use tower::ServiceExt;

#[derive(Clone, Default)]
struct LifecycleResponder {
    requests: Arc<StdMutex<Vec<crate::service_rpc::RpcRequest>>>,
    learning_routes: Arc<StdMutex<Vec<strategy::ScoutCoordinatorRouteCommand>>>,
    learning_router: Arc<StdMutex<axum::Router>>,
    idempotent_orders: Arc<StdMutex<BTreeMap<String, serde_json::Value>>>,
    barriers: Arc<StdMutex<BTreeMap<String, serde_json::Value>>>,
    accepted_order_count: Arc<std::sync::atomic::AtomicUsize>,
}

#[rsocket_rust::async_trait]
impl RSocket for LifecycleResponder {
    async fn metadata_push(&self, _: Payload) -> anyhow::Result<()> {
        anyhow::bail!("unsupported")
    }

    async fn fire_and_forget(&self, _: Payload) -> anyhow::Result<()> {
        anyhow::bail!("unsupported")
    }

    async fn request_response(&self, payload: Payload) -> anyhow::Result<Option<Payload>> {
        if let Ok(command) = serde_json::from_slice::<strategy::ScoutCoordinatorRequest>(
            payload
                .data()
                .ok_or_else(|| anyhow::anyhow!("request data missing"))?
                .as_ref(),
        ) {
            let strategy::ScoutCoordinatorRequest::LearningRoute(command) = command else {
                anyhow::bail!("lifecycle fixture expected a Learning route command");
            };
            self.learning_routes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(command.clone());
            let method = match command.method {
                strategy::ScoutWorkerHttpMethod::Get => "GET",
                strategy::ScoutWorkerHttpMethod::Post => "POST",
                strategy::ScoutWorkerHttpMethod::Put => "PUT",
                strategy::ScoutWorkerHttpMethod::Delete => "DELETE",
            };
            let request = axum::http::Request::builder()
                .method(method)
                .uri(&command.path)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::to_vec(
                    &command.payload,
                )?))?;
            let router = {
                self.learning_router
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone()
            };
            let response = router.oneshot(request).await?;
            let status = response.status().as_u16();
            let body = response.into_body().collect().await?.to_bytes();
            // Preserve non-JSON router errors as strings so lifecycle tests
            // report the actual HTTP status/reason instead of a transport
            // decode failure.
            let body = serde_json::from_slice(&body).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(&body).into_owned())
            });
            let response = strategy::ScoutCoordinatorResponse::LearningRoute(
                strategy::ScoutCoordinatorRouteResult {
                    request_id: command.request_id,
                    status,
                    body,
                },
            );
            return Ok(Some(
                Payload::builder()
                    .set_data(serde_json::to_vec(&response)?)
                    .build(),
            ));
        }
        let request: crate::service_rpc::RpcRequest = serde_json::from_slice(
            payload
                .data()
                .ok_or_else(|| anyhow::anyhow!("request data missing"))?
                .as_ref(),
        )?;
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request.clone());

        let (status, body) = match (request.method.as_str(), request.path.as_str()) {
            // The real Position Management service owns validation, fill
            // accounting, durable idempotency and ledger writes. This local
            // Paper Exchange responder supplies only a deterministic market
            // match so the integration fixture has no feed/runtime dependency.
            ("POST", "/internal/orders/match") => {
                let body = request.body.unwrap_or_default();
                let quantity = body["quantity"].as_f64().unwrap_or_default();
                let price = body["limit_price"]
                    .as_f64()
                    .unwrap_or_else(|| if body["side"] == "buy" { 100.0 } else { 101.0 });
                (
                    200,
                    serde_json::json!({
                        "reference_price": price,
                        "best_bid": 99.0,
                        "best_ask": 102.0,
                        "worst_ask": 102.0,
                        "book_updated_at_ms": 1_700_000_000_000u64,
                        "fills": [{"quantity": quantity, "price": price}],
                        "remaining_quantity": 0.0,
                        "filled_quantity": quantity,
                        "average_execution_price": price
                    }),
                )
            }
            ("POST", "/positions/orders") => {
                let body = request.body.unwrap_or_default();
                if body["order_type"] == "stop-market" {
                    let parent_order_id = body["client_order_id"]
                        .as_str()
                        .unwrap_or("fixture-stop")
                        .to_owned();
                    let barrier_id = format!("fixture-barrier-{parent_order_id}");
                    let created_at = current_time_ms();
                    let barrier_order = serde_json::json!({
                        "id":barrier_id,
                        "created_at":created_at,
                        "symbol":body["symbol"],
                        "side":body["side"],
                        "order_kind":"stop-market",
                        "execution_type":"market",
                        "quantity":body["quantity"],
                        "remaining_quantity":body["quantity"],
                        "filled_quantity":0.0,
                        "average_execution_price":null,
                        "fee_paid":0.0,
                        "trigger_price":body["trigger_price"],
                        "trigger_direction":body["trigger_direction"],
                        "source":"auto",
                        "status":"armed",
                        "parent_order_id":parent_order_id,
                        "client_order_id":body["client_order_id"],
                        "lifecycle":body["lifecycle"],
                        "trigger_adjustments":[]
                    });
                    self.barriers
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(barrier_id.clone(), barrier_order.clone());
                    (
                        200,
                        serde_json::json!({
                            "status": "armed",
                            "filled_quantity": 0.0,
                            "average_execution_price": null,
                            "estimated_fee": 0.0,
                            "barrier_order": barrier_order
                        }),
                    )
                } else {
                    let client_order_id = body
                        .get("client_order_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let mut idempotent = self
                        .idempotent_orders
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let response = idempotent
                        .entry(client_order_id.clone())
                        .or_insert_with(|| {
                            let fill_sequence = self
                                .accepted_order_count
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                                + 1;
                            let execution_price = if body["side"] == "buy" { 100.0 } else { 101.0 };
                            serde_json::json!({
                                "status": "filled",
                                "filled_quantity": body["quantity"],
                                "average_execution_price": execution_price,
                                "estimated_fee": 0.01,
                                "fee_complete": true,
                                "unknown_fee_fills": 0,
                                "fill_evidence": [{
                                    "id": format!("fixture-fill-{client_order_id}"),
                                    "fill_id": format!("pm-fill:v1:fixture:{fill_sequence}"),
                                    "symbol": body["symbol"],
                                    "side": body["side"],
                                    "quantity": body["quantity"],
                                    "execution_price": execution_price,
                                    "timestamp": 1_700_000_000_000u64,
                                    "source": "auto",
                                    "parent_order_id": client_order_id,
                                    "client_order_id": client_order_id,
                                    "lifecycle": body["lifecycle"],
                                    "fee": 0.01,
                                    "fee_known": true,
                                    "fee_currency": "USDT"
                                }],
                                // A fully executed post-only request has no resting
                                // remainder. AutoTrader treats an order object as a
                                // live/resting order even when the fill is complete.
                                "order": null
                            })
                        });
                    (200, response.clone())
                }
            }
            ("GET", "/positions/barriers") => (
                200,
                serde_json::json!(
                    self.barriers
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .values()
                        .cloned()
                        .collect::<Vec<_>>()
                ),
            ),
            ("POST", path)
                if path.starts_with("/positions/orders/") && path.ends_with("/trigger") =>
            {
                let barrier_id = path
                    .strip_prefix("/positions/orders/")
                    .and_then(|path| path.strip_suffix("/trigger"))
                    .unwrap_or_default();
                let body = request.body.unwrap_or_default();
                let mut barriers = self
                    .barriers
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(barrier) = barriers.get_mut(barrier_id) {
                    barrier["trigger_price"] = body["new_trigger_price"].clone();
                    let adjusted_at_ms = current_time_ms();
                    barrier["trigger_adjustments"].as_array_mut().unwrap().push(
                        serde_json::json!({
                            "request_id":body["request_id"],
                            "barrier_id":barrier_id,
                            "symbol":body["symbol"],
                            "side":body["side"],
                            "quantity":body["quantity"],
                            "parent_order_id":body["expected_parent_order_id"],
                            "lifecycle_trade_id":body["expected_lifecycle_trade_id"],
                            "profile_id":body["profile_id"],
                            "profile_version":body["profile_version"],
                            "policy_source":body["policy_source"],
                            "reason":body["reason"],
                            "target_net_return_percent":body["target_net_return_percent"],
                            "expected_trigger_price":body["expected_trigger_price"],
                            "new_trigger_price":body["new_trigger_price"],
                            "adjusted_at_ms":adjusted_at_ms
                        }),
                    );
                    (
                        200,
                        serde_json::json!({
                            "barrier_id":barrier_id,
                            "request_id":body["request_id"],
                            "trigger_price":body["new_trigger_price"],
                            "adjusted_at_ms":adjusted_at_ms,
                            "idempotent_replay":false
                        }),
                    )
                } else {
                    (404, serde_json::json!({"error":"barrier missing"}))
                }
            }
            ("DELETE", _) => (200, serde_json::json!({})),
            _ => (
                404,
                serde_json::json!({"error": "unexpected lifecycle fixture route"}),
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

    fn request_stream(
        &self,
        _: Payload,
    ) -> Pin<Box<dyn futures_util::Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
        Box::pin(futures_util::stream::empty())
    }

    fn request_channel(&self, _: Flux<anyhow::Result<Payload>>) -> Flux<anyhow::Result<Payload>> {
        Box::pin(futures_util::stream::empty())
    }
}

#[derive(Clone)]
struct DropFirstPositionAckProxy {
    position_management_endpoint: String,
    drop_next_submission_response: Arc<std::sync::atomic::AtomicBool>,
    dropped_submission_responses: Arc<std::sync::atomic::AtomicUsize>,
}

#[rsocket_rust::async_trait]
impl RSocket for DropFirstPositionAckProxy {
    async fn metadata_push(&self, _: Payload) -> anyhow::Result<()> {
        anyhow::bail!("unsupported")
    }

    async fn fire_and_forget(&self, _: Payload) -> anyhow::Result<()> {
        anyhow::bail!("unsupported")
    }

    async fn request_response(&self, payload: Payload) -> anyhow::Result<Option<Payload>> {
        let request: crate::service_rpc::RpcRequest = serde_json::from_slice(
            payload
                .data()
                .ok_or_else(|| anyhow::anyhow!("position request data missing"))?
                .as_ref(),
        )?;
        let response = crate::service_rpc::position_request_at(
            &self.position_management_endpoint,
            &request.method,
            &request.path,
            request.body.clone(),
        )
        .await?;
        let is_submission = request.method == "POST"
            && matches!(
                request.path.as_str(),
                "/positions/orders" | "/positions/vwap"
            );
        if is_submission
            && self
                .drop_next_submission_response
                .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            self.dropped_submission_responses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // PM has durably handled the request; the AutoTrader client sees
            // the same no-payload transport ambiguity as a lost acknowledgment.
            return Ok(None);
        }
        Ok(Some(
            Payload::builder()
                .set_data(serde_json::to_vec(&response)?)
                .build(),
        ))
    }

    fn request_stream(
        &self,
        _: Payload,
    ) -> Pin<Box<dyn futures_util::Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
        Box::pin(futures_util::stream::empty())
    }

    fn request_channel(&self, _: Flux<anyhow::Result<Payload>>) -> Flux<anyhow::Result<Payload>> {
        Box::pin(futures_util::stream::empty())
    }
}

fn unique_path(label: &str) -> PathBuf {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    std::env::temp_dir().join(format!(
        "auto-trader-lifecycle-{}-{}-{label}.json",
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ))
}

fn versioned_lifecycle_dashboard(fixture_index: usize, source_key: &str) -> Dashboard {
    let mut dashboard = benchmark_precomputed_ingress_dashboard(fixture_index, "BTC-USDT");
    let signal = dashboard
        .best
        .first_mut()
        .expect("benchmark dashboard contains one selected signal");
    let strategy_name = signal
        .strategy
        .as_deref()
        .expect("benchmark signal has a registered strategy");
    let strategy_kind = strategy::Strategy::from_name(strategy_name)
        .expect("benchmark strategy name is registered");
    let parameters = serde_json::from_value::<BTreeMap<String, f64>>(
        signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("strategy_parameters"))
            .cloned()
            .expect("benchmark signal includes strategy parameters"),
    )
    .expect("benchmark strategy parameters are numeric");
    let profile_id = strategy::profile_id(&signal.symbol, strategy_name, &parameters);
    let decision = signal
        .strategy_decision
        .get_or_insert_with(|| serde_json::json!({}));
    decision["learning_profile_id"] = serde_json::json!(profile_id);
    // The deterministic fixture uses the built-in version-zero profile; live
    // workers send the same field from their authoritative profile snapshot.
    decision["learning_profile_version"] = serde_json::json!(0);
    let source_event_id = format!("insights-event:v1:sha256:{source_key}");
    let identity = strategy::LiveStrategyEventIdentity::for_thesis(
        &source_event_id,
        strategy_kind,
        &signal.symbol,
        &profile_id,
        &signal.direction,
        None,
    );
    decision["live_worker_event_id"] = serde_json::json!(identity.event_id);
    decision["live_worker_event_identity_version"] = serde_json::json!(identity.schema_version);
    decision["live_worker_episode_id"] = serde_json::json!(identity.episode_id);
    dashboard
}

fn exit_profile_policy(
    symbol: &str,
    strategy: &str,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    as_of_ms: u64,
) -> strategy::LearnedExitProtectionPolicy {
    strategy::LearnedExitProtectionPolicy {
        symbol: symbol.to_owned(),
        strategy: strategy.to_owned(),
        profile_id: None,
        profile_version: None,
        stop_loss_percent: Some(stop_loss_percent),
        take_profit_percent: Some(take_profit_percent),
        activation_percent: 0.20,
        giveback_percent: 0.15,
        minimum_lock_percent: 0.10,
        maximum_holding_duration_ms: None,
        trailing_activation_percent: None,
        trailing_distance_percent: None,
        breakeven_activation_percent: None,
        sample_count: 40,
        training_count: 30,
        validation_count: 10,
        baseline_validation_mean_percent: 0.10,
        selected_validation_mean_percent: 0.15,
        validation_gain_percent: 0.05,
        validation_positive_rate: 0.70,
        validation_negative_difference_rate: 0.20,
        validation_tenth_percentile_difference_percent: -0.10,
        validation_worst_difference_percent: -0.25,
        as_of_ms,
    }
}

fn profile_exit_signal(symbol: &str, strategy: &str, timestamp: u64) -> Signal {
    Signal {
        range_location: None,
        symbol: symbol.to_owned(),
        direction: "bullish".into(),
        strength: 1.0,
        timestamp,
        stale: false,
        score: 1.0,
        ranging_probability: None,
        components: SignalComponents::default(),
        strategy_candidate: true,
        strategy: Some(strategy.to_owned()),
        strategy_decision: Some(serde_json::json!({})),
        market_regime: None,
        market_state: None,
        news_attribution: None,
        entry_gate: Some(serde_json::json!({"selection_mode":"exploratory"})),
    }
}

fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
    // SAFETY: this fixture serializes its own environment mutations and restores
    // each variable before returning. The test harness is single-process.
    unsafe {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
}

fn assert_json_close(expected: &serde_json::Value, actual: &serde_json::Value, path: &str) {
    match (expected, actual) {
        (serde_json::Value::Object(expected), serde_json::Value::Object(actual)) => {
            assert_eq!(
                expected.keys().collect::<Vec<_>>(),
                actual.keys().collect::<Vec<_>>(),
                "{path} keys"
            );
            for (key, expected_value) in expected {
                assert_json_close(expected_value, &actual[key], &format!("{path}.{key}"));
            }
        }
        (serde_json::Value::Array(expected), serde_json::Value::Array(actual)) => {
            assert_eq!(expected.len(), actual.len(), "{path} array length");
            for (index, (expected_value, actual_value)) in expected.iter().zip(actual).enumerate() {
                assert_json_close(expected_value, actual_value, &format!("{path}[{index}]"));
            }
        }
        (serde_json::Value::Number(expected), serde_json::Value::Number(actual)) => {
            let expected = expected.as_f64().expect("finite JSON number");
            let actual = actual.as_f64().expect("finite JSON number");
            let tolerance = 1e-12 * expected.abs().max(1.0);
            assert!(
                (expected - actual).abs() <= tolerance,
                "{path}: expected {expected}, got {actual}"
            );
        }
        _ => assert_eq!(expected, actual, "{path}"),
    }
}

async fn learning_json_request(
    router: &axum::Router,
    method: &str,
    path: &str,
    payload: Option<serde_json::Value>,
) -> (u16, serde_json::Value) {
    let mut request = axum::http::Request::builder().method(method).uri(path);
    let body = match payload {
        Some(payload) => {
            request = request.header("content-type", "application/json");
            axum::body::Body::from(serde_json::to_vec(&payload).unwrap())
        }
        None => axum::body::Body::empty(),
    };
    let response = router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

struct EndpointEnvironmentRestore {
    position: Option<std::ffi::OsString>,
    learning: Option<std::ffi::OsString>,
    paper_exchange: Option<std::ffi::OsString>,
}

impl Drop for EndpointEnvironmentRestore {
    fn drop(&mut self) {
        restore_env("POSITION_MANAGEMENT_RSOCKET_URL", self.position.take());
        restore_env("LEARNING_RSOCKET_URL", self.learning.take());
        restore_env("PAPER_EXCHANGE_RSOCKET_URL", self.paper_exchange.take());
        crate::service_rpc::clear_rpc_client_pool_for_tests();
    }
}

#[tokio::test]
async fn selected_exit_profiles_survive_global_fallback_updates_and_reach_barriers() {
    let _environment = super::service_endpoint_environment_lock();
    crate::service_rpc::clear_rpc_client_pool_for_tests();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let endpoint = format!("ws://{address}");
    let responder = LifecycleResponder::default();
    let service = responder.clone();
    let server = tokio::spawn(async move {
        RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_, _| Ok(Box::new(service.clone()))))
            .serve()
            .await
            .expect("exit profile fixture RSocket must run");
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let _restore_endpoints = EndpointEnvironmentRestore {
        position: std::env::var_os("POSITION_MANAGEMENT_RSOCKET_URL"),
        learning: std::env::var_os("LEARNING_RSOCKET_URL"),
        paper_exchange: std::env::var_os("PAPER_EXCHANGE_RSOCKET_URL"),
    };
    unsafe {
        std::env::set_var("POSITION_MANAGEMENT_RSOCKET_URL", &endpoint);
    }

    let now_ms = current_time_ms();
    let mut initial_settings = Settings::default();
    initial_settings.stop_loss_percent = 0.20;
    initial_settings.take_profit_percent = 0.75;
    initial_settings.risk_appetite = 0.0;
    initial_settings.closed_trade_returns_percent = vec![1.0; EXTENDED_HOLD_MIN_SAMPLES];
    adapt_exit_policy(&mut initial_settings);
    let low_appetite_fallback = initial_settings.take_profit_percent;
    let mut high_appetite_settings = Settings::default();
    high_appetite_settings.stop_loss_percent = 0.20;
    high_appetite_settings.take_profit_percent = 0.75;
    high_appetite_settings.risk_appetite = 100.0;
    high_appetite_settings.closed_trade_returns_percent = vec![1.0; EXTENDED_HOLD_MIN_SAMPLES];
    adapt_exit_policy(&mut high_appetite_settings);
    let high_appetite_fallback = high_appetite_settings.take_profit_percent;
    assert_ne!(low_appetite_fallback, high_appetite_fallback);

    let mut base = Settings::default();
    base.stop_loss_percent = 0.20;
    base.take_profit_percent = 0.75;
    base.max_order_notional = 100.0;
    base.daily_notional_limit = 1_000.0;
    for strategy in [Strategy::LiveMomentum, Strategy::LiveOrderBook] {
        if !base.strategies.contains(&strategy) {
            base.strategies.push(strategy);
        }
    }
    base.closed_trade_returns_percent = vec![1.0; EXTENDED_HOLD_MIN_SAMPLES];
    base.paper_learning_exploration_until_ms = now_ms.saturating_add(60_000);
    let state = strategy_cleanup_integration_tests::test_state(base.clone());
    let market = serde_json::json!({
        "symbol_freshness":{"contract_version":1,"state":"fresh","max_age_ms":5000},
        "updated_at_ms":now_ms,
        "bids":[{"price":100.0,"size":100.0}],
        "asks":[{"price":100.1,"size":100.0}]
    });
    let mut actual_profiles = Vec::new();
    let mut first_momentum_plan = None;
    let mut same_profile_risk_budgets = Vec::new();
    let mut client_sequence = 1_u64;

    // Repeat BTC/live-momentum at both risk-appetite extremes to prove the
    // global fallback changes while the selected validated policy does not.
    let cases = [
        ("BTC-USDT", "live-momentum", 0.60, 0.90, 0.0, true),
        // Same symbol, strategy, confidence and global safety fallback with a
        // wider validated stop. The selected notional must fall accordingly.
        ("BTC-USDT", "live-momentum", 1.20, 1.80, 0.0, false),
        ("BTC-USDT", "live-momentum", 0.60, 0.90, 100.0, false),
        ("BTC-USDT", "live-order-book", 0.80, 1.20, 100.0, true),
        ("ETH-USDT", "live-momentum", 0.90, 1.50, 100.0, true),
        // At risk_appetite 0 these selected policy values equal the global
        // fallback; source must still come from the explicit selector.
        ("SOL-USDT", "live-momentum", 0.20, 0.75, 0.0, false),
    ];
    for (symbol, strategy_name, stop, target, risk_appetite, create_position) in cases {
        let mut settings = state.settings.lock().await.clone();
        settings.risk_appetite = risk_appetite;
        settings.stop_loss_percent = 0.20;
        settings.take_profit_percent = 0.75;
        adapt_exit_policy(&mut settings);
        let (stop, target) = if symbol == "SOL-USDT" {
            (settings.stop_loss_percent, settings.take_profit_percent)
        } else {
            (stop, target)
        };
        settings.paper_learning_exploration_until_ms = now_ms.saturating_add(60_000);
        let strategy_kind =
            Strategy::from_name(strategy_name).expect("fixture strategy is registered");
        let profile_parameters = strategy_kind
            .parameter_specs()
            .iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let profile_id = strategy::profile_id(symbol, strategy_name, &profile_parameters);
        let profile_version = 1;
        settings.learned_profiles.insert(
            format!("{symbol}:{strategy_name}"),
            strategy::StrategyProfile {
                profile_id: profile_id.clone(),
                version: profile_version,
                symbol: symbol.to_owned(),
                strategy: strategy_name.to_owned(),
                parameters: profile_parameters,
                learning_samples: 40,
                positive_samples: 24,
                realized_pnl: 2.0,
            },
        );
        let mut policy = exit_profile_policy(symbol, strategy_name, stop, target, now_ms);
        policy.profile_id = Some(profile_id.clone());
        policy.profile_version = Some(profile_version);
        policy.trailing_activation_percent = Some(if strategy_name == "live-momentum" {
            0.30
        } else {
            0.60
        });
        policy.trailing_distance_percent = Some(if strategy_name == "live-momentum" {
            0.10
        } else {
            0.20
        });
        policy.breakeven_activation_percent = Some(if strategy_name == "live-momentum" {
            0.25
        } else {
            0.50
        });
        assert!(valid_exit_protection_policy(&policy, now_ms));
        settings.learned_exit_protection_policies.insert(
            exit_protection_policy_key(symbol, strategy_name, &profile_id, profile_version),
            policy,
        );
        let mut signal = profile_exit_signal(symbol, strategy_name, now_ms);
        inject_learned_parameters(&settings, &mut signal);
        let dashboard = Dashboard {
            best: vec![signal.clone()],
            missed_opportunity_bands: Arc::new(Vec::new()),
            markets: Arc::new(BTreeMap::new()),
            queued_at: None,
            queue_wait_ms: None,
        };
        let plan = prepare_signal_execution_plan(
            &state,
            settings.clone(),
            &signal,
            &dashboard,
            &market,
            now_ms,
            1.0,
            true,
            false,
            settings.stop_loss_percent,
            settings.take_profit_percent,
        )
        .await
        .expect("eligible profile with bounded paper exploration gets an execution plan");
        assert!((plan.effective_stop_loss_percent - stop).abs() < 1e-12);
        assert!((plan.effective_take_profit_percent - target).abs() < 1e-12);
        assert_eq!(
            plan.exit_parameter_provenance["stop_loss"]["source"],
            "validated_exit_policy"
        );
        assert_eq!(
            plan.exit_parameter_provenance["take_profit"]["source"],
            "validated_exit_policy"
        );
        assert_eq!(
            plan.exit_parameter_provenance["trailing_protection"]["source"],
            "validated_exit_policy"
        );
        assert_eq!(
            plan.exit_parameter_provenance["trailing_protection"]["activation_percent"],
            if strategy_name == "live-momentum" {
                0.30
            } else {
                0.60
            }
        );
        assert_eq!(
            plan.exit_parameter_provenance["trailing_protection"]["distance_percent"],
            if strategy_name == "live-momentum" {
                0.10
            } else {
                0.20
            }
        );
        for parameter in [
            "activation_percent",
            "giveback_percent",
            "minimum_lock_percent",
        ] {
            assert!(
                plan.exit_parameter_provenance["profit_protection"][parameter]
                    .as_f64()
                    .is_some_and(|value| value.is_finite() && value >= 0.0)
            );
        }
        assert_eq!(
            plan.exit_parameter_provenance["profit_protection"]["source"],
            "validated_exit_policy"
        );
        assert_eq!(
            plan.exit_parameter_provenance["risk_appetite_at_selection"],
            risk_appetite
        );
        assert_eq!(
            plan.exit_parameter_provenance["volatility_stop_adjustment"]["source"],
            if plan.exit_parameter_provenance["volatility_stop_adjustment"]["applied"] == true {
                "volatility_adjusted_strategy_profile"
            } else {
                "strategy_profile_unmodified"
            }
        );
        if symbol == "BTC-USDT" && strategy_name == "live-momentum" && risk_appetite == 0.0 {
            let round_trip_cost_bps = 2.0 * configured_taker_fee_rate() * 10_000.0
                + signal
                    .components
                    .aggressor_cost_bps
                    .filter(|cost| cost.is_finite() && *cost > 0.0)
                    .unwrap_or(0.0);
            same_profile_risk_budgets.push(
                plan.notional * (plan.effective_stop_loss_percent + round_trip_cost_bps / 100.0)
                    / 100.0,
            );
        }
        if symbol == "SOL-USDT" {
            assert_eq!(stop, settings.stop_loss_percent);
            assert_eq!(target, settings.take_profit_percent);
            assert_eq!(
                plan.exit_parameter_provenance["stop_loss"]["source"],
                "validated_exit_policy"
            );
            assert_eq!(
                plan.exit_parameter_provenance["take_profit"]["source"],
                "validated_exit_policy"
            );
        }
        if strategy_name == "live-momentum" && symbol == "BTC-USDT" {
            if (stop - 0.60).abs() < 1e-12 && risk_appetite == 0.0 {
                first_momentum_plan = Some((
                    plan.effective_stop_loss_percent,
                    plan.effective_take_profit_percent,
                    plan.exit_parameter_provenance.clone(),
                ));
            } else if (stop - 0.60).abs() < 1e-12 {
                let (first_stop, first_target, _) = first_momentum_plan
                    .as_ref()
                    .expect("the baseline profile parameters are exercised first");
                assert_eq!(plan.effective_stop_loss_percent, *first_stop);
                assert_eq!(plan.effective_take_profit_percent, *first_target);
            } else if (stop - 1.20).abs() < 1e-12 {
                let (first_stop, first_target, _) = first_momentum_plan
                    .as_ref()
                    .expect("baseline policy is captured before the wider-stop profile");
                assert_ne!(plan.effective_stop_loss_percent, *first_stop);
                assert_ne!(plan.effective_take_profit_percent, *first_target);
            }
        }

        if !create_position {
            continue;
        }

        let decision_id = format!("exit-profile-decision-{client_sequence}");
        let opportunity_id = format!("exit-profile-opportunity-{client_sequence}");
        let profile_id = signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("learning_profile_id"))
            .and_then(serde_json::Value::as_str)
            .expect("selected signal carries its profile ID")
            .to_owned();
        let profile_version = signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("learning_profile_version"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        let identity = CanonicalLifecycleIdentity {
            schema_version: 1,
            opportunity_id: opportunity_id.clone(),
            episode_id: format!("exit-profile-episode-{client_sequence}"),
            trade_id: decision_id.clone(),
            profile_id: Some(profile_id),
            profile_hash: None,
            profile_version: Some(profile_version),
            execution_mode: "paper".into(),
            entry_policy: "exploratory".into(),
            source: "auto_trader_entry_reservation".into(),
        };
        let side = "buy";
        apply_strategy_position_fill(
            &mut settings.strategy_positions,
            &mut settings.positions,
            symbol,
            strategy_name,
            side,
            1.0,
            100.0,
            0.01,
            "Limit",
            "profile-exit-fixture",
            true,
            None,
            Some(&decision_id),
            signal.strategy_decision.as_ref(),
            Some(plan.effective_stop_loss_percent),
            Some(plan.effective_take_profit_percent),
        );
        settings
            .canonical_lifecycle_links
            .insert(decision_id.clone(), identity.clone());
        let position = settings
            .strategy_positions
            .get_mut(symbol)
            .and_then(|lots| lots.get_mut(strategy_name))
            .expect("profile plan creates one strategy position");
        position.entry_decision_id = Some(decision_id.clone());
        position.trade_id = Some(decision_id.clone());
        position.lifecycle_identity = Some(identity);
        position.exit_parameter_provenance = Some(plan.exit_parameter_provenance.clone());
        let request_body = serde_json::json!({
            "client_order_id": opportunity_id,
            "symbol": symbol,
            "side": side
        });
        let fill_response = serde_json::json!({
            "filled_quantity": 1.0,
            "estimated_fee": 0.01,
            "fee_complete": true,
            "fill_evidence":[{
                "id":format!("profile-fill-row-{client_sequence}"),
                "fill_id":format!("pm-fill:v1:exit-profile:{client_sequence}"),
                "parent_order_id":request_body["client_order_id"],
                "client_order_id":request_body["client_order_id"],
                "symbol":symbol,
                "side":side,
                "quantity":1.0,
                "execution_price":100.0,
                "fee":0.01,
                "fee_known":true,
                "fee_currency":"USDT"
            }]
        });
        update_initial_risk_reference(
            position,
            &fill_response,
            &request_body,
            plan.effective_stop_loss_percent,
        );
        append_entry_fill_evidence(position, &fill_response, &request_body);
        assert_eq!(
            position.initial_risk_reference.as_ref().unwrap().status,
            "complete"
        );
        assert_eq!(
            position.strategy_decision.as_ref().unwrap()["strategy_parameters"]["trailing_activation_percent"],
            if strategy_name == "live-momentum" {
                0.30
            } else {
                0.60
            }
        );
        assert_eq!(
            position.strategy_decision.as_ref().unwrap()["strategy_parameters"]["trailing_distance_percent"],
            if strategy_name == "live-momentum" {
                0.10
            } else {
                0.20
            }
        );
        assert_eq!(
            position.strategy_decision.as_ref().unwrap()["strategy_parameters"]["breakeven_activation_percent"],
            if strategy_name == "live-momentum" {
                0.25
            } else {
                0.50
            }
        );
        let selected_stop = position.stop_loss_percent.unwrap();
        let selected_target = position.take_profit_percent.unwrap();
        settings = settings.clone();
        *state.settings.lock().await = settings.clone();
        arm_protective_stop(
            &state,
            symbol,
            strategy_name,
            "buy",
            1.0,
            100.0,
            0.01,
            Some(selected_stop),
            None,
        )
        .await
        .expect("the actual paper protective barrier accepts selected profile stop");
        actual_profiles.push((symbol, strategy_name, selected_stop, selected_target));
        tokio::time::sleep(Duration::from_millis(2)).await;
        client_sequence = client_sequence.saturating_add(1);
    }

    assert_eq!(same_profile_risk_budgets.len(), 2);
    assert!(
        (same_profile_risk_budgets[0] - same_profile_risk_budgets[1]).abs() < 1e-9,
        "widening the selected stop must preserve the capped monetary risk including execution costs: {same_profile_risk_budgets:?}"
    );

    assert!(
        (high_appetite_fallback - low_appetite_fallback).abs() > 1e-6,
        "global fallback actually varies with risk appetite"
    );
    assert_eq!(actual_profiles.len(), 3);
    assert!(actual_profiles.contains(&("BTC-USDT", "live-momentum", 0.60, 0.90)));
    assert!(actual_profiles.contains(&("BTC-USDT", "live-order-book", 0.80, 1.20)));
    assert!(actual_profiles.contains(&("ETH-USDT", "live-momentum", 0.90, 1.50)));

    let barrier_requests = responder
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.method == "POST" && request.path == "/positions/orders")
        .filter_map(|request| request.body.clone())
        .filter(|body| body["order_type"] == "stop-market")
        .collect::<Vec<_>>();
    assert_eq!(barrier_requests.len(), 3);
    let persisted_positions = state.settings.lock().await.clone();
    for (symbol, strategy_name, stop, _) in &actual_profiles {
        let profile_id = persisted_positions
            .strategy_positions
            .get(*symbol)
            .unwrap()
            .get(*strategy_name)
            .unwrap()
            .lifecycle_identity
            .as_ref()
            .and_then(|identity| identity.profile_id.as_deref())
            .expect("position retains selected profile identity");
        let request = barrier_requests
            .iter()
            .find(|request| {
                request["symbol"] == *symbol && request["lifecycle"]["profile_id"] == profile_id
            })
            .expect("protective barrier retains the selected profile identity");
        let fee_rate = configured_taker_fee_rate();
        let expected_trigger = (100.0 * (1.0 - stop / 100.0) + 0.01) / (1.0 - fee_rate);
        assert!((request["trigger_price"].as_f64().unwrap() - expected_trigger).abs() < 1e-10);
        assert_eq!(request["trigger_direction"], "below");
    }

    let btc_momentum = &actual_profiles[0];
    assert_eq!(
        position_exit_reason(
            false,
            false,
            false,
            false,
            false,
            0.0,
            btc_momentum.2,
            false,
            false,
            false,
            false,
            0.01,
            btc_momentum.3
        ),
        Some("take-profit")
    );
    let btc_order_book = &actual_profiles[1];
    assert_eq!(
        position_exit_reason(
            false,
            false,
            false,
            false,
            false,
            0.0,
            btc_order_book.2,
            false,
            false,
            false,
            false,
            0.01,
            btc_order_book.3
        ),
        None
    );
    assert_eq!(
        position_exit_reason(
            false,
            false,
            false,
            false,
            false,
            -0.007,
            btc_momentum.2,
            false,
            false,
            false,
            false,
            0.0,
            btc_momentum.3
        ),
        Some("stop-loss")
    );
    assert_eq!(
        position_exit_reason(
            false,
            false,
            false,
            false,
            false,
            -0.007,
            btc_order_book.2,
            false,
            false,
            false,
            false,
            0.0,
            btc_order_book.3
        ),
        None
    );

    let settings = state.settings.lock().await.clone();
    let durable_path = unique_path("exit-profile-restart").with_extension("sqlite");
    persist_durable_settings_snapshot(&durable_path, &settings).unwrap();
    let restored = read_settings_database(&durable_path).unwrap().unwrap();
    for (symbol, strategy_name, selected_stop, selected_target) in &actual_profiles {
        let restored_position = restored
            .strategy_positions
            .get(*symbol)
            .unwrap()
            .get(*strategy_name)
            .unwrap();
        assert_eq!(restored_position.stop_loss_percent, Some(*selected_stop));
        assert_eq!(
            restored_position.take_profit_percent,
            Some(*selected_target)
        );
        let provenance = restored_position
            .exit_parameter_provenance
            .as_ref()
            .unwrap();
        assert_eq!(provenance["stop_loss"]["selected_percent"], *selected_stop);
        assert_eq!(
            provenance["take_profit"]["selected_percent"],
            *selected_target
        );
        assert_eq!(
            provenance["risk_appetite_at_selection"].as_f64().is_some(),
            true
        );
        let risk = restored_position.initial_risk_reference.as_ref().unwrap();
        assert_eq!(risk.status, "complete");
        assert!((risk.quote_amount.unwrap() - selected_stop).abs() < 1e-12);
    }

    let _ = std::fs::remove_file(&state.settings_path);
    let _ = std::fs::remove_file(&durable_path);
    let _ = std::fs::remove_file(durable_path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(durable_path.with_extension("sqlite-shm"));
    server.abort();
}

#[tokio::test]
async fn profile_trailing_barrier_update_survives_lost_ack_and_keeps_initial_risk() {
    let _environment = super::service_endpoint_environment_lock();
    crate::service_rpc::clear_rpc_client_pool_for_tests();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let endpoint = format!("ws://{address}");
    let responder = LifecycleResponder::default();
    let trade_id = "profile-trailing-trade-v1";
    let profile_id = "profile-trailing-live-momentum-v1";
    let barrier_id = "profile-trailing-barrier-v1";
    responder.barriers.lock().unwrap().insert(
        barrier_id.into(),
        serde_json::json!({
            "id":barrier_id,
            "created_at":100u64,
            "symbol":"BTC-USDT",
            "side":"sell",
            "order_kind":"stop-market",
            "execution_type":"market",
            "quantity":1.0,
            "remaining_quantity":1.0,
            "filled_quantity":0.0,
            "average_execution_price":null,
            "fee_paid":0.0,
            "trigger_price":99.60960960960962,
            "trigger_direction":"below",
            "source":"auto",
            "status":"armed",
            "parent_order_id":"auto-stop-BTC-USDT-100",
            "client_order_id":"auto-stop-BTC-USDT-100",
            "lifecycle": {
                "schema_version":1,
                "opportunity_id":"profile-trailing-opportunity-v1",
                "episode_id":"profile-trailing-episode-v1",
                "trade_id":trade_id,
                "profile_id":profile_id,
                "profile_hash":"profile-hash-v1",
                "profile_version":1,
                "execution_mode":"paper",
                "entry_policy":"strict",
                "source":"auto_trader_entry_reservation"
            },
            "trigger_adjustments":[]
        }),
    );
    let service = responder.clone();
    let server = tokio::spawn(async move {
        RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_, _| Ok(Box::new(service.clone()))))
            .serve()
            .await
            .expect("profile barrier fixture RSocket must run");
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let _restore_endpoints = EndpointEnvironmentRestore {
        position: std::env::var_os("POSITION_MANAGEMENT_RSOCKET_URL"),
        learning: std::env::var_os("LEARNING_RSOCKET_URL"),
        paper_exchange: std::env::var_os("PAPER_EXCHANGE_RSOCKET_URL"),
    };
    unsafe {
        std::env::set_var("POSITION_MANAGEMENT_RSOCKET_URL", &endpoint);
    }

    let lifecycle = CanonicalLifecycleIdentity {
        schema_version: 1,
        opportunity_id: "profile-trailing-opportunity-v1".into(),
        episode_id: "profile-trailing-episode-v1".into(),
        trade_id: trade_id.into(),
        profile_id: Some(profile_id.into()),
        profile_hash: Some("profile-hash-v1".into()),
        profile_version: Some(1),
        execution_mode: "paper".into(),
        entry_policy: "strict".into(),
        source: "auto_trader_entry_reservation".into(),
    };
    let mut position = TraderPosition {
        quantity: 1.0,
        average_entry_price: 100.0,
        entry_fees: 0.01,
        initial_entry_notional: 100.0,
        entries: 1,
        entry_order_type: "Limit".into(),
        entry_decision_id: Some("profile-trailing-decision-v1".into()),
        trade_id: Some(trade_id.into()),
        lifecycle_identity: Some(lifecycle.clone()),
        strategy: Some("live-momentum".into()),
        strategy_decision: Some(serde_json::json!({
            "learning_profile_id":profile_id,
            "learning_profile_version":1,
            "strategy_parameters": {
                "stop_loss_percent":0.5,
                "take_profit_percent":0.8,
                "trailing_activation_percent":0.30,
                "trailing_distance_percent":0.10,
                "breakeven_activation_percent":0.25
            }
        })),
        stop_loss_percent: Some(0.5),
        take_profit_percent: Some(0.8),
        exit_parameter_provenance: Some(serde_json::json!({
            "profile_id":profile_id,
            "profile_version":1,
            "exit_policy_profile_id":profile_id,
            "exit_policy_profile_version":1,
            "trailing_protection": {"source":"validated_exit_policy", "activation_percent":0.30, "distance_percent":0.10},
            "breakeven_protection": {"source":"validated_exit_policy", "activation_percent":0.25}
        })),
        initial_risk_reference: Some(InitialRiskReference {
            schema_version: 1,
            quote_currency: Some("USDT".into()),
            stop_loss_percent: Some(0.5),
            quote_amount: Some(0.5),
            fill_ids: vec!["pm-fill:v1:entry:1".into()],
            status: "complete".into(),
        }),
        trailing_peak_return_percent: Some(0.40),
        entry_timestamp_ms: 100,
        ..TraderPosition::default()
    };
    position.protective_stops.push(ProtectiveStop {
        barrier_id: barrier_id.into(),
        strategy: "live-momentum".into(),
        order_timestamp: 100,
        quantity: 1.0,
        processed_quantity: 0.0,
        processed_notional: 0.0,
        processed_fee: 0.0,
    });
    let mut settings = Settings::default();
    settings.strategies.push(Strategy::LiveMomentum);
    settings
        .positions
        .insert("BTC-USDT".into(), position.clone());
    settings.strategy_positions.insert(
        "BTC-USDT".into(),
        BTreeMap::from([("live-momentum".into(), position)]),
    );
    settings
        .canonical_lifecycle_links
        .insert("profile-trailing-decision-v1".into(), lifecycle);
    settings.orders.push(new_protective_stop_order(
        100,
        "BTC-USDT",
        "sell",
        "live-momentum".into(),
        Some("profile-trailing-decision-v1".into()),
        1.0,
        99.60960960960962,
        barrier_id.into(),
    ));
    let state = strategy_cleanup_integration_tests::test_state(settings);

    ensure_protective_stops(&state)
        .await
        .expect("entry-captured trailing policy tightens the protected barrier");
    let first_update = responder
        .requests
        .lock()
        .unwrap()
        .iter()
        .find(|request| request.method == "POST" && request.path.ends_with("/trigger"))
        .and_then(|request| request.body.clone())
        .expect("profile threshold reaches the Position Management trigger route");
    assert_eq!(first_update["profile_id"], profile_id);
    assert_eq!(first_update["profile_version"], 1);
    assert_eq!(first_update["policy_source"], "validated_exit_policy");
    assert_eq!(first_update["reason"], "trailing-stop");
    assert!((first_update["target_net_return_percent"].as_f64().unwrap() - 0.30).abs() < 1e-12);
    assert!(first_update["new_trigger_price"].as_f64().unwrap() > 99.60960960960962);

    {
        let mut settings = state.settings.lock().await;
        settings
            .strategy_positions
            .get_mut("BTC-USDT")
            .unwrap()
            .get_mut("live-momentum")
            .unwrap()
            .exit_parameter_provenance
            .as_mut()
            .unwrap()["protection_adjustments"] = serde_json::json!([]);
    }
    persist_settings_snapshot_durable(&state)
        .await
        .expect("lost acknowledgment fixture state is durable before recovery");
    ensure_protective_stops(&state)
        .await
        .expect("durable PM adjustment history recovers the local audit record");
    let request_count = responder
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.method == "POST" && request.path.ends_with("/trigger"))
        .count();
    assert_eq!(
        request_count, 1,
        "recovery must not repeat the barrier update"
    );
    let recovered = state.settings.lock().await;
    let position = &recovered.strategy_positions["BTC-USDT"]["live-momentum"];
    let adjustments =
        position.exit_parameter_provenance.as_ref().unwrap()["protection_adjustments"]
            .as_array()
            .unwrap();
    assert_eq!(adjustments.len(), 1);
    assert_eq!(adjustments[0]["request_id"], first_update["request_id"]);
    assert_eq!(adjustments[0]["recovered_from_durable_pm_barrier"], true);
    assert_eq!(
        position
            .initial_risk_reference
            .as_ref()
            .unwrap()
            .quote_amount,
        Some(0.5)
    );
    drop(recovered);
    server.abort();
    crate::service_rpc::clear_rpc_client_pool_for_tests();
}

#[tokio::test]
async fn selected_entry_exit_outbox_and_sqlite_restart_keep_one_lifecycle() {
    let _environment = super::service_endpoint_environment_lock();
    crate::service_rpc::clear_rpc_client_pool_for_tests();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let endpoint = format!("ws://{address}");
    let learning_db_path = unique_path("learning").with_extension("sqlite");
    let learning_router = learning::router(learning_db_path.clone()).unwrap();
    let responder = LifecycleResponder {
        learning_router: Arc::new(StdMutex::new(learning_router)),
        ..LifecycleResponder::default()
    };
    let service = responder.clone();
    let server = tokio::spawn(async move {
        RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_, _| Ok(Box::new(service.clone()))))
            .serve()
            .await
            .expect("local lifecycle RSocket service must run");
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let previous_position_endpoint = std::env::var_os("POSITION_MANAGEMENT_RSOCKET_URL");
    let previous_learning_endpoint = std::env::var_os("LEARNING_RSOCKET_URL");
    let _restore_endpoints = EndpointEnvironmentRestore {
        position: previous_position_endpoint,
        learning: previous_learning_endpoint,
        paper_exchange: std::env::var_os("PAPER_EXCHANGE_RSOCKET_URL"),
    };
    // SAFETY: see restore_env; no live service endpoint is contacted.
    unsafe {
        std::env::set_var("POSITION_MANAGEMENT_RSOCKET_URL", &endpoint);
        std::env::set_var("LEARNING_RSOCKET_URL", &endpoint);
    }

    let fixture_index = 700_001;
    let replay_identity = versioned_lifecycle_dashboard(fixture_index, "source-700001")
        .best
        .into_iter()
        .next()
        .unwrap()
        .strategy_decision
        .unwrap();
    let exact_retry_identity = versioned_lifecycle_dashboard(fixture_index, "source-700001")
        .best
        .into_iter()
        .next()
        .unwrap()
        .strategy_decision
        .unwrap();
    let next_event_same_thesis = versioned_lifecycle_dashboard(fixture_index, "source-700002")
        .best
        .into_iter()
        .next()
        .unwrap()
        .strategy_decision
        .unwrap();
    assert_eq!(replay_identity, exact_retry_identity);
    assert_ne!(
        replay_identity["live_worker_event_id"],
        next_event_same_thesis["live_worker_event_id"]
    );
    assert_eq!(
        replay_identity["live_worker_episode_id"],
        next_event_same_thesis["live_worker_episode_id"]
    );

    let mut state = benchmark_precomputed_ingress_state("lifecycle");
    let dashboard = versioned_lifecycle_dashboard(fixture_index, "source-700001");
    state.settings_path = unique_path("state");
    state.dated_carry_path = unique_path("dated-carry");
    // persist_settings_snapshot writes SQLite directly at settings_path; the
    // .sqlite sibling is only used by the settings JSON-path resolver.
    let sqlite_path = state.settings_path.clone();
    let result = run_cycle_inner(&state, dashboard).await.unwrap();
    assert_eq!(
        result.status, "order_submitting",
        "selected signal should reserve and queue the production entry; actual status={} reason={:?}",
        result.status, result.reason,
    );

    let mut entry_order = None;
    for _ in 0..500 {
        entry_order = state
            .settings
            .lock()
            .await
            .orders
            .iter()
            .find(|order| order.purpose == "signal")
            .cloned();
        if entry_order
            .as_ref()
            .is_some_and(|order| order.status != "submitting")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let entry_order = entry_order.expect("production entry order should be recorded");
    assert_eq!(entry_order.status, "filled");
    let decision_id = entry_order
        .decision_id
        .clone()
        .expect("entry decision correlation key");
    let strategy_key = entry_order
        .strategy
        .clone()
        .unwrap_or_else(|| "unattributed".into());
    let (identity, position) = {
        let settings = state.settings.lock().await;
        (
            settings
                .canonical_lifecycle_links
                .get(&decision_id)
                .cloned()
                .expect("durable canonical reservation identity"),
            settings
                .strategy_positions
                .get(&entry_order.symbol)
                .and_then(|lots| lots.get(&strategy_key))
                .cloned()
                .expect("entry fill creates strategy position"),
        )
    };
    assert_eq!(
        identity.entry_policy, "exploratory",
        "fixture has no validated walk-forward evidence, so entry remains exploratory"
    );
    assert_eq!(
        position.trade_id.as_deref(),
        Some(decision_id.as_str()),
        "position retains the reservation lookup key used to recover canonical trade identity"
    );
    assert_eq!(position.lifecycle_identity.as_ref(), Some(&identity));
    assert_eq!(position.entry_fill_links.len(), 1);
    assert_eq!(position.entry_fill_links[0].symbol, entry_order.symbol);
    assert_eq!(
        position.entry_fill_links[0].client_order_id,
        identity.opportunity_id
    );
    assert_eq!(
        position.initial_risk_reference.as_ref().unwrap().status,
        "complete"
    );
    assert!(position.price_path_epoch.is_some());

    // Replaying a fresh production cycle for the same stable worker event must
    // stop at the durable exploratory reservation check, before the existing
    // position could mask a missing dedup check.
    let first_signal = versioned_lifecycle_dashboard(fixture_index, "source-700001")
        .best
        .into_iter()
        .next()
        .expect("benchmark dashboard contains the selected signal");
    let event_key = exploratory_event_key_for_signal(&first_signal)
        .expect("fixture signal has a stable exploratory event key");
    let order_count_before_replay = state.settings.lock().await.orders.len();
    // Keep the fixture focused on the source-event reservation check rather
    // than the unrelated hard position cap, then restore live state before
    // continuing through the real exit lifecycle below.
    let (saved_positions, saved_strategy_positions) = {
        let mut settings = state.settings.lock().await;
        (
            std::mem::take(&mut settings.positions),
            std::mem::take(&mut settings.strategy_positions),
        )
    };
    let duplicate = run_cycle_inner(
        &state,
        versioned_lifecycle_dashboard(fixture_index, "source-700001"),
    )
    .await
    .unwrap();
    assert_eq!(duplicate.status, "skipped");
    assert_eq!(
        duplicate.reason.as_deref(),
        Some("Exploratory source event already reserved for this profile")
    );
    let next_event_same_thesis = run_cycle_inner(
        &state,
        versioned_lifecycle_dashboard(fixture_index, "source-700002"),
    )
    .await
    .unwrap();
    assert_eq!(next_event_same_thesis.status, "skipped");
    assert_eq!(
        next_event_same_thesis.reason.as_deref(),
        Some("Exploratory source event already reserved for this profile")
    );
    {
        let settings = state.settings.lock().await;
        assert_eq!(settings.orders.len(), order_count_before_replay);
        assert_eq!(settings.exploratory_event_reservations.len(), 1);
        assert!(
            settings
                .exploratory_event_reservations
                .contains_key(&event_key)
        );
    }
    {
        let mut settings = state.settings.lock().await;
        settings.positions = saved_positions;
        settings.strategy_positions = saved_strategy_positions;
    }

    // Restore the durable snapshot into a fresh state object and repeat the
    // exact event cycle. This verifies restart recovery through the production
    // cycle boundary, not only serde round-tripping of the reservation map.
    persist_settings_snapshot_durable(&state).await.unwrap();
    let mut restored_settings = read_settings_database(&sqlite_path)
        .unwrap()
        .expect("durable settings snapshot after reservation");
    // Clear position gates in this isolated restored fixture so its result
    // demonstrates the persisted event key, not the existing-position gate.
    restored_settings.positions.clear();
    restored_settings.strategy_positions.clear();
    let mut restored_state = benchmark_fixture_state(restored_settings, "lifecycle-replay", 1);
    restored_state.settings_path = sqlite_path.clone();
    restored_state.dated_carry_path = unique_path("lifecycle-replay-carry");
    let restored_order_count = restored_state.settings.lock().await.orders.len();
    let duplicate_after_restart = run_cycle_inner(
        &restored_state,
        versioned_lifecycle_dashboard(fixture_index, "source-700002"),
    )
    .await
    .unwrap();
    assert_eq!(duplicate_after_restart.status, "skipped");
    assert_eq!(
        duplicate_after_restart.reason.as_deref(),
        Some("Exploratory source event already reserved for this profile")
    );
    let settings_after_restart_replay = restored_state.settings.lock().await;
    assert_eq!(
        settings_after_restart_replay.orders.len(),
        restored_order_count
    );
    assert_eq!(
        settings_after_restart_replay
            .exploratory_event_reservations
            .len(),
        1
    );
    drop(settings_after_restart_replay);

    // Retry the exact production order-submission helper with the stable
    // client-order key. The local PM double accepts once and returns the first
    // response for duplicates, mirroring the external idempotency contract.
    let accepted_before_retry = responder
        .accepted_order_count
        .load(std::sync::atomic::Ordering::Relaxed);
    let retry = submit_configured_order(
        &entry_order.symbol,
        &entry_order.side,
        entry_order.quantity,
        entry_order.reserved_notional,
        entry_order.decision_bid.unwrap_or(100.0),
        entry_order.decision_ask.unwrap_or(100.02),
        AutoOrderType::PostOnlyLimit,
        ExecutionParameters::default(),
        identity.opportunity_id.clone(),
        false,
        Some(&identity),
    )
    .await
    .expect("stable duplicate submission should be acknowledged");
    assert_eq!(
        retry["filled_quantity"].as_f64(),
        Some(entry_order.quantity)
    );
    assert_eq!(
        responder
            .accepted_order_count
            .load(std::sync::atomic::Ordering::Relaxed),
        accepted_before_retry,
        "replaying the entry client-order ID must not create another accepted order"
    );

    let exit_timestamp = entry_order.timestamp.saturating_add(1);
    let exit_result = submit_position_exit_order(
        &state,
        entry_order.symbol.clone(),
        strategy_key.clone(),
        position.clone(),
        100.99,
        101.01,
        "fixture-close",
        0.01,
        1.0,
        2.0,
        exit_timestamp,
        serde_json::json!({"fixture": "deterministic"}),
    )
    .await
    .unwrap();
    assert!(exit_result.is_some());
    let position_requests = responder.requests.lock().unwrap().clone();
    let submitted_orders = position_requests
        .iter()
        .filter(|request| {
            request.method == "POST"
                && request.path == "/positions/orders"
                && request
                    .body
                    .as_ref()
                    .is_some_and(|body| body["order_type"] != "stop-market")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        submitted_orders.len(),
        3,
        "entry, retry, and exit reach the production submission API"
    );
    for request in &submitted_orders {
        let lifecycle = &request.body.as_ref().unwrap()["lifecycle"];
        assert_eq!(lifecycle["opportunity_id"], identity.opportunity_id);
        assert_eq!(lifecycle["episode_id"], identity.episode_id);
        assert_eq!(lifecycle["trade_id"], identity.trade_id);
        assert_eq!(lifecycle["entry_policy"], identity.entry_policy);
    }
    assert_eq!(
        submitted_orders[0].body.as_ref().unwrap()["client_order_id"],
        submitted_orders[1].body.as_ref().unwrap()["client_order_id"],
        "entry retries retain the same client-order key",
    );
    assert_eq!(
        submitted_orders[2].body.as_ref().unwrap()["reduce_only"],
        true
    );

    let trade_outcome = {
        let settings = state.settings.lock().await;
        assert!(settings.positions[&entry_order.symbol].quantity.abs() < 1e-12);
        let outcome = settings.feedback_outbox.iter().find(|event| matches!(
            event,
            FeedbackEvent::TradeOutcome { trade_id: Some(trade_id), .. } if trade_id == &identity.trade_id
        )).cloned().expect("closed trade should queue canonical Learning outcome");
        if let FeedbackEvent::TradeOutcome {
            schema_version,
            opportunity_id,
            episode_id,
            execution_mode,
            entry_policy,
            source,
            ..
        } = &outcome
        {
            assert_eq!(schema_version.as_ref(), Some(&identity.schema_version));
            assert_eq!(
                opportunity_id.as_deref(),
                Some(identity.opportunity_id.as_str())
            );
            assert_eq!(episode_id.as_deref(), Some(identity.episode_id.as_str()));
            assert_eq!(execution_mode.as_deref(), Some("paper"));
            assert_eq!(
                entry_policy.as_deref(),
                Some(identity.entry_policy.as_str())
            );
            assert_eq!(source.as_deref(), Some("auto_trader_entry_reservation"));
        }
        outcome
    };
    // The persisted restart boundary is covered on both services. This fixture
    // cannot inject a process crash after PM acknowledges a fill but before
    // AutoTrader applies/persists that response: the production helper owns the
    // await and mutation as one private call and has no fault-injection seam.
    persist_settings_snapshot_durable(&state).await.unwrap();

    // AutoTrader forwards the canonical event through the existing Learning
    // RSocket route API into Learning's real Axum router and temporary SQLite.
    let learning_payload = serde_json::to_value(&trade_outcome).unwrap();
    let learning_result = crate::service_rpc::learning_request(
        strategy::ScoutWorkerHttpMethod::Post,
        "/feedback",
        Some(learning_payload.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        learning_result.status, 200,
        "Learning rejected outcome: {}; payload={learning_payload}",
        learning_result.body,
    );
    let forwarded = responder
        .learning_routes
        .lock()
        .unwrap()
        .last()
        .cloned()
        .unwrap();
    assert_eq!(forwarded.path, "/feedback");
    assert_eq!(forwarded.payload["schema_version"], identity.schema_version);
    assert_eq!(forwarded.payload["trade_id"], identity.trade_id);
    assert_eq!(forwarded.payload["opportunity_id"], identity.opportunity_id);
    assert_eq!(forwarded.payload["episode_id"], identity.episode_id);
    assert_eq!(forwarded.payload["execution_mode"], identity.execution_mode);
    assert_eq!(forwarded.payload["entry_policy"], identity.entry_policy);
    assert_eq!(forwarded.payload["source"], identity.source);
    assert_json_close(
        &learning_payload,
        &forwarded.payload,
        "Learning feedback payload",
    );
    let event_id = match &trade_outcome {
        FeedbackEvent::TradeOutcome { event_id, .. } => event_id.clone(),
        _ => unreachable!("fixture outcome must be a TradeOutcome"),
    };
    let replay_path = format!("/strategy-replays/{event_id}");
    // Re-delivery of the same outbox event must be safe across the boundary:
    // Learning's durable replay job is keyed by the canonical event ID, so a
    // retry cannot create a second job or alter its provenance.
    let duplicate_learning_result = crate::service_rpc::learning_request(
        strategy::ScoutWorkerHttpMethod::Post,
        "/feedback",
        Some(learning_payload.clone()),
    )
    .await
    .unwrap();
    assert_eq!(duplicate_learning_result.status, 200);
    assert_eq!(
        responder.learning_routes.lock().unwrap().len(),
        2,
        "original feedback and idempotent outbox retry both reach Learning"
    );
    let learning_routes = responder.learning_routes.lock().unwrap();
    assert_eq!(
        learning_routes[0].payload, learning_routes[1].payload,
        "outbox retry must preserve the exact canonical Learning payload"
    );
    drop(learning_routes);
    let replay = crate::service_rpc::learning_request(
        strategy::ScoutWorkerHttpMethod::Get,
        &replay_path,
        None,
    )
    .await
    .unwrap();
    assert_eq!(replay.status, 200);
    assert_eq!(replay.body["event_id"], event_id);
    assert_eq!(replay.body["trade_id"], identity.trade_id);
    assert_eq!(replay.body["opportunity_id"], identity.opportunity_id);
    assert_eq!(replay.body["episode_id"], identity.episode_id);

    let restarted = read_settings_database(&sqlite_path)
        .unwrap()
        .expect("SQLite snapshot after restart");
    assert_eq!(
        restarted.canonical_lifecycle_links.get(&decision_id),
        Some(&identity)
    );
    assert!(
        restarted.canonical_lifecycle_sequence > 0,
        "reservation sequence survives AutoTrader's durable restart boundary"
    );
    assert!(restarted.feedback_outbox.iter().any(|event| matches!(
        event,
        FeedbackEvent::TradeOutcome { trade_id: Some(trade_id), .. } if trade_id == &identity.trade_id
    )));

    // Reopen Learning from its temporary SQLite database and verify the
    // durable replay job retains the same canonical provenance.
    *responder
        .learning_router
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        learning::router(learning_db_path.clone()).unwrap();
    let replay_after_restart = crate::service_rpc::learning_request(
        strategy::ScoutWorkerHttpMethod::Get,
        &replay_path,
        None,
    )
    .await
    .unwrap();
    assert_eq!(replay_after_restart.status, 200);
    assert_eq!(replay_after_restart.body["trade_id"], identity.trade_id);
    assert_eq!(
        replay_after_restart.body["opportunity_id"],
        identity.opportunity_id
    );
    assert_eq!(replay_after_restart.body["episode_id"], identity.episode_id);

    let mut legacy_value = serde_json::to_value(&restarted).unwrap();
    let legacy_object = legacy_value.as_object_mut().unwrap();
    legacy_object.remove("canonical_lifecycle_links");
    legacy_object.remove("canonical_lifecycle_sequence");
    legacy_object.remove("canonical_lifecycle_epoch");
    let legacy: Settings = serde_json::from_value(legacy_value).unwrap();
    assert!(
        legacy.canonical_lifecycle_links.is_empty(),
        "legacy records stay unattributed"
    );
    assert_eq!(legacy.canonical_lifecycle_sequence, 0);

    // Start each eligibility case from the same durable reservation ledger but
    // without open exposure. This keeps unrelated position/risk gates from
    // obscuring whether the event-level identity is scoped correctly.
    for (case_index, case) in ["new_event", "new_profile", "new_direction", "no_stable_id"]
        .into_iter()
        .enumerate()
    {
        let mut isolated_settings = restarted.clone();
        isolated_settings.orders.clear();
        isolated_settings.positions.clear();
        isolated_settings.strategy_positions.clear();
        isolated_settings.used_notional_today = 0.0;
        isolated_settings.daily_strategy_budgets.clear();
        isolated_settings.intraday_strategy_budgets.clear();
        isolated_settings.last_signal_at.clear();
        isolated_settings.last_signal_direction.clear();
        isolated_settings.last_signal_strength.clear();
        let isolated_state = benchmark_fixture_state(
            isolated_settings,
            "exploratory-event-eligibility",
            case_index,
        );
        let mut dashboard = versioned_lifecycle_dashboard(fixture_index, "source-700001");
        let signal = &mut dashboard.best[0];
        let decision = signal.strategy_decision.as_mut().unwrap();
        match case {
            "new_event" => {
                decision["live_worker_event_id"] = serde_json::json!(
                    "strategy-insight:v1:insights-event:v1:sha256:different-worker-event"
                );
                decision["live_worker_episode_id"] = serde_json::json!(
                    "strategy-episode:v1:insights-event:v1:sha256:different-worker-event"
                );
            }
            "new_profile" => {
                decision["learning_profile_id"] = serde_json::json!("different-profile");
            }
            "new_direction" => {
                signal.direction = if signal.direction == "bullish" {
                    "bearish".to_owned()
                } else {
                    "bullish".to_owned()
                };
                decision["live_worker_event_id"] = serde_json::json!(
                    "strategy-insight:v1:insights-event:v1:sha256:direction-reset"
                );
                decision["live_worker_episode_id"] = serde_json::json!(
                    "strategy-episode:v1:insights-event:v1:sha256:direction-reset"
                );
            }
            "no_stable_id" => {
                decision
                    .as_object_mut()
                    .unwrap()
                    .remove("live_worker_event_id");
                decision
                    .as_object_mut()
                    .unwrap()
                    .remove("live_worker_event_identity_version");
                decision
                    .as_object_mut()
                    .unwrap()
                    .remove("live_worker_episode_id");
            }
            _ => unreachable!("case list is exhaustive"),
        }
        let outcome = run_cycle_inner(&isolated_state, dashboard).await.unwrap();
        assert_eq!(
            outcome.status, "order_submitting",
            "{case} must remain eligible despite the prior event reservation: {:?}",
            outcome.reason
        );
        let mut submitted_order = None;
        for _ in 0..500 {
            submitted_order = isolated_state
                .settings
                .lock()
                .await
                .orders
                .iter()
                .find(|order| order.purpose == "signal")
                .cloned();
            if submitted_order
                .as_ref()
                .is_some_and(|order| order.status != "submitting")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            submitted_order.as_ref().map(|order| order.status.as_str()),
            Some("filled"),
            "{case} production submission should complete"
        );
        let settings = isolated_state.settings.lock().await;
        assert_eq!(
            settings
                .orders
                .iter()
                .filter(|order| order.purpose == "signal")
                .count(),
            1,
            "{case} should reserve one entry order"
        );
        if case == "no_stable_id" {
            assert_eq!(
                settings.exploratory_event_reservations.len(),
                1,
                "signals without a stable source ID remain unkeyed/unknown"
            );
        } else {
            assert_eq!(
                settings.exploratory_event_reservations.len(),
                2,
                "{case} adds one distinct durable reservation"
            );
        }
        drop(settings);
        let _ = std::fs::remove_file(&isolated_state.settings_path);
        let _ = std::fs::remove_file(isolated_state.settings_path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(isolated_state.settings_path.with_extension("sqlite-shm"));
        let _ = std::fs::remove_file(&isolated_state.dated_carry_path);
    }

    let _ = std::fs::remove_file(&state.settings_path);
    let _ = std::fs::remove_file(&sqlite_path);
    let _ = std::fs::remove_file(sqlite_path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(sqlite_path.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(&state.dated_carry_path);
    let _ = std::fs::remove_file(&learning_db_path);
    let _ = std::fs::remove_file(learning_db_path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(learning_db_path.with_extension("sqlite-shm"));
    server.abort();
}

#[tokio::test]
async fn production_entry_exit_persist_in_real_position_management_ledger_across_restart() {
    let _environment = super::service_endpoint_environment_lock();
    crate::service_rpc::clear_rpc_client_pool_for_tests();

    let exchange_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let exchange_address = exchange_listener.local_addr().unwrap();
    drop(exchange_listener);
    let exchange_endpoint = format!("ws://{exchange_address}");

    let first_pm_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_pm_address = first_pm_listener.local_addr().unwrap();
    drop(first_pm_listener);
    let first_pm_endpoint = format!("ws://{first_pm_address}");
    let second_pm_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second_pm_address = second_pm_listener.local_addr().unwrap();
    drop(second_pm_listener);
    let second_pm_endpoint = format!("ws://{second_pm_address}");

    let pm_db_path = unique_path("real-pm-ledger").with_extension("sqlite");
    let learning_db_path = unique_path("real-pm-learning").with_extension("sqlite");
    let learning_router = learning::router(learning_db_path.clone()).unwrap();
    let responder = LifecycleResponder {
        learning_router: Arc::new(StdMutex::new(learning_router)),
        ..LifecycleResponder::default()
    };
    let service = responder.clone();
    let exchange_and_learning_server = tokio::spawn(async move {
        RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(exchange_address.to_string()))
            .acceptor(Box::new(move |_, _| Ok(Box::new(service.clone()))))
            .serve()
            .await
            .expect("local Paper Exchange and Learning test services must run");
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let previous_position_endpoint = std::env::var_os("POSITION_MANAGEMENT_RSOCKET_URL");
    let previous_learning_endpoint = std::env::var_os("LEARNING_RSOCKET_URL");
    let previous_paper_exchange_endpoint = std::env::var_os("PAPER_EXCHANGE_RSOCKET_URL");
    let _restore_endpoints = EndpointEnvironmentRestore {
        position: previous_position_endpoint,
        learning: previous_learning_endpoint,
        paper_exchange: previous_paper_exchange_endpoint,
    };
    // SAFETY: this fixture serializes endpoint mutations, uses local services,
    // and restores each variable before returning.
    unsafe {
        std::env::set_var("POSITION_MANAGEMENT_RSOCKET_URL", &first_pm_endpoint);
        std::env::set_var("LEARNING_RSOCKET_URL", &exchange_endpoint);
        std::env::set_var("PAPER_EXCHANGE_RSOCKET_URL", &exchange_endpoint);
    }

    position_management::positions::set_test_usd_rates(std::collections::BTreeMap::from([
        ("USD".to_owned(), 1.0),
        ("USDT".to_owned(), 1.0),
    ]));
    let mut pm_server = Some(
        position_management::positions::start_test_rsocket_service(
            pm_db_path.clone(),
            first_pm_address.to_string(),
        )
        .await
        .expect("real PM service should start on its isolated temporary ledger"),
    );
    tokio::time::sleep(Duration::from_millis(20)).await;

    let fixture_index = 700_101;
    let mut state = benchmark_precomputed_ingress_state("real-pm-lifecycle");
    state.settings_path = unique_path("real-pm-auto-trader");
    state.dated_carry_path = unique_path("real-pm-carry");
    let sqlite_path = state.settings_path.clone();
    let dashboard = versioned_lifecycle_dashboard(fixture_index, "real-pm-source-700101");
    let worker_decision = dashboard.best[0]
        .strategy_decision
        .clone()
        .expect("fixture dashboard has worker identity");
    // Hold the durable writer so the reservation pauses at its commit point.
    // The portfolio cycle lock must already be released, while PM must still
    // have no order: durable-before-submit and cross-symbol progress are both
    // required by the lock-scope optimization.
    let durable_writer_gate = state.settings_persist_write_lock.clone().lock_owned().await;
    let entry_state = state.clone();
    let entry_task = tokio::spawn(async move { run_cycle_inner(&entry_state, dashboard).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let reserved = state
                .settings
                .lock()
                .await
                .orders
                .iter()
                .any(|order| order.purpose == "signal" && order.status == "submitting");
            if reserved {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the in-memory reservation should reach its durability wait");
    let cycle_progress = state
        .cycle_lock
        .try_lock()
        .expect("durable storage wait must not serialize other portfolio cycles");
    drop(cycle_progress);
    let before_commit = crate::service_rpc::position_get("/positions/trades?source=auto")
        .await
        .expect("PM ledger read should be available before entry submission");
    assert_eq!(
        before_commit["trades"].as_array().map(Vec::len),
        Some(0),
        "no PM order may be dispatched before the durable reservation commits"
    );
    drop(durable_writer_gate);
    let entry_result = entry_task.await.unwrap().unwrap();
    assert_eq!(
        entry_result.status, "order_submitting",
        "the production entry should pass through the real PM service"
    );

    let mut entry_order = None;
    for _ in 0..500 {
        entry_order = state
            .settings
            .lock()
            .await
            .orders
            .iter()
            .find(|order| order.purpose == "signal")
            .cloned();
        if entry_order
            .as_ref()
            .is_some_and(|order| order.status != "submitting")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let entry_order = entry_order.expect("production entry should be recorded");
    assert_eq!(entry_order.status, "filled");
    let revalidation_identity = entry_order
        .revalidation_snapshot
        .as_ref()
        .expect("production entry captures worker identity for fresh revalidation");
    assert_eq!(
        revalidation_identity.event_identity_version,
        worker_decision["live_worker_event_identity_version"]
            .as_u64()
            .and_then(|version| u16::try_from(version).ok())
    );
    assert_eq!(
        revalidation_identity.event_id.as_deref(),
        worker_decision["live_worker_event_id"].as_str()
    );
    assert_eq!(
        revalidation_identity.episode_id.as_deref(),
        worker_decision["live_worker_episode_id"].as_str()
    );
    let decision_id = entry_order
        .decision_id
        .clone()
        .expect("entry decision correlation key");
    let strategy_key = entry_order
        .strategy
        .clone()
        .unwrap_or_else(|| "unattributed".into());
    let (identity, position) = {
        let settings = state.settings.lock().await;
        (
            settings
                .canonical_lifecycle_links
                .get(&decision_id)
                .cloned()
                .expect("durable canonical reservation identity"),
            settings
                .strategy_positions
                .get(&entry_order.symbol)
                .and_then(|lots| lots.get(&strategy_key))
                .cloned()
                .expect("real PM fill should create the AutoTrader position"),
        )
    };
    assert_eq!(
        identity.episode_id,
        worker_decision["live_worker_episode_id"]
            .as_str()
            .expect("versioned worker report has a stable thesis episode ID"),
        "canonical lifecycle identity preserves the worker episode across the real PM path"
    );

    let accepted_matches_before_retry = responder
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.path == "/internal/orders/match")
        .count();
    assert!(accepted_matches_before_retry >= 1);

    let exit_timestamp = entry_order.timestamp.saturating_add(1);
    let exit_result = submit_position_exit_order(
        &state,
        entry_order.symbol.clone(),
        strategy_key.clone(),
        position,
        100.99,
        101.01,
        "stop-loss",
        0.01,
        1.0,
        2.0,
        exit_timestamp,
        serde_json::json!({"fixture": "real-position-management-ledger"}),
    )
    .await
    .unwrap();
    assert!(exit_result.is_some());
    {
        let settings = state.settings.lock().await;
        let reset = settings
            .strategy_episode_reset_generations
            .values()
            .find(|reset| {
                reset.symbol == entry_order.symbol
                    && reset.strategy == strategy_key
                    && reset.profile_id == identity.profile_id.as_deref().unwrap_or_default()
            })
            .expect("fully filled closing stop advances the durable thesis reset");
        assert_eq!(reset.generation, 1);
        assert_eq!(
            reset.last_reset_episode_id.as_deref(),
            Some(identity.episode_id.as_str())
        );
    }

    let trades_response = crate::service_rpc::position_request("GET", "/positions/trades", None)
        .await
        .unwrap();
    assert_eq!(trades_response.status, 200);
    let trades = trades_response.body["trades"]
        .as_array()
        .expect("real PM trade ledger response");
    assert!(
        trades.len() >= 2,
        "entry and exit fills must be in PM SQLite ledger"
    );
    let lifecycle_trades = trades
        .iter()
        .filter(|trade| trade["lifecycle"]["trade_id"] == identity.trade_id)
        .collect::<Vec<_>>();
    assert!(
        lifecycle_trades.len() >= 2,
        "both fills retain canonical linkage"
    );
    let fill_ids = lifecycle_trades
        .iter()
        .map(|trade| {
            trade["fill_id"]
                .as_str()
                .filter(|fill_id| fill_id.starts_with("pm-fill:v1:"))
                .expect("PM linked fills have a versioned allocator ID")
                .to_owned()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        fill_ids.len(),
        lifecycle_trades.len(),
        "each persisted lifecycle fill has its own stable ID"
    );
    for trade in lifecycle_trades {
        assert_eq!(
            trade["lifecycle"]["opportunity_id"],
            identity.opportunity_id
        );
        assert_eq!(trade["lifecycle"]["episode_id"], identity.episode_id);
        assert_eq!(trade["lifecycle"]["trade_id"], identity.trade_id);
        assert_eq!(
            trade["lifecycle"]["profile_id"].as_str(),
            identity.profile_id.as_deref()
        );
        assert_eq!(
            trade["lifecycle"]["profile_hash"].as_str(),
            identity.profile_hash.as_deref()
        );
        assert_eq!(
            trade["lifecycle"]["profile_version"].as_u64(),
            identity.profile_version
        );
        assert_eq!(
            trade["lifecycle"]["execution_mode"],
            identity.execution_mode
        );
        assert_eq!(trade["lifecycle"]["source"], identity.source);
    }

    let trade_outcome = {
        let settings = state.settings.lock().await;
        settings
            .feedback_outbox
            .iter()
            .find(|event| {
                matches!(
                    event,
                    FeedbackEvent::TradeOutcome { trade_id: Some(trade_id), .. }
                        if trade_id == &identity.trade_id
                )
            })
            .cloned()
            .expect("closed trade should queue canonical Learning outcome")
    };
    let learning_payload = serde_json::to_value(&trade_outcome).unwrap();
    let learning_result = crate::service_rpc::learning_request(
        strategy::ScoutWorkerHttpMethod::Post,
        "/feedback",
        Some(learning_payload.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        learning_result.status, 200,
        "Learning rejected outcome: {}; payload={learning_payload}",
        learning_result.body
    );
    let event_id = match &trade_outcome {
        FeedbackEvent::TradeOutcome { event_id, .. } => event_id.clone(),
        _ => unreachable!("fixture outcome must be a TradeOutcome"),
    };
    let replay_path = format!("/strategy-replays/{event_id}");
    let replay = crate::service_rpc::learning_request(
        strategy::ScoutWorkerHttpMethod::Get,
        &replay_path,
        None,
    )
    .await
    .unwrap();
    assert_eq!(replay.status, 200);
    assert_eq!(replay.body["trade_id"], identity.trade_id);
    assert_eq!(replay.body["opportunity_id"], identity.opportunity_id);
    assert_eq!(replay.body["episode_id"], identity.episode_id);

    // Insert one genuine legacy-format ledger payload while the service is
    // stopped. Deserialization must retain its unknown provenance as None.
    let stopped_server = pm_server.take().unwrap();
    stopped_server.abort();
    let _ = stopped_server.await;
    let legacy_payload = serde_json::json!({
        "id": "legacy-no-provenance",
        "symbol": "BTC-USDT",
        "side": "buy",
        "quantity": 0.5,
        "execution_price": 99.0,
        "timestamp": 1,
        "source": "auto"
    });
    {
        let connection = rusqlite::Connection::open(&pm_db_path).unwrap();
        connection
            .execute(
                "INSERT INTO trades(id,symbol,timestamp,side,quantity,execution_price,payload) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                rusqlite::params![
                    "legacy-no-provenance",
                    "BTC-USDT",
                    1i64,
                    "buy",
                    0.5f64,
                    99.0f64,
                    legacy_payload.to_string(),
                ],
            )
            .unwrap();
    }

    unsafe {
        std::env::set_var("POSITION_MANAGEMENT_RSOCKET_URL", &second_pm_endpoint);
    }
    pm_server = Some(
        position_management::positions::start_test_rsocket_service(
            pm_db_path.clone(),
            second_pm_address.to_string(),
        )
        .await
        .expect("restarted PM service should reopen its same SQLite ledger"),
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    let duplicate_worker_event = run_cycle_inner(
        &state,
        versioned_lifecycle_dashboard(fixture_index, "real-pm-source-700101"),
    )
    .await
    .unwrap();
    assert_eq!(duplicate_worker_event.status, "skipped");
    assert_eq!(
        duplicate_worker_event.reason.as_deref(),
        Some("Exploratory source event already reserved for this profile")
    );
    let before_duplicate = crate::service_rpc::position_request("GET", "/positions/trades", None)
        .await
        .unwrap();
    assert_eq!(before_duplicate.status, 200);
    let before_duplicate_trades = before_duplicate.body["trades"].as_array().unwrap().len();
    let legacy = before_duplicate.body["trades"]
        .as_array()
        .unwrap()
        .iter()
        .find(|trade| trade["id"] == "legacy-no-provenance")
        .expect("legacy SQLite row should survive restart");
    assert!(legacy.get("lifecycle").is_none() || legacy["lifecycle"].is_null());

    // Replaying AutoTrader's exact entry request after PM restart must be
    // acknowledged from SQLite and must not invoke Paper Exchange again.
    let retry = submit_configured_order(
        &entry_order.symbol,
        &entry_order.side,
        entry_order.quantity,
        entry_order.reserved_notional,
        entry_order.decision_bid.unwrap_or(100.0),
        entry_order.decision_ask.unwrap_or(100.02),
        AutoOrderType::PostOnlyLimit,
        ExecutionParameters::default(),
        identity.opportunity_id.clone(),
        false,
        Some(&identity),
    )
    .await
    .expect("duplicate entry should be acknowledged from restarted PM ledger");
    assert_eq!(
        retry["filled_quantity"].as_f64(),
        Some(entry_order.quantity)
    );
    let matches_after_retry = responder
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.path == "/internal/orders/match")
        .count();
    assert_eq!(
        matches_after_retry,
        accepted_matches_before_retry + 1,
        "only the exit may add one match; duplicate entry after restart must not rematch"
    );

    let after_duplicate = crate::service_rpc::position_request("GET", "/positions/trades", None)
        .await
        .unwrap();
    assert_eq!(after_duplicate.status, 200);
    assert_eq!(
        after_duplicate.body["trades"].as_array().unwrap().len(),
        before_duplicate_trades,
        "duplicate request must not append a second ledger fill"
    );
    *responder
        .learning_router
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        learning::router(learning_db_path.clone()).unwrap();
    let replay_after_restart = crate::service_rpc::learning_request(
        strategy::ScoutWorkerHttpMethod::Get,
        &replay_path,
        None,
    )
    .await
    .unwrap();
    assert_eq!(replay_after_restart.status, 200);
    assert_eq!(replay_after_restart.body["trade_id"], identity.trade_id);
    assert_eq!(
        replay_after_restart.body["opportunity_id"],
        identity.opportunity_id
    );
    assert_eq!(replay_after_restart.body["episode_id"], identity.episode_id);

    if let Some(server) = pm_server.take() {
        server.abort();
    }
    exchange_and_learning_server.abort();
    let _ = std::fs::remove_file(&state.settings_path);
    let _ = std::fs::remove_file(&sqlite_path);
    let _ = std::fs::remove_file(sqlite_path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(sqlite_path.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(&state.dated_carry_path);
    let _ = std::fs::remove_file(&pm_db_path);
    let _ = std::fs::remove_file(pm_db_path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(pm_db_path.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(&learning_db_path);
    let _ = std::fs::remove_file(learning_db_path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(learning_db_path.with_extension("sqlite-shm"));
}

#[tokio::test]
async fn lost_pm_ack_replays_durable_entry_once_after_autotrader_settings_restart() {
    let _environment = super::service_endpoint_environment_lock();
    crate::service_rpc::clear_rpc_client_pool_for_tests();

    let exchange_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let exchange_address = exchange_listener.local_addr().unwrap();
    drop(exchange_listener);
    let exchange_endpoint = format!("ws://{exchange_address}");
    let pm_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pm_address = pm_listener.local_addr().unwrap();
    drop(pm_listener);
    let pm_endpoint = format!("ws://{pm_address}");
    let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_address = proxy_listener.local_addr().unwrap();
    drop(proxy_listener);
    let proxy_endpoint = format!("ws://{proxy_address}");

    let pm_db_path = unique_path("lost-ack-pm-ledger").with_extension("sqlite");
    let learning_db_path = unique_path("lost-ack-learning").with_extension("sqlite");
    let settings_path = unique_path("lost-ack-auto-trader").with_extension("sqlite");
    let carry_path = unique_path("lost-ack-carry").with_extension("sqlite");
    let learning_router = learning::router(learning_db_path.clone()).unwrap();
    let responder = LifecycleResponder {
        learning_router: Arc::new(StdMutex::new(learning_router)),
        ..LifecycleResponder::default()
    };
    let service = responder.clone();
    let exchange_and_learning_server = tokio::spawn(async move {
        RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(exchange_address.to_string()))
            .acceptor(Box::new(move |_, _| Ok(Box::new(service.clone()))))
            .serve()
            .await
            .expect("test Paper Exchange and Learning services remain available");
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let previous_position_endpoint = std::env::var_os("POSITION_MANAGEMENT_RSOCKET_URL");
    let previous_learning_endpoint = std::env::var_os("LEARNING_RSOCKET_URL");
    let previous_paper_exchange_endpoint = std::env::var_os("PAPER_EXCHANGE_RSOCKET_URL");
    let _restore_endpoints = EndpointEnvironmentRestore {
        position: previous_position_endpoint,
        learning: previous_learning_endpoint,
        paper_exchange: previous_paper_exchange_endpoint,
    };
    // SAFETY: endpoint changes are serialized by the fixture lock and target
    // only local temporary services.
    unsafe {
        std::env::set_var("POSITION_MANAGEMENT_RSOCKET_URL", &proxy_endpoint);
        std::env::set_var("LEARNING_RSOCKET_URL", &exchange_endpoint);
        std::env::set_var("PAPER_EXCHANGE_RSOCKET_URL", &exchange_endpoint);
    }
    position_management::positions::set_test_usd_rates(BTreeMap::from([
        ("USD".to_owned(), 1.0),
        ("USDT".to_owned(), 1.0),
    ]));
    let pm_server = position_management::positions::start_test_rsocket_service(
        pm_db_path.clone(),
        pm_address.to_string(),
    )
    .await
    .expect("real PM test service opens its temporary SQLite ledger");
    let proxy = DropFirstPositionAckProxy {
        position_management_endpoint: pm_endpoint.clone(),
        drop_next_submission_response: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        dropped_submission_responses: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    let proxy_for_acceptor = proxy.clone();
    let proxy_server = tokio::spawn(async move {
        RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(proxy_address.to_string()))
            .acceptor(Box::new(move |_, _| {
                Ok(Box::new(proxy_for_acceptor.clone()))
            }))
            .serve()
            .await
            .expect("ack-loss proxy RSocket service remains available");
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let fixture_index = 700_201;
    let mut state = benchmark_precomputed_ingress_state("lost-pm-ack");
    state.settings_path = settings_path.clone();
    state.dated_carry_path = carry_path.clone();
    let run_result = run_cycle_inner(
        &state,
        versioned_lifecycle_dashboard(fixture_index, "lost-ack-source-700201"),
    )
    .await
    .unwrap();
    assert_eq!(run_result.status, "order_submitting");

    let mut pending_order = None;
    for _ in 0..500 {
        pending_order = state
            .settings
            .lock()
            .await
            .orders
            .iter()
            .find(|order| order.purpose == "signal")
            .cloned();
        if pending_order.as_ref().is_some_and(|order| {
            order
                .failure_reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with("submission_pending_recovery:"))
        }) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pending_order = pending_order.expect("durable entry reservation remains present");
    assert_eq!(pending_order.status, "submitting");
    assert!(
        pending_order
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("submission_pending_recovery:"))
    );
    assert_eq!(
        proxy
            .dropped_submission_responses
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "PM committed one request before the proxy discarded its acknowledgment"
    );
    let initial_pm_history =
        crate::service_rpc::position_request_at(&pm_endpoint, "GET", "/positions/trades", None)
            .await
            .unwrap();
    assert_eq!(initial_pm_history.status, 200);
    assert_eq!(
        initial_pm_history.body["trades"].as_array().unwrap().len(),
        1
    );

    // Simulate process death by recovering the committed Settings snapshot in
    // a new AutoTraderState after PM committed but before AutoTrader applied
    // the response. No runtime database is used by this fixture.
    let mut restored = read_settings_database(&settings_path)
        .unwrap()
        .expect("ambiguous submitting reservation was durably persisted");
    let restored_order = restored
        .orders
        .iter()
        .find(|order| order.timestamp == pending_order.timestamp)
        .expect("restored Settings contains the same pending reservation");
    assert_eq!(restored_order.status, "submitting");
    assert!(
        restored_order
            .revalidation_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.submission_request.as_ref())
            .is_some()
    );
    drop(state);

    let mut restarted_state = benchmark_precomputed_ingress_state("lost-pm-ack-restarted");
    restarted_state.settings_path = settings_path.clone();
    restarted_state.dated_carry_path = carry_path.clone();
    *restarted_state.settings.lock().await = std::mem::take(&mut restored);
    unsafe {
        std::env::set_var("POSITION_MANAGEMENT_RSOCKET_URL", &pm_endpoint);
    }
    recover_submitting_entry_orders(&restarted_state).await;
    // A repeated maintenance pass sees no pending reservation and performs no
    // second request or local fill application.
    recover_submitting_entry_orders(&restarted_state).await;

    let (recovered_order, identity, position) = {
        let settings = restarted_state.settings.lock().await;
        let order = settings
            .orders
            .iter()
            .find(|order| order.timestamp == pending_order.timestamp)
            .cloned()
            .expect("recovered order remains in AutoTrader history");
        let decision_id = order.decision_id.as_deref().unwrap();
        let identity = settings.canonical_lifecycle_links[decision_id].clone();
        let strategy_key = order.strategy.as_deref().unwrap();
        let position = settings.strategy_positions[&order.symbol][strategy_key].clone();
        (order, identity, position)
    };
    assert_eq!(recovered_order.status, "filled");
    assert!((position.quantity - recovered_order.filled_quantity).abs() <= 1e-12);
    assert_eq!(
        position.trade_id.as_deref(),
        recovered_order.decision_id.as_deref(),
        "AutoTrader position retains the durable reservation lookup key"
    );
    assert_eq!(position.entry_fill_links.len(), 1);
    assert_eq!(
        position.initial_risk_reference.as_ref().unwrap().status,
        "complete"
    );
    let recovered_pm_history =
        crate::service_rpc::position_request_at(&pm_endpoint, "GET", "/positions/trades", None)
            .await
            .unwrap();
    let pm_trades = recovered_pm_history.body["trades"].as_array().unwrap();
    assert_eq!(
        pm_trades.len(),
        1,
        "idempotent replay must not create a second fill"
    );
    assert_eq!(
        pm_trades[0]["lifecycle"]["trade_id"], identity.trade_id,
        "the recovered fill retains its canonical lifecycle lineage"
    );
    let match_calls = responder
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.path == "/internal/orders/match")
        .count();
    assert_eq!(
        match_calls, 1,
        "retry is served by PM idempotency before rematching"
    );

    let exit_result = submit_position_exit_order(
        &restarted_state,
        recovered_order.symbol.clone(),
        recovered_order.strategy.clone().unwrap(),
        position,
        100.99,
        101.01,
        "fixture-close",
        0.01,
        1.0,
        2.0,
        recovered_order.timestamp.saturating_add(1),
        serde_json::json!({"fixture": "ack-loss-recovery"}),
    )
    .await
    .unwrap();
    assert!(exit_result.is_some());
    {
        let settings = restarted_state.settings.lock().await;
        let outcomes = settings
            .feedback_outbox
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    strategy::FeedbackEvent::TradeOutcome {
                        trade_id: Some(trade_id),
                        ..
                    } if trade_id == &identity.trade_id
                )
            })
            .count();
        assert_eq!(outcomes, 1, "the recovered trade queues one outcome");
    }
    let completed_pm_history =
        crate::service_rpc::position_request_at(&pm_endpoint, "GET", "/positions/trades", None)
            .await
            .unwrap();
    let completed_trades = completed_pm_history.body["trades"].as_array().unwrap();
    assert_eq!(completed_trades.len(), 2);
    assert!(
        completed_trades
            .iter()
            .all(|trade| trade["lifecycle"]["trade_id"] == identity.trade_id)
    );
    let fill_ids = completed_trades
        .iter()
        .filter_map(|trade| trade["fill_id"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        fill_ids.len(),
        2,
        "replayed entry and real exit have unique PM fills"
    );

    proxy_server.abort();
    pm_server.abort();
    exchange_and_learning_server.abort();
    let _ = std::fs::remove_file(&settings_path);
    let _ = std::fs::remove_file(settings_path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(settings_path.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(&carry_path);
    let _ = std::fs::remove_file(&pm_db_path);
    let _ = std::fs::remove_file(pm_db_path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(pm_db_path.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(&learning_db_path);
    let _ = std::fs::remove_file(learning_db_path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(learning_db_path.with_extension("sqlite-shm"));
}

#[tokio::test]
async fn executable_path_restart_and_learning_gate_exclude_interrupted_trade_evidence() {
    // All prices and fills below are deterministic isolated fixtures. They
    // exercise producer and consumer contracts; they are not historical or
    // empirical trading evidence.
    let learning_db_path = unique_path("executable-path-learning").with_extension("sqlite");
    let learning_router = learning::router(learning_db_path.clone()).unwrap();
    let symbol = "BTC-USDT";
    let strategy_name = "live-momentum";
    let mut specs = Strategy::LiveMomentum
        .parameter_specs()
        .into_iter()
        .map(|spec| strategy::ParameterSpec {
            key: spec.key.as_str().to_owned(),
            label: spec.label.to_owned(),
            default: spec.default,
            min: spec.min,
            max: spec.max,
            step: spec.step,
            unit: spec.unit.to_owned(),
            learning_direction: spec.learning_direction,
        })
        .collect::<Vec<_>>();
    let mut parameters = specs
        .iter()
        .map(|spec| (spec.key.clone(), spec.default))
        .collect::<BTreeMap<_, _>>();
    for (key, value) in [
        ("stop_loss_percent", 0.50),
        ("take_profit_percent", 0.80),
        ("profit_protection_activation_percent", 0.30),
        ("profit_protection_giveback_percent", 0.15),
        ("profit_protection_minimum_lock_percent", 0.05),
        ("trailing_activation_percent", 0.30),
        ("trailing_distance_percent", 0.10),
        ("breakeven_activation_percent", 0.25),
    ] {
        if let Some(spec) = specs.iter().find(|spec| spec.key == key) {
            assert!(
                (spec.min..=spec.max).contains(&value),
                "fixture {key} is in bounds"
            );
            parameters.insert(key.to_owned(), value);
        }
    }
    let profile_id = strategy::profile_id(symbol, strategy_name, &parameters);
    let profile_registration = strategy::StrategyProfileRegistration {
        profile: strategy::StrategyProfile {
            profile_id: profile_id.clone(),
            version: 1,
            symbol: symbol.to_owned(),
            strategy: strategy_name.to_owned(),
            parameters: parameters.clone(),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        },
        specs: std::mem::take(&mut specs),
    };
    let (status, _) = learning_json_request(
        &learning_router,
        "POST",
        "/profiles",
        Some(serde_json::to_value(profile_registration).unwrap()),
    )
    .await;
    assert_eq!(status, 202, "Learning accepts the exact fixture profile");

    let observed_at_ms = current_time_ms();
    let horizon_ms = 30 * 60 * 1_000;
    let exit_timestamp_ms = observed_at_ms.saturating_sub(horizon_ms);
    let entry_timestamp_ms = exit_timestamp_ms.saturating_sub(20_000);
    let path_epoch = "auto-trader-position-path:v1:fixture:trade-1";
    let entry_fill_id = "pm-fill:v1:fixture-entry:1";
    let exit_fill_id = "pm-fill:v1:fixture-exit:1";
    let identity = CanonicalLifecycleIdentity {
        schema_version: 1,
        opportunity_id: "fixture-opportunity-1".into(),
        episode_id: "fixture-episode-1".into(),
        trade_id: "fixture-trade-1".into(),
        profile_id: Some(profile_id.clone()),
        profile_hash: Some("fixture-profile-hash-v1".into()),
        profile_version: Some(1),
        execution_mode: "paper".into(),
        entry_policy: "exploratory".into(),
        source: "auto_trader_entry_reservation".into(),
    };
    let strategy_decision = serde_json::json!({
        "learning_profile_id": profile_id.clone(),
        "learning_profile_version": 1,
        "strategy_parameters": parameters,
    });
    let price_path = [100.10, 99.50, 100.60, 100.20]
        .into_iter()
        .enumerate()
        .map(|(index, bid)| {
            let timestamp = entry_timestamp_ms + (index as u64 + 1) * 5_000;
            TradePriceSample {
                timestamp,
                bid,
                ask: bid + 0.02,
                sequence: index as u64 + 1,
                epoch: Some(path_epoch.to_owned()),
                source_updated_at_ms: Some(timestamp.saturating_sub(10)),
                freshness_contract_version: Some(1),
                freshness_max_age_ms: Some(5_000),
                source_fresh: Some(true),
            }
        })
        .collect::<Vec<_>>();
    let initial_risk = InitialRiskReference {
        schema_version: 1,
        quote_currency: Some("USDT".into()),
        stop_loss_percent: Some(0.50),
        quote_amount: Some(0.50),
        fill_ids: vec![entry_fill_id.into()],
        status: "complete".into(),
    };
    let mut position = TraderPosition {
        quantity: 1.0,
        average_entry_price: 100.0,
        entry_fees: 0.01,
        initial_entry_notional: 100.0,
        total_fees_paid: 0.01,
        total_entry_fees_paid: 0.01,
        entry_fill_ids: vec![entry_fill_id.into()],
        entry_fill_links: vec![PositionEntryFillLink {
            fill_id: entry_fill_id.into(),
            client_order_id: "fixture-entry-order-1".into(),
            symbol: symbol.into(),
            side: "buy".into(),
            quantity: 1.0,
            execution_price: 100.0,
            timestamp_ms: Some(entry_timestamp_ms),
            fee_known: true,
            fee_amount: Some(0.01),
            fee_currency: Some("USDT".into()),
        }],
        entry_fee_currencies: vec!["USDT".into()],
        entry_fill_costs_complete: true,
        initial_risk_reference: Some(initial_risk),
        entry_context: "BTC:live-momentum:long".into(),
        entry_decision_id: Some("fixture-entry-decision-1".into()),
        trade_id: Some(identity.trade_id.clone()),
        lifecycle_identity: Some(identity.clone()),
        strategy: Some(strategy_name.into()),
        strategy_decision: Some(strategy_decision),
        stop_loss_percent: Some(0.50),
        take_profit_percent: Some(0.80),
        exit_parameter_provenance: Some(serde_json::json!({
            "profile_id":profile_id.clone(),
            "profile_version":1,
            "source":"validated_exit_policy",
            "stop_loss":{"selected_percent":0.50},
            "take_profit":{"selected_percent":0.80}
        })),
        price_path: price_path.clone(),
        price_path_epoch: Some(path_epoch.into()),
        price_path_total_samples: price_path.len() as u64,
        entry_timestamp_ms,
        ..TraderPosition::default()
    };
    let mut settings = Settings::default();
    settings
        .strategy_positions
        .entry(symbol.into())
        .or_default()
        .insert(strategy_name.into(), position.clone());
    let snapshot_path = unique_path("executable-path-restart");
    persist_durable_settings_snapshot(&snapshot_path, &settings).unwrap();
    let restored = read_settings_database(&snapshot_path)
        .unwrap()
        .expect("durable open position snapshot restores");
    let restored_position = restored.strategy_positions[symbol][strategy_name].clone();
    assert_eq!(restored_position.price_path.len(), price_path.len());
    assert_eq!(
        restored_position.price_path_total_samples,
        price_path.len() as u64
    );
    assert!(restored_position.price_path_restart_interrupted);
    assert_ne!(
        restored_position.price_path_epoch.as_deref(),
        Some(path_epoch)
    );

    let exit_response = serde_json::json!({
        "filled_quantity":1.0,
        "estimated_fee":0.01,
        "fee_complete":true,
        "fill_evidence":[{
            "id":"fixture-pm-exit-row-1",
            "fill_id":exit_fill_id,
            "parent_order_id":"fixture-exit-order-1",
            "client_order_id":"fixture-exit-order-1",
            "symbol":symbol,
            "side":"sell",
            "quantity":1.0,
            "execution_price":100.20,
            "fee":-0.01,
            "fee_known":true,
            "fee_currency":"USDT"
        }]
    });
    let build_audit = |position: &TraderPosition| {
        position_exit_audit_payload(
            "fixture-entry-decision-1",
            exit_timestamp_ms,
            exit_timestamp_ms.saturating_sub(100),
            symbol,
            "take-profit",
            "sell",
            "Ioc",
            &exit_response,
            Some("fixture-exit-order-1"),
            position.lifecycle_identity.as_ref(),
            position,
            1.0,
            1.0,
            Some(100.20),
            0.01,
            0.18,
            100.0,
            0.0018,
            100.20,
            100.22,
            1.0,
            0.50,
            0.80,
            &serde_json::json!({"fixture":"deterministic-executable-path"}),
        )
    };
    let complete_audit = build_audit(&position);
    assert_eq!(complete_audit["lineage_status"], "complete");
    assert_eq!(complete_audit["price_path_evidence"]["complete"], true);
    assert_eq!(
        complete_audit["price_path_evidence"]["identity_complete"],
        true
    );
    assert_eq!(
        complete_audit["price_path_evidence"]["policy_evaluation_eligible"],
        true
    );
    assert!(
        (complete_audit["price_path_evidence"]["gross_executable_mfe_percent"]
            .as_f64()
            .unwrap()
            - 0.60)
            .abs()
            < 1e-9
    );
    assert!(
        (complete_audit["price_path_evidence"]["gross_executable_mae_percent"]
            .as_f64()
            .unwrap()
            + 0.50)
            .abs()
            < 1e-9
    );
    assert_eq!(
        complete_audit["price_path_evidence"]["fee_adjusted_mfe_mae_status"],
        "not_computed_without_path_level_execution_costs",
        "fixture does not claim unmeasured path-level costs"
    );
    assert_eq!(complete_audit["realized_r_evidence"]["status"], "complete");
    assert!(
        (complete_audit["realized_r_evidence"]["net_realized_r_multiple"]
            .as_f64()
            .unwrap()
            - 0.36)
            .abs()
            < 1e-9
    );

    let interrupted_audit = build_audit(&restored_position);
    assert_eq!(
        interrupted_audit["price_path_evidence"]["continuity_complete"],
        false
    );
    assert_eq!(interrupted_audit["price_path_evidence"]["complete"], false);
    assert_eq!(
        interrupted_audit["price_path_evidence"]["policy_evaluation_eligible"],
        false
    );
    assert!(
        interrupted_audit["price_path_evidence"]["missing_coverage"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "price_path_restart_interrupted")
    );

    let mut pending_settings = Settings::default();
    queue_post_exit_observation(&mut pending_settings, &complete_audit);
    let pending = pending_settings
        .post_exit_observations
        .first()
        .expect("closed trade creates a post-exit evidence request");
    assert_eq!(pending["pre_exit_price_path"].as_array().unwrap().len(), 4);
    assert_eq!(pending["pre_exit_price_path_evidence"]["complete"], true);
    assert_eq!(pending["realized_r_evidence"]["status"], "complete");
    assert_eq!(pending["entry_context"]["entry_fill_costs_complete"], true);

    let make_learning_observation = |audit: &serde_json::Value,
                                     event_id: &str,
                                     pre_exit_complete: bool| {
        let post_exit_path = (0..=30)
            .map(|minute| {
                let timestamp = exit_timestamp_ms + minute * 60_000;
                let bid = 100.20 + minute as f64 * 0.001;
                serde_json::json!({"timestamp":timestamp,"bid":bid,"ask":bid + 0.02})
            })
            .collect::<Vec<_>>();
        let mut pre_exit_evidence = audit["price_path_evidence"].clone();
        if !pre_exit_complete {
            pre_exit_evidence["complete"] = serde_json::json!(false);
        }
        serde_json::json!({
            "event_id":event_id,
            "symbol":symbol,
            "strategy":strategy_name,
            "reason":"take-profit",
            "side":"sell",
            "exit_timestamp_ms":exit_timestamp_ms,
            "observed_at_ms":observed_at_ms,
            "horizon_ms":horizon_ms,
            "entry_context":audit["entry_context"],
            "strategy_decision":audit["entry_context"]["strategy_decision"],
            "exit_parameter_provenance":audit["exit_parameter_provenance"],
            "effective_stop_loss_percent":audit["effective_stop_loss_percent"],
            "effective_take_profit_percent":audit["effective_take_profit_percent"],
            "counterfactual_exit_fee_rate":configured_taker_fee_rate(),
            "exit_return_percent":audit["net_return_percent"],
            "post_exit_return_percent":0.10,
            "missed_return_percent":0.10,
            "pre_exit_price_path":audit["price_path"],
            "pre_exit_price_path_evidence":pre_exit_evidence,
            "exit_fill_evidence":audit["exit_fill_evidence"],
            "realized_r_evidence":audit["realized_r_evidence"],
            "independent_reconciliation":{"status":"pending","source":null,"evidence_epoch_id":null},
            "price_path":post_exit_path,
            "price_path_evidence":{"schema_version":1,"complete":true,"sample_count":31,"fresh_sample_count":31,"fee_adjusted":false}
        })
    };
    let complete_learning_observation =
        make_learning_observation(&complete_audit, "fixture-exit-timing-complete", true);
    let interrupted_learning_observation = make_learning_observation(
        &interrupted_audit,
        "fixture-exit-timing-restart-interrupted",
        false,
    );
    let mut accepted = 0;
    for observation in [
        complete_learning_observation,
        interrupted_learning_observation,
    ] {
        let (status, response) =
            learning_json_request(&learning_router, "POST", "/exit-timing", Some(observation))
                .await;
        assert_eq!(
            status, 200,
            "Learning accepts durable evidence input: {response}"
        );
        assert_eq!(response["accepted"], true);
        accepted += 1;
    }

    let (status, snapshot) =
        learning_json_request(&learning_router, "GET", "/decision-snapshot", None).await;
    assert_eq!(status, 200);
    let assessment = snapshot["exit_policy_assessments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|assessment| {
            assessment["symbol"] == symbol
                && assessment["strategy"] == strategy_name
                && assessment["profile_id"] == profile_id
                && assessment["profile_version"] == 1
        })
        .expect("Learning reports the exact profile's replay eligibility");
    assert_eq!(accepted, 2, "both raw observations were durably accepted");
    assert_eq!(
        assessment["sample_count"], 1,
        "the interrupted path is excluded"
    );
    assert_eq!(assessment["status"], "insufficient_evidence");
    assert_eq!(
        assessment["metrics"]["coverage"]["executable_path_source_coverage_percent"]["sample_count"],
        1
    );
    assert_eq!(
        assessment["metrics"]["values"]["executable_path_source_coverage_percent"],
        50.0
    );
    assert!(
        assessment["metrics"]["promotion_blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker == "insufficient_complete_executable_path_observations")
    );
    assert!(
        snapshot["exit_protection_policies"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let _ = std::fs::remove_file(&snapshot_path);
    let _ = std::fs::remove_file(snapshot_path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(snapshot_path.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(&learning_db_path);
    let _ = std::fs::remove_file(learning_db_path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(learning_db_path.with_extension("sqlite-shm"));
}
