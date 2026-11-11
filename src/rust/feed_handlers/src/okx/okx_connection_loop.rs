fn ticker_subscription_requests(instrument_data: &[Arc<str>]) -> Vec<Utf8Bytes> {
    instrument_data
        .chunks(240)
        .enumerate()
        .map(|(batch_index, instruments)| {
            let args = instruments
                .iter()
                .map(|instrument| json!({ "channel": "tickers", "instId": instrument.as_ref() }))
                .collect::<Vec<_>>();
            Utf8Bytes::from(
                json!({
                    "op": "subscribe",
                    "id": (batch_index + 1).to_string(),
                    "args": args
                })
                .to_string(),
            )
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum TickerHeartbeatAction {
    Wait,
    Ping,
    Timeout,
}

fn ticker_heartbeat_action(now: u64, last_message: u64, ping_started: u64) -> TickerHeartbeatAction {
    if ping_started != 0 && now.saturating_sub(ping_started) >= OKX_PONG_TIMEOUT.as_millis() as u64 {
        TickerHeartbeatAction::Timeout
    } else if ping_started == 0
        && now.saturating_sub(last_message) >= OKX_IDLE_HEARTBEAT_INTERVAL.as_millis() as u64
    {
        TickerHeartbeatAction::Ping
    } else {
        TickerHeartbeatAction::Wait
    }
}

fn encode_ticker_updates(
    text: &str,
    instrument_lookup: &HashSet<Arc<str>>,
) -> Result<Vec<Bytes>> {
    let update = match serde_json::from_str::<TickerMessage<'_>>(text) {
        Ok(update) => update,
        Err(error) => {
            crate::service_log!("ERROR", "Ignoring invalid OKX JSON message: {error}");
            return Ok(Vec::new());
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
        return Ok(Vec::new());
    }

    let mut encoded_updates = Vec::new();
    for item in update.data.unwrap_or_default() {
        let Some(instrument) = instrument_lookup.get(item.instrument) else {
            continue;
        };
        let Ok(price) = item.last.parse::<f64>() else {
            continue;
        };
        if !price.is_finite() || price <= 0.0 {
            continue;
        }
        crate::metrics::record_ticker();
        let timestamp_ms = item
            .ts
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or_else(now_ms);
        encoded_updates.push(encode_symbol_price(instrument, price, timestamp_ms)?);
    }
    Ok(encoded_updates)
}

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
    let last_message_at = Arc::new(AtomicU64::new(now_ms()));
    let ping_sent_at = Arc::new(AtomicU64::new(0));
    let monitor_last_message_at = Arc::clone(&last_message_at);
    let monitor_ping_sent_at = Arc::clone(&ping_sent_at);
    let ws = {
        let read = read.map(|message| message.map_err(anyhow::Error::from));
        read.try_for_each(move |message| {
            let instrument_lookup = instrument_lookup.clone();
            let price_updates = price_updates.clone();
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
                for encoded in encode_ticker_updates(text, &instrument_lookup)? {
                    let _ = price_updates.send(encoded);
                    crate::metrics::record_queue_depth("price_updates", price_updates.len());
                }
                Ok(())
            }
        })
    };

    for request in ticker_subscription_requests(instrument_data) {
        crate::service_log!("INFO", "Subscribing to {}", request);
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
                match ticker_heartbeat_action(
                    now,
                    monitor_last_message_at.load(Ordering::Relaxed),
                    ping_started,
                ) {
                    TickerHeartbeatAction::Timeout => anyhow::bail!("OKX ticker WebSocket did not answer ping within {}s", OKX_PONG_TIMEOUT.as_secs()),
                    TickerHeartbeatAction::Ping => {
                        monitor_ping_sent_at.store(now, Ordering::Relaxed);
                        write.send(tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from("ping"))).await?;
                    }
                    TickerHeartbeatAction::Wait => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod connection_loop_tests {
    use super::{
        TickerHeartbeatAction, encode_ticker_updates, ticker_heartbeat_action,
        ticker_subscription_requests,
    };
    use serde_json::Value;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[test]
    fn builds_a_subscription_request_for_each_batch_of_240_instruments() {
        let instruments = (0..241)
            .map(|index| Arc::<str>::from(format!("COIN{index}-USDT")))
            .collect::<Vec<_>>();

        let requests = ticker_subscription_requests(&instruments);

        assert_eq!(requests.len(), 2);
        let first: Value = serde_json::from_str(requests[0].as_str()).unwrap();
        let second: Value = serde_json::from_str(requests[1].as_str()).unwrap();
        assert_eq!(first["op"], "subscribe");
        assert_eq!(first["id"], "1");
        assert_eq!(first["args"].as_array().unwrap().len(), 240);
        assert_eq!(first["args"][0]["instId"], "COIN0-USDT");
        assert_eq!(first["args"][239]["instId"], "COIN239-USDT");
        assert_eq!(second["id"], "2");
        assert_eq!(second["args"].as_array().unwrap().len(), 1);
        assert_eq!(second["args"][0]["instId"], "COIN240-USDT");
    }

    #[test]
    fn does_not_emit_empty_subscription_batches() {
        assert!(ticker_subscription_requests(&[]).is_empty());
    }

    #[test]
    fn encodes_only_valid_tickers_for_subscribed_instruments() {
        let lookup = HashSet::from([Arc::<str>::from("BTC-USDT")]);
        let text = r#"{"data":[
            {"instId":"BTC-USDT","last":"65000.5","ts":"1700000000000"},
            {"instId":"ETH-USDT","last":"3000","ts":"1700000000000"},
            {"instId":"BTC-USDT","last":"NaN","ts":"1700000000001"}
        ]}"#;

        let encoded = encode_ticker_updates(text, &lookup).unwrap();

        assert_eq!(encoded.len(), 1);
        assert_eq!(
            f64::from_le_bytes(encoded[0][40..48].try_into().unwrap()),
            65000.5
        );
        assert!(encode_ticker_updates("not-json", &lookup)
            .unwrap()
            .is_empty());
        assert!(encode_ticker_updates(
            r#"{"event":"error","msg":"bad subscription"}"#,
            &lookup,
        )
        .is_err());
        assert!(encode_ticker_updates(
            r#"{"event":"subscribe"}"#,
            &lookup,
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn selects_wait_ping_and_timeout_heartbeat_actions() {
        assert_eq!(ticker_heartbeat_action(10_000, 9_000, 0), TickerHeartbeatAction::Wait);
        assert_eq!(ticker_heartbeat_action(30_000, 9_000, 0), TickerHeartbeatAction::Ping);
        assert_eq!(ticker_heartbeat_action(21_000, 0, 10_000), TickerHeartbeatAction::Timeout);
        assert_eq!(ticker_heartbeat_action(10_000, 0, 9_000), TickerHeartbeatAction::Wait);
    }
}
