use super::*;
use rsocket_rust::Client;

const SCOUT_SIGNALS_PER_BATCH: usize = 20;

#[derive(Debug)]
enum DashboardDeliveryError {
    Transport(anyhow::Error),
    Rejected(u16),
}

struct DeliveryPass {
    failed: bool,
    reconnect: bool,
}

async fn connect_auto_trader(rsocket_url: &str) -> anyhow::Result<Client> {
    Ok(RSocketFactory::connect()
        .transport(WebsocketClientTransport::from(rsocket_url))
        .acceptor(Box::new(|| Box::new(EchoRSocket)))
        .start()
        .await?)
}

async fn post_dashboard(client: &Client, dashboard: &Value) -> Result<(), DashboardDeliveryError> {
    let request_id = format!(
        "live-signals:{}:{}",
        dashboard["updated_at_ms"].as_u64().unwrap_or_default(),
        dashboard["best"][0]["symbol"].as_str().unwrap_or("unknown")
    );
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        client.request_response(
            Payload::builder()
                .set_data(
                    serde_json::to_vec(&strategies::ScoutCoordinatorRequest::WorkerRoute(
                        strategies::ScoutCoordinatorRouteCommand {
                            request_id,
                            strategy: None,
                            method: strategies::ScoutWorkerHttpMethod::Post,
                            path: "/internal/auto-trader/signal".into(),
                            payload: dashboard.clone(),
                        },
                    ))
                    .map_err(|error| DashboardDeliveryError::Transport(error.into()))?,
                )
                .build(),
        ),
    )
    .await
    .map_err(|_| {
        DashboardDeliveryError::Transport(anyhow::anyhow!("AutoTrader RSocket request timed out"))
    })?
    .map_err(|error| DashboardDeliveryError::Transport(error.into()))?
    .ok_or_else(|| {
        DashboardDeliveryError::Transport(anyhow::anyhow!(
            "AutoTrader returned an empty RSocket response"
        ))
    })?;
    let response: strategies::ScoutCoordinatorResponse = serde_json::from_slice(
        response
            .data()
            .ok_or_else(|| {
                DashboardDeliveryError::Transport(anyhow::anyhow!(
                    "AutoTrader RSocket response had no data"
                ))
            })?
            .as_ref(),
    )
    .map_err(|error| DashboardDeliveryError::Transport(error.into()))?;
    let strategies::ScoutCoordinatorResponse::WorkerRoute(response) = response else {
        return Err(DashboardDeliveryError::Transport(anyhow::anyhow!(
            "AutoTrader returned an unexpected RSocket response"
        )));
    };
    if response.status >= 400 {
        return Err(DashboardDeliveryError::Rejected(response.status));
    }
    Ok(())
}

fn scout_payload(dashboard: &Value) -> Option<Value> {
    let mut signals = dashboard.get("best")?.as_array()?.clone();
    let markets = dashboard.get("markets");
    signals.retain_mut(|signal| {
        let Some(symbol) = signal.get("symbol").and_then(Value::as_str) else {
            return false;
        };
        let Some(market) = markets.and_then(|markets| markets.get(symbol)) else {
            return false;
        };
        let bid = market
            .get("bids")
            .and_then(Value::as_array)
            .and_then(|levels| levels.first())
            .and_then(|level| level.get("price"))
            .and_then(Value::as_f64);
        let ask = market
            .get("asks")
            .and_then(Value::as_array)
            .and_then(|levels| levels.first())
            .and_then(|level| level.get("price"))
            .and_then(Value::as_f64);
        let updated = market.get("updated_at_ms").and_then(Value::as_u64);
        let (Some(bid), Some(ask), Some(updated)) = (bid, ask, updated) else {
            return false;
        };
        if !bid.is_finite() || !ask.is_finite() || bid <= 0.0 || ask < bid {
            return false;
        }
        signal["best_bid"] = json!(bid);
        signal["best_ask"] = json!(ask);
        signal["book_updated_at_ms"] = json!(updated);
        signal["price"] = json!(bid + (ask - bid) / 2.0);
        true
    });
    (!signals.is_empty()).then(|| json!({"signals":signals}))
}

