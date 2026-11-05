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
    let requests = ticker_sample_subscription_requests(instruments);
    for (index, request) in requests.into_iter().enumerate() {
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(request))
            .await?;
        if index + 1 < instruments.len().div_ceil(240) {
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
                let Some(samples) = parse_ticker_samples(text)? else { continue; };
                for sample in samples {
                    updates.send(sample).await.map_err(|_| anyhow::anyhow!("hedge ticker sample receiver was dropped"))?;
                }
            }
        }
    }
}

fn ticker_sample_subscription_requests(instruments: &[String]) -> Vec<Utf8Bytes> {
    instruments
        .chunks(240)
        .enumerate()
        .map(|(batch_index, batch)| {
            let args = batch
                .iter()
                .map(|instrument| json!({ "channel": "tickers", "instId": instrument }))
                .collect::<Vec<_>>();
            Utf8Bytes::from(
                json!({
                    "op": "subscribe",
                    "id": (batch_index + 1).to_string(),
                    "args": args,
                })
                .to_string(),
            )
        })
        .collect()
}

fn parse_ticker_samples(text: &str) -> Result<Option<Vec<TickerSample>>> {
    let update = match serde_json::from_str::<TickerMessage<'_>>(text) {
        Ok(update) => update,
        Err(error) => {
            crate::service_log!("ERROR", "Ignoring invalid OKX hedge ticker JSON: {error}");
            return Ok(None);
        }
    };
    if update.event == Some("error") {
        anyhow::bail!(
            "OKX hedge ticker subscription error: {}",
            update.msg.unwrap_or("unknown error")
        );
    }
    Ok(Some(
        update
            .data
            .unwrap_or_default()
            .into_iter()
            .filter_map(ticker_sample)
            .collect(),
    ))
}

fn ticker_sample(ticker: Ticker<'_>) -> Option<TickerSample> {
    let timestamp = ticker.ts?.parse::<u64>().ok()?;
    let price = ticker.last.parse::<f64>().ok()?;
    (price.is_finite() && price > 0.0).then(|| TickerSample {
        instrument: ticker.instrument.to_owned(),
        price,
        timestamp,
    })
}

#[cfg(test)]
mod ticker_trade_stream_tests {
    use super::{
        Ticker, parse_ticker_samples, ticker_sample, ticker_sample_subscription_requests,
    };

    #[test]
    fn converts_only_tickers_with_valid_positive_finite_price_and_timestamp() {
        let parse = |text: &str| {
            let ticker: Ticker<'_> = serde_json::from_str(text).unwrap();
            ticker_sample(ticker)
        };
        let sample = parse(r#"{"instId":"BTC-USDT","last":"65000.5","ts":"1700000000000"}"#).unwrap();
        assert_eq!(sample.instrument, "BTC-USDT");
        assert_eq!(sample.price, 65000.5);
        assert_eq!(sample.timestamp, 1_700_000_000_000);
        for invalid in [
            r#"{"instId":"BTC-USDT","last":"0","ts":"1"}"#,
            r#"{"instId":"BTC-USDT","last":"-1","ts":"1"}"#,
            r#"{"instId":"BTC-USDT","last":"NaN","ts":"1"}"#,
            r#"{"instId":"BTC-USDT","last":"1","ts":"invalid"}"#,
            r#"{"instId":"BTC-USDT","last":"1"}"#,
        ] {
            assert!(parse(invalid).is_none(), "accepted {invalid}");
        }
    }

    #[test]
    fn parses_valid_messages_and_subscription_errors() {
        let samples = parse_ticker_samples(
            r#"{"data":[{"instId":"ETH-USDT","last":"12.5","ts":"42"}]}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].instrument, "ETH-USDT");
        assert_eq!(samples[0].timestamp, 42);
        assert!(parse_ticker_samples("not-json").unwrap().is_none());
        assert!(parse_ticker_samples(r#"{"event":"error","msg":"denied"}"#).is_err());
        assert!(parse_ticker_samples(r#"{"event":"subscribe"}"#)
            .unwrap()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn creates_bounded_subscription_batches_with_incrementing_ids() {
        let instruments = (0..241).map(|index| format!("COIN{index}-USDT")).collect::<Vec<_>>();
        let requests = ticker_sample_subscription_requests(&instruments);
        assert_eq!(requests.len(), 2);
        let first: serde_json::Value = serde_json::from_str(requests[0].as_str()).unwrap();
        let second: serde_json::Value = serde_json::from_str(requests[1].as_str()).unwrap();
        assert_eq!(first["id"], "1");
        assert_eq!(first["args"].as_array().unwrap().len(), 240);
        assert_eq!(second["id"], "2");
        assert_eq!(second["args"][0]["instId"], "COIN240-USDT");
    }
}
