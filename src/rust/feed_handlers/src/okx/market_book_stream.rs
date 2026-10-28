pub async fn subscribe(instrument_data: &[Arc<str>], price_updates: Sender<Bytes>) -> Result<()> {
    let instrument_lookup = Arc::new(instrument_data.iter().cloned().collect::<HashSet<_>>());
    let mut retry_delay = INITIAL_RECONNECT_DELAY;

    loop {
        let attempt_started = Instant::now();
        let result = subscribe_once(
            instrument_data,
            Arc::clone(&instrument_lookup),
            price_updates.clone(),
        )
        .await;

        retry_delay = retry_delay_after_failure(retry_delay, attempt_started.elapsed());

        match result {
            Ok(()) => crate::service_log!("ERROR", 
                "OKX WebSocket stream ended; reconnecting in {}s",
                retry_delay.as_secs()
            ),
            Err(error) => crate::service_log!("ERROR", 
                "OKX WebSocket subscription failed: {error:#}; reconnecting in {}s",
                retry_delay.as_secs()
            ),
        }

        sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

pub async fn subscribe_book_once(
    instrument: &str,
    updates: mpsc::Sender<Result<Bytes>>,
) -> Result<()> {
    let (incoming_ws, _) = connect_async("wss://ws.okx.com:8443/ws/v5/public")
        .await
        .map_err(|error| anyhow::anyhow!("failed to connect to OKX book WebSocket: {error}"))?;
    let (mut write, mut read) = incoming_ws.split();
    let request = json!({
        "op": "subscribe",
        "args": [{ "channel": "books", "instId": instrument }]
    });
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            Utf8Bytes::from(request.to_string()),
        ))
        .await?;
    crate::service_log!("INFO", "OKX books WebSocket connected; subscribe request sent for {instrument}");
    let heartbeat = tokio::time::sleep(OKX_IDLE_HEARTBEAT_INTERVAL);
    let pong_timeout = tokio::time::sleep(Duration::from_secs(60 * 60 * 24 * 365));
    tokio::pin!(heartbeat);
    tokio::pin!(pong_timeout);
    let mut awaiting_pong = false;

    let mut bid_book = Vec::new();
    let mut ask_book = Vec::new();
    loop {
        tokio::select! {
            _ = &mut pong_timeout, if awaiting_pong => anyhow::bail!("OKX books WebSocket did not answer ping within {}s", OKX_PONG_TIMEOUT.as_secs()),
            _ = &mut heartbeat => {
                write.send(tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from("ping"))).await?;
                awaiting_pong = true;
                pong_timeout.as_mut().reset(tokio::time::Instant::now() + OKX_PONG_TIMEOUT);
                heartbeat.as_mut().reset(tokio::time::Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
            }
            _ = updates.closed() => {
                crate::service_log!("INFO", "UI unsubscribed from OKX books for {instrument}");
                let request = json!({
                    "op": "unsubscribe",
                    "args": [{ "channel": "books", "instId": instrument }]
                });
                write
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        Utf8Bytes::from(request.to_string()),
                    ))
                    .await
                    .map_err(|error| anyhow::anyhow!("failed to send OKX books unsubscribe for {instrument}: {error}"))?;
                crate::service_log!("INFO", "Sent OKX books unsubscribe for {instrument}");
                return Ok(());
            },
            message = read.next() => {
                let Some(message) = message else {
                    anyhow::bail!("OKX book WebSocket stream ended");
                };
                let message = message?;
                heartbeat.as_mut().reset(tokio::time::Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
                let text = match message.to_text() {
                    Ok(text) => text,
                    Err(error) => {
                        crate::service_log!("ERROR", "Ignoring non-text OKX book message: {error}");
                        continue;
                    }
                };
                if text == "pong" {
                    awaiting_pong = false;
                    pong_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(60 * 60 * 24 * 365));
                    continue;
                }
                let update = match serde_json::from_str::<BookMessage<'_>>(text) {
                    Ok(update) => update,
                    Err(error) => {
                        crate::service_log!("ERROR", "Ignoring invalid OKX book JSON message: {error}");
                        continue;
                    }
                };

                if update.event == Some("error") {
                    anyhow::bail!(
                        "OKX book subscription error: {}",
                        update.msg.unwrap_or("unknown subscription error")
                    );
                }
                if update.event == Some("subscribe") {
                    crate::service_log!("INFO", "OKX books subscription acknowledged for {instrument}");
                    continue;
                }

                let Some(instrument_arg) = update.arg else {
                    continue;
                };
                if instrument_arg.instrument != instrument {
                    continue;
                }
                let Some(items) = update.data else {
                    continue;
                };
                for item in items {
                    if update.action == Some("snapshot") {
                        bid_book.clear();
                        ask_book.clear();
                    }
                    apply_book_side(&mut bid_book, &item.bids)?;
                    apply_book_side(&mut ask_book, &item.asks)?;
                    let bids = top_book_side(&bid_book, true);
                    let asks = top_book_side(&ask_book, false);
                    let timestamp = item
                        .ts
                        .parse::<u64>()
                        .map_err(|error| anyhow::anyhow!("invalid OKX book timestamp: {error}"))?;
                    let encoded = encode_book_depth(instrument, timestamp, &bids, &asks)?;
                    updates
                        .send(Ok(encoded))
                        .await
                        .map_err(|_| anyhow::anyhow!("book stream receiver was dropped"))?;
                }
            }
        }
    }
}

