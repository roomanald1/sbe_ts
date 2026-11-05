use super::*;

pub(super) async fn run_shards(state: Arc<LiveState>, mut receiver: mpsc::Receiver<SignalEvent>) {
    let mut senders = Vec::with_capacity(EVENT_SHARDS);
    for _ in 0..EVENT_SHARDS {
        let (sender, mut shard) = mpsc::channel::<SignalEvent>(EVENT_QUEUE_CAPACITY);
        senders.push(sender);
        let worker_state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut actors = HashMap::<String, (SignalEngine, Option<(u64, f64)>)>::new();
            while let Some(event) = shard.recv().await {
                crate::latency::record_event_queue_wait(event.queued_at.elapsed());
                let (engine, last_price) = actors.entry(event.symbol.clone()).or_default();
                process_event(event, engine, last_price, &worker_state);
            }
        });
    }
    while let Some(event) = receiver.recv().await {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        event.symbol.hash(&mut hasher);
        let shard = hasher.finish() as usize % senders.len();
        if senders[shard].send(event).await.is_err() {
            crate::service_log!("ERROR", "Live Signals shard {shard} stopped");
        }
    }
}

fn process_event(
    event: SignalEvent,
    engine: &mut SignalEngine,
    last_price: &mut Option<(u64, f64)>,
    state: &LiveState,
) {
    match event.event {
        MarketEvent::Price {
            timestamp,
            price,
            volatility_1m,
        } => {
            if !price.is_finite()
                || price <= 0.0
                || last_price.is_some_and(|(last, _)| timestamp < last)
            {
                return;
            }
            let previous = last_price.replace((timestamp, price));
            engine.on_price(timestamp, price, volatility_1m);
            if previous != Some((timestamp, price)) && engine.live_snapshot_due(timestamp) {
                let snapshot_started = Instant::now();
                if let Some(snapshot) =
                    engine.snapshot_with_reasons(&event.symbol, timestamp, false)
                {
                    enqueue_snapshot(state, &event.symbol, &snapshot);
                }
                crate::latency::record_snapshot_compute(snapshot_started.elapsed());
            }
        }
        MarketEvent::Trade {
            timestamp,
            price,
            size,
            is_buy,
        } => engine.on_trade(timestamp, price, size, is_buy),
        MarketEvent::Book {
            timestamp,
            best_bid,
            best_ask,
            bid_sizes,
            ask_sizes,
            bid_levels,
            ask_levels,
        } => {
            engine.on_book(
                timestamp,
                best_bid,
                best_ask,
                &bid_sizes,
                &ask_sizes,
                &bid_levels,
                &ask_levels,
            );
            if best_bid.is_finite()
                && best_ask.is_finite()
                && best_bid > 0.0
                && best_ask >= best_bid
            {
                let market = book_value(timestamp, &bid_levels, &ask_levels);
                state.markets.insert(event.symbol.clone(), market.clone());
                if let Some(mut pending) = state.pending_delivery.get_mut(&event.symbol) {
                    let mut markets = pending
                        .get("markets")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    markets.insert(event.symbol, market);
                    pending["markets"] = Value::Object(markets);
                    drop(pending);
                    state.delivery_notify.notify_one();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Arc<LiveState> {
        let (_, news) = watch::channel(Arc::new(NewsContext::default()));
        Arc::new(LiveState::new(news))
    }

    #[tokio::test]
    async fn run_shards_processes_ordered_price_trade_and_book_events() {
        let state = state();
        let (sender, receiver) = mpsc::channel(256);
        let worker = tokio::spawn(run_shards(Arc::clone(&state), receiver));
        sender
            .send(SignalEvent {
                symbol: "BTC-USDT".into(),
                event: MarketEvent::Book {
                    timestamp: 1_000,
                    best_bid: 99.9,
                    best_ask: 100.1,
                    bid_sizes: [1.0; 7],
                    ask_sizes: [1.0; 7],
                    bid_levels: [(99.9, 1.0); 7],
                    ask_levels: [(100.1, 1.0); 7],
                },
                queued_at: Instant::now(),
            })
            .await
            .unwrap();
        sender
            .send(SignalEvent {
                symbol: "BTC-USDT".into(),
                event: MarketEvent::Trade {
                    timestamp: 1_500,
                    price: 100.0,
                    size: 0.5,
                    is_buy: true,
                },
                queued_at: Instant::now(),
            })
            .await
            .unwrap();
        for second in 0..=65 {
            let timestamp = 2_000 + second * 1_000;
            sender
                .send(SignalEvent {
                    symbol: "BTC-USDT".into(),
                    event: MarketEvent::Price {
                        timestamp,
                        price: 100.0 + second as f64 * 0.01,
                        volatility_1m: 0.1,
                    },
                    queued_at: Instant::now(),
                })
                .await
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            while !state.latest.contains_key("BTC-USDT") {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(state.latest.get("BTC-USDT").unwrap()["symbol"], "BTC-USDT");
        assert_eq!(
            state.markets.get("BTC-USDT").unwrap()["bids"][0]["price"],
            99.9
        );
        assert!(state.pending_delivery.contains_key("BTC-USDT"));
        drop(sender);
        worker.await.unwrap();
    }

    #[test]
    fn process_event_rejects_invalid_and_out_of_order_prices() {
        let state = state();
        let mut engine = SignalEngine::default();
        let mut last_price = None;
        process_event(
            SignalEvent {
                symbol: "BTC-USDT".into(),
                event: MarketEvent::Price {
                    timestamp: 2_000,
                    price: 100.0,
                    volatility_1m: 0.0,
                },
                queued_at: Instant::now(),
            },
            &mut engine,
            &mut last_price,
            &state,
        );
        process_event(
            SignalEvent {
                symbol: "BTC-USDT".into(),
                event: MarketEvent::Price {
                    timestamp: 1_000,
                    price: 90.0,
                    volatility_1m: 0.0,
                },
                queued_at: Instant::now(),
            },
            &mut engine,
            &mut last_price,
            &state,
        );
        process_event(
            SignalEvent {
                symbol: "BTC-USDT".into(),
                event: MarketEvent::Price {
                    timestamp: 3_000,
                    price: f64::NAN,
                    volatility_1m: 0.0,
                },
                queued_at: Instant::now(),
            },
            &mut engine,
            &mut last_price,
            &state,
        );
        assert_eq!(last_price, Some((2_000, 100.0)));
        assert!(state.latest.is_empty());
    }
}