#[cfg(test)]
fn scout_payload_batches<'a>(dashboards: impl Iterator<Item = &'a Value>) -> Vec<Value> {
    let signals = dashboards
        .filter_map(scout_payload)
        .filter_map(|payload| payload.get("signals").and_then(Value::as_array).cloned())
        .flatten()
        .collect::<Vec<_>>();

    signals
        .chunks(SCOUT_SIGNALS_PER_BATCH)
        .map(|signals| json!({"signals": signals}))
        .collect()
}

async fn post_scout_dashboard(
    learning_rsocket_url: &str,
    request_id: &str,
    payload: &Value,
) -> anyhow::Result<()> {
    let response = strategies::request_scout_worker_route(
        learning_rsocket_url,
        &strategies::ScoutCoordinatorRouteCommand {
            request_id: request_id.to_owned(),
            strategy: None,
            method: strategies::ScoutWorkerHttpMethod::Post,
            path: "/live-inputs".into(),
            payload: payload.clone(),
        },
    )
    .await?;
    if response.status >= 400 {
        anyhow::bail!(
            "Learning rejected Scout live inputs: status {}",
            response.status
        );
    }
    Ok(())
}

fn acknowledge_delivery(state: &LiveState, symbol: &str, dashboard: &Value) -> bool {
    if state
        .pending_delivery
        .get(symbol)
        .is_some_and(|latest| *latest == *dashboard)
    {
        state.pending_delivery.remove(symbol);
        true
    } else {
        false
    }
}

fn acknowledge_scout_delivery(state: &LiveState, symbol: &str, dashboard: &Value) -> bool {
    if state
        .pending_scout_delivery
        .get(symbol)
        .is_some_and(|latest| *latest == *dashboard)
    {
        state.pending_scout_delivery.remove(symbol);
        true
    } else {
        false
    }
}

async fn deliver_pending_once(state: &LiveState, client: &Client) -> DeliveryPass {
    let mut values = state
        .pending_delivery
        .iter()
        .map(|entry| (entry.key().clone(), entry.value().clone()))
        .collect::<Vec<_>>();
    values.sort_by(|a, b| a.0.cmp(&b.0));

    let auto_trader_delivery = futures_util::stream::iter(values)
        .map(|(symbol, dashboard)| async move {
            let started = Instant::now();
            let result = post_dashboard(client, &dashboard).await;
            crate::latency::record_auto_trader_delivery(started.elapsed());
            (symbol, dashboard, result)
        })
        .buffer_unordered(PUBLISH_CONCURRENCY)
        .collect::<Vec<_>>();
    let results = auto_trader_delivery.await;
    let mut pass = DeliveryPass {
        failed: false,
        reconnect: false,
    };
    for (symbol, dashboard, result) in results {
        match result {
            Ok(()) => {
                acknowledge_delivery(state, &symbol, &dashboard);
            }
            Err(DashboardDeliveryError::Transport(error)) => {
                pass.failed = true;
                pass.reconnect = true;
                crate::service_log!("WARN", "Live signal delivery failed for {symbol}: {error}");
            }
            Err(DashboardDeliveryError::Rejected(status)) => {
                pass.failed = true;
                crate::service_log!(
                    "WARN",
                    "AutoTrader rejected live signal for {symbol}: status {status}"
                );
            }
        }
    }
    pass
}

async fn deliver_scout_pending_once(state: &LiveState, learning_rsocket_url: &str) -> bool {
    let mut values = state
        .pending_scout_delivery
        .iter()
        .map(|entry| (entry.key().clone(), entry.value().clone()))
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.0.cmp(&right.0));

    let signals = values
        .into_iter()
        .filter_map(|(symbol, dashboard)| {
            let signal = scout_payload(&dashboard)?
                .get("signals")?
                .as_array()?
                .first()?
                .clone();
            Some((symbol, dashboard, signal))
        })
        .collect::<Vec<_>>();

    let mut failed = false;
    for batch in signals.chunks(SCOUT_SIGNALS_PER_BATCH) {
        let payload = json!({
            "signals": batch.iter().map(|(_, _, signal)| signal.clone()).collect::<Vec<_>>()
        });
        let request_id = format!(
            "live-signals:{}:{}",
            batch
                .first()
                .map(|(_, dashboard, _)| dashboard["updated_at_ms"].as_u64().unwrap_or_default())
                .unwrap_or_default(),
            batch
                .iter()
                .map(|(symbol, _, _)| symbol.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );
        match post_scout_dashboard(learning_rsocket_url, &request_id, &payload).await {
            Ok(()) => {
                for (symbol, dashboard, _) in batch {
                    acknowledge_scout_delivery(state, symbol, dashboard);
                }
            }
            Err(error) => {
                failed = true;
                crate::service_log!("WARN", "Live Scout batch input delivery failed: {error}");
            }
        }
    }
    failed
}

