pub async fn serve() -> Result<()> {
    let instrument_data = get_instrument_data()
        .await?
        .into_iter()
        .map(Arc::<str>::from)
        .collect::<Vec<_>>();
    let all_spot_instruments = match get_all_spot_instruments().await {
        Ok(symbols) => symbols,
        Err(error) => {
            crate::service_log!("ERROR", 
                "Unable to load full spot instrument lookup: {error:#}; using top ticker symbols"
            );
            instrument_data.iter().map(ToString::to_string).collect()
        }
    };
    let instrument_lookup = Arc::new(
        all_spot_instruments
            .into_iter()
            .map(Arc::<str>::from)
            .collect::<HashSet<_>>(),
    );
    let (price_updates, _) = broadcast::channel::<Bytes>(PRICE_UPDATE_CAPACITY);
    let sub_sender = price_updates.clone();
    let sub = subscribe(&instrument_data, sub_sender);
    let (ticker_batches, _) = broadcast::channel::<Bytes>(256);
    let initial_usd_rates = Bytes::from_static(br#"{"rates":{"USD":1.0},"updated_at":0,"updated_at_by_currency":{"USD":0}}"#);
    let (usd_rate_snapshot, _) = watch::channel(initial_usd_rates);
    tokio::spawn(run_usd_rate_stream(usd_rate_snapshot.clone()));
    let (news_snapshot, _) = watch::channel(Bytes::from_static(br#"{"updated_at_ms":0,"source":"CoinDesk RSS","stale":true,"error":null,"articles":[]}"#));
    let _ = NEWS_SNAPSHOT.set(news_snapshot.clone());
    let signal_feeds = SIGNAL_FEEDS.get_or_init(|| Arc::new(DashMap::new())).clone();
    let signal_price_feeds = SIGNAL_PRICE_FEEDS.get_or_init(|| Arc::new(DashMap::new())).clone();
    let (signal_events, _) = broadcast::channel::<Bytes>(65_536);
    let mut ticker_router = price_updates.subscribe();
    let routed_prices = Arc::clone(&signal_price_feeds);
    tokio::spawn(async move {
        loop {
            match ticker_router.recv().await {
                Ok(bytes) => {
                    if let Some(symbol) = symbol_from_sbe(&bytes) {
                        let output = routed_prices.get(symbol).map(|entry| entry.value().clone());
                        if let Some(output) = output { let _ = output.send(bytes); }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    crate::service_log!("WARN", "Signal price router lagged; skipped {skipped} ticker updates");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    let mut ticker_batch_input = price_updates.subscribe();
    let ticker_batch_output = ticker_batches.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICKER_BATCH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut batch = Vec::with_capacity(TICKER_BATCH_CAPACITY);
        loop {
            tokio::select! {
                received = ticker_batch_input.recv() => match received {
                    Ok(message) => {
                        batch.extend_from_slice(&message);
                        if batch.len() >= TICKER_BATCH_CAPACITY {
                            let _ = ticker_batch_output.send(Bytes::from(std::mem::replace(
                                &mut batch,
                                Vec::with_capacity(TICKER_BATCH_CAPACITY),
                            )));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = interval.tick() => {
                    if !batch.is_empty() {
                        let _ = ticker_batch_output.send(Bytes::from(std::mem::replace(
                            &mut batch,
                            Vec::with_capacity(TICKER_BATCH_CAPACITY),
                        )));
                    }
                }
            }
        }
    });
    for instrument in instrument_data.iter().take(SIGNAL_INSTRUMENT_LIMIT) {
        start_signal_feed(instrument, &signal_feeds, &signal_price_feeds, &signal_events);
    }
    let port = env::var("PORT").unwrap_or_else(|_| "10000".to_string());
    let addr = format!("0.0.0.0:{port}");

    crate::service_log!("INFO", "RSocket WebSocket server on ws://{addr}");

    let r_socket = RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(addr))
        .acceptor(Box::new(move |setup, _socket| {
            crate::service_log!("INFO", "socket establish: setup={:?}", setup);
            Ok(Box::new(ServerResponder {
                price_updates: ticker_batches.clone(),
                instrument_lookup: Arc::clone(&instrument_lookup),
                signal_feeds: Arc::clone(&signal_feeds),
                signal_price_feeds: Arc::clone(&signal_price_feeds),
                signal_events: signal_events.clone(),
                usd_rate_snapshot: usd_rate_snapshot.clone(),
                news_snapshot: news_snapshot.clone(),
            }))
        }))
        .serve();

    tokio::try_join!(sub, r_socket)?;
    Ok(())
}

