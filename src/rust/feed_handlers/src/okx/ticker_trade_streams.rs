pub async fn subscribe_ticker_samples(
    instruments: &[String],
    updates: mpsc::Sender<TickerSample>,
) -> Result<()> {
    let (incoming_ws, _) = connect_async("wss://ws.okx.com:8443/ws/v5/public")
        .await
        .map_err(|error| {
            anyhow::anyhow!("failed to connect to OKX hedge ticker stream: {error}")
        })?;
    let (mut write, mut read) = incoming_ws.split();
    for (batch_index, batch) in instruments.chunks(240).enumerate() {
        let args = batch
            .iter()
            .map(|instrument| json!({ "channel": "tickers", "instId": instrument }))
            .collect::<Vec<_>>();
        let request = json!({
            "op": "subscribe",
            "id": (batch_index + 1).to_string(),
            "args": args,
        });
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                Utf8Bytes::from(request.to_string()),
            ))
            .await?;
        if batch_index + 1 < instruments.chunks(240).len() {
            sleep(Duration::from_millis(500)).await;
        }
    }
    loop {
        tokio::select! {
            _ = updates.closed() => return Ok(()),
            message = read.next() => {
                let Some(message) = message else { anyhow::bail!("OKX hedge ticker stream ended") };
                let message = message?;
                if !message.is_text() { continue; }
                let update = match serde_json::from_str::<TickerMessage<'_>>(message.to_text()?) {
                    Ok(update) => update,
                    Err(error) => { crate::service_log!("ERROR", "Ignoring invalid OKX hedge ticker JSON: {error}"); continue; }
                };
                if update.event == Some("error") {
                    anyhow::bail!("OKX hedge ticker subscription error: {}", update.msg.unwrap_or("unknown error"));
                }
                for ticker in update.data.unwrap_or_default() {
                    let (Some(timestamp), Ok(price)) = (
                        ticker.ts.and_then(|value| value.parse::<u64>().ok()),
                        ticker.last.parse::<f64>(),
                    ) else { continue; };
                    if !price.is_finite() || price <= 0.0 { continue; }
                    updates.send(TickerSample {
                        instrument: ticker.instrument.to_owned(),
                        price,
                        timestamp,
                    }).await.map_err(|_| anyhow::anyhow!("hedge ticker sample receiver was dropped"))?;
                }
            }
        }
    }
}

pub async fn subscribe_trades_once(
    instrument: &str,
    updates: mpsc::Sender<Result<Bytes>>,
) -> Result<()> {
    let (incoming_ws, _) = connect_async("wss://ws.okx.com:8443/ws/v5/public")
        .await
        .map_err(|error| anyhow::anyhow!("failed to connect to OKX trades WebSocket: {error}"))?;
    let (mut write, mut read) = incoming_ws.split();
    let request =
        json!({"op": "subscribe", "args": [{ "channel": "trades", "instId": instrument }]});
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            Utf8Bytes::from(request.to_string()),
        ))
        .await?;
    crate::service_log!("INFO", "OKX trades WebSocket connected; subscribe request sent for {instrument}");
    let heartbeat = tokio::time::sleep(OKX_IDLE_HEARTBEAT_INTERVAL);
    let pong_timeout = tokio::time::sleep(Duration::from_secs(60 * 60 * 24 * 365));
    tokio::pin!(heartbeat);
    tokio::pin!(pong_timeout);
    let mut awaiting_pong = false;
    loop {
        tokio::select! {
            _ = &mut pong_timeout, if awaiting_pong => anyhow::bail!("OKX trades WebSocket did not answer ping within {}s", OKX_PONG_TIMEOUT.as_secs()),
            _ = &mut heartbeat => {
                write.send(tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from("ping"))).await?;
                awaiting_pong = true;
                pong_timeout.as_mut().reset(tokio::time::Instant::now() + OKX_PONG_TIMEOUT);
                heartbeat.as_mut().reset(tokio::time::Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
            }
            _ = updates.closed() => {
                let request = json!({"op": "unsubscribe", "args": [{ "channel": "trades", "instId": instrument }]});
                let _ = write.send(tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from(request.to_string()))).await;
                return Ok(());
            }
            message = read.next() => {
                let Some(message) = message else { anyhow::bail!("OKX trades WebSocket stream ended") };
                let message = message?;
                heartbeat.as_mut().reset(tokio::time::Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
                if !message.is_text() { continue; }
                let text = message.to_text()?;
                if text == "pong" {
                    awaiting_pong = false;
                    pong_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(60 * 60 * 24 * 365));
                    continue;
                }
                let update: TradeMessage<'_> = match serde_json::from_str(text) { Ok(update) => update, Err(error) => { crate::service_log!("ERROR", "Ignoring invalid OKX trades JSON message: {error}"); continue; } };
                if update.event == Some("error") { anyhow::bail!("OKX trades subscription error: {}", update.msg.unwrap_or("unknown error")); }
                if update.event == Some("subscribe") { crate::service_log!("INFO", "OKX trades subscription acknowledged for {instrument}"); continue; }
                let Some(trades) = update.data else { continue; };
                for trade in trades {
                    if trade.instrument != instrument {
                        continue;
                    }
                    let (Ok(price), Ok(size), Ok(timestamp)) = (
                        trade.px.parse::<f64>(),
                        trade.sz.parse::<f64>(),
                        trade.ts.parse::<u64>(),
                    ) else {
                        continue;
                    };
                    let encoded = encode_trade_tick(
                        instrument,
                        trade.trade_id,
                        timestamp,
                        price,
                        size,
                        trade.side,
                    )?;
                    updates
                        .send(Ok(encoded))
                        .await
                        .map_err(|_| anyhow::anyhow!("trades stream receiver was dropped"))?;
                }
            }
        }
    }
}

