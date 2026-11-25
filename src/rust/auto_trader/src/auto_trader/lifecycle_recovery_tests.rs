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
            let body = serde_json::from_slice(&body)?;
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
                    let response = idempotent.entry(client_order_id).or_insert_with(|| {
                    self.accepted_order_count
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    serde_json::json!({
                        "status": "filled",
                        "filled_quantity": body["quantity"],
                        "average_execution_price": if body["side"] == "buy" { 100.0 } else { 101.0 },
                        "estimated_fee": 0.01,
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

fn unique_path(label: &str) -> PathBuf {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    std::env::temp_dir().join(format!(
        "auto-trader-lifecycle-{}-{}-{label}.json",
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ))
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
    }
}

#[tokio::test]
async fn selected_entry_exit_outbox_and_sqlite_restart_keep_one_lifecycle() {
    let _environment = super::service_endpoint_environment_lock();
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
    let mut state = benchmark_precomputed_ingress_state("lifecycle");
    let dashboard = benchmark_precomputed_ingress_dashboard(fixture_index, "BTC-USDT");
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

    // Replaying a fresh production cycle for the same stable worker event must
    // stop at the durable exploratory reservation check, before the existing
    // position could mask a missing dedup check.
    let first_signal = benchmark_precomputed_ingress_dashboard(fixture_index, "BTC-USDT")
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
        benchmark_precomputed_ingress_dashboard(fixture_index, "BTC-USDT"),
    )
    .await
    .unwrap();
    assert_eq!(duplicate.status, "skipped");
    assert_eq!(
        duplicate.reason.as_deref(),
        Some("Exploratory source event already reserved for this profile")
    );
    {
        let settings = state.settings.lock().await;
        assert_eq!(settings.orders.len(), order_count_before_replay);
        assert_eq!(settings.exploratory_event_reservations.len(), 1);
        assert!(settings
            .exploratory_event_reservations
            .contains_key(&event_key));
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
        benchmark_precomputed_ingress_dashboard(fixture_index, "BTC-USDT"),
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
        strategy_key,
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
    assert_eq!(learning_result.status, 200);
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
        let mut dashboard = benchmark_precomputed_ingress_dashboard(fixture_index, "BTC-USDT");
        let signal = &mut dashboard.best[0];
        let decision = signal.strategy_decision.as_mut().unwrap();
        match case {
            "new_event" => {
                decision["live_worker_event_id"] = serde_json::json!("different-worker-event");
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
            }
            "no_stable_id" => {
                decision
                    .as_object_mut()
                    .unwrap()
                    .remove("live_worker_event_id");
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
    let dashboard = benchmark_precomputed_ingress_dashboard(fixture_index, "BTC-USDT");
    let entry_result = run_cycle_inner(&state, dashboard).await.unwrap();
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
        strategy_key,
        position,
        100.99,
        101.01,
        "fixture-close",
        0.01,
        1.0,
        2.0,
        exit_timestamp,
        serde_json::json!({"fixture": "real-position-management-ledger"}),
    )
    .await
    .unwrap();
    assert!(exit_result.is_some());

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
    assert_eq!(learning_result.status, 200);
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