pub(super) async fn run_delivery_worker(state: Arc<LiveState>) {
    let auto_trader_state = Arc::clone(&state);
    tokio::spawn(async move {
        let rsocket_url = std::env::var("AUTO_TRADER_RSOCKET_URL")
            .unwrap_or_else(|_| "ws://127.0.0.1:10006".into());
        let mut retry = Duration::from_millis(250);
        let mut client = None;
        loop {
            let notified = auto_trader_state.delivery_notify.notified();
            tokio::pin!(notified);
            if auto_trader_state.pending_delivery.is_empty() {
                notified.await;
                continue;
            }
            if client.is_none() {
                match connect_auto_trader(&rsocket_url).await {
                    Ok(connected) => client = Some(connected),
                    Err(error) => {
                        crate::service_log!(
                            "WARN",
                            "Could not connect to AutoTrader RSocket: {error}"
                        );
                        tokio::time::sleep(retry).await;
                        retry = (retry * 2).min(Duration::from_secs(5));
                        continue;
                    }
                }
            }
            let pass = deliver_pending_once(&auto_trader_state, client.as_ref().unwrap()).await;
            if pass.reconnect {
                client = None;
            }
            if pass.failed {
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(Duration::from_secs(5));
            } else {
                retry = Duration::from_millis(250);
            }
        }
    });

    tokio::spawn(async move {
        let learning_rsocket_url =
            std::env::var("LEARNING_RSOCKET_URL").unwrap_or_else(|_| "ws://127.0.0.1:10010".into());
        let mut retry = Duration::from_millis(250);
        loop {
            let notified = state.scout_delivery_notify.notified();
            tokio::pin!(notified);
            if state.pending_scout_delivery.is_empty() {
                notified.await;
                continue;
            }
            if deliver_scout_pending_once(&state, &learning_rsocket_url).await {
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(Duration::from_secs(5));
            } else {
                retry = Duration::from_millis(250);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn state() -> LiveState {
        let (_, news) = watch::channel(Arc::new(NewsContext::default()));
        LiveState::new(news)
    }

    #[test]
    fn scout_payload_adds_fresh_top_of_book_to_live_signals() {
        let dashboard = json!({
            "best":[{"symbol":"BTC-USDT","direction":"bullish"}],
            "markets":{"BTC-USDT":{"bids":[{"price":99.0}],"asks":[{"price":101.0}],"updated_at_ms":1234}}
        });
        let payload = scout_payload(&dashboard).unwrap();
        assert_eq!(payload["signals"][0]["best_bid"], 99.0);
        assert_eq!(payload["signals"][0]["best_ask"], 101.0);
        assert_eq!(payload["signals"][0]["book_updated_at_ms"], 1234);
        assert_eq!(payload["signals"][0]["price"], 100.0);
    }

    #[test]
    fn scout_payload_batches_combine_symbols_and_respect_scout_plan_limit() {
        let dashboards = (0..SCOUT_SIGNALS_PER_BATCH + 1)
            .map(|index| {
                let symbol = format!("TEST-{index}");
                let mut markets = serde_json::Map::new();
                markets.insert(
                    symbol.clone(),
                    json!({
                        "bids": [{"price": 99.0}],
                        "asks": [{"price": 101.0}],
                        "updated_at_ms": 1234,
                    }),
                );
                json!({
                    "best": [{"symbol": symbol}],
                    "markets": markets
                })
            })
            .collect::<Vec<_>>();

        let batches = scout_payload_batches(dashboards.iter());

        assert_eq!(batches.len(), 2);
        assert_eq!(
            batches[0]["signals"].as_array().unwrap().len(),
            SCOUT_SIGNALS_PER_BATCH
        );
        assert_eq!(batches[1]["signals"].as_array().unwrap().len(), 1);
        assert_eq!(batches[0]["signals"][0]["symbol"], "TEST-0");
        assert_eq!(batches[1]["signals"][0]["symbol"], "TEST-20");
    }

    #[tokio::test]
    async fn scout_delivery_uses_learning_rsocket_and_acknowledges_after_success() {
        use rsocket_rust_transport_websocket::WebsocketServerTransport;

        struct TestCoordinator(Arc<StdMutex<Option<Value>>>);

        #[rsocket_rust::async_trait]
        impl RSocket for TestCoordinator {
            async fn metadata_push(&self, _request: Payload) -> anyhow::Result<()> {
                anyhow::bail!("unsupported")
            }
            async fn fire_and_forget(&self, _request: Payload) -> anyhow::Result<()> {
                anyhow::bail!("unsupported")
            }
            async fn request_response(&self, request: Payload) -> anyhow::Result<Option<Payload>> {
                let request: strategies::ScoutCoordinatorRequest =
                    serde_json::from_slice(request.data().unwrap())?;
                let strategies::ScoutCoordinatorRequest::WorkerRoute(request) = request else {
                    anyhow::bail!("expected worker route command")
                };
                assert_eq!(request.path, "/live-inputs");
                *self.0.lock().unwrap() = Some(request.payload);
                let response = strategies::ScoutCoordinatorResponse::WorkerRoute(
                    strategies::ScoutCoordinatorRouteResult {
                        request_id: request.request_id,
                        status: 204,
                        body: Value::Null,
                    },
                );
                Ok(Some(
                    Payload::builder()
                        .set_data(serde_json::to_vec(&response)?)
                        .build(),
                ))
            }
            fn request_stream(
                &self,
                _request: Payload,
            ) -> futures_util::stream::BoxStream<'static, anyhow::Result<Payload>> {
                futures_util::stream::empty().boxed()
            }
            fn request_channel(
                &self,
                requests: Flux<anyhow::Result<Payload>>,
            ) -> Flux<anyhow::Result<Payload>> {
                requests
            }
        }

        let received = Arc::new(StdMutex::new(None));
        let worker_received = Arc::clone(&received);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_setup, _socket| {
                Ok(Box::new(TestCoordinator(Arc::clone(&worker_received))))
            }))
            .serve();
        let server_task = tokio::spawn(async move { server.await });
        tokio::time::sleep(Duration::from_millis(20)).await;

        let state = state();
        let dashboard = json!({
            "updated_at_ms": 1234,
            "best": [{"symbol":"BTC-USDT"}],
            "markets": {"BTC-USDT": {"bids":[{"price":99.0}],"asks":[{"price":101.0}],"updated_at_ms":1234}}
        });
        state
            .pending_scout_delivery
            .insert("BTC-USDT".into(), dashboard.clone());
        let url = format!("ws://{address}");
        assert!(!deliver_scout_pending_once(&state, &url).await);
        assert!(state.pending_scout_delivery.is_empty());
        assert_eq!(
            received.lock().unwrap().as_ref().unwrap()["signals"][0]["symbol"],
            "BTC-USDT"
        );

        server_task.abort();
    }

    struct TestRouteResponder {
        payloads: Arc<StdMutex<Vec<Value>>>,
        status: u16,
    }

    #[rsocket_rust::async_trait]
    impl RSocket for TestRouteResponder {
        async fn metadata_push(&self, _request: Payload) -> anyhow::Result<()> {
            anyhow::bail!("unsupported")
        }
        async fn fire_and_forget(&self, _request: Payload) -> anyhow::Result<()> {
            anyhow::bail!("unsupported")
        }
        async fn request_response(&self, request: Payload) -> anyhow::Result<Option<Payload>> {
            let request: strategies::ScoutCoordinatorRequest =
                serde_json::from_slice(request.data().unwrap())?;
            let strategies::ScoutCoordinatorRequest::WorkerRoute(request) = request else {
                anyhow::bail!("expected route command")
            };
            assert_eq!(request.path, "/internal/auto-trader/signal");
            self.payloads.lock().unwrap().push(request.payload);
            let response = strategies::ScoutCoordinatorResponse::WorkerRoute(
                strategies::ScoutCoordinatorRouteResult {
                    request_id: request.request_id,
                    status: self.status,
                    body: Value::Null,
                },
            );
            Ok(Some(
                Payload::builder()
                    .set_data(serde_json::to_vec(&response)?)
                    .build(),
            ))
        }
        fn request_stream(
            &self,
            _request: Payload,
        ) -> futures_util::stream::BoxStream<'static, anyhow::Result<Payload>> {
            futures_util::stream::empty().boxed()
        }
        fn request_channel(
            &self,
            requests: Flux<anyhow::Result<Payload>>,
        ) -> Flux<anyhow::Result<Payload>> {
            requests
        }
    }

    async fn start_route_responder(
        status: u16,
    ) -> (
        String,
        Arc<StdMutex<Vec<Value>>>,
        Arc<std::sync::atomic::AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        use rsocket_rust_transport_websocket::WebsocketServerTransport;

        let payloads = Arc::new(StdMutex::new(Vec::new()));
        let worker_payloads = Arc::clone(&payloads);
        let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_connections = Arc::clone(&connections);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_setup, _socket| {
                worker_connections.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(Box::new(TestRouteResponder {
                    payloads: Arc::clone(&worker_payloads),
                    status,
                }))
            }))
            .serve();
        let task = tokio::spawn(async move {
            let _ = server.await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        (format!("ws://{address}"), payloads, connections, task)
    }

    #[tokio::test]
    async fn delivery_cycle_uses_rsocket_and_acknowledges_only_success() {
        let (ok_url, received, _, ok_task) = start_route_responder(204).await;
        let (fail_url, _, _, fail_task) = start_route_responder(503).await;
        let ok_client = connect_auto_trader(&ok_url).await.unwrap();
        let fail_client = connect_auto_trader(&fail_url).await.unwrap();
        let state = state();
        let payload = json!({"updated_at_ms":1234,"best":[{"symbol":"BTC-USDT"}]});
        state
            .pending_delivery
            .insert("BTC-USDT".into(), payload.clone());
        assert!(!deliver_pending_once(&state, &ok_client).await.failed);
        assert!(state.pending_delivery.is_empty());
        assert_eq!(received.lock().unwrap().as_slice(), &[payload]);

        state
            .pending_delivery
            .insert("ETH-USDT".into(), json!({"best":[]}));
        assert!(deliver_pending_once(&state, &fail_client).await.failed);
        assert!(state.pending_delivery.contains_key("ETH-USDT"));
        ok_task.abort();
        fail_task.abort();
    }

    #[tokio::test]
    async fn live_signal_delivery_reuses_one_rsocket_connection_for_multiple_requests() {
        let (url, received, connections, server_task) = start_route_responder(204).await;
        let client = connect_auto_trader(&url).await.unwrap();

        for (timestamp, symbol) in [(1, "BTC-USDT"), (2, "ETH-USDT")] {
            let dashboard = json!({
                "updated_at_ms": timestamp,
                "best": [{"symbol": symbol}],
            });
            post_dashboard(&client, &dashboard).await.unwrap();
        }

        assert_eq!(connections.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(received.lock().unwrap().len(), 2);
        server_task.abort();
    }

    #[test]
    fn acknowledges_only_the_dashboard_that_was_sent() {
        let state = state();
        let sent = json!({"best": [{"timestamp": 1}]});
        state
            .pending_delivery
            .insert("BTC-USDT".into(), sent.clone());
        state
            .pending_delivery
            .insert("ETH-USDT".into(), json!({"best": [{"timestamp": 2}]}));

        assert!(acknowledge_delivery(&state, "BTC-USDT", &sent));
        assert!(!state.pending_delivery.contains_key("BTC-USDT"));
        assert!(!acknowledge_delivery(&state, "ETH-USDT", &sent));
        assert!(state.pending_delivery.contains_key("ETH-USDT"));
    }

    #[test]
    fn acknowledges_only_the_scout_dashboard_that_was_sent() {
        let state = state();
        let sent = json!({"best": [{"timestamp": 1}]});
        state
            .pending_scout_delivery
            .insert("BTC-USDT".into(), sent.clone());
        state
            .pending_scout_delivery
            .insert("ETH-USDT".into(), json!({"best": [{"timestamp": 2}]}));

        assert!(acknowledge_scout_delivery(&state, "BTC-USDT", &sent));
        assert!(!state.pending_scout_delivery.contains_key("BTC-USDT"));
        assert!(!acknowledge_scout_delivery(&state, "ETH-USDT", &sent));
        assert!(state.pending_scout_delivery.contains_key("ETH-USDT"));
    }
}
