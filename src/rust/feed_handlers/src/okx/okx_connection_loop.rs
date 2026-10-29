async fn subscribe_once(
    instrument_data: &[Arc<str>],
    instrument_lookup: Arc<HashSet<Arc<str>>>,
    price_updates: Sender<Bytes>,
) -> Result<()> {
    wait_for_okx_connection_slot().await;
    let (incoming_ws, _) = connect_async(OKX_PUBLIC_WS_URL)
        .await
        .map_err(|error| anyhow::anyhow!("failed to connect to OKX WebSocket: {error}"))?;

    crate::service_log!("INFO", "OKX WebSocket handshake has been successfully completed");
    let (mut write, read) = incoming_ws.split();
    let volatility_by_instrument = Arc::new({
        let states = DashMap::<Arc<str>, RollingVolatility>::with_capacity(instrument_lookup.len());
        for instrument in instrument_lookup.iter() { states.insert(Arc::clone(instrument), RollingVolatility::default()); }
        states
    });
    let last_message_at = Arc::new(AtomicU64::new(now_ms()));
    let ping_sent_at = Arc::new(AtomicU64::new(0));
    let monitor_last_message_at = Arc::clone(&last_message_at);
    let monitor_ping_sent_at = Arc::clone(&ping_sent_at);
    let ws = {
        let read = read.map(|message| message.map_err(anyhow::Error::from));
        read.try_for_each(move |message| {
            let instrument_lookup = instrument_lookup.clone();
            let price_updates = price_updates.clone();
            let volatility_by_instrument = Arc::clone(&volatility_by_instrument);
            let last_message_at = Arc::clone(&last_message_at);
            let ping_sent_at = Arc::clone(&ping_sent_at);
            async move {
                last_message_at.store(now_ms(), Ordering::Relaxed);
                if message.is_pong() || message.to_text().ok() == Some("pong") {
                    ping_sent_at.store(0, Ordering::Relaxed);
                    return Ok(());
                }
                let text = match message.to_text() {
                    Ok(text) => text,
                    Err(error) => {
                        crate::service_log!("ERROR", "Ignoring non-text OKX WebSocket message: {error}");
                        return Ok(());
                    }
                };
                let update = match serde_json::from_str::<TickerMessage<'_>>(text) {
                    Ok(update) => update,
                    Err(error) => {
                        crate::service_log!("ERROR", "Ignoring invalid OKX JSON message: {error}");
                        return Ok(());
                    }
                };

                if update.event == Some("error") {
                    anyhow::bail!(
                        "OKX subscription error: {}",
                        update.msg.unwrap_or("unknown subscription error")
                    );
                }
                if update.event == Some("subscribe") {
                    crate::service_log!("INFO", "OKX tickers subscription batch acknowledged");
                    return Ok(());
                }

                if let Some(items) = update.data {
                    for item in items {
                        let Some(instrument) = instrument_lookup.get(item.instrument) else {
                            continue;
                        };
                        let Ok(price) = item.last.parse::<f64>() else {
                            continue;
                        };
                        if price.is_finite() && price > 0.0 {
                            let timestamp_ms = item
                                .ts
                                .and_then(|value| value.parse::<u64>().ok())
                                .unwrap_or_else(|| {
                                    SystemTime::now()
                                        .duration_since(UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis() as u64
                                });
                            let Some(mut volatility) = volatility_by_instrument.get_mut(instrument) else { continue; };
                            let (volatility_1m, volatility_5m) = volatility.update(timestamp_ms, price);
                            let encoded = encode_symbol_price(
                                instrument,
                                price,
                                volatility_1m,
                                volatility_5m,
                            )?;
                            let _ = price_updates.send(encoded);
                        }
                    }
                }
                Ok(())
            }
        })
    };

    for (batch_index, instruments) in instrument_data.chunks(240).enumerate() {
        let args = instruments
            .iter()
            .map(|instrument| json!({ "channel": "tickers", "instId": instrument.as_ref() }))
            .collect::<Vec<_>>();
        let request = json!({
            "op": "subscribe",
            "id": (batch_index + 1).to_string(),
            "args": args
        });
        crate::service_log!("INFO", "Subscribing to {}", request);
        let request = Utf8Bytes::from(request.to_string());

        write
            .send(tokio_tungstenite::tungstenite::Message::Text(request))
            .await?;
    }

    let mut ws = Box::pin(ws);
    let mut liveness_check = tokio::time::interval(Duration::from_secs(1));
    liveness_check.tick().await;
    loop {
        tokio::select! {
            result = &mut ws => {
                result?;
                anyhow::bail!("OKX WebSocket stream ended");
            }
            _ = liveness_check.tick() => {
                let now = now_ms();
                let ping_started = monitor_ping_sent_at.load(Ordering::Relaxed);
                if ping_started != 0 && now.saturating_sub(ping_started) >= OKX_PONG_TIMEOUT.as_millis() as u64 {
                    anyhow::bail!("OKX ticker WebSocket did not answer ping within {}s", OKX_PONG_TIMEOUT.as_secs());
                }
                let last_message = monitor_last_message_at.load(Ordering::Relaxed);
                if ping_started == 0 && now.saturating_sub(last_message) >= OKX_IDLE_HEARTBEAT_INTERVAL.as_millis() as u64 {
                    monitor_ping_sent_at.store(now, Ordering::Relaxed);
                    write.send(tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from("ping"))).await?;
                }
            }
        }
    }
}
