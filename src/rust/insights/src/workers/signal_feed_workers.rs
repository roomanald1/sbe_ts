fn decode_signal_event(bytes: &[u8]) -> Option<SignalEventRequest> {
    let symbol_bytes = bytes.get(8..40)?;
    let symbol = std::str::from_utf8(symbol_bytes)
        .ok()?
        .trim_end_matches([' ', '\0'])
        .to_owned();
    let template = u16::from_le_bytes(bytes.get(2..4)?.try_into().ok()?);
    let event = match template {
        1 if bytes.len() >= 72 => SignalFeedEvent::Price {
            timestamp: u64::from_le_bytes(bytes.get(64..72)?.try_into().ok()?),
            price: f64::from_le_bytes(bytes.get(40..48)?.try_into().ok()?),
            volatility_1m: f64::from_le_bytes(bytes.get(48..56)?.try_into().ok()?),
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
    Some(SignalEventRequest { symbol, queued_at: None, event })
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
                crate::service_log!(
                    "INFO",
                    "Connected to FeedHandlers RSocket signal-event stream"
                );
                let mut heartbeat = tokio::time::interval(Duration::from_secs(60));
                heartbeat.tick().await;
                let mut price_events = 0_u64;
                let mut trade_events = 0_u64;
                let mut book_events = 0_u64;
                let mut invalid_events = 0_u64;
                let mut last_event_at: Option<Instant> = None;
                let mut last_price_event_at: HashMap<String, Instant> = HashMap::new();
                loop {
                    tokio::select! {
                        _ = heartbeat.tick() => {
                            let idle_seconds = last_event_at
                                .map(|last_event| last_event.elapsed().as_secs())
                                .map(|seconds| seconds.to_string())
                                .unwrap_or_else(|| "never".to_owned());
                            let mut stale_price_symbols = last_price_event_at
                                .iter()
                                .map(|(symbol, last_event)| (symbol.clone(), last_event.elapsed().as_secs()))
                                .filter(|(_, age_seconds)| *age_seconds >= 60)
                                .collect::<Vec<_>>();
                            stale_price_symbols.sort_by(|left, right| right.1.cmp(&left.1));
                            let stale_price_count = stale_price_symbols.len();
                            stale_price_symbols.truncate(10);
                            crate::service_log!(
                                "INFO",
                                "FeedHandlers signal-event stream heartbeat: price={price_events} trade={trade_events} book={book_events} invalid={invalid_events} last_event_idle_seconds={idle_seconds} symbols_without_price_for_60s={stale_price_count} stalest_price_events={stale_price_symbols:?}"
                            );
                            price_events = 0;
                            trade_events = 0;
                            book_events = 0;
                            invalid_events = 0;
                        }
                        event = events.next() => {
                            match event {
                                Some(Ok(bytes)) => match decode_signal_event(&bytes) {
                                    Some(mut request) => {
                                        last_event_at = Some(Instant::now());
                                        match &request.event {
                                            SignalFeedEvent::Price { .. } => {
                                                price_events += 1;
                                                last_price_event_at.insert(request.symbol.clone(), Instant::now());
                                            }
                                            SignalFeedEvent::Trade { .. } => trade_events += 1,
                                            SignalFeedEvent::Book { .. } => book_events += 1,
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
                if connection_started.elapsed() >= Duration::from_secs(60) {
                    retry_delay = Duration::from_secs(1);
                }
                crate::service_log!(
                    "WARN",
                    "FeedHandlers RSocket signal-event stream disconnected; reconnecting in {}s",
                    retry_delay.as_secs()
                );
            }
            Err(error) => crate::service_log!(
                "WARN",
                "Could not connect to FeedHandlers signal-event stream: {error:#}; retrying in {}s",
                retry_delay.as_secs()
            ),
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
    client: reqwest::Client,
    endpoint: String,
    symbol: String,
    dashboard: serde_json::Value,
) -> (String, serde_json::Value, Result<(), reqwest::Error>) {
    let started = Instant::now();
    let result = client
        .post(endpoint)
        .timeout(AUTO_TRADER_SIGNAL_PUSH_TIMEOUT)
        .json(&dashboard)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map(|_| ());
    crate::latency::record_auto_trader_delivery(started.elapsed());

    (symbol, dashboard, result)
}

async fn deliver_signal_batch(
    client: &reqwest::Client,
    base: &str,
    dashboards: Vec<(String, serde_json::Value)>,
) -> (BTreeMap<String, serde_json::Value>, Option<String>) {
    // Bound load on AutoTrader while allowing unrelated symbols to progress
    // together. Keep this below the old burst of 32 simultaneous requests,
    // which competed with scout-learning writes for AutoTrader's state lock.
    let results = futures_util::stream::iter(dashboards)
        .map(|(symbol, dashboard)| {
            deliver_signal_dashboard(
                client.clone(),
                format!("{base}/internal/auto-trader/signal"),
                symbol,
                dashboard,
            )
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

async fn run_ui_snapshot_push_worker(
    pending_snapshots: Arc<DashMap<String, serde_json::Value>>,
    notify: Arc<Notify>,
) {
    let client = POSITION_MANAGEMENT_CLIENT.get_or_init(reqwest::Client::new);
    let base =
        std::env::var("FEED_HANDLERS_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_owned());
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        let snapshots = drain_latest(&pending_snapshots);
        if snapshots.is_empty() {
            notified.await;
            continue;
        };
        for (_, snapshot) in snapshots {
            if let Err(error) = client
                .post(format!("{base}/internal/signals/snapshot"))
                .timeout(std::time::Duration::from_millis(250))
                .json(&snapshot)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
            {
                crate::service_log!(
                    "WARN",
                    "Signal snapshot fanout to FeedHandlers failed: {error}"
                );
            }
        }
    }
}

async fn run_signal_push_worker(
    pending_signals: Arc<DashMap<String, serde_json::Value>>,
    notify: Arc<Notify>,
) {
    let client = POSITION_MANAGEMENT_CLIENT.get_or_init(reqwest::Client::new);
    let base =
        std::env::var("AUTO_TRADER_URL").unwrap_or_else(|_| "http://127.0.0.1:3005".to_owned());
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
        let (failed, first_error) = deliver_signal_batch(&client, &base, dashboards).await;
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
        let mut price = base_event(1, 72);
        price[40..48].copy_from_slice(&100.5_f64.to_le_bytes());
        price[48..56].copy_from_slice(&0.2_f64.to_le_bytes());
        price[64..72].copy_from_slice(&123_u64.to_le_bytes());
        let decoded = decode_signal_event(&price).unwrap();
        assert_eq!(decoded.symbol, "BTC-USDT");
        assert!(matches!(decoded.event, SignalFeedEvent::Price { timestamp: 123, price: 100.5, volatility_1m: 0.2 }));

        let mut trade = base_event(3, 97);
        trade[72..80].copy_from_slice(&456_u64.to_le_bytes());
        trade[80..88].copy_from_slice(&101.0_f64.to_le_bytes());
        trade[88..96].copy_from_slice(&2.0_f64.to_le_bytes());
        trade[96] = 1;
        assert!(matches!(decode_signal_event(&trade).unwrap().event, SignalFeedEvent::Trade { timestamp: 456, price: 101.0, size: 2.0, is_buy: true }));

        assert!(decode_signal_event(&price[..71]).is_none());
        assert!(decode_signal_event(&base_event(99, 100)).is_none());
        assert!(decode_signal_event(&[0; 20]).is_none());
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
}

