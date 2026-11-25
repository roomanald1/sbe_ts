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
                    (
                        200,
                        serde_json::json!({
                            "status": "armed",
                            "filled_quantity": 0.0,
                            "average_execution_price": null,
                            "estimated_fee": 0.0,
                            "barrier_order": {"id": "fixture-barrier", "created_at": 1}
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
                            self.accepted_order_count
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                                    "fill_id": format!("pm-fill:v1:fixture:{client_order_id}"),
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
            ("GET", "/positions/barriers") => (200, serde_json::json!([])),
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
    let strategy = Strategy::from_name("live-momentum").unwrap();
    let budget = {
        let settings = state.settings.lock().await;
        signal_weighted_strategy_budget(
            &settings,
            &dashboard,
            strategy,
            StrategyCategory::Intraday,
            current_time_ms(),
        )
    };
    assert!(
        budget > 0.0,
        "fresh active candidate must retain its opportunity floor"
    );
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
