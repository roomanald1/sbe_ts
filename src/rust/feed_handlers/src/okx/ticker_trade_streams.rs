pub async fn subscribe_ticker_samples(
    instruments: &[String],
    updates: mpsc::Sender<TickerSample>,
) -> Result<()> {
    wait_for_okx_connection_slot().await;
    let (incoming_ws, _) = connect_async(OKX_PUBLIC_WS_URL)
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
    let heartbeat = tokio::time::sleep(OKX_IDLE_HEARTBEAT_INTERVAL);
    let pong_timeout = tokio::time::sleep(Duration::from_secs(60 * 60 * 24 * 365));
    tokio::pin!(heartbeat);
    tokio::pin!(pong_timeout);
    let mut awaiting_pong = false;
    loop {
        tokio::select! {
            _ = &mut pong_timeout, if awaiting_pong => anyhow::bail!("OKX hedge ticker WebSocket did not answer ping within {}s", OKX_PONG_TIMEOUT.as_secs()),
            _ = &mut heartbeat => {
                write.send(tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from("ping"))).await?;
                awaiting_pong = true;
                pong_timeout.as_mut().reset(tokio::time::Instant::now() + OKX_PONG_TIMEOUT);
                heartbeat.as_mut().reset(tokio::time::Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
            }
            _ = updates.closed() => return Ok(()),
            message = read.next() => {
                let Some(message) = message else { anyhow::bail!("OKX hedge ticker stream ended") };
                let message = message?;
                heartbeat.as_mut().reset(tokio::time::Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
                if !message.is_text() { continue; }
                let text = message.to_text()?;
                if text == "pong" {
                    awaiting_pong = false;
                    pong_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(60 * 60 * 24 * 365));
                    continue;
                }
                let update = match serde_json::from_str::<TickerMessage<'_>>(text) {
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
