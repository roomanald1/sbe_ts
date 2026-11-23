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
    if let Ok(mut tracked) = runtime.signal_event_shards.write() {
        *tracked = senders.clone();
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
    // The signal engine samples prices at 1s cadence. Keep only the newest
    // pending ticker per symbol so raw bursts cannot backlog the shared queue.
    const PRICE_FLUSH_INTERVAL: Duration = Duration::from_millis(250);
    loop {
        match feed_client::ticker_events().await {
            Ok(mut events) => {
                crate::service_log!("INFO", "Connected to FeedHandlers raw ticker RSocket stream");
                let mut pending = std::collections::BTreeMap::<String, SignalEventRequest>::new();
                let is_at_least_as_new = |current: &SignalEventRequest, incoming: &SignalEventRequest| {
                    match (&current.event, &incoming.event) {
                        (SignalFeedEvent::Price { timestamp: old, .. }, SignalFeedEvent::Price { timestamp: new, .. }) => new >= old,
                        _ => true,
                    }
                };
                let mut flush = tokio::time::interval(PRICE_FLUSH_INTERVAL);
                flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                flush.tick().await;
                loop {
                    tokio::select! {
                        event = events.next() => {
                            let Some(event) = event else { break };
                            match event {
                                Ok(bytes) => {
                                    for request in decode_ticker_batch(&bytes) {
                                        crate::latency::record_signal_feed_event("price");
                                        let replace = pending.get(&request.symbol)
                                            .is_none_or(|current| is_at_least_as_new(current, &request));
                                        if replace {
                                            pending.insert(request.symbol.clone(), request);
                                        }
                                    }
                                }
                                Err(error) => crate::service_log!("WARN", "Invalid FeedHandlers raw ticker batch: {error:#}"),
                            }
                        }
                        _ = flush.tick() => {
                            for (symbol, mut request) in std::mem::take(&mut pending) {
                                request.queued_at = Some(Instant::now());
                                match event_sender.try_send(request) {
                                    Ok(()) => {}
                                    Err(tokio::sync::mpsc::error::TrySendError::Full(mut request)) => {
                                        // Keep reading feed updates while the shared ingress is full.
                                        request.queued_at = None;
                                        let replace = pending.get(&symbol)
                                            .is_none_or(|current| is_at_least_as_new(current, &request));
                                        if replace { pending.insert(symbol, request); }
                                    }
                                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                        crate::service_log!("ERROR", "Signal event processing queue stopped");
                                        return;
                                    }
                                }
                            }
                        }
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

async fn run_ui_snapshot_push_worker(
    pending_snapshots: Arc<DashMap<String, serde_json::Value>>,
    notify: Arc<Notify>,
) {
    const SNAPSHOT_PUSH_BATCH_DELAY: Duration = Duration::from_millis(100);
    let feed_rsocket_url = std::env::var("FEED_HANDLERS_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10001".to_owned());
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        if pending_snapshots.is_empty() {
            notified.await;
            continue;
        }
        // Snapshots are latest-value updates by symbol. A short coalescing
        // window batches the current feed burst into one pooled RSocket call.
        tokio::time::sleep(SNAPSHOT_PUSH_BATCH_DELAY).await;
        let snapshots = drain_latest(&pending_snapshots)
            .into_iter()
            .map(|(_, snapshot)| snapshot)
            .collect::<Vec<_>>();
        if snapshots.is_empty() {
            continue;
        }
        let push_started = Instant::now();
        let result = strategy::request_scout_service_route(
            &feed_rsocket_url,
            &strategy::ScoutCoordinatorRouteCommand {
                request_id: format!("insights-snapshot-batch:{}", now_ms()),
                strategy: None,
                method: strategy::ScoutWorkerHttpMethod::Post,
                path: "/internal/signals/snapshots".into(),
                payload: serde_json::json!({ "snapshots": snapshots }),
            },
        )
        .await;
        crate::latency::record_ui_snapshot_push(push_started.elapsed());
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


}

