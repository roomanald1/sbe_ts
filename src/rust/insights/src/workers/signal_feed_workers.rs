fn is_raw_symbol_price(bytes: &[u8]) -> bool {
    bytes.len() >= 56
        && u16::from_le_bytes(bytes[0..2].try_into().unwrap_or_default()) == 48
        && u16::from_le_bytes(bytes[2..4].try_into().unwrap_or_default()) == 1
        && u16::from_le_bytes(bytes[4..6].try_into().unwrap_or_default()) == 1
        && u16::from_le_bytes(bytes[6..8].try_into().unwrap_or_default()) == 9
}

fn decode_signal_event(bytes: &[u8]) -> Option<SignalEventRequest> {
    let symbol_bytes = bytes.get(8..40)?;
    let symbol = std::str::from_utf8(symbol_bytes)
        .ok()?
        .trim_end_matches([' ', '\0'])
        .to_owned();
    let template = u16::from_le_bytes(bytes.get(2..4)?.try_into().ok()?);
    let event = match template {
        1 if is_raw_symbol_price(bytes) => SignalFeedEvent::Price {
            timestamp: u64::from_le_bytes(bytes.get(48..56)?.try_into().ok()?),
            price: f64::from_le_bytes(bytes.get(40..48)?.try_into().ok()?),
        },
        2 if bytes.len() >= 272 => {
            let bid_levels = std::array::from_fn(|index| {
                (
                    f64::from_le_bytes(
                        bytes[48 + index * 16..56 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                    f64::from_le_bytes(
                        bytes[56 + index * 16..64 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                )
            });
            let ask_levels = std::array::from_fn(|index| {
                (
                    f64::from_le_bytes(
                        bytes[160 + index * 16..168 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                    f64::from_le_bytes(
                        bytes[168 + index * 16..176 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                )
            });
            SignalFeedEvent::Book {
                timestamp: u64::from_le_bytes(bytes.get(40..48)?.try_into().ok()?),
                best_bid: bid_levels[0].0,
                best_ask: ask_levels[0].0,
                bid_sizes: bid_levels.map(|(_, size)| size),
                ask_sizes: ask_levels.map(|(_, size)| size),
                bid_levels,
                ask_levels,
            }
        }
        3 if bytes.len() >= 97 => SignalFeedEvent::Trade {
            timestamp: u64::from_le_bytes(bytes.get(72..80)?.try_into().ok()?),
            price: f64::from_le_bytes(bytes.get(80..88)?.try_into().ok()?),
            size: f64::from_le_bytes(bytes.get(88..96)?.try_into().ok()?),
            is_buy: bytes[96] == 1,
        },
        _ => return None,
    };
    Some(SignalEventRequest {
        symbol,
        raw_payload: Some(Bytes::copy_from_slice(bytes)),
        queued_at: None,
        event,
    })
}

fn decode_ticker_batch(bytes: &[u8]) -> Vec<SignalEventRequest> {
    bytes
        .chunks_exact(56)
        .filter_map(decode_signal_event)
        .filter(|request| matches!(request.event, SignalFeedEvent::Price { .. }))
        .collect()
}

const SIGNAL_EVENT_SHARD_COUNT: usize = 16;
const SIGNAL_EVENT_SHARD_CAPACITY: usize = 1_024;

async fn run_signal_event_workers(
    runtime: Arc<SignalRuntime>,
    mut receiver: tokio::sync::mpsc::Receiver<SignalEventRequest>,
) {
    use std::hash::{Hash, Hasher};

    let mut senders: Vec<tokio::sync::mpsc::Sender<SignalEventRequest>> =
        Vec::with_capacity(SIGNAL_EVENT_SHARD_COUNT);
    for _ in 0..SIGNAL_EVENT_SHARD_COUNT {
        let (sender, mut shard) = tokio::sync::mpsc::channel(SIGNAL_EVENT_SHARD_CAPACITY);
        senders.push(sender);
        let worker_runtime = Arc::clone(&runtime);
        tokio::spawn(async move {
            while let Some(request) = shard.recv().await {
                if let Some(queued_at) = request.queued_at {
                    crate::latency::record_event_queue_wait(queued_at.elapsed());
                }
                if !matches!(&request.event, SignalFeedEvent::Price { .. }) {
                    if let Some(raw) = request.raw_payload.as_ref() {
                        let _ = worker_runtime.enriched_market_events.send(raw.clone());
                    }
                }
                if let Err((status, error)) = ingest_signal_event(
                    axum::extract::State(Arc::clone(&worker_runtime)),
                    Json(request),
                )
                .await
                {
                    crate::service_log!("WARN", "Signal event rejected with {status}: {error}");
                }
            }
        });
    }

    while let Some(request) = receiver.recv().await {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        request.symbol.hash(&mut hasher);
        let shard = hasher.finish() as usize % senders.len();
        if senders[shard].send(request).await.is_err() {
            crate::service_log!("ERROR", "Signal event shard {shard} stopped");
        }
    }
}

async fn run_research_record_writer(
    store: SignalStore,
    mut receiver: tokio::sync::mpsc::Receiver<Vec<StoredSignalRecord>>,
) {
    while let Some(mut records) = receiver.recv().await {
        for _ in 1..128 {
            match receiver.try_recv() {
                Ok(mut batch) => records.append(&mut batch),
                Err(_) => break,
            }
        }
        if let Err(error) = store.append_records(records).await {
            crate::service_log!("ERROR", "Could not persist signal research batch: {error}");
        }
    }
}

async fn run_feed_signal_consumer(event_sender: tokio::sync::mpsc::Sender<SignalEventRequest>) {
    let mut retry_delay = Duration::from_secs(1);
    loop {
        let connection_started = Instant::now();
        match feed_client::signal_events().await {
            Ok(mut events) => {
                crate::latency::set_signal_feed_connected(true);
                crate::service_log!(
                    "INFO",
                    "Connected to FeedHandlers RSocket signal-event stream"
                );
                let mut heartbeat = tokio::time::interval(Duration::from_secs(60));
                heartbeat.tick().await;
                let mut trade_events = 0_u64;
                let mut book_events = 0_u64;
                let mut invalid_events = 0_u64;
                let mut last_event_at: Option<Instant> = None;
                loop {
                    tokio::select! {
                        _ = heartbeat.tick() => {
                            let idle_seconds = last_event_at
                                .map(|last_event| last_event.elapsed().as_secs())
                                .map(|seconds| seconds.to_string())
                                .unwrap_or_else(|| "never".to_owned());
                            crate::service_log!(
                                "INFO",
                                "FeedHandlers signal-event stream heartbeat: trade={trade_events} book={book_events} invalid={invalid_events} last_event_idle_seconds={idle_seconds}"
                            );
                            trade_events = 0;
                            book_events = 0;
                            invalid_events = 0;
                        }
                        event = events.next() => {
                            match event {
                                Some(Ok(bytes)) => match decode_signal_event(&bytes) {
                                    Some(mut request) => {
                                        match &request.event {
                                            SignalFeedEvent::Price { .. } => {
                                                continue;
                                            }
                                            SignalFeedEvent::Trade { .. } => {
                                                last_event_at = Some(Instant::now());
                                                crate::latency::record_signal_feed_event("trade");
                                                trade_events += 1;
                                            }
                                            SignalFeedEvent::Book { .. } => {
                                                last_event_at = Some(Instant::now());
                                                crate::latency::record_signal_feed_event("book");
                                                book_events += 1;
                                            }
                                        }
                                        request.queued_at = Some(Instant::now());
                                        if event_sender.send(request).await.is_err() {
                                            crate::service_log!("ERROR", "Signal event processing queue stopped");
                                            break;
                                        }
                                    }
                                    None => {
                                        invalid_events += 1;
                                        crate::service_log!("WARN", "Invalid FeedHandlers binary signal event");
                                    }
                                },
                                Some(Err(error)) => {
                                    invalid_events += 1;
                                    crate::service_log!("WARN", "Invalid FeedHandlers signal event: {error:#}");
                                }
                                None => break,
                            }
                        }
                    }
                }
                crate::latency::set_signal_feed_connected(false);
                if connection_started.elapsed() >= Duration::from_secs(60) {
                    retry_delay = Duration::from_secs(1);
                }
                crate::service_log!(
                    "WARN",
                    "FeedHandlers RSocket signal-event stream disconnected; reconnecting in {}s",
                    retry_delay.as_secs()
                );
            }
            Err(error) => {
                crate::latency::set_signal_feed_connected(false);
                crate::service_log!(
                    "WARN",
                    "Could not connect to FeedHandlers signal-event stream: {error:#}; retrying in {}s",
                    retry_delay.as_secs()
                );
            }
        }
        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
    }
}

async fn run_feed_ticker_consumer(event_sender: tokio::sync::mpsc::Sender<SignalEventRequest>) {
    let mut retry_delay = Duration::from_secs(1);
    loop {
        match feed_client::ticker_events().await {
            Ok(mut events) => {
                crate::service_log!("INFO", "Connected to FeedHandlers raw ticker RSocket stream");
                while let Some(event) = events.next().await {
                    match event {
                        Ok(bytes) => {
                            for request in decode_ticker_batch(&bytes) {
                                crate::latency::record_signal_feed_event("price");
                                if event_sender.send(request).await.is_err() {
                                    crate::service_log!("ERROR", "Signal event processing queue stopped");
                                    return;
                                }
                            }
                        }
                        Err(error) => crate::service_log!("WARN", "Invalid FeedHandlers raw ticker batch: {error:#}"),
                    }
                }
                crate::service_log!("WARN", "FeedHandlers raw ticker RSocket stream disconnected; reconnecting in {}s", retry_delay.as_secs());
            }
            Err(error) => crate::service_log!("WARN", "Could not connect to FeedHandlers raw ticker stream: {error:#}; retrying in {}s", retry_delay.as_secs()),
        }
        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
    }
}

const AUTO_TRADER_SIGNAL_PUSH_CONCURRENCY: usize = 8;
const AUTO_TRADER_SIGNAL_PUSH_TIMEOUT: Duration = Duration::from_secs(2);

fn drain_latest(map: &DashMap<String, serde_json::Value>) -> Vec<(String, serde_json::Value)> {
    // Snapshot and sort keys before removing entries. Each shard guard is short
    // lived, and output order stays consistent with the previous BTreeMap queue.
    let mut keys = map
        .iter()
        .map(|entry| entry.key().clone())
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.into_iter()
        .filter_map(|key| map.remove(&key).map(|(_, value)| (key, value)))
        .collect()
}

async fn deliver_signal_dashboard(
    auto_trader_rsocket_url: String,
    symbol: String,
    dashboard: serde_json::Value,
) -> (String, serde_json::Value, Result<(), String>) {
    let started = Instant::now();
    let request_id = format!(
        "insights-signal:{}:{}",
        dashboard["symbol"].as_str().unwrap_or(&symbol),
        dashboard["timestamp"].as_u64().unwrap_or_default()
    );
    let result = tokio::time::timeout(
        AUTO_TRADER_SIGNAL_PUSH_TIMEOUT,
        strategies::request_scout_service_route(
            &auto_trader_rsocket_url,
            &strategies::ScoutCoordinatorRouteCommand {
                request_id,
                strategy: None,
                method: strategies::ScoutWorkerHttpMethod::Post,
                path: "/internal/auto-trader/signal".into(),
                payload: dashboard.clone(),
            },
        ),
    )
    .await
    .map_err(|_| "AutoTrader RSocket signal delivery timed out".to_owned())
    .and_then(|response| response.map_err(|error| error.to_string()))
    .and_then(|response| {
        if response.status >= 400 {
            Err(format!("AutoTrader rejected signal: status {}", response.status))
        } else {
            Ok(())
        }
    });
    crate::latency::record_auto_trader_delivery(started.elapsed());

    (symbol, dashboard, result)
}

async fn deliver_signal_batch(
    auto_trader_rsocket_url: &str,
    dashboards: Vec<(String, serde_json::Value)>,
) -> (BTreeMap<String, serde_json::Value>, Option<String>) {
    // Bound load on AutoTrader while allowing unrelated symbols to progress
    // together. Keep this below the old burst of 32 simultaneous requests,
    // which competed with scout-learning writes for AutoTrader's state lock.
    let results = futures_util::stream::iter(dashboards)
        .map(|(symbol, dashboard)| {
            deliver_signal_dashboard(auto_trader_rsocket_url.to_owned(), symbol, dashboard)
        })
        .buffer_unordered(AUTO_TRADER_SIGNAL_PUSH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    let mut failed = BTreeMap::new();
    let mut first_error = None;
    for (symbol, dashboard, result) in results {
        if let Err(error) = result {
            first_error.get_or_insert_with(|| error.to_string());
            failed.insert(symbol, dashboard);
        }
    }
    (failed, first_error)
}

fn scout_dashboard_for_delivery(dashboard: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "best": dashboard.get("best").cloned().unwrap_or_default(),
        "markets": dashboard.get("markets").cloned().unwrap_or_default(),
    })
}

fn scout_delivery_signals(dashboard: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut signals = Vec::new();
    let Some(best) = dashboard.get("best").and_then(serde_json::Value::as_array) else {
        return signals;
    };
    let markets = dashboard.get("markets");
    for source_signal in best {
        let mut signal = source_signal.clone();
        let Some(symbol) = signal.get("symbol").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(market) = markets.and_then(|markets| markets.get(symbol)) else {
            continue;
        };
        let bid = market.get("bids").and_then(serde_json::Value::as_array).and_then(|items| items.first())
            .and_then(|level| level.get("price")).and_then(serde_json::Value::as_f64);
        let ask = market.get("asks").and_then(serde_json::Value::as_array).and_then(|items| items.first())
            .and_then(|level| level.get("price")).and_then(serde_json::Value::as_f64);
        let updated_at = market.get("updated_at_ms").and_then(serde_json::Value::as_u64);
        let (Some(bid), Some(ask), Some(updated_at)) = (bid, ask, updated_at) else {
            continue;
        };
        if now_ms().saturating_sub(updated_at) > 5_000
            || !bid.is_finite()
            || !ask.is_finite()
            || bid <= 0.0
            || ask < bid
        {
            continue;
        }
        signal["best_bid"] = serde_json::json!(bid);
        signal["best_ask"] = serde_json::json!(ask);
        signal["price"] = serde_json::json!(bid + (ask - bid) / 2.0);
        signal["book_updated_at_ms"] = serde_json::json!(updated_at);
        signal["scout_taker_fee_rate"] =
            serde_json::json!(insights::signals::configured_signal_taker_fee_rate());
        signals.push(signal);
    }
    signals
}

async fn deliver_scout_inputs(dashboards: &[(String, serde_json::Value)]) {
    let learning_rsocket_url = std::env::var("LEARNING_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10010".to_owned());
    for (_symbol, dashboard) in dashboards {
        let signals = scout_delivery_signals(dashboard);
        if signals.is_empty() {
            continue;
        }
        let result = strategies::request_scout_worker_route(
            &learning_rsocket_url,
            &strategies::ScoutCoordinatorRouteCommand {
                request_id: format!("insights-live:{}:{}", signals[0]["symbol"].as_str().unwrap_or(""), signals[0]["timestamp"].as_u64().unwrap_or_default()),
                strategy: None,
                method: strategies::ScoutWorkerHttpMethod::Post,
                path: "/live-inputs".into(),
                payload: serde_json::json!({ "signals": signals }),
            },
        )
        .await;
        match result {
            Ok(response) if response.status < 400 => {}
            Ok(response) => crate::service_log!(
                "WARN",
                "Learning rejected live Scout input delivery with status {}",
                response.status
            ),
            Err(error) => {
                crate::service_log!("WARN", "Live Scout input delivery failed: {error}");
            }
        }
    }
}

async fn run_scout_input_push_worker(
    pending_scout_inputs: Arc<DashMap<String, serde_json::Value>>,
    scout_notify: Arc<Notify>,
) {
    loop {
        let notified = scout_notify.notified();
        tokio::pin!(notified);
        let dashboards = drain_latest(&pending_scout_inputs);
        if dashboards.is_empty() {
            notified.await;
            continue;
        }
        deliver_scout_inputs(&dashboards).await;
    }
}

async fn run_ui_snapshot_push_worker(
    pending_snapshots: Arc<DashMap<String, serde_json::Value>>,
    notify: Arc<Notify>,
) {
    let feed_rsocket_url = std::env::var("FEED_HANDLERS_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10001".to_owned());
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        let snapshots = drain_latest(&pending_snapshots);
        if snapshots.is_empty() {
            notified.await;
            continue;
        };
        for (_, snapshot) in snapshots {
            let symbol = snapshot["symbol"].as_str().unwrap_or("unknown");
            let timestamp = snapshot["timestamp"].as_u64().unwrap_or_default();
            let result = strategies::request_scout_service_route(
                &feed_rsocket_url,
                &strategies::ScoutCoordinatorRouteCommand {
                    request_id: format!("insights-snapshot:{symbol}:{timestamp}"),
                    strategy: None,
                    method: strategies::ScoutWorkerHttpMethod::Post,
                    path: "/internal/signals/snapshot".into(),
                    payload: snapshot,
                },
            )
            .await;
            match result {
                Ok(response) if response.status < 400 => {}
                Ok(response) => crate::service_log!(
                    "WARN",
                    "Signal snapshot fanout to FeedHandlers failed: status {}",
                    response.status
                ),
                Err(error) => crate::service_log!(
                    "WARN",
                    "Signal snapshot fanout to FeedHandlers failed: {error}"
                ),
            }
        }
    }
}

async fn run_signal_push_worker(
    pending_signals: Arc<DashMap<String, serde_json::Value>>,
    notify: Arc<Notify>,
    pending_scout_inputs: Arc<DashMap<String, serde_json::Value>>,
    scout_notify: Arc<Notify>,
) {
    let auto_trader_rsocket_url = std::env::var("AUTO_TRADER_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10006".to_owned());
    let mut retry_delay = std::time::Duration::from_millis(250);
    let mut endpoint_available = true;
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        let dashboards = drain_latest(&pending_signals);
        if dashboards.is_empty() {
            notified.await;
            continue;
        };
        // Send symbols in parallel so one slow request cannot block the batch.
        for (symbol, dashboard) in &dashboards {
            pending_scout_inputs.insert(
                symbol.clone(),
                scout_dashboard_for_delivery(dashboard),
            );
        }
        scout_notify.notify_one();
        let (failed, first_error) =
            deliver_signal_batch(&auto_trader_rsocket_url, dashboards).await;
        if !failed.is_empty() {
            if endpoint_available {
                crate::service_log!(
                    "WARN",
                    "Auto-trader signal delivery failed; retaining latest signal per symbol and retrying: {}",
                    first_error.as_deref().unwrap_or("unknown request error")
                );
                endpoint_available = false;
            }
            for (symbol, dashboard) in failed {
                pending_signals.entry(symbol).or_insert(dashboard);
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(5));
        } else {
            if !endpoint_available {
                crate::service_log!("INFO", "Auto-trader signal delivery recovered");
            }
            endpoint_available = true;
            retry_delay = std::time::Duration::from_millis(250);
        }
    }
}

#[cfg(test)]
mod signal_feed_worker_tests {
    use super::*;

    fn base_event(template: u16, length: usize) -> Vec<u8> {
        let mut bytes = vec![0_u8; length];
        bytes[2..4].copy_from_slice(&template.to_le_bytes());
        bytes[8..16].copy_from_slice(b"BTC-USDT");
        bytes
    }

    #[test]
    fn decoder_reads_price_and_trade_contract_fields_and_rejects_truncated_events() {
        let mut price = base_event(1, 56);
        price[0..2].copy_from_slice(&48_u16.to_le_bytes());
        price[4..6].copy_from_slice(&1_u16.to_le_bytes());
        price[6..8].copy_from_slice(&9_u16.to_le_bytes());
        price[40..48].copy_from_slice(&100.5_f64.to_le_bytes());
        price[48..56].copy_from_slice(&123_u64.to_le_bytes());
        let decoded = decode_signal_event(&price).unwrap();
        assert_eq!(decoded.symbol, "BTC-USDT");
        assert!(matches!(decoded.event, SignalFeedEvent::Price { timestamp: 123, price: 100.5 }));

        let mut trade = base_event(3, 97);
        trade[72..80].copy_from_slice(&456_u64.to_le_bytes());
        trade[80..88].copy_from_slice(&101.0_f64.to_le_bytes());
        trade[88..96].copy_from_slice(&2.0_f64.to_le_bytes());
        trade[96] = 1;
        assert!(matches!(decode_signal_event(&trade).unwrap().event, SignalFeedEvent::Trade { timestamp: 456, price: 101.0, size: 2.0, is_buy: true }));

        assert!(decode_signal_event(&price[..55]).is_none());
        assert!(decode_signal_event(&base_event(99, 100)).is_none());
        assert!(decode_signal_event(&[0; 20]).is_none());
    }

    #[test]
    fn ticker_batch_decoder_reads_multiple_raw_prices_and_ignores_other_templates() {
        let mut first = base_event(1, 56);
        first[0..2].copy_from_slice(&48_u16.to_le_bytes());
        first[4..6].copy_from_slice(&1_u16.to_le_bytes());
        first[6..8].copy_from_slice(&9_u16.to_le_bytes());
        first[40..48].copy_from_slice(&100.0_f64.to_le_bytes());
        first[48..56].copy_from_slice(&1_u64.to_le_bytes());
        let mut second = base_event(1, 56);
        second[0..2].copy_from_slice(&48_u16.to_le_bytes());
        second[4..6].copy_from_slice(&1_u16.to_le_bytes());
        second[6..8].copy_from_slice(&9_u16.to_le_bytes());
        second[8..16].copy_from_slice(b"ETH-USDT");
        second[40..48].copy_from_slice(&200.0_f64.to_le_bytes());
        second[48..56].copy_from_slice(&2_u64.to_le_bytes());
        let trade = base_event(3, 56);
        let mut batch = first;
        batch.extend_from_slice(&second);
        batch.extend_from_slice(&trade);

        let decoded = decode_ticker_batch(&batch);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].symbol, "BTC-USDT");
        assert_eq!(decoded[1].symbol, "ETH-USDT");
        assert!(matches!(decoded[0].event, SignalFeedEvent::Price { timestamp: 1, price: 100.0 }));
        assert!(matches!(decoded[1].event, SignalFeedEvent::Price { timestamp: 2, price: 200.0 }));
    }

    #[test]
    fn draining_latest_cache_returns_each_symbol_value_and_empties_cache() {
        let cache = DashMap::new();
        cache.insert("BTC".to_owned(), serde_json::json!({"n": 1}));
        cache.insert("ETH".to_owned(), serde_json::json!({"n": 2}));
        let drained = drain_latest(&cache);
        assert_eq!(drained.len(), 2);
        assert!(drained.iter().any(|(symbol, value)| symbol == "BTC" && value["n"] == 1));
        assert!(drained.iter().any(|(symbol, value)| symbol == "ETH" && value["n"] == 2));
        assert!(cache.is_empty());
    }

    #[test]
    fn scout_delivery_queue_keeps_only_latest_slim_dashboard_per_symbol() {
        fn rss_kib() -> u64 {
            std::fs::read_to_string("/proc/self/status")
                .unwrap()
                .lines()
                .find_map(|line| {
                    line.strip_prefix("VmRSS:")?
                        .split_whitespace()
                        .next()?
                        .parse::<u64>()
                        .ok()
                })
                .unwrap()
        }

        let cache = DashMap::new();
        let rss_before = rss_kib();
        let bulky_dashboard = serde_json::json!({
            "best": [{"symbol":"BTC-USDT", "sequence":0}],
            "markets": {"BTC-USDT": {"updated_at_ms":now_ms(), "bids":[{"price":100.0}], "asks":[{"price":101.0}]}},
            "walkForward": vec!["x".repeat(1024); 10_000],
            "missedOpportunityBands": vec!["y".repeat(1024); 10_000],
        });
        cache.insert(
            "BTC-USDT".to_owned(),
            scout_dashboard_for_delivery(&bulky_dashboard),
        );
        drop(bulky_dashboard);
        for index in 0..50_000 {
            let updated_at = now_ms();
            let dashboard = serde_json::json!({
                "best": [{"symbol":"BTC-USDT", "sequence":index}],
                "markets": {"BTC-USDT": {"updated_at_ms":updated_at, "bids":[{"price":100.0}], "asks":[{"price":101.0}]}},
            });
            cache.insert(
                "BTC-USDT".to_owned(),
                scout_dashboard_for_delivery(&dashboard),
            );
        }
        assert_eq!(cache.len(), 1);
        let queued = drain_latest(&cache);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].1["best"][0]["sequence"], 49_999);
        assert!(queued[0].1.get("walkForward").is_none());
        assert!(queued[0].1.get("missedOpportunityBands").is_none());

        let signals = scout_delivery_signals(&queued[0].1);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0]["best_bid"], 100.0);
        assert_eq!(signals[0]["best_ask"], 101.0);
        assert_eq!(signals[0]["price"], 100.5);
        assert_eq!(
            signals[0]["scout_taker_fee_rate"],
            insights::signals::configured_signal_taker_fee_rate()
        );
        assert!(signals[0]["book_updated_at_ms"].as_u64().unwrap() <= now_ms());
        assert!(serde_json::to_vec(&queued[0].1).unwrap().len() < 1_024);
        let stale_dashboard = serde_json::json!({
            "best": [{"symbol":"BTC-USDT"}],
            "markets": {"BTC-USDT": {"updated_at_ms":now_ms().saturating_sub(6_000), "bids":[{"price":100.0}], "asks":[{"price":101.0}]}},
        });
        assert!(scout_delivery_signals(&stale_dashboard).is_empty());
        eprintln!(
            "SCOUT_INPUT_BENCH updates=50000 queued_symbols={} final_payload_bytes={} rss_before_kib={rss_before} rss_after_kib={}",
            queued.len(),
            serde_json::to_vec(&queued[0].1).unwrap().len(),
            rss_kib()
        );
    }
}

