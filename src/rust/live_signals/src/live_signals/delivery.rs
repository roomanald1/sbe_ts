use super::*;

const SCOUT_SIGNALS_PER_BATCH: usize = 20;
const SCOUT_DELIVERY_TIMEOUT: Duration = Duration::from_secs(5);

async fn post_dashboard(
    client: &reqwest::Client,
    endpoint: &str,
    dashboard: &Value,
) -> Result<(), reqwest::Error> {
    client
        .post(endpoint)
        .timeout(Duration::from_secs(2))
        .json(dashboard)
        .send()
        .await?
        .error_for_status()?;
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
    client: &reqwest::Client,
    endpoint: &str,
    payload: &Value,
) -> Result<(), reqwest::Error> {
    client
        .post(endpoint)
        .timeout(SCOUT_DELIVERY_TIMEOUT)
        .json(payload)
        .send()
        .await?
        .error_for_status()?;
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

async fn deliver_pending_once(state: &LiveState, client: &reqwest::Client, base: &str) -> bool {
    let mut values = state
        .pending_delivery
        .iter()
        .map(|entry| (entry.key().clone(), entry.value().clone()))
        .collect::<Vec<_>>();
    values.sort_by(|a, b| a.0.cmp(&b.0));

    let auto_trader_delivery = futures_util::stream::iter(values)
        .map(|(symbol, dashboard)| {
            let client = client.clone();
            let endpoint = format!("{base}/internal/auto-trader/signal");
            async move {
                let started = Instant::now();
                let result = post_dashboard(&client, &endpoint, &dashboard).await;
                crate::latency::record_auto_trader_delivery(started.elapsed());
                (symbol, dashboard, result)
            }
        })
        .buffer_unordered(PUBLISH_CONCURRENCY)
        .collect::<Vec<_>>();
    let results = auto_trader_delivery.await;
    let mut failed = false;
    for (symbol, dashboard, result) in results {
        match result {
            Ok(()) => {
                acknowledge_delivery(state, &symbol, &dashboard);
            }
            Err(error) => {
                failed = true;
                crate::service_log!("WARN", "Live signal delivery failed for {symbol}: {error}");
            }
        }
    }
    failed
}

async fn deliver_scout_pending_once(
    state: &LiveState,
    client: &reqwest::Client,
    endpoint: &str,
) -> bool {
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
        match post_scout_dashboard(client, endpoint, &payload).await {
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
    let client = HTTP_CLIENT.get_or_init(reqwest::Client::new).clone();
    let auto_trader_state = Arc::clone(&state);
    let auto_trader_client = client.clone();
    tokio::spawn(async move {
        let base =
            std::env::var("AUTO_TRADER_URL").unwrap_or_else(|_| "http://127.0.0.1:3005".into());
        let mut retry = Duration::from_millis(250);
        loop {
            let notified = auto_trader_state.delivery_notify.notified();
            tokio::pin!(notified);
            if auto_trader_state.pending_delivery.is_empty() {
                notified.await;
                continue;
            }
            if deliver_pending_once(&auto_trader_state, &auto_trader_client, &base).await {
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(Duration::from_secs(5));
            } else {
                retry = Duration::from_millis(250);
            }
        }
    });

    tokio::spawn(async move {
        let base =
            std::env::var("SCOUTS_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3011".into());
        let endpoint = format!("{}/live-inputs", base.trim_end_matches('/'));
        let mut retry = Duration::from_millis(250);
        loop {
            let notified = state.scout_delivery_notify.notified();
            tokio::pin!(notified);
            if state.pending_scout_delivery.is_empty() {
                notified.await;
                continue;
            }
            if deliver_scout_pending_once(&state, &client, &endpoint).await {
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
    use axum::{Json, Router, routing::post};
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
    async fn scout_delivery_allows_handler_to_complete_past_previous_timeout() {
        let app = Router::new().route(
            "/live-inputs",
            post(|| async {
                tokio::time::sleep(Duration::from_millis(350)).await;
                axum::http::StatusCode::ACCEPTED
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        post_scout_dashboard(
            &reqwest::Client::new(),
            &format!("http://{address}/live-inputs"),
            &json!({"signals": [{"symbol": "BTC-USDT"}]}),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn posts_dashboard_payload_and_accepts_success_status() {
        let captured = Arc::new(StdMutex::new(None::<Value>));
        let app = Router::new().route(
            "/signal",
            post({
                let captured = Arc::clone(&captured);
                move |Json(payload): Json<Value>| {
                    *captured.lock().unwrap() = Some(payload);
                    async { axum::http::StatusCode::NO_CONTENT }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let payload = json!({"best": [{"symbol": "BTC-USDT"}]});

        post_dashboard(
            &reqwest::Client::new(),
            &format!("http://{address}/signal"),
            &payload,
        )
        .await
        .unwrap();

        assert_eq!(captured.lock().unwrap().as_ref(), Some(&payload));
    }

    #[tokio::test]
    async fn post_dashboard_reports_server_failure() {
        let app = Router::new().route(
            "/signal",
            post(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        assert!(
            post_dashboard(
                &reqwest::Client::new(),
                &format!("http://{address}/signal"),
                &json!({"best": []}),
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn delivery_cycle_removes_acknowledged_items_and_retains_failures() {
        let ok_app = Router::new().route(
            "/internal/auto-trader/signal",
            post(|| async { axum::http::StatusCode::NO_CONTENT }),
        );
        let ok_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ok_address = ok_listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(ok_listener, ok_app).await;
        });

        let state = state();
        state
            .pending_delivery
            .insert("BTC-USDT".into(), json!({"best": []}));
        state
            .pending_scout_delivery
            .insert("BTC-USDT".into(), json!({"best": []}));
        assert!(
            !deliver_pending_once(
                &state,
                &reqwest::Client::new(),
                &format!("http://{ok_address}"),
            )
            .await
        );
        assert!(state.pending_delivery.is_empty());
        assert!(state.pending_scout_delivery.contains_key("BTC-USDT"));

        let fail_app = Router::new().route(
            "/internal/auto-trader/signal",
            post(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
        );
        let fail_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fail_address = fail_listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(fail_listener, fail_app).await;
        });
        state
            .pending_delivery
            .insert("ETH-USDT".into(), json!({"best": []}));

        assert!(
            deliver_pending_once(
                &state,
                &reqwest::Client::new(),
                &format!("http://{fail_address}"),
            )
            .await
        );
        assert!(state.pending_delivery.contains_key("ETH-USDT"));
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
